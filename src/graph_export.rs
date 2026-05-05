//! Bounded D3 finding-graph export.
//!
//! Builds a small JSON document of the form
//! `{ run, nodes, links, limits }` from the latest persisted
//! clustering run plus the current `evidence` snapshot. The shape
//! is deliberately D3-friendly — every `link` references node `id`s
//! by string — so a static `viewer.html` can pick the file up and
//! render it without a build step.
//!
//! ## Bounded by construction
//!
//! - **depth = 1**: only direct evidence on the cluster members;
//!   no recursive expansion (a funder's other funders are never
//!   pulled in).
//! - **`max_identifier_nodes`**: caps the address subjects.
//!   Truncation prefers whole clusters, sorted by `cluster_id`,
//!   so the smallest-id clusters always make it in.
//! - **`max_evidence_nodes`**: caps the `(kind, key)` evidence
//!   nodes. Each unique evidence value collapses many attestations
//!   into one node, so even busy seed sets typically stay well
//!   under the cap.
//! - **fan-out cap**: matches the linker. `(kind, key)` groups
//!   whose member count exceeds the cap (default 50) are dropped
//!   here too — bridges, CEX hot wallets, batch distributors do
//!   not appear as evidence nodes even if some attestations
//!   slipped past the hardcoded CEX blacklist upstream.
//!
//! ## Scroll vs Ethereum mainnet
//!
//! **Scroll** (small seed sets): default caps are usually enough for a
//! full qualitative graph in `viewer/viewer.html`.
//!
//! **Ethereum mainnet** at DAO scale: the SQLite `evidence` table can
//! hold **many attestation rows** (10⁵+); this exporter still emits at
//! most **`max_evidence_nodes` unique `(kind, key)` evidence nodes**
//! (default 200), not one node per row. Tighten
//! `--max-identifier-nodes`, `--max-evidence-nodes`, and
//! `--max-pairwise-links` for interactive loads; use
//! `scripts/graph_diag.sql` for SQL summaries (kind counts, top shared
//! keys, cluster sizes) without opening the raw table. See
//! `docs/finding-graph.md` for the full framing.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::pairwise::{
    candidate_address_pairs, score_address_pairs, LinkTier, LinkageParams,
};
use crate::storage::Repo;

/// Default fan-out cap, mirroring `linking::features::FAN_OUT_CAP`.
pub const DEFAULT_FAN_OUT_CAP: usize = 50;
pub const DEFAULT_MAX_IDENTIFIER_NODES: usize = 50;
pub const DEFAULT_MAX_EVIDENCE_NODES: usize = 200;

#[derive(Debug, Serialize)]
pub struct Graph {
    /// `evidence` — bipartite identifier↔evidence graph (debug / audit view).  
    /// `pairwise` — identifier↔identifier edges with scores and tiers.
    pub graph_mode: String,
    pub run: RunSummary,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub limits: Limits,
    /// Per-row attestations behind the export (evidence mode only). Drives
    /// timeline / time-window filtering in `graph-explorer.html`. Empty for
    /// `pairwise` graphs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_events: Vec<EvidenceEvent>,
}

