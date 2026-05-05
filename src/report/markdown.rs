use std::collections::HashMap;

use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::{ClusterReport, SkippedKey};
use crate::storage::ClusteringRunSummary;

use super::edges::{passing_edges, run_params, ClusterEdge};

/// Top of the per-cluster section: never include more than this many
/// multi-address clusters in detail. Anything beyond is summarized as
/// "and N more". Keeps the report readable when N is large.
const TOP_CLUSTERS_LIMIT: usize = 10;

/// Cap on the suspected-service-keys table. The full set is always
/// queryable from `suspected_service_keys` for audit; this is just
/// presentation.
const SUSPECTED_KEYS_LIMIT: usize = 20;

pub struct ReportInputs<'a> {
    pub run: &'a ClusteringRunSummary,
    pub clusters: &'a [ClusterReport],
    pub skipped: &'a [SkippedKey],
    /// Current evidence rows for every address in every cluster. Used
    /// to label clusters by their dominant evidence kind ("controller-
    /// level cluster", "shared-owner governance-control cluster", …)
    /// without needing a separate kind-aware projection in the linking
    /// pipeline. Empty `&[]` is acceptable — clusters then fall back
    /// to a generic "evidence-supported cluster" descriptor.
    pub attestations: &'a [Attestation],
    pub nakamoto: Option<u64>,
    pub gini: Option<f64>,
    pub nakamoto_threshold: f64,
}

pub fn render_markdown(input: &ReportInputs<'_>) -> String {
    let n_addresses: usize = input.clusters.iter().map(|c| c.addresses.len()).sum();
    let n_clusters = input.clusters.len();

    let mut s = String::new();
    s.push_str("# unmasking-did Report\n\n");
    s.push_str(&format!(
        "**Run**: `{}` (started {})\n",
        input.run.run_id, input.run.started_at
    ));
    s.push_str(&format!(
        "**Parameters**: `{}`\n\n",
        input.run.params_json.trim()
    ));

    s.push_str("## Summary\n\n");
    s.push_str(&format!("- Identifiers analyzed: **{n_addresses}**\n"));
    s.push_str(&format!("- Inferred clusters: **{n_clusters}**\n"));
    if n_clusters > 0 {
        let ratio = n_addresses as f64 / n_clusters as f64;
        s.push_str(&format!("- Identifiers per cluster: **{ratio:.2}**\n"));
    }
    if let Some(n) = input.nakamoto {
        s.push_str(&format!(
            "- Nakamoto coefficient (>{:.0}% of population): **{n}**\n",
            input.nakamoto_threshold * 100.0
        ));
    }
    if let Some(g) = input.gini {
        s.push_str(&format!("- Gini coefficient: **{g:.3}**\n"));
    }
    s.push('\n');

    // Reconstruct merge-passing edges from the current evidence
    // snapshot. The renderer only labels and lists evidence that
    // actually contributed to merging this cluster — see the note
    // in `report::edges` for why "any kind any member has" overstates
    // the cluster's basis.
    let (min_evidence, fan_out_cap) = run_params(&input.run.params_json);
    let edges = passing_edges(
        input.attestations,
        input.clusters,
        min_evidence,
        fan_out_cap,
    );

    let address_to_cluster: HashMap<&str, usize> = input
        .clusters
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.addresses.iter().map(move |a| (a.as_str(), i)))
        .collect();

    let mut edges_by_cluster: HashMap<usize, Vec<&ClusterEdge<'_>>> = HashMap::new();
    for e in &edges {
        if let Some(&ci) = address_to_cluster.get(e.src) {
            edges_by_cluster.entry(ci).or_default().push(e);
        }
    }

    s.push_str("## Top Clusters\n\n");
    let multi: Vec<(usize, &ClusterReport)> = input
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| c.addresses.len() > 1)
        .collect();
    let n_singletons = input.clusters.len() - multi.len();
    if multi.is_empty() {
        s.push_str("_No multi-address clusters in this run._\n\n");
    } else {
        for (cluster_idx, cluster) in multi.iter().take(TOP_CLUSTERS_LIMIT) {
            let cluster_edges = edges_by_cluster
                .get(cluster_idx)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let descriptor = cluster_descriptor(cluster_edges);
            s.push_str(&format!(
                "### {descriptor} — `{}` ({} identifiers)\n\n",
                short_addr(&cluster.cluster_id),
                cluster.addresses.len()
            ));
            // Shared-evidence list: derived from merge-passing edges,
            // not from the (potentially over-stated) persisted
            // `shared_evidence_keys`. Each key carries the kind that
            // actually contributed it.
            let key_kinds = collect_passing_keys(cluster_edges);
            if !key_kinds.is_empty() {
                s.push_str("Shared evidence keys (merge-passing only):\n");
                for (key, kinds) in &key_kinds {
                    let suffix = format_kinds_suffix(kinds);
                    s.push_str(&format!("- `{key}`{suffix}\n"));
                }
                s.push('\n');
            }
            s.push_str("Members:\n");
            for addr in &cluster.addresses {
                s.push_str(&format!("- `{addr}`\n"));
            }
            s.push('\n');
        }
        let total = multi.len();
        if total > TOP_CLUSTERS_LIMIT {
            s.push_str(&format!(
                "_… and {} more multi-identifier cluster(s) — full list in `entity_clusters` table._\n\n",
                total - TOP_CLUSTERS_LIMIT
            ));
        }
    }
    if n_singletons > 0 {
        s.push_str(&format!(
            "_{n_singletons} singleton cluster(s) not detailed above (each contributes one identifier with no shared-evidence edges; useful as negative controls)._\n\n"
        ));
    }

    if !input.skipped.is_empty() {
        s.push_str("## Suspected Service Keys\n\n");
        s.push_str(
            "These `(kind, key)` groups exceeded the fan-out cap and were excluded from edge generation. Inspect them to confirm the cap was correct (a real CEX / batch distributor / faucet) and not a missed legitimate entity:\n\n",
        );
        s.push_str("| Kind | Key | Fan-out |\n|---|---|---|\n");
        for sk in input.skipped.iter().take(SUSPECTED_KEYS_LIMIT) {
            s.push_str(&format!(
                "| `{}` | `{}` | {} |\n",
                sk.kind, sk.key, sk.fan_out
            ));
        }
        s.push('\n');
    }

    s.push_str("## Reproducibility\n\n");
    s.push_str(&format!(
        "Cluster identities are deterministic: `cluster_id = min(address)`. \
         Re-running the same `link` invocation against the same `evidence` \
         rows that produced run `{}` will yield byte-identical clusters. \
         Run metadata, parameters, evidence trail, and cluster membership \
         are all preserved in SQLite tables `clustering_runs`, `evidence`, \
         `entity_clusters`, and `suspected_service_keys`.\n",
        input.run.run_id
    ));

    s
}

