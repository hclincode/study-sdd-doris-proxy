//! The request-side stage pipeline.
//!
//! Every client command passes through an ordered list of stages before it is
//! written to the backend. A stage either leaves the command alone or replaces
//! its payload, and [`Pipeline::run`] returns a [`Cow`] so the common case
//! forwards the original bytes without copying them.
//!
//! Phase one registers a single observe-only stage. The signature is what
//! matters: a stage that needs to modify a command already has somewhere to
//! return the new bytes, so adding one is a new stage rather than a change to
//! this machinery.

use std::borrow::Cow;

use crate::protocol::command::Command;
use crate::sql::analyze::SkipReason;
use crate::sql::digest::{digest, Digest};

/// What the row filter did with a command.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum FilterOutcome {
    /// No rule was involved: the command carries no SQL, reads no table, or
    /// reads a table nobody configured a filter for. Deliberately distinct from
    /// a skip, so that skip counts measure filters that were wanted and did not
    /// happen rather than traffic nobody meant to filter.
    #[default]
    NotApplicable,
    Rewritten {
        table: String,
        forwarded: String,
    },
    Skipped(SkipReason),
}

/// Per-command scratch space shared by stages and read by the logger once the
/// response completes.
#[derive(Debug, Default)]
pub struct StageContext {
    pub digest: Option<Digest>,
    /// Set when the statement carried SQL that could not be normalized.
    pub digest_unavailable: bool,
    pub filter: FilterOutcome,
}

/// What a stage decided to do with the command.
#[derive(Debug)]
pub enum StageOutput {
    /// Forward the bytes unchanged.
    Unchanged,
    /// Forward these bytes instead.
    Replaced(Vec<u8>),
}

pub trait Stage: std::fmt::Debug + Send + Sync {
    /// `payload` is the command as the previous stages left it, which is not
    /// necessarily `cmd.payload`.
    fn apply(&self, cmd: &Command, payload: &[u8], ctx: &mut StageContext) -> StageOutput;
}

#[derive(Debug)]
pub struct Pipeline {
    stages: Vec<Box<dyn Stage>>,
}

impl Pipeline {
    pub fn new(stages: Vec<Box<dyn Stage>>) -> Self {
        Self { stages }
    }

    /// The phase-one pipeline: observe, change nothing.
    pub fn observe_only() -> Self {
        Self::new(vec![Box::new(ObserveStage)])
    }

    pub fn run<'a>(&self, cmd: &'a Command, ctx: &mut StageContext) -> Cow<'a, [u8]> {
        let mut current: Cow<'a, [u8]> = Cow::Borrowed(&cmd.payload);
        for stage in &self.stages {
            if let StageOutput::Replaced(bytes) = stage.apply(cmd, &current, ctx) {
                current = Cow::Owned(bytes);
            }
        }
        current
    }
}

/// Computes the digest of any statement the command carries. Returns the
/// command untouched.
#[derive(Debug)]
pub struct ObserveStage;

impl Stage for ObserveStage {
    fn apply(&self, cmd: &Command, _payload: &[u8], ctx: &mut StageContext) -> StageOutput {
        if let Some(sql) = cmd.statement() {
            match digest(sql) {
                Some(d) => ctx.digest = Some(d),
                None => ctx.digest_unavailable = true,
            }
        }
        StageOutput::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn command(bytes: &[u8]) -> Command {
        Command::new(Bytes::copy_from_slice(bytes), 0, 1)
    }

    #[derive(Debug)]
    struct AppendStage(&'static [u8]);

    impl Stage for AppendStage {
        fn apply(&self, _cmd: &Command, payload: &[u8], _ctx: &mut StageContext) -> StageOutput {
            let mut out = payload.to_vec();
            out.extend_from_slice(self.0);
            StageOutput::Replaced(out)
        }
    }

    #[test]
    fn unchanged_stages_forward_the_original_bytes_without_copying() {
        let cmd = command(b"\x03SELECT 1");
        let mut ctx = StageContext::default();
        let out = Pipeline::observe_only().run(&cmd, &mut ctx);
        assert!(matches!(out, Cow::Borrowed(_)), "pass-through must not allocate");
        assert_eq!(&out[..], &cmd.payload[..]);
    }

    #[test]
    fn observe_stage_records_a_digest() {
        let cmd = command(b"\x03SELECT * FROM t WHERE id = 7");
        let mut ctx = StageContext::default();
        Pipeline::observe_only().run(&cmd, &mut ctx);
        assert_eq!(
            ctx.digest.as_ref().unwrap().text,
            "SELECT * FROM t WHERE id = ?"
        );
        assert!(!ctx.digest_unavailable);
    }

    #[test]
    fn observe_stage_marks_unparsable_statements() {
        let cmd = command(b"\x03SELECT 'unterminated");
        let mut ctx = StageContext::default();
        Pipeline::observe_only().run(&cmd, &mut ctx);
        assert!(ctx.digest.is_none());
        assert!(ctx.digest_unavailable);
    }

    #[test]
    fn commands_without_sql_produce_no_digest() {
        let cmd = command(b"\x0e");
        let mut ctx = StageContext::default();
        Pipeline::observe_only().run(&cmd, &mut ctx);
        assert!(ctx.digest.is_none());
        assert!(!ctx.digest_unavailable);
    }

    #[test]
    fn a_replacing_stage_yields_owned_bytes() {
        let cmd = command(b"\x03SELECT 1");
        let mut ctx = StageContext::default();
        let pipeline = Pipeline::new(vec![Box::new(ObserveStage), Box::new(AppendStage(b" LIMIT 1"))]);
        let out = pipeline.run(&cmd, &mut ctx);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(&out[..], b"\x03SELECT 1 LIMIT 1");
    }

    #[test]
    fn later_stages_see_earlier_replacements() {
        let cmd = command(b"\x03A");
        let mut ctx = StageContext::default();
        let pipeline = Pipeline::new(vec![
            Box::new(AppendStage(b"B")),
            Box::new(AppendStage(b"C")),
        ]);
        let out = pipeline.run(&cmd, &mut ctx);
        assert_eq!(&out[..], b"\x03ABC");
    }
}
