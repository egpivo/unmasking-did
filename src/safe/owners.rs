use serde::{Deserialize, Serialize};

/// One Safe → owner edge. The Safe is identified by `safe_address`;
/// the owner can be an EOA or another Safe (`owner_is_safe = true`).
/// Only EOA-owner edges are emitted as `safe_owner` evidence by the
/// extractor — Safe-of-safe ownership tells us nothing about
/// human-level control on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeOwner {
    pub safe_address: String,
    pub owner_address: String,
    pub owner_is_safe: bool,
    pub threshold: Option<i64>,
    pub observed_block: Option<i64>,
    pub source: String,
}
