use crate::linking::{ClusterReport, SkippedKey};
use crate::storage::ClusteringRunSummary;

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
    pub nakamoto: Option<u64>,
    pub gini: Option<f64>,
    pub nakamoto_threshold: f64,
}

pub fn render_markdown(input: &ReportInputs<'_>) -> String {
    let n_addresses: usize = input.clusters.iter().map(|c| c.addresses.len()).sum();
    let n_entities = input.clusters.len();

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
    s.push_str(&format!("- Addresses analyzed: **{n_addresses}**\n"));
    s.push_str(&format!("- Inferred entities: **{n_entities}**\n"));
    if n_entities > 0 {
        let ratio = n_addresses as f64 / n_entities as f64;
        s.push_str(&format!("- Identifiers per entity: **{ratio:.2}**\n"));
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

    s.push_str("## Top Clusters\n\n");
    let multi: Vec<&ClusterReport> = input
        .clusters
        .iter()
        .filter(|c| c.addresses.len() > 1)
        .collect();
    if multi.is_empty() {
        s.push_str("_No multi-address clusters in this run._\n\n");
    } else {
        for (i, cluster) in multi.iter().take(TOP_CLUSTERS_LIMIT).enumerate() {
            s.push_str(&format!(
                "### Cluster {} — `{}` ({} addresses)\n\n",
                i + 1,
                short_addr(&cluster.cluster_id),
                cluster.addresses.len()
            ));
            if !cluster.shared_evidence_keys.is_empty() {
                s.push_str("Connected via:\n");
                for k in &cluster.shared_evidence_keys {
                    s.push_str(&format!("- `{k}`\n"));
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
                "_… and {} more multi-address cluster(s) — full list in `entity_clusters` table._\n\n",
                total - TOP_CLUSTERS_LIMIT
            ));
        }
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

        let out = render_markdown(&ReportInputs {
            run: &run,
            clusters: &clusters,
            skipped: &skipped,
            nakamoto: Some(1),
            gini: Some(0.333),
            nakamoto_threshold: 0.5,
        });

        // Header
        assert!(out.contains("# unmasking-did Report"));
        assert!(out.contains("`run-test-123`"));
        // Summary numbers (3 total addresses across the two clusters)
        assert!(out.contains("Addresses analyzed: **3**"));
        assert!(out.contains("Inferred entities: **2**"));
        assert!(out.contains("Nakamoto coefficient"));
        assert!(out.contains("Gini coefficient: **0.333**"));
        // Multi-member cluster section
        assert!(out.contains("Cluster 1 —"));
        assert!(out.contains("(2 addresses)"));
        assert!(out.contains("twitter:joseph"));
        // Singleton cluster is filtered out of the Top Clusters detail
        assert!(!out.contains("Cluster 2 —"));
        // Suspected service keys table
        assert!(out.contains("## Suspected Service Keys"));
        assert!(out.contains("`funded_by`"));
        assert!(out.contains("247"));
        // Reproducibility footer
        assert!(out.contains("## Reproducibility"));
        assert!(out.contains("min(address)"));
    }

    #[test]
    fn renders_empty_clusters_gracefully() {
        let out = render_markdown(&ReportInputs {
            run: &synth_run(),
            clusters: &[],
            skipped: &[],
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });
        assert!(out.contains("Addresses analyzed: **0**"));
        assert!(out.contains("Inferred entities: **0**"));
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
            nakamoto: None,
            gini: None,
            nakamoto_threshold: 0.5,
        });

        assert!(out.contains("Cluster 10 —"));
        assert!(!out.contains("Cluster 11 —"));
        assert!(out.contains("and 2 more"));
    }
}
