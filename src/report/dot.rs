//! Graphviz DOT export of a clustering run.
//!
//! Each cluster becomes a `subgraph cluster_<i>` (the `cluster_`
//! prefix is what triggers Graphviz's rounded-box rendering). Each
//! identifier is a node. Each per-pair-per-kind aggregated edge is
//! a labelled edge in the top-level graph.
//!
//! Edge data is rebuilt from the `attestations` snapshot the caller
//! passes in; this is the same set of evidence rows the markdown
//! renderer uses, so both outputs reflect the same view of the
//! `evidence` cache. Important caveat: that cache is the *current*
//! state, not a snapshot at `run.run_id` — if `evidence` has been
//! touched since the run, the rendered graph may diverge from the
//! persisted cluster shape. Persisting per-pair edges per run would
//! fix that and is on the M3.5+ backlog (see the calling note in
//! `main.rs::run_report`).
//!
//! Rendering is deterministic: clusters sorted by `cluster_id`,
//! addresses sorted within each cluster, edges sorted by
//! `(src, dst, kind, key)`. Identical inputs produce byte-identical
//! DOT output, so the export plays nicely with snapshot tests and
//! version control.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::evidence::{Attestation, EvidenceKind};
use crate::linking::{ClusterReport, SkippedKey};
use crate::storage::ClusteringRunSummary;

/// `(kind, key)` groups exceeding this many identifiers are treated
/// as service-like (CEX hot wallets, bridges, batch distributors,
/// faucets) and not rendered as clustering edges. Mirrors the
/// `FAN_OUT_CAP` in `linking::features` so the visualization shows
/// the same edges the merge invariant actually saw.
const FAN_OUT_CAP: usize = 50;

pub struct DotInputs<'a> {
    pub run: &'a ClusteringRunSummary,
    pub clusters: &'a [ClusterReport],
    pub skipped: &'a [SkippedKey],
    pub attestations: &'a [Attestation],
}

pub fn render_dot(input: &DotInputs<'_>) -> String {
    let mut out = String::new();
    out.push_str("graph unmasking_did {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    fontname=\"monospace\";\n");
    out.push_str("    node [shape=box, fontname=\"monospace\"];\n");
    out.push_str("    edge [fontname=\"monospace\"];\n\n");

    // Header comment with the run id + parameters so a reader of the
    // raw .dot file can trace it back to the SQLite tables.
    out.push_str(&format!(
        "    // run_id    : {}\n",
        escape_comment(&input.run.run_id)
    ));
    out.push_str(&format!(
        "    // started_at: {}\n",
        escape_comment(&input.run.started_at)
    ));
    out.push_str(&format!(
        "    // params    : {}\n\n",
        escape_comment(input.run.params_json.trim())
    ));

    // -- clusters --------------------------------------------------
    let mut clusters: Vec<&ClusterReport> = input.clusters.iter().collect();
    clusters.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));

    let address_to_cluster: HashMap<&str, usize> = clusters
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.addresses.iter().map(move |a| (a.as_str(), i)))
        .collect();

    let kinds_per_address = group_kinds_by_address(input.attestations);

    for (i, cluster) in clusters.iter().enumerate() {
        out.push_str(&format!("    subgraph cluster_{i} {{\n"));
        let descriptor = cluster_descriptor(cluster, &kinds_per_address);
        out.push_str(&format!(
            "        label=\"{} \\n {} ({} identifier{})\";\n",
            descriptor,
            escape_label(&short_addr(&cluster.cluster_id)),
            cluster.addresses.len(),
            if cluster.addresses.len() == 1 { "" } else { "s" }
        ));
        out.push_str("        style=rounded;\n");
        out.push_str("        color=\"#888888\";\n");
        let mut sorted_addrs: Vec<&String> = cluster.addresses.iter().collect();
        sorted_addrs.sort();
        for addr in sorted_addrs {
            out.push_str(&format!(
                "        \"{}\" [label=\"{}\"];\n",
                escape_id(addr),
                escape_label(&short_addr(addr))
            ));
        }
        out.push_str("    }\n\n");
    }

    // -- edges ----------------------------------------------------
    // Aggregate attestations into per-pair-per-kind edges. We only
    // emit edges between identifiers that share a cluster — edges
    // crossing cluster boundaries would imply the merge invariant
    // rejected them, and showing them here would clutter the graph
    // with non-clustering signal. The `Suspected service keys` table
    // (see the markdown report) is the right place to surface what
    // got dropped.
    let mut by_kind_key: HashMap<(EvidenceKind, &str), Vec<&Attestation>> = HashMap::new();
    for att in input.attestations {
        by_kind_key
            .entry((att.kind, att.key.as_str()))
            .or_default()
            .push(att);
    }

    type PairKey<'a> = (&'a str, &'a str, EvidenceKind);
    struct Aggregate<'a> {
        keys: Vec<&'a str>,
        strength: crate::evidence::Strength,
    }
    let mut by_pair: BTreeMap<PairKey<'_>, Aggregate<'_>> = BTreeMap::new();

    for ((kind, key), atts) in by_kind_key {
        if atts.len() < 2 || atts.len() > FAN_OUT_CAP {
            continue;
        }
        // De-duplicate addresses inside a single (kind, key) group so
        // a self-loop never lands in the visualization, and emit one
        // edge per unordered pair of distinct addresses.
        let mut addrs: Vec<&str> = atts.iter().map(|a| a.address.as_str()).collect();
        addrs.sort();
        addrs.dedup();
        if addrs.len() < 2 {
            continue;
        }
        for i in 0..addrs.len() {
            for j in (i + 1)..addrs.len() {
                let a = addrs[i];
                let b = addrs[j];
                // Same-cluster filter: skip if the pair was rejected
                // by the merge invariant.
                let (Some(&ca), Some(&cb)) =
                    (address_to_cluster.get(a), address_to_cluster.get(b))
                else {
                    continue;
                };
                if ca != cb {
                    continue;
                }
                let pair_kind = (a, b, kind);
                let entry = by_pair.entry(pair_kind).or_insert(Aggregate {
                    keys: Vec::new(),
                    strength: atts[0].strength,
                });
                if !entry.keys.contains(&key) {
                    entry.keys.push(key);
                }
                if atts[0].strength > entry.strength {
                    entry.strength = atts[0].strength;
                }
            }
        }
    }

    if !by_pair.is_empty() {
        out.push_str("    // edges: per-pair-per-kind aggregated from `evidence`\n");
    }
    for ((src, dst, kind), agg) in &by_pair {
        let label = edge_label(*kind, agg.strength, &agg.keys);
        out.push_str(&format!(
            "    \"{}\" -- \"{}\" [label=\"{}\"];\n",
            escape_id(src),
            escape_id(dst),
            escape_label(&label)
        ));
    }

    if !input.skipped.is_empty() {
        out.push_str("\n    // suspected service keys (excluded from edges by fan-out cap):\n");
        for sk in input.skipped {
            out.push_str(&format!(
                "    //   {} -> {} (fan-out {})\n",
                escape_comment(&sk.kind),
                escape_comment(&sk.key),
                sk.fan_out
            ));
        }
    }

    out.push_str("}\n");
    out
}

