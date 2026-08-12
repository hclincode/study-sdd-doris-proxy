//! MySQL packet framing.
//!
//! Every packet on the wire is a 4-byte header (3-byte little-endian payload
//! length, 1-byte sequence id) followed by the payload. A logical message whose
//! payload reaches [`MAX_PAYLOAD`] is split across several packets; every
//! non-final packet carries exactly [`MAX_PAYLOAD`] bytes, so the split is
//! canonical for a given payload length. That property is what lets the proxy
//! re-emit an unmodified message and reproduce the original packet count and
//! sequence ids exactly.

use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum payload carried by a single packet.
pub const MAX_PAYLOAD: usize = 0xFF_FF_FF;

/// Length of the packet header.
pub const HEADER_LEN: usize = 4;

/// Number of packets a message of `payload_len` bytes occupies on the wire.
///
/// A payload that is an exact multiple of [`MAX_PAYLOAD`] needs a trailing
/// empty packet to mark the end, which is why this is not a plain `div_ceil`.
pub fn packet_count(payload_len: usize) -> usize {
    payload_len / MAX_PAYLOAD + 1
}

/// A single packet as read from the wire.
#[derive(Debug, Clone)]
pub struct Packet {
    pub seq: u8,
    pub payload: Bytes,
}

impl Packet {
    /// True when this packet is exactly full, meaning the message continues in
    /// the next packet.
    pub fn continues(&self) -> bool {
        self.payload.len() == MAX_PAYLOAD
    }
}

/// A complete logical message, reassembled from one or more packets.
#[derive(Debug, Clone)]
pub struct Message {
    /// Sequence id of the message's first packet.
    pub first_seq: u8,
    /// How many packets the message occupied on the wire.
    pub packet_count: usize,
    pub payload: Bytes,
}

/// Buffered packet reader.
///
/// [`PacketReader::next_packet`] is cancel-safe: bytes that have been read from
/// the socket are always retained in the internal buffer before the next await
/// point, so dropping the future loses nothing. The connection loop relies on
/// this to watch an idle backend for EOF while waiting on the client.
#[derive(Debug)]
pub struct PacketReader<R> {
    inner: R,
    buf: BytesMut,
}

impl<R: AsyncRead + Unpin> PacketReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: BytesMut::with_capacity(16 * 1024),
        }
    }

    /// Reads the next packet. Returns `Ok(None)` on a clean end of stream.
    pub async fn next_packet(&mut self) -> io::Result<Option<Packet>> {
        loop {
            if self.buf.len() >= HEADER_LEN {
                let len =
                    u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], 0]) as usize;
                if self.buf.len() >= HEADER_LEN + len {
                    let seq = self.buf[3];
                    self.buf.advance(HEADER_LEN);
                    let payload = self.buf.split_to(len).freeze();
                    return Ok(Some(Packet { seq, payload }));
                }
                self.buf.reserve(HEADER_LEN + len - self.buf.len());
            } else {
                self.buf.reserve(HEADER_LEN * 4);
            }

            let n = self.inner.read_buf(&mut self.buf).await?;
            if n == 0 {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed mid-packet",
                    ))
                };
            }
        }
    }

    /// Reads a complete logical message, following continuation packets.
    pub async fn next_message(&mut self) -> io::Result<Option<Message>> {
        let Some(first) = self.next_packet().await? else {
            return Ok(None);
        };
        let first_seq = first.seq;
        let mut packet_count = 1;

        if !first.continues() {
            return Ok(Some(Message {
                first_seq,
                packet_count,
                payload: first.payload,
            }));
        }

        let mut payload = BytesMut::from(&first.payload[..]);
        loop {
            let next = self.next_packet().await?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed mid-message",
                )
            })?;
            packet_count += 1;
            let continues = next.continues();
            payload.extend_from_slice(&next.payload);
            if !continues {
                break;
            }
        }

        Ok(Some(Message {
            first_seq,
            packet_count,
            payload: payload.freeze(),
        }))
    }

    /// Resolves when the peer sends data or closes while the proxy considers
    /// the connection idle. Used to notice a backend hanging up between
    /// commands. Cancel-safe for the same reason as [`Self::next_packet`].
    pub async fn wait_for_activity(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            return Ok(());
        }
        let n = self.inner.read_buf(&mut self.buf).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "peer closed the connection",
            ));
        }
        Ok(())
    }
}

