use anyhow::Result;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::unionfind::UnionFind as PetUnionFind;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::evidence::{
    extract_did_controller, extract_ens_handle, extract_funded_by, extract_safe_owner, Attestation,
    EvidenceKind, Strength,
};
use crate::storage::Repo;

/// Maximum number of addresses that may share a single `(kind, key)`
/// before the key is flagged as service-like and excluded from edge
/// generation. The cap is intentionally low: real entity-control
/// signals fan out narrowly, while CEX hot wallets, bridges, batch
/// distributors, and faucets fan out broadly. Behavioral detection
/// catches new services that no hardcoded blacklist could anticipate.
/// Run-level fan-out cap for `(kind, key)` groups. Shared by the rule
/// linker, graph export, and the pairwise linkage feature builder.
pub const FAN_OUT_CAP: usize = 50;
pub const FUNDED_BY_BURST_BLOCK_DELTA: i64 = 5_000;
pub const FUNDED_BY_MIN_SHARED_KEYS: usize = 2;
pub const FUNDED_BY_MIN_SHORT_BURST_HITS: usize = 2;

const CEX_BLACKLIST: &[&str] = &[
    // Binance hot wallets
    "0x28c6c06298d514db089934071355e5743bf21d60",
    "0x21a31ee1afc51d94c2efccaa2092ad1028285549",
    "0xdfd5293d8e347dfe59e90efd55b2956a1343963d",
    "0x56eddb7aa87536c09ccc2793473599fd21a8b17f",
    "0x9696f59e4d72e237be84ffd425dcad154bf96976",
    // Coinbase
    "0x71660c4005ba85c37ccec55d0c4493e66fe775d3",
    "0x503828976d22510aad0201ac7ec88293211d23da",
    "0xa090e606e30bd747d4e6245a1517ebe430f0057e",
    "0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740",
    "0x3cd751e6b0078be393132286c442345e5dc49699",
    // Kraken
    "0x267be1c1d684f78cb4f6a176c4911b741e4ffdc0",
    "0x2910543af39aba0cd09dbb2d50200b3e800a63d2",
    "0xe853c56864a2ebe4576a807d26fdc4a0ada51919",
    "0x53d284357ec70ce289d6d64134dfac8e511c8a3d",
];

pub fn cex_blacklist() -> HashSet<String> {
    CEX_BLACKLIST.iter().map(|s| s.to_string()).collect()
}

const ALWAYS_SUPPRESSED_FUNDED_BY_KEYS: &[&str] = &[
    "0x0000000000000000000000000000000000000000", // system/mint semantics
    "0x111111125421ca6dc452d289314280a0f8842a65", // 1inch router
    "0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae", // bridge/router
    "0x2342deb6f5749ef6ce6943a275a1d3e7486f5fbf", // bridge/router variant
];

#[derive(Debug, Clone, Serialize)]
pub struct FundedByMergePolicy {
    pub enabled: bool,
    pub service_fan_out_cap: usize,
    pub min_shared_keys: usize,
    pub min_short_burst_hits: usize,
    pub short_burst_block_delta: i64,
}

impl FundedByMergePolicy {
    pub fn legacy_disabled() -> Self {
        Self {
            enabled: false,
            service_fan_out_cap: FAN_OUT_CAP,
            min_shared_keys: FUNDED_BY_MIN_SHARED_KEYS,
            min_short_burst_hits: FUNDED_BY_MIN_SHORT_BURST_HITS,
            short_burst_block_delta: FUNDED_BY_BURST_BLOCK_DELTA,
        }
    }
}

