//! Reconstruct per-pair-per-kind merge-passing edges at render time.
//!
//! Cluster membership is transitive: if A merges with B via STRONG
//! evidence and B merges with C via STRONG evidence, A and C land in
//! the same connected component even when A and C share no
//! merge-passing evidence between themselves. The renderers (markdown
//! and DOT) must NOT show such "transitive-only" pairs as if they were
//! direct evidence — that would overclaim the cluster's basis.
//!
//! The helper here re-applies the same merge invariant
//! `linking::features::build_clusters` uses, but on the public
//! `attestations` snapshot the renderer already has access to. It
//! returns the set of edges that *actually* passed: each
//! (src, dst, kind) pair gets one entry with its supporting keys and
//! max strength. Renderers should only draw edges / derive cluster
//! descriptors from this set.
//!
//! See the inline note in `main.rs::run_report` for the broader
//! caveat: this reflects the *current* `evidence` state, not a
//! snapshot at `run_id`. Persisting per-pair edges per run would let
//! us replay historical visualizations exactly; that's still on the
//! M3.5+ backlog.

use std::collections::{BTreeMap, HashMap};

use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::ClusterReport;

/// Default fallback when `params_json` is missing or malformed.
/// Matches the constant in `linking::features`.
const DEFAULT_MIN_EVIDENCE: usize = 1;

/// Default fan-out cap matching `linking::features::FAN_OUT_CAP`.
const DEFAULT_FAN_OUT_CAP: usize = 50;

/// One per-pair-per-kind edge that passed the merge invariant.
/// Borrows lifetimes from the `attestations` slice the caller passed
/// in, so no string copies happen on the hot path.
#[derive(Debug)]
pub struct ClusterEdge<'a> {
    /// Lexicographically smaller endpoint (so `(src, dst)` is a
    /// canonical, undirected pair key).
    pub src: &'a str,
    pub dst: &'a str,
    pub kind: EvidenceKind,
    /// Distinct evidence keys of this `kind` shared by `src` and
    /// `dst`. For `safe_owner` this is the list of shared owner
    /// EOAs; for `funded_by` it's the shared funders; etc.
    pub keys: Vec<&'a str>,
    /// Max strength observed across all (kind, key) groups
    /// contributing to this pair-and-kind edge. With our current
    /// extractors a kind has a fixed strength, but rendering uses
    /// max so future per-key strength variation just works.
    pub strength: Strength,
}

/// Read `min_evidence` and `fan_out_cap` out of the run's parameters
/// JSON. Falls back to module defaults when fields are absent —
/// older runs predating the schema, or downgraded JSON, won't crash
/// the renderer.
pub fn run_params(params_json: &str) -> (usize, usize) {
    let parsed: serde_json::Value = serde_json::from_str(params_json).unwrap_or_default();
    let min_evidence = parsed
        .get("min_evidence")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MIN_EVIDENCE);
    let fan_out_cap = parsed
        .get("fan_out_cap")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_FAN_OUT_CAP);
    (min_evidence, fan_out_cap)
}

