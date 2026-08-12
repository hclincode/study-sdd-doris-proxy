//! The log writer task and the handle connection tasks use to reach it.
//!
//! The path from a connection to the file is bounded and lossy on purpose: a
//! slow or full disk must degrade observability, never stall a query. Emitting
//! a record is a non-blocking send that increments a counter on failure, and
//! the counter is itself written to the file so a reader can see the stream is
//! incomplete.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use super::record::{CommandRecord, DroppedRecord, Record};
use crate::timestamp::Timestamp;

/// Handle held by connection tasks.
#[derive(Debug, Clone)]
pub struct LogHandle {
    tx: mpsc::Sender<CommandRecord>,
    dropped: Arc<AtomicU64>,
}

impl LogHandle {
    /// Offers a record to the writer. Never awaits, never fails the caller: if
    /// the queue is full the record is discarded and counted.
    pub fn emit(&self, record: CommandRecord) {
        if self.tx.try_send(record).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Opens the log file, creating it with owner-only permissions.
///
/// Permissions are applied at creation; an existing file keeps whatever mode it
/// already has, which is the operator's business.
async fn open_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    options.open(path).await
}

/// Starts the writer for one listener.
///
/// The file is opened before the task is spawned so that an unusable log path
/// fails startup rather than surfacing later as silent data loss.
pub async fn spawn(
    listener: String,
    path: PathBuf,
    capacity: usize,
    reopen: Arc<Notify>,
) -> io::Result<(LogHandle, JoinHandle<()>)> {
    let file = open_log(&path).await?;
    let (tx, rx) = mpsc::channel::<CommandRecord>(capacity);
    let dropped = Arc::new(AtomicU64::new(0));

    let task = Writer {
        listener,
        path,
        file: Some(file),
        rx,
        dropped: Arc::clone(&dropped),
        reported_drops: 0,
        buf: Vec::with_capacity(64 * 1024),
    };

    let handle = tokio::spawn(task.run(reopen));
    Ok((LogHandle { tx, dropped }, handle))
}

struct Writer {
    listener: String,
    path: PathBuf,
    /// `None` while the file is unusable; records are discarded and counted
    /// until it can be reopened.
    file: Option<File>,
    rx: mpsc::Receiver<CommandRecord>,
    dropped: Arc<AtomicU64>,
    reported_drops: u64,
    buf: Vec<u8>,
}

impl Writer {
    async fn run(mut self, reopen: Arc<Notify>) {
        loop {
            tokio::select! {
                biased;
                _ = reopen.notified() => {
                    self.reopen().await;
                }
                msg = self.rx.recv() => {
                    let Some(first) = msg else { break };
                    self.buf.clear();
                    self.stage_drop_report();
                    self.stage(Record::Command(Box::new(first)));
                    // Drain whatever else is queued so a busy proxy writes in
                    // batches rather than one syscall per statement.
                    while let Ok(next) = self.rx.try_recv() {
                        self.stage(Record::Command(Box::new(next)));
                    }
                    self.flush().await;
                }
            }
        }

        // Clean shutdown: everything already accepted into the channel is
        // written before the task exits.
        self.buf.clear();
        while let Ok(next) = self.rx.try_recv() {
            self.stage(Record::Command(Box::new(next)));
        }
        self.stage_drop_report();
        self.flush().await;
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush().await;
        }
    }

    /// Appends a record to the pending batch. Serialization failure is treated
    /// as a discard rather than as a reason to stop logging.
    fn stage(&mut self, record: Record) {
        match serde_json::to_vec(&record) {
            Ok(mut line) => {
                line.push(b'\n');
                self.buf.extend_from_slice(&line);
            }
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Emits a record describing discards, if any have happened since the last
    /// time we said so.
    fn stage_drop_report(&mut self) {
        let total = self.dropped.load(Ordering::Relaxed);
        if total <= self.reported_drops {
            return;
        }
        let since = total - self.reported_drops;
        self.reported_drops = total;
        let ts = Timestamp::now();
        let record = Record::Dropped(DroppedRecord {
            ts: ts.to_rfc3339(),
            ts_unix_ms: ts.unix_ms,
            listener: self.listener.clone(),
            dropped_total: total,
            dropped_since_last: since,
        });
        self.stage(record);
    }

    /// Writes the pending batch. A failure discards the batch and counts it;
    /// the next batch retries, so a transient condition such as a full disk
    /// recovers on its own.
    async fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }

        if self.file.is_none() {
            self.reopen().await;
        }

        let lines = self.buf.iter().filter(|&&b| b == b'\n').count() as u64;
        let failed = match self.file.as_mut() {
            None => true,
            Some(file) => file.write_all(&self.buf).await.is_err(),
        };

        if failed {
            self.dropped.fetch_add(lines, Ordering::Relaxed);
            // Drop the handle so the next batch attempts a fresh open.
            self.file = None;
        }
        self.buf.clear();
    }