fn always_suppressed_funded_by_keys() -> HashSet<String> {
    ALWAYS_SUPPRESSED_FUNDED_BY_KEYS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterReport {
    /// Deterministic cluster identifier: the lexicographically smallest
    /// lowercase address in the cluster. Stable across runs given the
    /// same inputs.
    pub cluster_id: String,
    pub addresses: Vec<String>,
    /// The set of evidence keys (funder addresses, `service:handle`
    /// strings, controller keys, …) that justified at least one merge
    /// inside this cluster.
    pub shared_evidence_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedKey {
    pub kind: String,
    pub key: String,
    pub fan_out: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkingOutput {
    pub clusters: Vec<ClusterReport>,
    pub skipped_service_keys: Vec<SkippedKey>,
}

#[derive(Debug, Clone)]
struct EdgeLabel {
    kind: EvidenceKind,
    key: String,
    strength: Strength,
}

#[derive(Debug, Clone)]
struct PairStats {
    total_count: usize,
    total_max_strength: Strength,
    keys: Vec<String>,
    non_funded_count: usize,
    non_funded_max_strength: Strength,
    funded_unique_keys: HashSet<String>,
    funded_short_burst_hits: usize,
}

/// Backwards-compatible orchestrator: extract funded_by attestations,
/// persist them, build clusters, return just the clusters. Callers that
/// also need the list of skipped service-like keys should use
/// [`link_addresses`] directly.
pub async fn cluster_by_funding(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
) -> Result<Vec<ClusterReport>> {
    Ok(link_addresses(repo, addresses, min_evidence)
        .await?
        .clusters)
}

/// End-to-end M1 link pass with full audit persistence: opens a
/// `clustering_runs` row, runs [`link_addresses`], writes clusters into
/// `entity_clusters` and any fan-out-cap hits into `suspected_service_keys`.
/// Returns the generated `run_id` alongside the in-memory output so the
/// CLI can print both.
pub async fn link_and_persist(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
) -> Result<(String, LinkingOutput)> {
    link_and_persist_with_fanout(
        repo,
        addresses,
        min_evidence,
        FAN_OUT_CAP,
        None,
        &FundedByMergePolicy::legacy_disabled(),
    )
    .await
}

pub async fn link_and_persist_with_fanout(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
    fan_out_cap: usize,
    extra_funder_deny: Option<&HashSet<String>>,
    funded_by_policy: &FundedByMergePolicy,
) -> Result<(String, LinkingOutput)> {
    let run_id = generate_run_id();
    let params = serde_json::json!({
        "min_evidence": min_evidence,
        "address_count": addresses.len(),
        "fan_out_cap": fan_out_cap,
        "funded_by_policy": funded_by_policy,
    })
    .to_string();
    repo.start_clustering_run(&run_id, &params).await?;

    let output = link_addresses_with_fanout(
        repo,
        addresses,
        min_evidence,
        fan_out_cap,
        extra_funder_deny,
        funded_by_policy,
    )
    .await?;

    for cluster in &output.clusters {
        let evidence_json = serde_json::json!({
            "shared_evidence_keys": cluster.shared_evidence_keys,
        })
        .to_string();
        repo.insert_cluster(
            &run_id,
            &cluster.cluster_id,
            &cluster.addresses,
            &evidence_json,
        )
        .await?;
    }
    for skipped in &output.skipped_service_keys {
        let kind = EvidenceKind::parse(&skipped.kind).unwrap_or(EvidenceKind::FundedBy);
        repo.record_suspected_service_key(&run_id, kind, &skipped.key, skipped.fan_out)
            .await?;
    }

    Ok((run_id, output))
}

fn generate_run_id() -> String {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    format!("run-{micros}")
}

/// Full link pass: extract every supported evidence kind from the local
/// caches, persist new attestations, build clusters from the union of all
/// persisted evidence. Persistence to `entity_clusters` and
/// `suspected_service_keys` is handled by the caller (the CLI), so this
/// function stays pure with respect to downstream cluster tables.
pub async fn link_addresses(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
) -> Result<LinkingOutput> {
    link_addresses_with_fanout(
        repo,
        addresses,
        min_evidence,
        FAN_OUT_CAP,
        None,
        &FundedByMergePolicy::legacy_disabled(),
    )
    .await
}

/// Same as [`link_addresses`] but with an explicit per-run `(kind, key)`
/// fan-out cap for service-like key suppression at merge time.
pub async fn link_addresses_with_fanout(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
    fan_out_cap: usize,
    extra_funder_deny: Option<&HashSet<String>>,
    funded_by_policy: &FundedByMergePolicy,
) -> Result<LinkingOutput> {
    let blacklist = cex_blacklist();

    // Normalize + dedup the input set. Duplicates would otherwise cause
    // each extractor to emit the same attestation row twice, blowing up
    // the evidence UNIQUE constraint at insert time. Sorting also makes
    // node-index assignment in cluster_from_evidence deterministic
    // regardless of the caller's input ordering.
    let mut normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    normalized.sort();
    normalized.dedup();

    let mut funded = extract_funded_by(repo, &normalized, &blacklist).await?;
    if let Some(deny) = extra_funder_deny {
        funded.retain(|a| !deny.contains(&a.key));
    }
    let ens = extract_ens_handle(repo, &normalized).await?;
    let safe = extract_safe_owner(repo, &normalized).await?;
    let did = extract_did_controller(repo, &normalized).await?;

    // Replace, not append, but ONLY for the kinds this pipeline owns.
    // Each kind is derived from a specific cache (transfers /
    // ens_records / safe_owners / did_documents) and a re-extract
    // should reflect the current state of that cache. Other kinds in
    // the evidence table — anything inserted by callers outside
    // link_addresses — must survive untouched.
    repo.replace_attestations_for_kind(&normalized, EvidenceKind::FundedBy, &funded)
        .await?;
    repo.replace_attestations_for_kind(&normalized, EvidenceKind::EnsHandle, &ens)
        .await?;
    repo.replace_attestations_for_kind(&normalized, EvidenceKind::SafeOwner, &safe)
        .await?;
    repo.replace_attestations_for_kind(&normalized, EvidenceKind::DidController, &did)
        .await?;

    cluster_from_evidence_with_fanout(
        repo,
        &normalized,
        min_evidence,
        fan_out_cap,
        funded_by_policy,
    )
    .await
}

/// Build clusters strictly from attestations already persisted in the
/// `evidence` table. Useful for re-running clustering with different
/// thresholds without re-fetching, and for unit tests that inject
/// non-funded_by evidence kinds directly.
pub async fn cluster_from_evidence(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
) -> Result<LinkingOutput> {
    cluster_from_evidence_with_fanout(
        repo,
        addresses,
        min_evidence,
        FAN_OUT_CAP,
        &FundedByMergePolicy::legacy_disabled(),
    )
    .await
}

pub async fn cluster_from_evidence_with_fanout(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
    fan_out_cap: usize,
    funded_by_policy: &FundedByMergePolicy,
) -> Result<LinkingOutput> {
    let normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    let stored = repo.attestations_for(&normalized).await?;
    cluster_from_attestations(
        &normalized,
        &stored,
        min_evidence,
        fan_out_cap,
        funded_by_policy,
    )
}

/// Same merge policy as [`cluster_from_evidence`], but uses caller-supplied
/// attestations (e.g. ablation-filtered rows for evaluation).
pub fn cluster_from_attestations(
    addresses: &[String],
    attestations: &[Attestation],
    min_evidence: usize,
    fan_out_cap: usize,
    funded_by_policy: &FundedByMergePolicy,
) -> Result<LinkingOutput> {
    build_clusters(
        addresses,
        attestations,
        min_evidence,
        fan_out_cap,
        funded_by_policy,
    )
}

fn build_clusters(
    addresses: &[String],
    attestations: &[Attestation],
    min_evidence: usize,
    fan_out_cap: usize,
    funded_by_policy: &FundedByMergePolicy,
) -> Result<LinkingOutput> {
    let mut graph: UnGraph<String, EdgeLabel> = UnGraph::new_undirected();
    let mut node_of: HashMap<String, NodeIndex> = HashMap::new();
    for addr in addresses {
        let idx = graph.add_node(addr.clone());
        node_of.insert(addr.clone(), idx);
    }

    let mut by_key: HashMap<(EvidenceKind, String), Vec<&Attestation>> = HashMap::new();
    for a in attestations {
        by_key.entry((a.kind, a.key.clone())).or_default().push(a);
    }

    let mut skipped: Vec<SkippedKey> = Vec::new();
    let suppressed_funded = always_suppressed_funded_by_keys();
    for ((kind, key), atts) in &by_key {
        if atts.len() < 2 {
            continue;
        }
        let service_like_funded = *kind == EvidenceKind::FundedBy
            && funded_by_policy.enabled
            && (suppressed_funded.contains(key)
                || atts.len() > funded_by_policy.service_fan_out_cap);
        if atts.len() > fan_out_cap || service_like_funded {
            skipped.push(SkippedKey {
                kind: kind.as_str().to_string(),
                key: key.clone(),
                fan_out: atts.len(),
            });
            continue;
        }
        for i in 0..atts.len() {
            for j in (i + 1)..atts.len() {
                let (Some(&ai), Some(&bi)) =
                    (node_of.get(&atts[i].address), node_of.get(&atts[j].address))
                else {
                    continue;
                };
                let strength = atts[i].strength.max(atts[j].strength);
                graph.add_edge(
                    ai,
                    bi,
                    EdgeLabel {
                        kind: *kind,
                        key: key.clone(),
                        strength,
                    },
                );
            }
        }
    }

    let mut per_pair: HashMap<(NodeIndex, NodeIndex), PairStats> = HashMap::new();
    for edge in graph.edge_indices() {
        let (a, b) = graph.edge_endpoints(edge).unwrap();
        let label = graph.edge_weight(edge).unwrap();
        let key_pair = if a.index() < b.index() {
            (a, b)
        } else {
            (b, a)
        };
        let entry = per_pair.entry(key_pair).or_insert_with(|| PairStats {
            total_count: 0,
            total_max_strength: Strength::Weak,
            keys: Vec::new(),
            non_funded_count: 0,
            non_funded_max_strength: Strength::Weak,
            funded_unique_keys: HashSet::new(),
            funded_short_burst_hits: 0,
        });
        entry.total_count += 1;
        if label.strength > entry.total_max_strength {
            entry.total_max_strength = label.strength;
        }
        entry.keys.push(label.key.clone());
        if label.kind == EvidenceKind::FundedBy {
            entry.funded_unique_keys.insert(label.key.clone());
        } else {
            entry.non_funded_count += 1;
            if label.strength > entry.non_funded_max_strength {
                entry.non_funded_max_strength = label.strength;
            }
        }
    }

    if funded_by_policy.enabled {
        // Exact funded_by burst accounting from raw attestation groups.
        let mut by_funded_key: HashMap<String, Vec<&Attestation>> = HashMap::new();
        for a in attestations {
            if a.kind == EvidenceKind::FundedBy {
                by_funded_key.entry(a.key.clone()).or_default().push(a);
            }
        }
        for (key, atts) in by_funded_key {
            if atts.len() < 2 {
                continue;
            }
            if suppressed_funded.contains(&key)
                || atts.len() > fan_out_cap
                || atts.len() > funded_by_policy.service_fan_out_cap
            {
                continue;
            }
            for i in 0..atts.len() {
                for j in (i + 1)..atts.len() {
                    let (Some(&ai), Some(&bi)) =
                        (node_of.get(&atts[i].address), node_of.get(&atts[j].address))
                    else {
                        continue;
                    };
                    let pair = if ai.index() < bi.index() {
                        (ai, bi)
                    } else {
                        (bi, ai)
                    };
                    if let Some(entry) = per_pair.get_mut(&pair) {
                        let delta = (atts[i].observed_block - atts[j].observed_block).abs();
                        if delta <= funded_by_policy.short_burst_block_delta {
                            entry.funded_short_burst_hits += 1;
                        }
                    }
                }
            }
        }
    }

    // Merge invariant — see CLAUDE-skill "Linking Rule":
    //   * Strong evidence may merge on its own.
    //   * Otherwise, need ≥ min_evidence edges AND at least one ≥ MEDIUM.
    //   * Weak alone never merges; weak edges only count toward bulk if
    //     accompanied by ≥ 1 medium+ edge.
    let needed = min_evidence.max(1);
    let mut uf = PetUnionFind::<usize>::new(graph.node_count());
    for ((a, b), stats) in &per_pair {
        let merge = if !funded_by_policy.enabled {
            stats.total_max_strength == Strength::Strong
                || (stats.total_count >= needed && stats.total_max_strength >= Strength::Medium)
        } else {
            let strong_non_funded = stats.non_funded_max_strength == Strength::Strong;
            let medium_non_funded =
                stats.non_funded_count >= 1 && stats.non_funded_max_strength >= Strength::Medium;
            let combined_with_non_funded = medium_non_funded
                && ((stats.non_funded_count + stats.funded_unique_keys.len()) >= needed);
            let funded_only_repeated_burst = stats.funded_unique_keys.len()
                >= funded_by_policy.min_shared_keys
                && stats.funded_short_burst_hits >= funded_by_policy.min_short_burst_hits;
            strong_non_funded || combined_with_non_funded || funded_only_repeated_burst
        };
        if merge {
            uf.union(a.index(), b.index());
        }
    }

    let labels = uf.into_labeling();
    let mut by_label: HashMap<usize, Vec<NodeIndex>> = HashMap::new();
    for (i, label) in labels.iter().enumerate() {
        by_label.entry(*label).or_default().push(NodeIndex::new(i));
    }

    let mut reports: Vec<ClusterReport> = by_label
        .into_values()
        .map(|indices| {
            let mut addresses: Vec<String> = indices.iter().map(|&i| graph[i].clone()).collect();
            addresses.sort();
            let cluster_id = addresses[0].clone();
            let shared_evidence_keys = collect_shared_keys(&indices, &per_pair);
            ClusterReport {
                cluster_id,
                addresses,
                shared_evidence_keys,
            }
        })
        .collect();

    reports.sort_by(|x, y| {
        y.addresses
            .len()
            .cmp(&x.addresses.len())
            .then_with(|| x.cluster_id.cmp(&y.cluster_id))
    });
    skipped.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.key.cmp(&b.key)));

    Ok(LinkingOutput {
        clusters: reports,
        skipped_service_keys: skipped,
    })
}

fn collect_shared_keys(
    indices: &[NodeIndex],
    per_pair: &HashMap<(NodeIndex, NodeIndex), PairStats>,
) -> Vec<String> {
    let mut all: HashSet<String> = HashSet::new();
    for i in 0..indices.len() {
        for j in (i + 1)..indices.len() {
            let key_pair = if indices[i].index() < indices[j].index() {
                (indices[i], indices[j])
            } else {
                (indices[j], indices[i])
            };
            if let Some(stats) = per_pair.get(&key_pair) {
                for k in &stats.keys {
                    all.insert(k.clone());
                }
            }
        }
    }
    let mut v: Vec<String> = all.into_iter().collect();
    v.sort();
    v
}