/// Reconstruct the merge-passing edges between addresses inside the
/// given clusters. Returns one entry per `(src, dst, kind)` triple
/// that satisfied either the strong-alone bypass or the
/// `min_evidence + ≥MEDIUM` rule. Pairs whose membership in the same
/// cluster is purely transitive — no direct merge-passing evidence —
/// are absent from the output.
///
/// Both filters apply:
///   1. **Same-cluster**: skip pairs whose addresses are not in the
///      same cluster (a cross-cluster edge would mean the link layer
///      rejected it; renderers should not show it).
///   2. **Merge invariant**: of the same-cluster pairs, only those
///      whose total `(count, max_strength)` passes are emitted.
pub fn passing_edges<'a>(
    attestations: &'a [Attestation],
    clusters: &[ClusterReport],
    min_evidence: usize,
    fan_out_cap: usize,
) -> Vec<ClusterEdge<'a>> {
    let address_to_cluster: HashMap<&str, usize> = clusters
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.addresses.iter().map(move |a| (a.as_str(), i)))
        .collect();

    // Group attestations by `(kind, key)`. A group of size N becomes
    // (N-choose-2) candidate edges, all sharing the same key — the
    // same shape link-time clustering uses.
    let mut by_kind_key: HashMap<(EvidenceKind, &'a str), Vec<&'a Attestation>> = HashMap::new();
    for att in attestations {
        by_kind_key
            .entry((att.kind, att.key.as_str()))
            .or_default()
            .push(att);
    }

    // Pass 1 — per-pair totals across all kinds. Iterates RAW
    // attestation pairs `(atts[i], atts[j])` and matches the linker
    // (`linking::features::build_clusters`) exactly.
    //
    // The linker treats every raw row pairing across two distinct
    // addresses as a separate graph edge. So if address `A` has
    // two `(kind, key)` rows from different sources and address `B`
    // has one, the linker emits 2 cross edges for that single shared
    // key — `count(A, B) == 2` from one key alone, enough to clear
    // `min_evidence = 2` on its own.
    //
    // That semantic is questionable — a single shared key gets
    // weighted by source multiplicity rather than mutual evidence —
    // but reconstruction MUST match the production link path, or
    // the renderers disagree with the persisted cluster shape. A
    // semantic cleanup (dedup per `(address, kind, key)` in the
    // linker as well) is a deliberate behavior change and out of
    // scope for this branch.
    //
    // Per-pair strength is `max(atts[i].strength, atts[j].strength)`,
    // computed per raw pair — addresses MAY appear in multiple rows
    // with different strengths, and the strongest cross-row pairing
    // wins (mirrors the linker, regression-tested below).
    type PairKey<'a> = (&'a str, &'a str);
    let mut pair_totals: HashMap<PairKey<'a>, (usize, Strength)> = HashMap::new();
    for ((_kind, _key), atts) in &by_kind_key {
        if atts.len() < 2 || atts.len() > fan_out_cap {
            continue;
        }
        for i in 0..atts.len() {
            for j in (i + 1)..atts.len() {
                let a_i = atts[i].address.as_str();
                let a_j = atts[j].address.as_str();
                if a_i == a_j {
                    // Self-loop — the linker emits these too, but
                    // `uf.union(a, a)` is a no-op so they don't
                    // contribute to clustering. Drop them here so
                    // they don't flood the visualization.
                    continue;
                }
                let (lo, hi) = canonical_pair(a_i, a_j);
                let pair_strength = atts[i].strength.max(atts[j].strength);
                let entry = pair_totals.entry((lo, hi)).or_insert((0, Strength::Weak));
                entry.0 += 1;
                if pair_strength > entry.1 {
                    entry.1 = pair_strength;
                }
            }
        }
    }

    // Pass 2 — emit per-(pair, kind) aggregates, filtered by the
    // merge invariant + same-cluster guard. Same raw-row iteration
    // as Pass 1; deduping happens at the `keys` field level so the
    // rendered label says "3 shared owners" rather than "3 sources
    // for owner X".
    struct Builder<'a> {
        keys: Vec<&'a str>,
        strength: Strength,
    }
    let mut by_pair_kind: BTreeMap<(&'a str, &'a str, EvidenceKind), Builder<'a>> = BTreeMap::new();
    let needed = min_evidence.max(1);

    for ((kind, key), atts) in by_kind_key {
        if atts.len() < 2 || atts.len() > fan_out_cap {
            continue;
        }
        for i in 0..atts.len() {
            for j in (i + 1)..atts.len() {
                let a_i = atts[i].address.as_str();
                let a_j = atts[j].address.as_str();
                if a_i == a_j {
                    continue;
                }
                let (lo, hi) = canonical_pair(a_i, a_j);

                let (Some(&ca), Some(&cb)) =
                    (address_to_cluster.get(lo), address_to_cluster.get(hi))
                else {
                    continue;
                };
                if ca != cb {
                    continue;
                }

                let &(count, max_strength) =
                    pair_totals.get(&(lo, hi)).unwrap_or(&(0, Strength::Weak));
                let passed = max_strength == Strength::Strong
                    || (count >= needed && max_strength >= Strength::Medium);
                if !passed {
                    continue;
                }

                let pair_strength = atts[i].strength.max(atts[j].strength);
                let entry = by_pair_kind
                    .entry((lo, hi, kind))
                    .or_insert_with(|| Builder {
                        keys: Vec::new(),
                        strength: pair_strength,
                    });
                if !entry.keys.contains(&key) {
                    entry.keys.push(key);
                }
                if pair_strength > entry.strength {
                    entry.strength = pair_strength;
                }
            }
        }
    }

    by_pair_kind
        .into_iter()
        .map(|((src, dst, kind), b)| ClusterEdge {
            src,
            dst,
            kind,
            keys: b.keys,
            strength: b.strength,
        })
        .collect()
}

