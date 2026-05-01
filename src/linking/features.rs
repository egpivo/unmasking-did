use anyhow::Result;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::unionfind::UnionFind as PetUnionFind;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::evidence::{
    extract_did_controller, extract_ens_handle, extract_funded_by, extract_safe_owner,
    Attestation, EvidenceKind, Strength,
};
use crate::storage::Repo;

/// Maximum number of addresses that may share a single `(kind, key)`
/// before the key is flagged as service-like and excluded from edge
/// generation. The cap is intentionally low: real entity-control
/// signals fan out narrowly, while CEX hot wallets, bridges, batch
/// distributors, and faucets fan out broadly. Behavioral detection
/// catches new services that no hardcoded blacklist could anticipate.
const FAN_OUT_CAP: usize = 50;

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
    key: String,
    strength: Strength,
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
    Ok(link_addresses(repo, addresses, min_evidence).await?.clusters)
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
    let run_id = generate_run_id();
    let params = serde_json::json!({
        "min_evidence": min_evidence,
        "address_count": addresses.len(),
        "fan_out_cap": FAN_OUT_CAP,
    })
    .to_string();
    repo.start_clustering_run(&run_id, &params).await?;

    let output = link_addresses(repo, addresses, min_evidence).await?;

    for cluster in &output.clusters {
        let evidence_json = serde_json::json!({
            "shared_evidence_keys": cluster.shared_evidence_keys,
        })
        .to_string();
        repo.insert_cluster(&run_id, &cluster.cluster_id, &cluster.addresses, &evidence_json)
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
    let blacklist = cex_blacklist();

    // Normalize + dedup the input set. Duplicates would otherwise cause
    // each extractor to emit the same attestation row twice, blowing up
    // the evidence UNIQUE constraint at insert time. Sorting also makes
    // node-index assignment in cluster_from_evidence deterministic
    // regardless of the caller's input ordering.
    let mut normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    normalized.sort();
    normalized.dedup();

    let funded = extract_funded_by(repo, &normalized, &blacklist).await?;
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

    cluster_from_evidence(repo, &normalized, min_evidence).await
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
    let normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();
    let stored = repo.attestations_for(&normalized).await?;
    build_clusters(&normalized, &stored, min_evidence)
}

fn build_clusters(
    addresses: &[String],
    attestations: &[Attestation],
    min_evidence: usize,
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
    for ((kind, key), atts) in &by_key {
        if atts.len() < 2 {
            continue;
        }
        if atts.len() > FAN_OUT_CAP {
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
                        key: key.clone(),
                        strength,
                    },
                );
            }
        }
    }

    type PairStats = (usize, Strength, Vec<String>);
    let mut per_pair: HashMap<(NodeIndex, NodeIndex), PairStats> = HashMap::new();
    for edge in graph.edge_indices() {
        let (a, b) = graph.edge_endpoints(edge).unwrap();
        let label = graph.edge_weight(edge).unwrap();
        let key_pair = if a.index() < b.index() { (a, b) } else { (b, a) };
        let entry = per_pair
            .entry(key_pair)
            .or_insert_with(|| (0, Strength::Weak, Vec::new()));
        entry.0 += 1;
        if label.strength > entry.1 {
            entry.1 = label.strength;
        }
        entry.2.push(label.key.clone());
    }

    // Merge invariant — see CLAUDE-skill "Linking Rule":
    //   * Strong evidence may merge on its own.
    //   * Otherwise, need ≥ min_evidence edges AND at least one ≥ MEDIUM.
    //   * Weak alone never merges; weak edges only count toward bulk if
    //     accompanied by ≥ 1 medium+ edge.
    let needed = min_evidence.max(1);
    let mut uf = PetUnionFind::<usize>::new(graph.node_count());
    for ((a, b), (count, max_strength, _)) in &per_pair {
        let merge = *max_strength == Strength::Strong
            || (*count >= needed && *max_strength >= Strength::Medium);
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
            let mut addresses: Vec<String> =
                indices.iter().map(|&i| graph[i].clone()).collect();
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
    per_pair: &HashMap<(NodeIndex, NodeIndex), (usize, Strength, Vec<String>)>,
) -> Vec<String> {
    let mut all: HashSet<String> = HashSet::new();
    for i in 0..indices.len() {
        for j in (i + 1)..indices.len() {
            let key_pair = if indices[i].index() < indices[j].index() {
                (indices[i], indices[j])
            } else {
                (indices[j], indices[i])
            };
            if let Some((_, _, keys)) = per_pair.get(&key_pair) {
                for k in keys {
                    all.insert(k.clone());
                }
            }
        }
    }
    let mut v: Vec<String> = all.into_iter().collect();
    v.sort();
    v
}
