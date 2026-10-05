//! AIEN Prime Drag Race suite runner. See CONTRACT.md next to this crate.
//!
//! Std plus serde_json, sha2, chrono and clap (all already in the parent Cargo.lock).

pub mod exec;
pub mod receipt;
pub mod reference;
pub mod runner;
pub mod sweep;
pub mod validate;

/// Frozen contract version (see prime-drag-race/CONTRACT.md).
pub const CONTRACT_VERSION: &str = "1";
pub const REPORT_SCHEMA: &str = "aien-prime-race/impl-report/v1";
pub const RECEIPT_SCHEMA: &str = "aien-prime-race/receipt/v1";