fn canonical_pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn cluster(addresses: &[&str]) -> ClusterReport {
        let mut sorted: Vec<String> = addresses.iter().map(|s| s.to_string()).collect();
        sorted.sort();
        ClusterReport {
            cluster_id: sorted[0].clone(),
            addresses: sorted,
            shared_evidence_keys: Vec::new(),
        }
    }

    #[test]
    fn transitive_pair_without_merge_passing_evidence_is_excluded() {
        // P1 regression. min_evidence=2.
        // A-B share two safe_owners → merge passes.
        // B-C share two safe_owners → merge passes.
        // A-C share ONE funded_by → does NOT pass min_evidence=2.
        // All three end up in one cluster by transitivity, but the
        // A-C pair must not produce an edge in the rendered graph.
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let c = "0xcccccccccccccccccccccccccccccccccccccccc";
        let owner1 = "0xeeee00000000000000000000000000000000eee1";
        let owner2 = "0xeeee00000000000000000000000000000000eee2";
        let owner3 = "0xeeee00000000000000000000000000000000eee3";
        let owner4 = "0xeeee00000000000000000000000000000000eee4";
        let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";

        let attestations = vec![
            // A-B share owner1 + owner2 (count=2 in pair_totals)
            att(a, EvidenceKind::SafeOwner, owner1, Strength::Medium),
            att(b, EvidenceKind::SafeOwner, owner1, Strength::Medium),
            att(a, EvidenceKind::SafeOwner, owner2, Strength::Medium),
            att(b, EvidenceKind::SafeOwner, owner2, Strength::Medium),
            // B-C share owner3 + owner4 (count=2)
            att(b, EvidenceKind::SafeOwner, owner3, Strength::Medium),
            att(c, EvidenceKind::SafeOwner, owner3, Strength::Medium),
            att(b, EvidenceKind::SafeOwner, owner4, Strength::Medium),
            att(c, EvidenceKind::SafeOwner, owner4, Strength::Medium),
            // A-C share ONE funder only (count=1, < min_evidence=2,
            // medium strength — must NOT pass the merge rule)
            att(a, EvidenceKind::FundedBy, funder, Strength::Medium),
            att(c, EvidenceKind::FundedBy, funder, Strength::Medium),
        ];
        let clusters = vec![cluster(&[a, b, c])];

        let edges = passing_edges(&attestations, &clusters, 2, 50);
        // Must include the A-B and B-C edges, must NOT include A-C.
        let pair_set: std::collections::HashSet<(&str, &str)> =
            edges.iter().map(|e| (e.src, e.dst)).collect();
        assert!(pair_set.contains(&(a, b)), "A-B edge expected");
        assert!(pair_set.contains(&(b, c)), "B-C edge expected");
        assert!(
            !pair_set.contains(&(a, c)),
            "A-C must not appear: shared 1 funded_by < min_evidence=2"
        );
    }

    #[test]
    fn unrelated_kind_on_one_member_does_not_show_up() {
        // P2 regression. Cluster {safe_a, safe_b} via shared
        // safe_owner. safe_a *separately* has a did_controller
        // attestation pointing at a controller that safe_b does NOT
        // share. The merge-passing edge between safe_a and safe_b
        // is purely safe_owner — there must be no
        // (safe_a, safe_b, did_controller) edge in the output, and
        // any cluster descriptor derived from these edges should not
        // claim "controller-level" status.
        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner = "0xeeee00000000000000000000000000000000eee0";
        let unshared_controller = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";

        let attestations = vec![
            att(safe_a, EvidenceKind::SafeOwner, owner, Strength::Medium),
            att(safe_b, EvidenceKind::SafeOwner, owner, Strength::Medium),
            // safe_a alone — no matching B side, so no edge.
            att(
                safe_a,
                EvidenceKind::DidController,
                unshared_controller,
                Strength::Strong,
            ),
        ];
        let clusters = vec![cluster(&[safe_a, safe_b])];

        let edges = passing_edges(&attestations, &clusters, 1, 50);
        assert_eq!(edges.len(), 1, "expected exactly one merge-passing edge");
        assert_eq!(edges[0].kind, EvidenceKind::SafeOwner);
        assert!(
            edges.iter().all(|e| e.kind != EvidenceKind::DidController),
            "DidController must not appear: no shared controller between members"
        );
    }

    #[test]
    fn pair_strength_uses_per_endpoint_max_not_first_attestation() {
        // Regression: before this fix, Pass 1 and Pass 2 both took
        // `atts[0].strength` as the strength of every edge in the
        // (kind, key) group. When attestations within the same group
        // had mixed strengths (today via direct manual injection,
        // tomorrow once a weighted `funded_by` variant ships), the
        // reconstructed merge decision depended on row order — pairs
        // that the linker would merge via the strong-alone bypass
        // could disappear from DOT/Markdown depending on which
        // attestation happened to be first in the cache scan.
        //
        // The linker's actual edge strength is
        //   max(atts[i].strength, atts[j].strength)
        // for each pair. The reconstruction must match.
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let key = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";

        // Order chosen so that `atts[0]` is WEAK — the buggy code
        // would conclude the edge is WEAK and skip it. With the fix
        // the per-endpoint max(WEAK, STRONG) = STRONG and the pair
        // merges via the strong-alone bypass even at min_evidence=2.
        let attestations = vec![
            att(b, EvidenceKind::DidController, key, Strength::Weak),
            att(a, EvidenceKind::DidController, key, Strength::Strong),
        ];
        let clusters = vec![cluster(&[a, b])];

        let edges = passing_edges(&attestations, &clusters, 2, 50);
        assert_eq!(
            edges.len(),
            1,
            "mixed-strength pair must produce one edge under strong-alone"
        );
        assert_eq!(edges[0].kind, EvidenceKind::DidController);
        assert_eq!(
            edges[0].strength,
            Strength::Strong,
            "edge strength must be max(endpoints), not atts[0]"
        );

        // Reverse the row order — must produce identical output.
        let attestations_reversed: Vec<_> = attestations.iter().cloned().rev().collect();
        let edges_reversed = passing_edges(&attestations_reversed, &clusters, 2, 50);
        assert_eq!(
            edges_reversed.len(),
            edges.len(),
            "row order must not affect merge reconstruction"
        );
        assert_eq!(edges_reversed[0].strength, edges[0].strength);
    }

    #[test]
    fn per_source_duplication_matches_linker_count_inflation() {
        // Regression: when one address has multiple attestations for
        // the same `(kind, key)` from different sources, the linker
        // emits one cross-pair edge per raw-row pairing, so a single
        // shared key can satisfy `min_evidence > 1` on its own.
        //
        // Setup: A has TWO `funded_by` rows for the same funder
        // (different sources — legal under the evidence UNIQUE
        // constraint, which includes `source`). B has one. With
        // `min_evidence = 2`, the linker computes
        // `count(A, B) = m * n = 2 * 1 = 2` and merges. Reconstruction
        // must agree, otherwise persisted clusters would silently
        // disagree with rendered DOT/Markdown.
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";

        let mut atts = vec![
            att(a, EvidenceKind::FundedBy, funder, Strength::Medium),
            att(a, EvidenceKind::FundedBy, funder, Strength::Medium),
            att(b, EvidenceKind::FundedBy, funder, Strength::Medium),
        ];
        // Override `source` so the three rows are actually distinct
        // under UNIQUE(address, kind, key, source).
        atts[0].source = "alchemy_getAssetTransfers@1".into();
        atts[1].source = "alchemy_getAssetTransfers@2".into();
        atts[2].source = "alchemy_getAssetTransfers@3".into();

        let clusters = vec![cluster(&[a, b])];
        let edges = passing_edges(&atts, &clusters, 2, 50);
        assert_eq!(
            edges.len(),
            1,
            "linker counts m*n cross edges from per-source duplication; reconstruction must agree"
        );
        assert_eq!(edges[0].kind, EvidenceKind::FundedBy);
    }

    #[test]
    fn weak_alone_is_filtered_even_with_high_count() {
        // Defensive test: 5 weak edges between A-B (count=5) must
        // not pass even with min_evidence=2, because max_strength is
        // Weak which is below the MEDIUM floor in the AND branch.
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let attestations: Vec<_> = (0..5)
            .flat_map(|i| {
                let key = format!("0xweak0000000000000000000000000000000000{i:02}");
                vec![
                    att(a, EvidenceKind::FundedBy, &key, Strength::Weak),
                    att(b, EvidenceKind::FundedBy, &key, Strength::Weak),
                ]
            })
            .collect();
        let clusters = vec![cluster(&[a, b])];

        let edges = passing_edges(&attestations, &clusters, 2, 50);
        assert!(
            edges.is_empty(),
            "weak-only must never produce passing edges"
        );
    }
}
