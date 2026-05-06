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
            if let Some(handle) = value.map(|s| s.as_str()).and_then(non_empty) {
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

fn non_empty(s: &str) -> Option<&str> {
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

/// Build `did_controller` attestations from the cached `did_documents`
/// table. Each non-trivial document (where the recorded `controller`
/// differs from the `subject_address`) becomes one **STRONG**
/// attestation `(subject, did_controller, controller)`.
///
/// Self-controlled DIDs — `did:pkh` by construction, and any
/// freshly-minted `did:ethr` whose owner has never been changed — are
/// dropped at extraction time. Including them would emit a
/// self-referential edge that contributes no clustering signal: every
/// address trivially controls its own implicit DID.
///
/// **Dedup**: multiple `did_documents` rows can share the same
/// `(subject_address, controller)` — for example a single owner
/// registering `did:ethr:0xabc` on Ethereum and `did:ethr:scroll:0xabc`
/// on Scroll, both pointing at the same key. Without dedup, the
/// extractor would emit one attestation per row, which has two
/// independent failure modes:
///   1. plain INSERT in `replace_attestations_for_kind` collides on
///      the evidence `UNIQUE(address, kind, key, source)` index
///      because `source` doesn't disambiguate by DID, and the entire
///      link run aborts;
///   2. even with the source field disambiguated, the per-pair edge
///      count would inflate to N² for N DIDs supporting the same
///      controller — a single logical fact gets weighted as if it
///      were N independent observations, breaking the `min_evidence`
///      threshold's semantic.
///
/// We collapse to one attestation per `(subject, controller)` pair
/// here, with the full list of supporting DIDs preserved in
/// `payload_json` so audit information isn't lost. The
/// "first-seen" `did_documents` row (lowest `observed_block`, then
/// lexicographically smallest `did`) drives the attestation's
/// `source` and `observed_block` fields — the underlying query
/// orders rows that way for determinism.
pub async fn extract_did_controller(repo: &Repo, addresses: &[String]) -> Result<Vec<Attestation>> {
    use std::collections::HashMap;

    let normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    let docs = repo.did_documents_for(&normalized).await?;

    struct Aggregated {
        first_method: String,
        first_source: String,
        first_block: i64,
        supporting_dids: Vec<String>,
        supporting_methods: Vec<String>,
    }

    let mut by_pair: HashMap<(String, String), Aggregated> = HashMap::new();
    for doc in docs {
        let subject = doc.subject_address.to_lowercase();
        let controller = doc.controller.to_lowercase();
        if subject == controller {
            continue;
        }
        let entry = by_pair
            .entry((subject, controller))
            .or_insert_with(|| Aggregated {
                first_method: doc.method.clone(),
                first_source: doc.source.clone(),
                first_block: doc.observed_block.unwrap_or(0),
                supporting_dids: Vec::new(),
                supporting_methods: Vec::new(),
            });
        entry.supporting_dids.push(doc.did);
        entry.supporting_methods.push(doc.method);
    }

    let mut out: Vec<Attestation> = by_pair
        .into_iter()
        .map(|((subject, controller), agg)| {
            let payload = serde_json::json!({
                "dids": agg.supporting_dids,
                "methods": agg.supporting_methods,
            })
            .to_string();
            Attestation {
                address: subject,
                kind: EvidenceKind::DidController,
                key: controller,
                strength: Strength::Strong,
                source: format!("did_documents:{}:{}", agg.first_method, agg.first_source),
                observed_block: agg.first_block,
                payload_json: Some(payload),
            }
        })
        .collect();

    // Deterministic output order so downstream snapshot tests don't
    // race on HashMap iteration.
    out.sort_by(|a, b| a.address.cmp(&b.address).then_with(|| a.key.cmp(&b.key)));
    Ok(out)
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
