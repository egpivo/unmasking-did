//! Graphviz DOT export of a clustering run.
//!
//! Each cluster becomes a `subgraph cluster_<i>` (the `cluster_`
//! prefix is what triggers Graphviz's rounded-box rendering). Each
//! identifier is a node. Each per-pair-per-kind edge that **passed
//! the merge invariant** becomes a labelled edge in the top-level
//! graph — see `edges::passing_edges` for the per-pair reconstruction
//! that prevents transitive cluster membership from being shown as
//! direct evidence.
//!
//! Edge data is rebuilt from the `attestations` snapshot the caller
//! passes in. Important caveat: that cache is the *current* state,
//! not a snapshot at `run.run_id` — if `evidence` has been touched
//! since the run, the rendered graph may diverge from the persisted
//! cluster shape. Persisting per-pair edges per run would fix that
//! and is on the M3.5+ backlog (see `main.rs::run_report`).
//!
//! Rendering is deterministic: clusters sorted by `cluster_id`,
//! addresses sorted within each cluster, edges sorted by
//! `(src, dst, kind)` via the `BTreeMap` in `passing_edges`.
//! Identical inputs produce byte-identical DOT output.

use std::collections::HashMap;

use crate::evidence::{EvidenceKind, Strength};
use crate::linking::{ClusterReport, SkippedKey};
use crate::storage::ClusteringRunSummary;

use super::edges::{passing_edges, run_params, ClusterEdge};

pub struct DotInputs<'a> {
    pub run: &'a ClusteringRunSummary,
    pub clusters: &'a [ClusterReport],
    pub skipped: &'a [SkippedKey],
    pub attestations: &'a [crate::evidence::Attestation],
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
    // Reconstruct merge-passing edges from the current evidence
    // snapshot. This is the only correct basis for both edge
    // rendering and cluster-descriptor selection — using all
    // attestations on member addresses (the previous approach) would
    // overstate the cluster's evidence basis whenever any member
    // happened to carry an unrelated, unshared kind, and would draw
    // edges between transitively-connected pairs that did not
    // themselves pass the merge rule.
    let (min_evidence, fan_out_cap) = run_params(&input.run.params_json);
    let edges = passing_edges(input.attestations, input.clusters, min_evidence, fan_out_cap);

    // Index passing edges by cluster index so we can label each
    // cluster by its dominant *contributing* evidence kind.
    let mut clusters: Vec<&ClusterReport> = input.clusters.iter().collect();
    clusters.sort_by(|a, b| a.cluster_id.cmp(&b.cluster_id));

    let address_to_cluster: HashMap<&str, usize> = clusters
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

    for (i, cluster) in clusters.iter().enumerate() {
        out.push_str(&format!("    subgraph cluster_{i} {{\n"));
        let descriptor =
            cluster_descriptor(edges_by_cluster.get(&i).map(|v| v.as_slice()).unwrap_or(&[]));
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

    if !edges.is_empty() {
        out.push_str(
            "    // edges: per-pair-per-kind, restricted to pairs that passed\n    //        the merge invariant for this run (see report/edges.rs)\n",
        );
    }
    for edge in &edges {
        let label = edge_label(edge.kind, edge.strength, &edge.keys);
        out.push_str(&format!(
            "    \"{}\" -- \"{}\" [label=\"{}\"];\n",
            escape_id(edge.src),
            escape_id(edge.dst),
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

/// Pick a phrase for a cluster heading based on the strongest evidence
/// kind that **actually contributed merge-passing edges** within the
/// cluster — not on every kind any member happened to have. A cluster
/// merged purely via `safe_owner` will not be relabelled
/// "controller-level" just because one member separately carries an
/// unrelated `did_controller` attestation.
fn cluster_descriptor(edges: &[&ClusterEdge<'_>]) -> &'static str {
    if edges.is_empty() {
        return "evidence-supported cluster";
    }
    let max_strength = edges.iter().map(|e| e.strength).max();
    if max_strength == Some(Strength::Strong) {
        return "controller-level cluster";
    }
    if edges.iter().any(|e| e.kind == EvidenceKind::SafeOwner) {
        return "shared-owner cluster";
    }
    "evidence-supported cluster"
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
    use crate::evidence::{Attestation, Strength};

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