/// Buffered packet writer.
#[derive(Debug)]
pub struct PacketWriter<W> {
    inner: W,
    scratch: BytesMut,
}

impl<W: AsyncWrite + Unpin> PacketWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            scratch: BytesMut::with_capacity(16 * 1024),
        }
    }

    /// Writes one packet verbatim. The payload must not exceed [`MAX_PAYLOAD`].
    pub async fn write_packet(&mut self, seq: u8, payload: &[u8]) -> io::Result<()> {
        debug_assert!(payload.len() <= MAX_PAYLOAD);
        self.scratch.clear();
        self.scratch.reserve(HEADER_LEN + payload.len());
        self.scratch.put_uint_le(payload.len() as u64, 3);
        self.scratch.put_u8(seq);
        self.scratch.put_slice(payload);
        self.inner.write_all(&self.scratch).await?;
        self.inner.flush().await
    }

    /// Writes a logical message, splitting it into packets as required.
    ///
    /// Returns the sequence id the next message should start from.
    pub async fn write_message(&mut self, first_seq: u8, payload: &[u8]) -> io::Result<u8> {
        self.scratch.clear();
        let mut seq = first_seq;
        let mut offset = 0;
        loop {
            let chunk = (payload.len() - offset).min(MAX_PAYLOAD);
            self.scratch.reserve(HEADER_LEN + chunk);
            self.scratch.put_uint_le(chunk as u64, 3);
            self.scratch.put_u8(seq);
            self.scratch.put_slice(&payload[offset..offset + chunk]);
            seq = seq.wrapping_add(1);
            offset += chunk;
            if chunk < MAX_PAYLOAD {
                break;
            }
        }
        self.inner.write_all(&self.scratch).await?;
        self.inner.flush().await?;
        Ok(seq)
    }
}

