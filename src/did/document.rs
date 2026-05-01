use serde::{Deserialize, Serialize};

/// One observed DID document, cached for evidence extraction.
///
/// `subject_address` is the EVM address embedded in the DID
/// (e.g. `did:ethr:0xabc...` -> `0xabc...`). `controller` is the
/// address authorised to update the DID document — when distinct from
/// `subject_address`, the relationship constitutes cryptographic-level
/// shared control and is emitted as STRONG `did_controller` evidence.
///
/// `method` records which DID method produced this document (`ethr`,
/// `pkh`, `web`, `key`); the extractor uses it for source provenance
/// only — clustering itself is method-agnostic and only cares about
/// the (subject, controller) edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidDocument {
    pub did: String,
    pub subject_address: String,
    pub controller: String,
    pub method: String,
    pub document_json: Option<String>,
    pub observed_block: Option<i64>,
    pub source: String,
}
