use anyhow::Result;
use std::collections::HashSet;

use crate::storage::Repo;

use super::model::{Attestation, EvidenceKind, Strength};

/// Build `funded_by` attestations from the cached `transfers` table.
/// Each non-blacklisted funder of `addr` becomes one MEDIUM-strength
/// attestation. Blacklist hits are dropped at extraction time so the
/// `evidence` table never sees uninformative service-address edges.
///
/// The returned attestations are NOT yet persisted; the caller is
/// responsible for writing them via [`Repo::insert_attestations`] so that
/// extraction stays a pure function of the cache.
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
