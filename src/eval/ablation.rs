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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{Attestation, Strength};

    fn att(kind: EvidenceKind) -> Attestation {
        Attestation {
            address: "0x1".to_string(),
            kind,
            key: "k".to_string(),
            strength: Strength::Medium,
            source: "test".to_string(),
            observed_block: 1,
            payload_json: None,
        }
    }

    #[test]
    fn parse_aliases_and_errors() {
        assert_eq!(
            AblationMode::parse("all").unwrap(),
            AblationMode::AllEvidence
        );
        assert_eq!(
            AblationMode::parse(" ALL_EVIDENCE ").unwrap(),
            AblationMode::AllEvidence
        );
        assert_eq!(
            AblationMode::parse("safe_owner+funded_by").unwrap(),
            AblationMode::SafeOwnerAndFundedBy
        );
        assert_eq!(
            AblationMode::parse("no_ens").unwrap(),
            AblationMode::WithoutEns
        );
        let err = AblationMode::parse("nope").unwrap_err();
        assert!(err.to_string().contains("unknown ablation"));
    }

    #[test]
    fn parse_list_all_and_csv() {
        let all = AblationMode::parse_list("all").unwrap();
        assert_eq!(all, AblationMode::preset_matrix());
        let two = AblationMode::parse_list("safe_owner_only, did_controller_only ").unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two[0], AblationMode::SafeOwnerOnly);
        assert_eq!(two[1], AblationMode::DidControllerOnly);
        assert!(AblationMode::parse_list("bad").is_err());
    }

    #[test]
    fn preset_matrix_covers_all_as_str_variants() {
        for m in AblationMode::preset_matrix() {
            let round = AblationMode::parse(m.as_str()).unwrap();
            assert_eq!(round, m);
        }
    }

    #[test]
    fn filter_per_mode() {
        let kinds = [
            EvidenceKind::SafeOwner,
            EvidenceKind::DidController,
            EvidenceKind::FundedBy,
            EvidenceKind::EnsHandle,
        ];
        let atts: Vec<_> = kinds.iter().copied().map(att).collect();

        assert_eq!(AblationMode::AllEvidence.filter(&atts).len(), 4);
        assert_eq!(AblationMode::SafeOwnerOnly.filter(&atts).len(), 1);
        assert_eq!(AblationMode::DidControllerOnly.filter(&atts).len(), 1);
        assert_eq!(AblationMode::FundedByOnly.filter(&atts).len(), 1);
        assert_eq!(AblationMode::EnsHandleOnly.filter(&atts).len(), 1);
        let sf = AblationMode::SafeOwnerAndFundedBy.filter(&atts);
        assert_eq!(sf.len(), 2);
        assert!(sf
            .iter()
            .all(|a| matches!(a.kind, EvidenceKind::SafeOwner | EvidenceKind::FundedBy)));
        let no_ens = AblationMode::WithoutEns.filter(&atts);
        assert_eq!(no_ens.len(), 3);
        assert!(!no_ens.iter().any(|a| a.kind == EvidenceKind::EnsHandle));
    }
}
