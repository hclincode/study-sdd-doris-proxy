//! L7 MySQL proxy for Apache Doris enforcing row-level filtering by authenticated user.
//!
//! This is a security control. The governing rule, from which everything else
//! follows: a statement whose policy-bearing table references cannot all be
//! proven constrained is **rejected, never forwarded**. See
//! `openspec/changes/add-row-filter-proxy-mvp/design.md`.

pub mod analyze;
pub mod error;
pub mod policy;
pub mod rewrite;
pub mod session;

pub use error::{ProxyError, Result};