fn group_kinds_by_address(
    attestations: &[Attestation],
) -> HashMap<String, HashSet<EvidenceKind>> {
    let mut by_addr: HashMap<String, HashSet<EvidenceKind>> = HashMap::new();
    for a in attestations {
        by_addr.entry(a.address.clone()).or_default().insert(a.kind);
    }
    by_addr
}

fn cluster_descriptor(
    cluster: &ClusterReport,
    kinds_per_address: &HashMap<String, HashSet<EvidenceKind>>,
) -> &'static str {
    let mut all_kinds: HashSet<EvidenceKind> = HashSet::new();
    for addr in &cluster.addresses {
        if let Some(ks) = kinds_per_address.get(addr) {
            all_kinds.extend(ks.iter().copied());
        }
    }
    if all_kinds.contains(&EvidenceKind::DidController) {
        "controller-level cluster"
    } else if all_kinds.contains(&EvidenceKind::SafeOwner) {
        "shared-owner cluster"
    } else {
        "evidence-supported cluster"
    }
}

fn edge_label(kind: EvidenceKind, strength: crate::evidence::Strength, keys: &[&str]) -> String {
    let strength_lc = match strength {
        crate::evidence::Strength::Strong => "strong",
        crate::evidence::Strength::Medium => "medium",
        crate::evidence::Strength::Weak => "weak",
    };
    if keys.len() == 1 {
        format!("{} | {} | {}", kind.as_str(), strength_lc, short_addr(keys[0]))
    } else {
        let noun = match kind {
            EvidenceKind::SafeOwner => "shared owners",
            EvidenceKind::FundedBy => "shared funders",
            EvidenceKind::EnsHandle => "shared handles",
            EvidenceKind::DidController => "shared controllers",
        };
        format!("{} | {} | {} {}", kind.as_str(), strength_lc, keys.len(), noun)
    }
}

fn short_addr(addr: &str) -> String {
    if addr.len() < 12 {
        addr.to_string()
    } else {
        format!("{}…{}", &addr[..6], &addr[addr.len() - 4..])
    }
}

