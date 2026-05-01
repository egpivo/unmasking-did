//! End-to-end DOT export: ingest evidence → link_and_persist →
//! re-read attestations → render Graphviz DOT.
//!
//! The renderer's own properties (label format, deterministic ordering,
//! cross-cluster suppression, singleton handling) are unit-tested
//! inside `src/report/dot.rs`. This integration test only exists to
//! lock the contract between the renderer and the storage layer:
//! `Repo::attestations_for` returns rows in the shape `render_dot`
//! expects, and the SQLite round-trip preserves enough information
//! to label the resulting graph.

use unmasking_did::alchemy::Transfer;
use unmasking_did::did::DidDocument;
use unmasking_did::linking::link_and_persist;
use unmasking_did::report::{render_dot, DotInputs};
use unmasking_did::safe::SafeOwner;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn transfer(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
    Transfer {
        from_addr: from.into(),
        to_addr: to.into(),
        value: Some("1".into()),
        block_num: Some(block),
        tx_hash: Some(tx.into()),
        asset: Some("ETH".into()),
    }
}

#[tokio::test]
async fn dot_export_round_trip_through_persisted_run() {
    // Two Safes sharing one EOA owner (medium evidence, count=1, so
    // this would NOT merge under min_evidence=2 alone) PLUS a shared
    // DID controller (strong evidence, strong-alone bypass merges
    // them regardless). The DOT output should:
    //   - declare both Safe addresses as nodes
    //   - render an edge with `did_controller | strong` label
    //   - render an edge with `safe_owner | medium` label
    //   - pick "controller-level cluster" as the heading because
    //     STRONG is present
    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let owner = "0xeeee0000000000000000000000000000000000ee";
    let controller = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";

    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_a.into(),
        owner_address: owner.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_b.into(),
        owner_address: owner.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_did_document(&DidDocument {
        did: format!("did:ethr:{safe_a}"),
        subject_address: safe_a.into(),
        controller: controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_did_document(&DidDocument {
        did: format!("did:ethr:{safe_b}"),
        subject_address: safe_b.into(),
        controller: controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    let (run_id, _) = link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();

    let run = repo.latest_clustering_run().await.unwrap().unwrap();
    assert_eq!(run.run_id, run_id);

    let clusters = repo.clusters_for_run(&run.run_id).await.unwrap();
    let skipped = repo.suspected_keys_for_run(&run.run_id).await.unwrap();
    let cluster_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let attestations = repo.attestations_for(&cluster_addresses).await.unwrap();

    let dot = render_dot(&DotInputs {
        run: &run,
        clusters: &clusters,
        skipped: &skipped,
        attestations: &attestations,
    });

    // Header
    assert!(dot.starts_with("graph unmasking_did {"));
    assert!(dot.contains(&run.run_id), "DOT must include run_id in audit comment");
    // Both Safes appear as nodes (lowercased internally).
    assert!(dot.contains(safe_a));
    assert!(dot.contains(safe_b));
    // Cluster heading reflects strongest kind present.
    assert!(
        dot.contains("controller-level cluster"),
        "expected controller-level descriptor; got:\n{dot}"
    );
    // Edge labels include kind + strength.
    assert!(
        dot.contains("did_controller | strong"),
        "expected did_controller edge label; got:\n{dot}"
    );
    assert!(
        dot.contains("safe_owner | medium"),
        "expected safe_owner edge label; got:\n{dot}"
    );
    // No singletons in this scenario; both Safes merged into one
    // cluster, so we get one edge per kind = exactly two ` -- ` lines.
    assert_eq!(
        dot.matches(" -- ").count(),
        2,
        "expected exactly two edges (one per evidence kind), got:\n{dot}"
    );
}

#[tokio::test]
async fn dot_does_not_render_transitively_merged_pairs_as_evidence() {
    // P1 regression — agent finding 2026-05-01.
    //
    // Setup with `min_evidence = 2`:
    //   * A and B each have a did:ethr DID controlled by ctrl_ab —
    //     STRONG evidence, single edge merges A-B via the
    //     strong-alone bypass.
    //   * B and C each have a did:ethr DID controlled by ctrl_bc —
    //     same story, merges B-C.
    //   * A and C share *one* funded_by edge — count=1, medium
    //     strength, fails `min_evidence=2`.
    //
    // After link_and_persist, all three land in one cluster by
    // transitivity. The DOT view must NOT render the A-C funded_by
    // edge: the merge invariant rejected that pair, so showing it
    // would be the agent's exact P1 case (transitive membership
    // misread as direct evidence).
    let repo = fresh_repo().await;
    let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let c = "0xcccccccccccccccccccccccccccccccccccccccc";
    let ctrl_ab = "0xdddddddddddddddddddddddddddddddddddddddd";
    let ctrl_bc = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";

    // STRONG ladder: A↔B via ctrl_ab, B↔C via ctrl_bc.
    for (subject, controller, did_suffix) in [
        (a, ctrl_ab, "a"),
        (b, ctrl_ab, "b1"),
        (b, ctrl_bc, "b2"),
        (c, ctrl_bc, "c"),
    ] {
        repo.upsert_did_document(&DidDocument {
            did: format!("did:ethr:{did_suffix}"),
            subject_address: subject.into(),
            controller: controller.into(),
            method: "ethr".into(),
            document_json: None,
            observed_block: Some(100),
            source: "test".into(),
        })
        .await
        .unwrap();
    }

    // ONE shared funder between A and C (the transitive-bait edge).
    repo.insert_transfer(&transfer(funder, a, 200, "0xta")).await.unwrap();
    repo.insert_transfer(&transfer(funder, c, 201, "0xtc")).await.unwrap();
    repo.upsert_address(a, None).await.unwrap();
    repo.upsert_address(b, None).await.unwrap();
    repo.upsert_address(c, None).await.unwrap();

    let (run_id, output) = link_and_persist(&repo, &[a.into(), b.into(), c.into()], 2)
        .await
        .unwrap();
    assert_eq!(output.clusters.len(), 1, "all three must transitively merge into one cluster");
    assert_eq!(output.clusters[0].addresses.len(), 3);

    let run = repo.latest_clustering_run().await.unwrap().unwrap();
    let clusters = repo.clusters_for_run(&run.run_id).await.unwrap();
    let skipped = repo.suspected_keys_for_run(&run.run_id).await.unwrap();
    let cluster_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let attestations = repo.attestations_for(&cluster_addresses).await.unwrap();

    let dot = render_dot(&DotInputs {
        run: &run,
        clusters: &clusters,
        skipped: &skipped,
        attestations: &attestations,
    });

    // Sanity: run_id audit comment present.
    assert!(dot.contains(&run_id));
    // Two STRONG did_controller edges that PASSED the merge rule
    // (A↔B via ctrl_ab, B↔C via ctrl_bc).
    assert!(
        dot.contains("did_controller | strong"),
        "expected at least one strong did_controller edge; got:\n{dot}"
    );
    let strong_edges = dot.matches("did_controller | strong").count();
    assert_eq!(strong_edges, 2, "expected exactly two strong DID edges (A-B and B-C)");
    // Critical: the A-C funded_by edge MUST NOT be drawn — that
    // pair is in the cluster only by transitivity.
    assert!(
        !dot.contains("funded_by"),
        "A-C funded_by edge must not appear; pair did not pass min_evidence=2:\n{dot}"
    );
    // Cluster heading reflects the strongest contributing kind
    // (DID controller), not "shared-funder".
    assert!(
        dot.contains("controller-level cluster"),
        "expected controller-level descriptor; got:\n{dot}"
    );
}

#[tokio::test]
async fn dot_descriptor_ignores_unrelated_kind_on_one_member() {
    // P2 regression — agent finding 2026-05-01.
    //
    // Two Safes share one EOA owner (medium safe_owner, count=1
    // passes `min_evidence=1`). One of the Safes additionally has a
    // did_controller attestation that the OTHER Safe does NOT
    // share. The cluster heading must reflect what merged the
    // cluster — `safe_owner` — not the unrelated strong attestation
    // sitting on a single member.
    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let owner = "0xeeee00000000000000000000000000000000eee0";
    let lonely_controller = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";

    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_a.into(),
        owner_address: owner.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_b.into(),
        owner_address: owner.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    // Only safe_a has this DID — safe_b has no matching one, so
    // there is no shared-controller edge.
    repo.upsert_did_document(&DidDocument {
        did: format!("did:ethr:{safe_a}"),
        subject_address: safe_a.into(),
        controller: lonely_controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    let (_run_id, _) = link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();

    let run = repo.latest_clustering_run().await.unwrap().unwrap();
    let clusters = repo.clusters_for_run(&run.run_id).await.unwrap();
    let skipped = repo.suspected_keys_for_run(&run.run_id).await.unwrap();
    let cluster_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let attestations = repo.attestations_for(&cluster_addresses).await.unwrap();

    let dot = render_dot(&DotInputs {
        run: &run,
        clusters: &clusters,
        skipped: &skipped,
        attestations: &attestations,
    });

    // The cluster is shared-owner, not controller-level.
    assert!(
        dot.contains("shared-owner cluster"),
        "expected shared-owner descriptor; got:\n{dot}"
    );
    assert!(
        !dot.contains("controller-level"),
        "DID controller is unshared and must not influence descriptor; got:\n{dot}"
    );
    // No did_controller edge — the attestation has only one address.
    assert!(
        !dot.contains("did_controller"),
        "no shared did_controller edge expected; got:\n{dot}"
    );
}
