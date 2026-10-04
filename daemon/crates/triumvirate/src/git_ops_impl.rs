//! Moved to `fleet::git_ops` (2026-10-04) so the Temporal fleet engine's activities can drive the
//! same merge path as the legacy engine. Re-exported here so existing call sites are unchanged.
pub use fleet::git_ops::RealGitOps;
