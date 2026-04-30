use anyhow::Result;
use std::collections::HashSet;

use crate::storage::Repo;

use super::model::{Attestation, EvidenceKind, Strength};

/// Build `funded_by` attestations from the cached `transfers` table.
/// Each non-blacklisted funder of `addr` becomes one MEDIUM-strength
/// attestation. Blacklist hits are dropped at extraction time so the
/// `evidence` table never sees uninformative service-address edges.
pub async fn extract_funded_by(
    repo: &Repo,
    addresses: &[String],
    blacklist: &HashSet<String>,
) -> Result<Vec<Attestation>> {
    let mut out = Vec::new();
    for addr in addresses {
        let normalized = addr.to_lowercase();
        for (funder, first_block) in repo.incoming_funders(&normalized).await? {
            if blacklist.contains(&funder) {
                continue;
            }
            out.push(Attestation {
                address: normalized.clone(),
                kind: EvidenceKind::FundedBy,
                key: funder,
                strength: Strength::Medium,
                source: format!("alchemy_getAssetTransfers@{first_block}"),
                observed_block: first_block,
                payload_json: None,
            });
        }
    }
    Ok(out)
}

/// Build `ens_handle` attestations from the cached `ens_records` table.
/// Each non-empty off-chain handle (twitter / github / telegram) becomes
/// one MEDIUM-strength attestation keyed as `"<service>:<handle>"`.
///
/// The ENS `name` itself is intentionally NOT emitted as evidence: ENS
/// primary names are unique per address by construction, so two
/// addresses can never share one — there is no link to discover.
pub async fn extract_ens_handle(repo: &Repo, addresses: &[String]) -> Result<Vec<Attestation>> {
    let normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    let records = repo.ens_records_for(&normalized).await?;

    let mut out = Vec::new();
    for record in records {
        let address = record.address.to_lowercase();
        let payload = serde_json::json!({
            "ens_name": record.name,
        })
        .to_string();
        for (service, value) in [
            ("twitter", record.twitter.as_ref()),
            ("github", record.github.as_ref()),
            ("telegram", record.telegram.as_ref()),
        ] {
            if let Some(handle) = value.and_then(non_empty) {
                out.push(Attestation {
                    address: address.clone(),
                    kind: EvidenceKind::EnsHandle,
                    key: format!("{service}:{}", normalize_handle(handle)),
                    strength: Strength::Medium,
                    source: "ens_records".to_string(),
                    observed_block: 0,
                    payload_json: Some(payload.clone()),
                });
            }
        }
    }
    Ok(out)
}

fn non_empty(s: &String) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn normalize_handle(s: &str) -> String {
    s.trim().trim_start_matches('@').to_lowercase()
}

/// Build `safe_owner` attestations from the cached `safe_owners` table.
///
/// For every input address that is a known Safe, each EOA owner becomes
/// one MEDIUM-strength attestation `(safe_address, safe_owner, owner)`.
/// Owners flagged as Safes themselves (`owner_is_safe = true`) are
/// dropped: shared Safe-of-safe ownership tells us nothing about
/// human-level control on its own. Per the project taxonomy, only EOA
/// owners qualify.
pub async fn extract_safe_owner(repo: &Repo, addresses: &[String]) -> Result<Vec<Attestation>> {
    let mut out = Vec::new();
    for addr in addresses {
        let normalized = addr.to_lowercase();
        for owner in repo.safe_owners_of(&normalized).await? {
            if owner.owner_is_safe {
                continue;
            }
            let payload = serde_json::json!({
                "threshold": owner.threshold,
                "owner_source": owner.source,
            })
            .to_string();
            out.push(Attestation {
                address: normalized.clone(),
                kind: EvidenceKind::SafeOwner,
                key: owner.owner_address.to_lowercase(),
                strength: Strength::Medium,
                source: format!("safe_owners:{}", owner.source),
                observed_block: owner.observed_block.unwrap_or(0),
                payload_json: Some(payload),
            });
        }
    }
    Ok(out)
}
