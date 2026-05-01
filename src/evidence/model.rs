use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Weak = 1,
    Medium = 2,
    Strong = 3,
}

impl Strength {
    pub fn as_int(self) -> i64 {
        self as i64
    }

    pub fn from_int(n: i64) -> Option<Self> {
        match n {
            1 => Some(Self::Weak),
            2 => Some(Self::Medium),
            3 => Some(Self::Strong),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    FundedBy,
    SafeOwner,
    EnsHandle,
    DidController,
}

impl EvidenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FundedBy => "funded_by",
            Self::SafeOwner => "safe_owner",
            Self::EnsHandle => "ens_handle",
            Self::DidController => "did_controller",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "funded_by" => Some(Self::FundedBy),
            "safe_owner" => Some(Self::SafeOwner),
            "ens_handle" => Some(Self::EnsHandle),
            "did_controller" => Some(Self::DidController),
            _ => None,
        }
    }
}

/// One typed observation about an address. Persisted into the `evidence`
/// table append-only; pairs of attestations sharing the same `(kind, key)`
/// become clustering edges at graph build time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    pub address: String,
    pub kind: EvidenceKind,
    pub key: String,
    pub strength: Strength,
    pub source: String,
    pub observed_block: i64,
    pub payload_json: Option<String>,
}