/// Little-endian reader over a packet payload.
///
/// Every accessor returns `None` rather than panicking on a short buffer, so a
/// malformed packet degrades to a parse failure instead of killing the task.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        if self.remaining() < n {
            return None;
        }
        self.pos += n;
        Some(())
    }

    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }

    pub fn uint_le(&mut self, n: usize) -> Option<u64> {
        if self.remaining() < n {
            return None;
        }
        let mut v: u64 = 0;
        for i in 0..n {
            v |= (self.buf[self.pos + i] as u64) << (8 * i);
        }
        self.pos += n;
        Some(v)
    }

    pub fn u16_le(&mut self) -> Option<u16> {
        self.uint_le(2).map(|v| v as u16)
    }

    pub fn u32_le(&mut self) -> Option<u32> {
        self.uint_le(4).map(|v| v as u32)
    }

    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.remaining() < n {
            return None;
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    /// Length-encoded integer. `None` on a short buffer; `Some(None)` for the
    /// 0xFB NULL marker.
    pub fn lenenc_int(&mut self) -> Option<Option<u64>> {
        let first = self.u8()?;
        match first {
            0xFB => Some(None),
            0xFC => self.uint_le(2).map(Some),
            0xFD => self.uint_le(3).map(Some),
            0xFE => self.uint_le(8).map(Some),
            other => Some(Some(other as u64)),
        }
    }

    pub fn lenenc_bytes(&mut self) -> Option<&'a [u8]> {
        let len = self.lenenc_int()??;
        self.bytes(len as usize)
    }

    /// NUL-terminated string, excluding the terminator.
    pub fn nul_bytes(&mut self) -> Option<&'a [u8]> {
        let start = self.pos;
        let end = self.buf[start..].iter().position(|&b| b == 0)? + start;
        self.pos = end + 1;
        Some(&self.buf[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_count_accounts_for_trailing_empty_packet() {
        assert_eq!(packet_count(0), 1);
        assert_eq!(packet_count(10), 1);
        assert_eq!(packet_count(MAX_PAYLOAD - 1), 1);
        // An exact multiple needs a trailing empty packet to terminate.
        assert_eq!(packet_count(MAX_PAYLOAD), 2);
        assert_eq!(packet_count(MAX_PAYLOAD + 1), 2);
        assert_eq!(packet_count(2 * MAX_PAYLOAD), 3);
    }

    #[tokio::test]
    async fn reads_a_simple_packet() {
        let wire = [0x03, 0x00, 0x00, 0x00, b'a', b'b', b'c'];
        let mut r = PacketReader::new(&wire[..]);
        let p = r.next_packet().await.unwrap().unwrap();
        assert_eq!(p.seq, 0);
        assert_eq!(&p.payload[..], b"abc");
        assert!(r.next_packet().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reads_an_empty_packet() {
        let wire = [0x00, 0x00, 0x00, 0x07];
        let mut r = PacketReader::new(&wire[..]);
        let p = r.next_packet().await.unwrap().unwrap();
        assert_eq!(p.seq, 7);
        assert!(p.payload.is_empty());
    }

    #[tokio::test]
    async fn short_read_mid_packet_is_an_error() {
        let wire = [0x05, 0x00, 0x00, 0x00, b'a'];
        let mut r = PacketReader::new(&wire[..]);
        assert!(r.next_packet().await.is_err());
    }

    #[tokio::test]
    async fn reassembles_a_split_message() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
        wire.extend(std::iter::repeat(b'x').take(MAX_PAYLOAD));
        wire.extend_from_slice(&[0x02, 0x00, 0x00, 0x01, b'y', b'z']);

        let mut r = PacketReader::new(&wire[..]);
        let m = r.next_message().await.unwrap().unwrap();
        assert_eq!(m.first_seq, 0);
        assert_eq!(m.packet_count, 2);
        assert_eq!(m.payload.len(), MAX_PAYLOAD + 2);
        assert_eq!(&m.payload[MAX_PAYLOAD..], b"yz");
    }

    #[tokio::test]
    async fn round_trip_preserves_packet_count_and_sequence() {
        for len in [0usize, 1, 100, MAX_PAYLOAD - 1, MAX_PAYLOAD, MAX_PAYLOAD + 3] {
            let payload = vec![b'q'; len];
            let mut out = Vec::new();
            let next = PacketWriter::new(&mut out)
                .write_message(4, &payload)
                .await
                .unwrap();

            let mut r = PacketReader::new(&out[..]);
            let m = r.next_message().await.unwrap().unwrap();
            assert_eq!(m.first_seq, 4, "len={len}");
            assert_eq!(m.packet_count, packet_count(len), "len={len}");
            assert_eq!(m.payload.len(), len, "len={len}");
            assert_eq!(next, 4u8.wrapping_add(packet_count(len) as u8), "len={len}");
        }
    }

    #[test]
    fn cursor_reads_length_encoded_values() {
        let buf = [0xFC, 0x01, 0x01, 0x03, b'a', b'b', b'c', b'h', b'i', 0x00];
        let mut c = Cursor::new(&buf);
        assert_eq!(c.lenenc_int().unwrap(), Some(0x0101));
        assert_eq!(c.lenenc_bytes().unwrap(), b"abc");
        assert_eq!(c.nul_bytes().unwrap(), b"hi");
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn cursor_reports_null_and_short_buffers() {
        let mut c = Cursor::new(&[0xFB]);
        assert_eq!(c.lenenc_int().unwrap(), None);

        let mut c = Cursor::new(&[0xFC, 0x01]);
        assert!(c.lenenc_int().is_none());

        let mut c = Cursor::new(b"ab");
        assert!(c.nul_bytes().is_none());
    }
}