fn escape_id(s: &str) -> String {
    s.replace('"', "\\\"")
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_comment(s: &str) -> String {
    // Strip newlines so a multi-line value can't break the `//`
    // single-line-comment context.
    s.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Strength;

    fn synth_run() -> ClusteringRunSummary {
        ClusteringRunSummary {
            run_id: "run-test-123".to_string(),
            params_json: r#"{"min_evidence":1}"#.to_string(),
            started_at: "2026-05-01 00:00:00".to_string(),
        }
    }

    fn att(addr: &str, kind: EvidenceKind, key: &str, strength: Strength) -> Attestation {
        Attestation {
            address: addr.to_string(),
            kind,
            key: key.to_string(),
            strength,
            source: "test".to_string(),
            observed_block: 0,
            payload_json: None,
        }
    }

    #[test]
    fn renders_singleton_without_panic_or_edges() {
        let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let cluster = ClusterReport {
            cluster_id: alice.clone(),
            addresses: vec![alice.clone()],
            shared_evidence_keys: vec![],
        };
        let out = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &[],
        });
        assert!(out.starts_with("graph unmasking_did {"));
        assert!(out.contains(&alice));
        // Subgraph "1 identifier" (singular) + no edges.
        assert!(out.contains("(1 identifier)"));
        assert!(!out.contains(" -- "));
    }

    #[test]
    fn renders_strong_did_controller_edge_with_kind_strength_key() {
        let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let ctrl = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        let cluster = ClusterReport {
            cluster_id: alice.into(),
            addresses: vec![alice.into(), bob.into()],
            shared_evidence_keys: vec![ctrl.into()],
        };
        let attestations = vec![
            att(alice, EvidenceKind::DidController, ctrl, Strength::Strong),
            att(bob, EvidenceKind::DidController, ctrl, Strength::Strong),
        ];
        let out = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &attestations,
        });
        // Edge label format: "kind | strength | <short-key>"
        assert!(out.contains("did_controller | strong | 0xc0c0…c0c0"));
        // Cluster descriptor reflects the strongest evidence kind.
        assert!(out.contains("controller-level cluster"));
        // Both endpoints declared as nodes inside the subgraph.
        assert!(out.contains(alice));
        assert!(out.contains(bob));
    }

    #[test]
    fn aggregates_multiple_safe_owner_keys_into_count_label() {
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owners = [
            "0xeoa1000000000000000000000000000000000000",
            "0xeoa2000000000000000000000000000000000000",
            "0xeoa3000000000000000000000000000000000000",
        ];
        let cluster = ClusterReport {
            cluster_id: safe_a.into(),
            addresses: vec![safe_a.into(), safe_b.into()],
            shared_evidence_keys: owners.iter().map(|s| s.to_string()).collect(),
        };
        let mut attestations = Vec::new();
        for owner in owners {
            attestations.push(att(safe_a, EvidenceKind::SafeOwner, owner, Strength::Medium));
            attestations.push(att(safe_b, EvidenceKind::SafeOwner, owner, Strength::Medium));
        }
        let out = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &attestations,
        });
        // Three shared owners between (safe_a, safe_b) collapse into
        // one labelled edge with the count and kind-specific noun.
        assert!(
            out.contains("safe_owner | medium | 3 shared owners"),
            "expected aggregated edge label, got:\n{out}"
        );
        // And exactly one edge line — not three.
        assert_eq!(out.matches(" -- ").count(), 1);
    }

    #[test]
    fn output_is_byte_deterministic_across_input_orderings() {
        let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let ctrl = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        let cluster = ClusterReport {
            cluster_id: alice.into(),
            addresses: vec![alice.into(), bob.into()],
            shared_evidence_keys: vec![ctrl.into()],
        };
        let atts_forward = vec![
            att(alice, EvidenceKind::DidController, ctrl, Strength::Strong),
            att(bob, EvidenceKind::DidController, ctrl, Strength::Strong),
        ];
        let atts_reverse: Vec<_> = atts_forward.iter().cloned().rev().collect();

        let cluster_with_reversed_addrs = ClusterReport {
            cluster_id: alice.into(),
            addresses: vec![bob.into(), alice.into()],
            shared_evidence_keys: vec![ctrl.into()],
        };

        let a = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster],
            skipped: &[],
            attestations: &atts_forward,
        });
        let b = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster_with_reversed_addrs],
            skipped: &[],
            attestations: &atts_reverse,
        });
        assert_eq!(a, b, "DOT output must be deterministic across input orderings");
    }

    #[test]
    fn cross_cluster_edges_are_suppressed() {
        // Two clusters of one identifier each, but both addresses
        // happen to share an attestation key (e.g. they were both
        // funded by the same source under min_evidence=2 so neither
        // pair count crossed the threshold). The DOT view should
        // NOT draw an edge: the merge invariant rejected it, and
        // showing it here would mislead.
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";
        let cluster_a = ClusterReport {
            cluster_id: a.into(),
            addresses: vec![a.into()],
            shared_evidence_keys: vec![],
        };
        let cluster_b = ClusterReport {
            cluster_id: b.into(),
            addresses: vec![b.into()],
            shared_evidence_keys: vec![],
        };
        let atts = vec![
            att(a, EvidenceKind::FundedBy, funder, Strength::Medium),
            att(b, EvidenceKind::FundedBy, funder, Strength::Medium),
        ];
        let out = render_dot(&DotInputs {
            run: &synth_run(),
            clusters: &[cluster_a, cluster_b],
            skipped: &[],
            attestations: &atts,
        });
        assert!(
            !out.contains(" -- "),
            "cross-cluster edges must be suppressed, got:\n{out}"
        );
    }
}
