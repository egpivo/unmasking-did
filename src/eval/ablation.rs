//! Evidence-channel ablations for evaluation runs.

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::evidence::{Attestation, EvidenceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AblationMode {
    /// No filtering — all kinds present in the slice.
    AllEvidence,
    SafeOwnerOnly,
    DidControllerOnly,
    FundedByOnly,
    EnsHandleOnly,
    SafeOwnerAndFundedBy,
    /// All kinds except ENS / social handle channel.
    WithoutEns,
}

impl AblationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllEvidence => "all_evidence",
            Self::SafeOwnerOnly => "safe_owner_only",
            Self::DidControllerOnly => "did_controller_only",
            Self::FundedByOnly => "funded_by_only",
            Self::EnsHandleOnly => "ens_handle_only",
            Self::SafeOwnerAndFundedBy => "safe_owner_and_funded_by",
            Self::WithoutEns => "without_ens",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "all" | "all_evidence" => Ok(Self::AllEvidence),
            "safe_owner_only" => Ok(Self::SafeOwnerOnly),
            "did_controller_only" => Ok(Self::DidControllerOnly),
            "funded_by_only" => Ok(Self::FundedByOnly),
            "ens_handle_only" => Ok(Self::EnsHandleOnly),
            "safe_owner_and_funded_by" | "safe_owner+funded_by" => Ok(Self::SafeOwnerAndFundedBy),
            "without_ens" | "no_ens" => Ok(Self::WithoutEns),
            other => Err(anyhow!("unknown ablation mode: {other}")),
        }
    }

    pub fn preset_matrix() -> Vec<Self> {
        vec![
            Self::AllEvidence,
            Self::SafeOwnerOnly,
            Self::DidControllerOnly,
            Self::FundedByOnly,
            Self::EnsHandleOnly,
            Self::SafeOwnerAndFundedBy,
            Self::WithoutEns,
        ]
    }

    pub fn parse_list(s: &str) -> Result<Vec<Self>> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Ok(Self::preset_matrix());
        }
        s.split(',').map(|p| Self::parse(p.trim())).collect()
    }

    pub fn filter(&self, attestations: &[Attestation]) -> Vec<Attestation> {
        let keep = |k: EvidenceKind| match self {
            Self::AllEvidence => true,
            Self::SafeOwnerOnly => k == EvidenceKind::SafeOwner,
            Self::DidControllerOnly => k == EvidenceKind::DidController,
            Self::FundedByOnly => k == EvidenceKind::FundedBy,
            Self::EnsHandleOnly => k == EvidenceKind::EnsHandle,
            Self::SafeOwnerAndFundedBy => {
                k == EvidenceKind::SafeOwner || k == EvidenceKind::FundedBy
            }
            Self::WithoutEns => k != EvidenceKind::EnsHandle,
        };
        attestations
            .iter()
            .filter(|a| keep(a.kind))
            .cloned()
            .collect()
    }
}