/// One persisted attestation row, scoped to the export’s identifier and
/// evidence-node caps (same rows that justify bipartite links).
#[derive(Debug, Serialize)]
pub struct EvidenceEvent {
    pub identifier_id: String,
    pub evidence_id: String,
    #[serde(rename = "type")]
    pub evidence_kind: String,
    pub strength: &'static str,
    pub source: String,
    pub observed_block: i64,
    /// Same as the evidence key (`funded_by` funder, `safe_owner` owner, …).
    pub counterparty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub started_at: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct Node {
    pub id: String,
    /// `"identifier"` (a clustering subject) or `"evidence"`
    /// (a shared `(kind, key)` value).
    pub kind: &'static str,
    /// For identifiers: `"address"`. For evidence: the evidence
    /// kind (`"safe_owner"`, `"funded_by"`, `"ens_handle"`,
    /// `"did_controller"`).
    #[serde(rename = "type")]
    pub node_type: String,
    /// Human-friendly short label (`0xa1a1…a1a1`, `twitter:joseph`).
    pub label: String,
    /// Full underlying value (full address, full handle string).
    /// Lets the viewer show the unabridged form on hover.
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Link {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub link_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_probability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributions: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deterministic_anchor: Option<bool>,
    /// Evidence-mode only: min `observed_block` across merged attestations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen_block: Option<i64>,
    /// Evidence-mode only: max `observed_block` across merged attestations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_block: Option<i64>,
    /// Evidence-mode only: number of attestation rows merged into this link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_count: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Limits {
    pub max_identifier_nodes: usize,
    pub max_evidence_nodes: usize,
    pub depth: usize,
    pub fan_out_cap: usize,
    pub truncated_identifiers: bool,
    pub truncated_evidence: bool,
    pub applied_filters: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pairwise_links: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkage_params_source: Option<String>,
}

/// Build the finding graph for the given persisted run.
///
/// Returns `Err` if no such run exists, or if the run is the latest
/// and there is no latest. Caller must have already populated the
/// `evidence` table for the run's identifiers (which is true any
/// time `link` has run against them).
pub async fn build_graph(
    repo: &Repo,
    run_id: Option<&str>,
    max_identifier_nodes: usize,
    max_evidence_nodes: usize,
    fan_out_cap: usize,
) -> Result<Graph> {
    let run = match run_id {
        Some(id) => {
            // Resolve a specific run id by looking it up via the
            // existing latest-run path then sanity-checking.
            // Repo::latest_clustering_run is the only summary getter
            // today; for an explicit id we'd need a separate query.
            // For now, only the latest run is supported — explicit
            // run_id selection is a future enhancement.
            let latest = repo
                .latest_clustering_run()
                .await?
                .ok_or_else(|| anyhow!("no clustering runs found"))?;
            if latest.run_id != id {
                return Err(anyhow!(
                    "explicit run_id selection is not yet supported; latest run is {}",
                    latest.run_id
                ));
            }
            latest
        }
        None => repo
            .latest_clustering_run()
            .await?
            .ok_or_else(|| anyhow!("no clustering runs found — run `link` first"))?,
    };

    let clusters = repo.clusters_for_run(&run.run_id).await?;

    // ---- service-key detection (BEFORE identifier truncation) -
    // Compute (kind, key) fan-out across the FULL run, not just the
    // selected identifier subset. If we deferred this until after
    // truncation, a service-like key shared by N+1 addresses could
    // sneak through whenever the truncated subset happened to
    // contain only N of those addresses — `atts.len()` would no
    // longer exceed the cap, and the evidence node would render.
    // Fan-out is a property of the run, not of the visualization
    // window, so it has to be computed at run scope.
    let all_run_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let all_run_attestations = repo.attestations_for(&all_run_addresses).await?;
    let mut full_run_fanout: HashMap<(EvidenceKind, String), usize> = HashMap::new();
    for att in &all_run_attestations {
        *full_run_fanout
            .entry((att.kind, att.key.clone()))
            .or_insert(0) += 1;
    }
    let service_keys: HashSet<(EvidenceKind, String)> = full_run_fanout
        .iter()
        .filter(|(_, &count)| count > fan_out_cap)
        .map(|(k, _)| k.clone())
        .collect();

    // ---- identifier nodes -------------------------------------
    // Whole-cluster semantics: a cluster is either fully included
    // or skipped. If a cluster on its own is bigger than the cap,
    // it is skipped (truncated_identifiers=true) and iteration
    // continues — smaller clusters can still fill the budget. We
    // never emit a partial-cluster slice, which would be
    // misleading: a viewer of a half-rendered cluster would
    // mistakenly read it as the full cluster.
    let mut identifier_addresses: Vec<(String, String)> = Vec::new();
    let mut truncated_identifiers = false;
    let mut skipped_cluster_ids: Vec<String> = Vec::new();
    for cluster in &clusters {
        if identifier_addresses.len() + cluster.addresses.len() > max_identifier_nodes {
            truncated_identifiers = true;
            skipped_cluster_ids.push(cluster.cluster_id.clone());
            continue;
        }
        for addr in &cluster.addresses {
            identifier_addresses.push((addr.clone(), cluster.cluster_id.clone()));
        }
    }

    let selected_addrs: HashSet<String> = identifier_addresses
        .iter()
        .map(|(a, _)| a.clone())
        .collect();

    // Restrict the existing all-run attestations to those whose
    // address ended up in the selected identifier set. Re-using
    // the `all_run_attestations` slice here avoids a second SQL
    // round-trip.
    let attestations: Vec<&Attestation> = all_run_attestations
        .iter()
        .filter(|a| selected_addrs.contains(&a.address))
        .collect();

    // ---- evidence nodes (one per (kind, key)) -----------------
    // Service-key detection already happened above; here we group
    // attestations on the SELECTED identifiers to decide which
    // (kind, key) values become rendered evidence nodes, dropping
    // any keys flagged service-like at the run level.
    let mut by_kind_key: BTreeMap<(EvidenceKind, String), Vec<&Attestation>> = BTreeMap::new();
    for att in &attestations {
        by_kind_key
            .entry((att.kind, att.key.clone()))
            .or_default()
            .push(*att);
    }

    let mut evidence_nodes: Vec<((EvidenceKind, String), Strength)> = Vec::new();
    let mut truncated_evidence = false;
    for ((kind, key), atts) in &by_kind_key {
        if service_keys.contains(&(*kind, key.clone())) {
            // Run-level fan-out cap hit; keep it out of the graph
            // even if the selected subset only contains a few
            // attestations for this key.
            continue;
        }
        if evidence_nodes.len() >= max_evidence_nodes {
            truncated_evidence = true;
            break;
        }
        let strength = atts
            .iter()
            .map(|a| a.strength)
            .max()
            .unwrap_or(Strength::Weak);
        evidence_nodes.push(((*kind, key.clone()), strength));
    }
    let evidence_set: HashSet<(EvidenceKind, String)> =
        evidence_nodes.iter().map(|(k, _)| k.clone()).collect();

    // ---- assemble nodes + links -------------------------------
    let mut nodes: Vec<Node> =
        Vec::with_capacity(identifier_addresses.len() + evidence_nodes.len());
    for (addr, cluster_id) in &identifier_addresses {
        nodes.push(Node {
            id: identifier_node_id(addr),
            kind: "identifier",
            node_type: "address".to_string(),
            label: short_label(addr),
            value: addr.clone(),
            cluster_id: Some(cluster_id.clone()),
            strength: None,
        });
    }
    for ((kind, key), strength) in &evidence_nodes {
        nodes.push(Node {
            id: evidence_node_id(*kind, key),
            kind: "evidence",
            node_type: kind.as_str().to_string(),
            label: short_label(key),
            value: key.clone(),
            cluster_id: None,
            strength: Some(strength_label(*strength)),
        });
    }

    // De-duplicate links by `(identifier, evidence_node)` — multiple
    // attestations collapse into one rendered link. We keep the strongest
    // strength, min/max `observed_block`, and a row count for the viewer.
    #[derive(Clone)]
    struct LinkAgg {
        kind: EvidenceKind,
        strength: Strength,
        min_block: i64,
        max_block: i64,
        count: u32,
    }

    impl LinkAgg {
        fn from_att(att: &Attestation) -> Self {
            Self {
                kind: att.kind,
                strength: att.strength,
                min_block: att.observed_block,
                max_block: att.observed_block,
                count: 1,
            }
        }

        fn merge(&mut self, att: &Attestation) {
            if att.strength > self.strength {
                self.strength = att.strength;
                self.kind = att.kind;
            }
            self.min_block = self.min_block.min(att.observed_block);
            self.max_block = self.max_block.max(att.observed_block);
            self.count = self.count.saturating_add(1);
        }
    }

    let mut by_link: BTreeMap<(String, String), LinkAgg> = BTreeMap::new();
    for att in &attestations {
        let key = (att.kind, att.key.clone());
        if !evidence_set.contains(&key) {
            continue;
        }
        let src = identifier_node_id(&att.address);
        let dst = evidence_node_id(att.kind, &att.key);
        by_link
            .entry((src, dst))
            .and_modify(|agg| agg.merge(att))
            .or_insert_with(|| LinkAgg::from_att(att));
    }
    let mut links: Vec<Link> = by_link
        .into_iter()
        .map(|((source, target), agg)| Link {
            source,
            target,
            link_type: agg.kind.as_str().to_string(),
            strength: Some(strength_label(agg.strength)),
            tier: None,
            score: None,
            link_probability: None,
            contributions: None,
            deterministic_anchor: None,
            first_seen_block: Some(agg.min_block),
            last_seen_block: Some(agg.max_block),
            attestation_count: Some(agg.count),
        })
        .collect();
    // Stable order for the output JSON.
    links.sort_by(|a, b| {
        (a.source.as_str(), a.target.as_str()).cmp(&(b.source.as_str(), b.target.as_str()))
    });

    let mut evidence_events: Vec<EvidenceEvent> = Vec::new();
    for att in &attestations {
        let key = (att.kind, att.key.clone());
        if !evidence_set.contains(&key) {
            continue;
        }
        let payload = att
            .payload_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        evidence_events.push(EvidenceEvent {
            identifier_id: identifier_node_id(&att.address),
            evidence_id: evidence_node_id(att.kind, &att.key),
            evidence_kind: att.kind.as_str().to_string(),
            strength: strength_label(att.strength),
            source: att.source.clone(),
            observed_block: att.observed_block,
            counterparty: att.key.clone(),
            payload,
        });
    }
    evidence_events.sort_by(|a, b| {
        a.observed_block
            .cmp(&b.observed_block)
            .then_with(|| a.identifier_id.cmp(&b.identifier_id))
            .then_with(|| a.evidence_id.cmp(&b.evidence_id))
            .then_with(|| a.source.cmp(&b.source))
    });

    // ---- run summary ------------------------------------------
    let params: serde_json::Value =
        serde_json::from_str(&run.params_json).unwrap_or(serde_json::json!(run.params_json));

    let mut applied_filters = vec![
        format!("fan_out_cap = {fan_out_cap} (computed on full run, before identifier truncation)"),
        "cex_blacklist (applied at extraction in extract_funded_by)".to_string(),
        "depth = 1 (no recursion past direct evidence on cluster members)".to_string(),
    ];
    if !skipped_cluster_ids.is_empty() {
        applied_filters.push(format!(
            "skipped {} oversized cluster(s) that would individually exceed max_identifier_nodes={}: {}",
            skipped_cluster_ids.len(),
            max_identifier_nodes,
            skipped_cluster_ids.join(", ")
        ));
    }
    if truncated_identifiers && skipped_cluster_ids.is_empty() {
        // Truncated for combined-budget reasons (whole clusters dropped because
        // they would have pushed past the cap when added to earlier ones).
        applied_filters.push(format!(
            "stopped including further clusters once max_identifier_nodes={max_identifier_nodes} would be exceeded"
        ));
    }
    if truncated_evidence {
        applied_filters.push(format!(
            "truncated to first {max_evidence_nodes} evidence nodes"
        ));
    }
    if !service_keys.is_empty() {
        applied_filters.push(format!(
            "{} service-key (kind, key) group(s) with run-level fan-out > {} suppressed",
            service_keys.len(),
            fan_out_cap
        ));
    }

    Ok(Graph {
        graph_mode: "evidence".to_string(),
        run: RunSummary {
            run_id: run.run_id,
            started_at: run.started_at,
            params,
        },
        nodes,
        links,
        limits: Limits {
            max_identifier_nodes,
            max_evidence_nodes,
            depth: 1,
            fan_out_cap,
            truncated_identifiers,
            truncated_evidence,
            applied_filters,
            max_pairwise_links: None,
            linkage_params_source: None,
        },
        evidence_events,
    })
}

/// Pairwise identifier graph: edges carry interpretable scores, tiers,
/// and per-channel contributions. Identifier selection and fan-out
/// suppression mirror [`build_graph`].
pub async fn build_pairwise_graph(
    repo: &Repo,
    run_id: Option<&str>,
    max_identifier_nodes: usize,
    fan_out_cap: usize,
    max_pairwise_links: usize,
    linkage_params: LinkageParams,
    linkage_params_source: &str,
) -> Result<Graph> {
    let run = match run_id {
        Some(id) => {
            let latest = repo
                .latest_clustering_run()
                .await?
                .ok_or_else(|| anyhow!("no clustering runs found"))?;
            if latest.run_id != id {
                return Err(anyhow!(
                    "explicit run_id selection is not yet supported; latest run is {}",
                    latest.run_id
                ));
            }
            latest
        }
        None => repo
            .latest_clustering_run()
            .await?
            .ok_or_else(|| anyhow!("no clustering runs found — run `link` first"))?,
    };

    let clusters = repo.clusters_for_run(&run.run_id).await?;
    let all_run_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let all_run_attestations = repo.attestations_for(&all_run_addresses).await?;
    let mut full_run_fanout: HashMap<(EvidenceKind, String), usize> = HashMap::new();
    for att in &all_run_attestations {
        *full_run_fanout
            .entry((att.kind, att.key.clone()))
            .or_insert(0) += 1;
    }
    let service_keys: HashSet<(EvidenceKind, String)> = full_run_fanout
        .iter()
        .filter(|(_, &count)| count > fan_out_cap)
        .map(|(k, _)| k.clone())
        .collect();

    let mut identifier_addresses: Vec<(String, String)> = Vec::new();
    let mut truncated_identifiers = false;
    let mut skipped_cluster_ids: Vec<String> = Vec::new();
    for cluster in &clusters {
        if identifier_addresses.len() + cluster.addresses.len() > max_identifier_nodes {
            truncated_identifiers = true;
            skipped_cluster_ids.push(cluster.cluster_id.clone());
            continue;
        }
        for addr in &cluster.addresses {
            identifier_addresses.push((addr.clone(), cluster.cluster_id.clone()));
        }
    }

    let selected_addrs: HashSet<String> = identifier_addresses
        .iter()
        .map(|(a, _)| a.clone())
        .collect();

    let selected_atts: Vec<Attestation> = all_run_attestations
        .into_iter()
        .filter(|a| selected_addrs.contains(&a.address))
        .collect();

    let address_list: Vec<String> = identifier_addresses
        .iter()
        .map(|(a, _)| a.clone())
        .collect();

    let pairs = candidate_address_pairs(&address_list, &selected_atts, max_pairwise_links);
    let scored = score_address_pairs(&pairs, &selected_atts, &linkage_params);

    let mut nodes: Vec<Node> = Vec::with_capacity(identifier_addresses.len());
    for (addr, cluster_id) in &identifier_addresses {
        nodes.push(Node {
            id: identifier_node_id(addr),
            kind: "identifier",
            node_type: "address".to_string(),
            label: short_label(addr),
            value: addr.clone(),
            cluster_id: Some(cluster_id.clone()),
            strength: None,
        });
    }

    let mut links: Vec<Link> = Vec::new();
    for sc in scored {
        if sc.tier == LinkTier::Rejected {
            continue;
        }
        links.push(Link {
            source: identifier_node_id(&sc.address_a),
            target: identifier_node_id(&sc.address_b),
            link_type: "linkage".to_string(),
            strength: None,
            tier: Some(sc.tier.as_str().to_string()),
            score: Some(sc.score),
            link_probability: Some(sc.link_probability),
            contributions: Some(sc.contributions.clone()),
            deterministic_anchor: Some(sc.deterministic_anchor),
            first_seen_block: None,
            last_seen_block: None,
            attestation_count: None,
        });
    }
    links.sort_by(|a, b| {
        let ta = a.tier.as_deref().unwrap_or("");
        let tb = b.tier.as_deref().unwrap_or("");
        tb.cmp(ta)
            .then_with(|| {
                let sa = a.score.unwrap_or(0.0);
                let sb = b.score.unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.target.cmp(&b.target))
    });

    let params: serde_json::Value =
        serde_json::from_str(&run.params_json).unwrap_or(serde_json::json!(run.params_json));

    let mut applied_filters = vec![
        format!("graph_mode = pairwise"),
        format!("linkage_params = {linkage_params_source}"),
        format!("fan_out_cap = {fan_out_cap} (run-level, before identifier truncation)"),
        format!("max_pairwise_candidate_pairs = {max_pairwise_links}"),
    ];
    if !skipped_cluster_ids.is_empty() {
        applied_filters.push(format!(
            "skipped {} oversized cluster(s) that would individually exceed max_identifier_nodes={}: {}",
            skipped_cluster_ids.len(),
            max_identifier_nodes,
            skipped_cluster_ids.join(", ")
        ));
    }
    if truncated_identifiers && skipped_cluster_ids.is_empty() {
        applied_filters.push(format!(
            "stopped including further clusters once max_identifier_nodes={max_identifier_nodes} would be exceeded"
        ));
    }
    if !service_keys.is_empty() {
        applied_filters.push(format!(
            "{} service-key (kind, key) group(s) with run-level fan-out > {} suppressed",
            service_keys.len(),
            fan_out_cap
        ));
    }

    Ok(Graph {
        graph_mode: "pairwise".to_string(),
        run: RunSummary {
            run_id: run.run_id,
            started_at: run.started_at,
            params,
        },
        nodes,
        links,
        limits: Limits {
            max_identifier_nodes,
            max_evidence_nodes: 0,
            depth: 1,
            fan_out_cap,
            truncated_identifiers,
            truncated_evidence: false,
            applied_filters,
            max_pairwise_links: Some(max_pairwise_links),
            linkage_params_source: Some(linkage_params_source.to_string()),
        },
        evidence_events: Vec::new(),
    })
}

pub fn write_graph_json(graph: &Graph, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let pretty = serde_json::to_string_pretty(graph)?;
    std::fs::write(path, pretty)
        .with_context(|| format!("failed to write graph to {}", path.display()))?;
    Ok(())
}

fn identifier_node_id(address: &str) -> String {
    format!("addr:{address}")
}

fn evidence_node_id(kind: EvidenceKind, key: &str) -> String {
    format!("ev:{}:{}", kind.as_str(), key)
}

fn strength_label(s: Strength) -> &'static str {
    match s {
        Strength::Strong => "strong",
        Strength::Medium => "medium",
        Strength::Weak => "weak",
    }
}

fn short_label(value: &str) -> String {
    if value.starts_with("0x") && value.len() >= 12 {
        format!("{}…{}", &value[..6], &value[value.len() - 4..])
    } else if value.len() > 24 {
        format!("{}…", &value[..24])
    } else {
        value.to_string()
    }
}

/// Suppress an unused-import warning when the file is built without
/// the test cfg pulling in `HashMap`. Real callers should ignore.
#[allow(dead_code)]
fn _hashmap_marker() -> HashMap<(), ()> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linking::{link_and_persist, LinkageParams};
    use crate::safe::SafeOwner;
    use crate::storage::{connect, run_migrations};

    async fn fresh_repo() -> Repo {
        let pool = connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        Repo::new(pool)
    }

    fn safe_owner(safe: &str, owner: &str) -> SafeOwner {
        SafeOwner {
            safe_address: safe.to_string(),
            owner_address: owner.to_string(),
            owner_is_safe: false,
            threshold: Some(2),
            observed_block: Some(100),
            source: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn graph_includes_identifier_and_evidence_nodes_with_link() {
        // Two Safes share one EOA owner; expect:
        //   - 2 identifier nodes (the Safes)
        //   - 1 evidence node (the shared owner)
        //   - 2 links (one per attestation)
        let repo = fresh_repo().await;
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        repo.upsert_safe_owner(&safe_owner(safe_a, owner))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_b, owner))
            .await
            .unwrap();
        repo.upsert_address(safe_a, None).await.unwrap();
        repo.upsert_address(safe_b, None).await.unwrap();

        let _ = link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
            .await
            .unwrap();

        let graph = build_graph(&repo, None, 50, 200, 50).await.unwrap();
        assert_eq!(graph.graph_mode, "evidence");

        let identifiers: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "identifier")
            .collect();
        let evidence: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "evidence")
            .collect();
        assert_eq!(identifiers.len(), 2);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].node_type, "safe_owner");
        assert_eq!(evidence[0].value, owner);
        assert_eq!(graph.links.len(), 2);
        assert!(graph.links.iter().all(|l| l.link_type == "safe_owner"));
        assert_eq!(graph.evidence_events.len(), 2);
        assert!(graph.links.iter().all(|l| l.attestation_count == Some(1)));
        assert_eq!(graph.limits.depth, 1);
        assert!(!graph.limits.truncated_identifiers);
        assert!(!graph.limits.truncated_evidence);
    }

    #[tokio::test]
    async fn fan_out_cap_drops_service_keys_from_evidence_nodes() {
        // 51 Safes share one EOA owner. With fan_out_cap=50 the
        // (safe_owner, owner) group is treated as service-like and
        // produces no evidence node — same logic the linker uses.
        let repo = fresh_repo().await;
        let owner = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";
        let mut addrs = Vec::with_capacity(51);
        for i in 0..51u32 {
            let safe = format!("0x{:040x}", i + 1);
            repo.upsert_safe_owner(&safe_owner(&safe, owner))
                .await
                .unwrap();
            repo.upsert_address(&safe, None).await.unwrap();
            addrs.push(safe);
        }
        let _ = link_and_persist(&repo, &addrs, 1).await.unwrap();

        let graph = build_graph(&repo, None, 200, 200, 50).await.unwrap();

        let evidence: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "evidence")
            .collect();
        assert!(
            evidence.is_empty(),
            "service-fan-out evidence node must not be rendered"
        );
        assert!(
            graph
                .limits
                .applied_filters
                .iter()
                .any(|f| f.contains("fan_out_cap")),
            "fan-out filter must appear in applied_filters audit trail"
        );
    }

    #[tokio::test]
    async fn max_identifier_nodes_truncates_at_cluster_boundary() {
        // Three singleton clusters; cap at 2 → only the first two
        // come through, the third is dropped, truncated flag set.
        let repo = fresh_repo().await;
        for i in 1..=3u32 {
            let safe = format!("0x{:040x}", i);
            let owner = format!("0xeoa0000000000000000000000000000000000{i:03}");
            repo.upsert_safe_owner(&safe_owner(&safe, &owner))
                .await
                .unwrap();
            repo.upsert_address(&safe, None).await.unwrap();
        }
        let inputs: Vec<String> = (1..=3u32).map(|i| format!("0x{:040x}", i)).collect();
        let _ = link_and_persist(&repo, &inputs, 1).await.unwrap();

        let graph = build_graph(&repo, None, 2, 200, 50).await.unwrap();

        let identifiers: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "identifier")
            .collect();
        assert_eq!(identifiers.len(), 2);
        assert!(graph.limits.truncated_identifiers);
    }

    #[tokio::test]
    async fn service_key_with_full_run_fan_out_above_cap_is_dropped_even_when_truncated_subset_falls_below(
    ) {
        // Regression: fan-out filtering must run BEFORE identifier
        // truncation. A `(kind, key)` shared by N+1 addresses at run
        // scope is service-like; if we truncate down to N selected
        // identifiers, the per-subset count drops to N (≤ cap) and
        // a buggy implementation would let the evidence node
        // through. With cap=10 and 11 Safes sharing one owner, we
        // truncate to 10 identifiers — the service key must stay
        // out of the rendered graph.
        let repo = fresh_repo().await;
        let owner = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";
        let mut addrs = Vec::with_capacity(11);
        for i in 0..11u32 {
            let safe = format!("0x{:040x}", i + 1);
            repo.upsert_safe_owner(&safe_owner(&safe, owner))
                .await
                .unwrap();
            repo.upsert_address(&safe, None).await.unwrap();
            addrs.push(safe);
        }
        let _ = link_and_persist(&repo, &addrs, 1).await.unwrap();

        // max_identifier_nodes = 10 forces truncation; fan_out_cap = 10
        // means run-level fan-out (11) exceeds the cap, so the key
        // must NOT show up regardless of truncation.
        let graph = build_graph(&repo, None, 10, 200, 10).await.unwrap();

        let evidence: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "evidence")
            .collect();
        assert!(
            evidence.is_empty(),
            "service key (run-level fan-out 11 > cap 10) must be dropped even when only 10 of 11 identifiers were selected, got: {evidence:?}"
        );
        assert!(
            graph
                .limits
                .applied_filters
                .iter()
                .any(|f| f.contains("service-key")),
            "applied_filters must surface that a service-key group was suppressed; got: {:?}",
            graph.limits.applied_filters
        );
    }

    #[tokio::test]
    async fn oversized_cluster_is_skipped_not_partially_included() {
        // Regression: when a single cluster is bigger than the
        // identifier cap, the old code emitted a partial slice of
        // that cluster's addresses. New behavior: skip the cluster
        // entirely (record it in applied_filters), continue
        // iteration so smaller clusters can still fill the budget.
        //
        // Setup: cluster A has 3 members merged via a shared owner;
        // singleton clusters B and C. Cap = 2. With the old buggy
        // path, A's first 2 addresses would render as a half-cluster.
        // With the fix, A is skipped (truncated_identifiers=true),
        // and B + C come through as 2 separate singletons.
        let repo = fresh_repo().await;

        // Cluster A: three Safes share the same owner.
        let shared_owner = "0xeeee0000000000000000000000000000000000ee";
        let safe_a1 = "0x000000000000000000000000000000000000000a";
        let safe_a2 = "0x000000000000000000000000000000000000000b";
        let safe_a3 = "0x000000000000000000000000000000000000000c";
        for s in [safe_a1, safe_a2, safe_a3] {
            repo.upsert_safe_owner(&safe_owner(s, shared_owner))
                .await
                .unwrap();
            repo.upsert_address(s, None).await.unwrap();
        }
        // Two singleton Safes with their own private owners.
        let safe_b = "0x00000000000000000000000000000000000000b1";
        let safe_c = "0x00000000000000000000000000000000000000c1";
        let owner_b = "0x00000000000000000000000000000000000000b2";
        let owner_c = "0x00000000000000000000000000000000000000c2";
        repo.upsert_safe_owner(&safe_owner(safe_b, owner_b))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_c, owner_c))
            .await
            .unwrap();
        repo.upsert_address(safe_b, None).await.unwrap();
        repo.upsert_address(safe_c, None).await.unwrap();

        let inputs: Vec<String> = [safe_a1, safe_a2, safe_a3, safe_b, safe_c]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let _ = link_and_persist(&repo, &inputs, 1).await.unwrap();

        let graph = build_graph(&repo, None, 2, 200, 50).await.unwrap();

        let identifier_addrs: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "identifier")
            .map(|n| n.value.as_str())
            .collect();
        assert!(
            !identifier_addrs.contains(&safe_a1)
                && !identifier_addrs.contains(&safe_a2)
                && !identifier_addrs.contains(&safe_a3),
            "the 3-member cluster must be skipped entirely, not partially included; got {identifier_addrs:?}"
        );
        assert!(
            identifier_addrs.contains(&safe_b) && identifier_addrs.contains(&safe_c),
            "the two singletons fit within the cap and must come through; got {identifier_addrs:?}"
        );
        assert_eq!(identifier_addrs.len(), 2);
        assert!(graph.limits.truncated_identifiers);
        assert!(
            graph
                .limits
                .applied_filters
                .iter()
                .any(|f| f.contains("skipped") && f.contains("oversized")),
            "applied_filters must record the skipped oversized cluster; got: {:?}",
            graph.limits.applied_filters
        );
    }

    #[tokio::test]
    async fn deterministic_link_order_is_stable() {
        let repo = fresh_repo().await;
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner1 = "0xeoa1000000000000000000000000000000000000";
        let owner2 = "0xeoa2000000000000000000000000000000000000";
        repo.upsert_safe_owner(&safe_owner(safe_a, owner1))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_b, owner1))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_a, owner2))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_b, owner2))
            .await
            .unwrap();
        repo.upsert_address(safe_a, None).await.unwrap();
        repo.upsert_address(safe_b, None).await.unwrap();
        let _ = link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
            .await
            .unwrap();

        let g1 = build_graph(&repo, None, 50, 200, 50).await.unwrap();
        let g2 = build_graph(&repo, None, 50, 200, 50).await.unwrap();
        let l1: Vec<_> = g1.links.iter().map(|l| (&l.source, &l.target)).collect();
        let l2: Vec<_> = g2.links.iter().map(|l| (&l.source, &l.target)).collect();
        assert_eq!(l1, l2, "link order must be deterministic across runs");
    }

    #[tokio::test]
    async fn pairwise_graph_is_identifier_only_with_tiered_links() {
        let repo = fresh_repo().await;
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        repo.upsert_safe_owner(&safe_owner(safe_a, owner))
            .await
            .unwrap();
        repo.upsert_safe_owner(&safe_owner(safe_b, owner))
            .await
            .unwrap();
        repo.upsert_address(safe_a, None).await.unwrap();
        repo.upsert_address(safe_b, None).await.unwrap();

        let _ = link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
            .await
            .unwrap();

        let params = LinkageParams::bundled_default().unwrap();
        let graph = build_pairwise_graph(&repo, None, 50, 50, 500, params, "test bundled")
            .await
            .unwrap();

        assert_eq!(graph.graph_mode, "pairwise");
        assert!(graph.nodes.iter().all(|n| n.kind == "identifier"));
        assert!(!graph.links.is_empty());
        let e = &graph.links[0];
        assert_eq!(e.link_type, "linkage");
        assert!(e.tier.is_some());
        assert!(e.score.is_some());
        assert!(e.contributions.is_some());
    }
}