    /// Reopens the configured path, which is how external rotation is
    /// supported: the tool renames the file and signals, and writing resumes
    /// against a newly created one.
    async fn reopen(&mut self) {
        match open_log(&self.path).await {
            Ok(file) => self.file = Some(file),
            Err(_) => self.file = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::record::CommandRecord;

    fn record(n: u64) -> CommandRecord {
        let ts = Timestamp { unix_ms: 0 };
        CommandRecord {
            ts: ts.to_rfc3339(),
            ts_unix_ms: ts.unix_ms,
            listener: "t".into(),
            connection_id: n,
            backend_connection_id: 1,
            client_addr: "127.0.0.1:1".into(),
            username: "u".into(),
            database: None,
            command: "COM_QUERY",
            statement: Some(format!("SELECT {n}")),
            digest: Some("SELECT ?".into()),
            digest_hash: Some("0".into()),
            digest_unavailable: false,
            duration_us: 1,
            outcome: "ok".into(),
            affected_rows: None,
            returned_rows: None,
            result_sets: None,
            error_code: None,
            sql_state: None,
            error_message: None,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        // Counter rather than clock: parallel tests must not collide on a path,
        // and the clock is not guaranteed to be fine grained enough for that.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "mysql-proxy-test-{}-{}-{name}.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    #[tokio::test]
    async fn writes_json_lines_and_flushes_on_shutdown() {
        let path = temp_path("basic");
        let notify = Arc::new(Notify::new());
        let (handle, task) = spawn("t".into(), path.clone(), 64, notify).await.unwrap();

        for i in 0..5 {
            handle.emit(record(i));
        }
        drop(handle);
        task.await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["type"], "command");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn appends_to_an_existing_file() {
        let path = temp_path("append");
        std::fs::write(&path, "{\"pre\":true}\n").unwrap();

        let notify = Arc::new(Notify::new());
        let (handle, task) = spawn("t".into(), path.clone(), 8, notify).await.unwrap();
        handle.emit(record(1));
        drop(handle);
        task.await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\"pre\":true}"));
        assert_eq!(text.lines().count(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_the_file_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("perms");
        let notify = Arc::new(Notify::new());
        let (handle, task) = spawn("t".into(), path.clone(), 8, notify).await.unwrap();
        handle.emit(record(1));
        drop(handle);
        task.await.unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn unopenable_path_fails_before_the_task_starts() {
        let notify = Arc::new(Notify::new());
        let result = spawn(
            "t".into(),
            PathBuf::from("/nonexistent-directory-xyz/log.jsonl"),
            8,
            notify,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn full_queue_discards_and_reports() {
        let path = temp_path("drops");
        let notify = Arc::new(Notify::new());
        // Capacity 1 with no writer progress yet guarantees overflow.
        let (handle, task) = spawn("t".into(), path.clone(), 1, notify).await.unwrap();

        for i in 0..200 {
            handle.emit(record(i));
        }
        let dropped = handle.dropped_count();
        assert!(dropped > 0, "expected some records to be discarded");

        drop(handle);
        task.await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let reported: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .filter(|v: &serde_json::Value| v["type"] == "dropped")
            .collect();
        assert!(!reported.is_empty(), "discards must be reported in the file");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn reopen_resumes_writing_after_rotation() {
        let path = temp_path("rotate");
        let notify = Arc::new(Notify::new());
        let (handle, task) = spawn("t".into(), path.clone(), 64, Arc::clone(&notify))
            .await
            .unwrap();

        handle.emit(record(1));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let rotated = path.with_extension("1");
        std::fs::rename(&path, &rotated).unwrap();
        notify.notify_waiters();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        handle.emit(record(2));
        drop(handle);
        task.await.unwrap();

        let old = std::fs::read_to_string(&rotated).unwrap();
        let new = std::fs::read_to_string(&path).unwrap();
        assert!(old.contains("SELECT 1"));
        assert!(new.contains("SELECT 2"), "writing must resume at the same path");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&rotated);
    }
}