/// Pick a phrase for a cluster heading based on the strongest evidence
/// kind that **actually contributed merge-passing edges** within the
/// cluster — not on every kind any member happened to have. A cluster
/// merged purely via `safe_owner` will not be relabelled
/// "controller-level" just because one member separately carries an
/// unrelated `did_controller` attestation.
fn cluster_descriptor(edges: &[&ClusterEdge<'_>]) -> &'static str {
    if edges.is_empty() {
        return "Evidence-supported cluster";
    }
    let max_strength = edges.iter().map(|e| e.strength).max();
    if max_strength == Some(Strength::Strong) {
        return "Controller-level cluster";
    }
    if edges.iter().any(|e| e.kind == EvidenceKind::SafeOwner) {
        return "Shared-owner governance-control cluster";
    }
    "Evidence-supported cluster"
}

/// Collect the distinct evidence keys that justified merges within a
/// single cluster, paired with the set of kinds that emitted each key.
/// Order is stable: keys appear in (kind, key) sort order.
fn collect_passing_keys<'a>(edges: &[&'a ClusterEdge<'a>]) -> Vec<(&'a str, Vec<EvidenceKind>)> {
    let mut by_key: std::collections::BTreeMap<&'a str, std::collections::BTreeSet<EvidenceKind>> =
        std::collections::BTreeMap::new();
    for e in edges {
        for k in &e.keys {
            by_key.entry(*k).or_default().insert(e.kind);
        }
    }
    by_key
        .into_iter()
        .map(|(k, kinds)| (k, kinds.into_iter().collect()))
        .collect()
}

/// `"  (safe_owner)"` or `"  (safe_owner, funded_by)"`. Empty when
/// `kinds` is empty — should not happen for entries returned by
/// `collect_passing_keys`, but defensive.
fn format_kinds_suffix(kinds: &[EvidenceKind]) -> String {
    if kinds.is_empty() {
        return String::new();
    }
    let names: Vec<&'static str> = kinds.iter().map(|k| k.as_str()).collect();
    format!("  ({})", names.join(", "))
}

fn short_addr(addr: &str) -> String {
    if addr.len() < 12 {
        addr.to_string()
    } else {
        format!("{}…{}", &addr[..6], &addr[addr.len() - 4..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_run() -> ClusteringRunSummary {
        ClusteringRunSummary {
            run_id: "run-test-123".to_string(),
            params_json: r#"{"min_evidence":1,"address_count":3}"#.to_string(),
            started_at: "2026-04-30 18:23:00".to_string(),
        }
    }

    #[test]
    fn renders_summary_and_cluster_section() {
        let run = synth_run();
        let clusters = vec![
            ClusterReport {
                cluster_id: "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1".into(),
                addresses: vec![
                    "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1".into(),
                    "0xb2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2".into(),
                ],
                shared_evidence_keys: vec!["twitter:joseph".into(), "0xfee0".into()],
            },
            ClusterReport {
                cluster_id: "0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3".into(),
                addresses: vec!["0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3".into()],
                shared_evidence_keys: vec![],
            },
        ];
        let skipped = vec![SkippedKey {
            kind: "funded_by".into(),
            key: "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0".into(),
            fan_out: 247,
        }];

        // Provide attestations matching the cluster's stated
        // shared_evidence_keys so passing_edges() actually finds
        // merge-passing edges. Without attestations, the new
        // renderer correctly emits an empty shared-keys list — see
        // the agent-driven P2 fix in src/report/edges.rs.
        let alice = "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
        let bob = "0xb2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
        let attestations = vec![
            Attestation {
                address: alice.into(),
                kind: EvidenceKind::EnsHandle,
                key: "twitter:joseph".into(),
                strength: Strength::Medium,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
            Attestation {
                address: bob.into(),
                kind: EvidenceKind::EnsHandle,
                key: "twitter:joseph".into(),
                strength: Strength::Medium,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
        ];
        let out = render_markdown(&ReportInputs {
            run: &run,
            clusters: &clusters,
            skipped: &skipped,
            attestations: &attestations,
            nakamoto: Some(1),
            gini: Some(0.333),
            nakamoto_threshold: 0.5,
        });

        // Header
        assert!(out.contains("# unmasking-did Report"));
        assert!(out.contains("`run-test-123`"));
        // Summary numbers (3 total addresses across the two clusters)
        assert!(out.contains("Identifiers analyzed: **3**"));
        assert!(out.contains("Inferred clusters: **2**"));
        assert!(
            !out.contains("Inferred entities"),
            "wording cleanup: 'entities' must not appear"
        );
        assert!(out.contains("Nakamoto coefficient"));
        assert!(out.contains("Gini coefficient: **0.333**"));
        // Cluster heading uses an evidence-aware descriptor and
        // counts identifiers (not "addresses" — those are one form
        // of identifier, not the only one).
        assert!(out.contains("(2 identifiers)"));
        // Merge-passing key list now drives the rendering; the key
        // appears AND is annotated with its kind.
        assert!(out.contains("twitter:joseph"));
        assert!(out.contains("(ens_handle)"));
        // Singleton cluster is summarized below, not promoted into
        // the top-clusters detail.
        assert!(out.contains("singleton cluster"));
        // Suspected service keys table
        assert!(out.contains("## Suspected Service Keys"));
        assert!(out.contains("`funded_by`"));
        assert!(out.contains("247"));
        // Reproducibility footer
        assert!(out.contains("## Reproducibility"));
        assert!(out.contains("min(address)"));
    }

    #[test]
    fn cluster_descriptor_picks_strongest_evidence_kind() {
        // STRONG did_controller present anywhere in the cluster's
        // evidence -> "Controller-level cluster".
        let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let cluster = ClusterReport {
            cluster_id: alice.into(),
            addresses: vec![alice.into(), bob.into()],
            shared_evidence_keys: vec!["0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0".into()],
        };
        let attestations = vec![
            Attestation {
                address: alice.into(),
                kind: EvidenceKind::DidController,
                key: "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0".into(),
                strength: Strength::Strong,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
            Attestation {
                address: bob.into(),
                kind: EvidenceKind::DidController,
                key: "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0".into(),
                strength: Strength::Strong,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
        ];

        let out = render_markdown(&ReportInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &attestations,
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });
        assert!(
            out.contains("Controller-level cluster"),
            "expected strong-evidence cluster heading, got:\n{out}"
        );
        // Each shared key gets its evidence-kind annotated in
        // parentheses so readers don't have to guess what a hex
        // string represents.
        assert!(
            out.contains("(did_controller)"),
            "key labels must annotate kind"
        );
    }

    #[test]
    fn cluster_descriptor_falls_back_to_safe_owner_then_generic() {
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let cluster = ClusterReport {
            cluster_id: safe_a.into(),
            addresses: vec![safe_a.into(), safe_b.into()],
            shared_evidence_keys: vec!["0xeoa00000000000000000000000000000000000000".into()],
        };
        let attestations = vec![
            Attestation {
                address: safe_a.into(),
                kind: EvidenceKind::SafeOwner,
                key: "0xeoa00000000000000000000000000000000000000".into(),
                strength: Strength::Medium,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
            Attestation {
                address: safe_b.into(),
                kind: EvidenceKind::SafeOwner,
                key: "0xeoa00000000000000000000000000000000000000".into(),
                strength: Strength::Medium,
                source: "test".into(),
                observed_block: 0,
                payload_json: None,
            },
        ];
        let out = render_markdown(&ReportInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &attestations,
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });
        assert!(
            out.contains("Shared-owner governance-control cluster"),
            "expected Safe-owner heading, got:\n{out}"
        );
    }

    #[test]
    fn renders_empty_clusters_gracefully() {
        let out = render_markdown(&ReportInputs {
            run: &synth_run(),
            clusters: &[],
            skipped: &[],
            attestations: &[],
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });
        assert!(out.contains("Identifiers analyzed: **0**"));
        assert!(out.contains("Inferred clusters: **0**"));
        assert!(out.contains("_No multi-address clusters in this run._"));
        // No Suspected Service Keys section when there are none.
        assert!(!out.contains("## Suspected Service Keys"));
    }

    #[test]
    fn truncates_top_clusters_with_summary_line() {
        // 12 multi-member clusters; report should show first 10 + "and 2 more".
        let mut clusters: Vec<ClusterReport> = (0..12)
            .map(|i| ClusterReport {
                cluster_id: format!("0x{:040x}", i + 1),
                addresses: vec![
                    format!("0x{:040x}", (i * 2) + 100),
                    format!("0x{:040x}", (i * 2) + 101),
                ],
                shared_evidence_keys: vec![],
            })
            .collect();
        // Sort by size desc, then cluster_id asc — matches Repo::clusters_for_run.
        clusters.sort_by(|a, b| {
            b.addresses
                .len()
                .cmp(&a.addresses.len())
                .then_with(|| a.cluster_id.cmp(&b.cluster_id))
        });

        let out = render_markdown(&ReportInputs {
            run: &synth_run(),
            clusters: &clusters,
            skipped: &[],
            attestations: &[],
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });

        // After the wording cleanup, clusters get evidence-aware
        // descriptors. With empty attestations, every cluster falls
        // back to "Evidence-supported cluster" — so we can't count by
        // a specific heading. We test the truncation summary
        // directly, which is the property under test.
        assert!(
            out.contains("and 2 more"),
            "expected 'and 2 more' summary line, got:\n{out}"
        );
        // Sanity: 10 of the 12 clusters should be rendered as
        // top-level sections (matches TOP_CLUSTERS_LIMIT).
        let detailed = out.matches("### ").count();
        assert_eq!(detailed, 10);
    }
}
