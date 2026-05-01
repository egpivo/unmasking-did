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
