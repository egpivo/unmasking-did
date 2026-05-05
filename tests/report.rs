//! End-to-end test for the report-time read path:
//!
//!   link_and_persist  →  entity_clusters / suspected_service_keys
//!         ↓
//!   Repo::clusters_for_run / suspected_keys_for_run
//!         ↓
//!   render_markdown (lib unit tests cover its formatting)
//!
//! This test locks the round-trip: cluster membership + shared evidence
//! keys read back from SQLite must match what the in-memory pipeline
//! produced. It's the contract `report` and `metrics` rely on now that
//! both read persisted state instead of re-clustering.

use unmasking_did::alchemy::Transfer;
use unmasking_did::ens::EnsRecord;
use unmasking_did::linking::link_and_persist;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn t(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
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
async fn cluster_round_trip_through_entity_clusters() {
    let repo = fresh_repo().await;

    // Two addresses linked by a shared funder AND a shared twitter
    // handle. Together they should form one cluster of size 2 with
    // both evidence keys present in shared_evidence_keys.
    let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    repo.insert_transfer(&t(funder, alice, 100, "0x1"))
        .await
        .unwrap();
    repo.insert_transfer(&t(funder, bob, 101, "0x2"))
        .await
        .unwrap();
    repo.upsert_ens_record(&EnsRecord {
        address: alice.into(),
        name: Some("alice.eth".into()),
        twitter: Some("@joseph".into()),
        github: None,
        telegram: None,
    })
    .await
    .unwrap();
    repo.upsert_ens_record(&EnsRecord {
        address: bob.into(),
        name: None,
        twitter: Some("Joseph".into()),
        github: None,
        telegram: None,
    })
    .await
    .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(101)).await.unwrap();

    let (run_id, output) = link_and_persist(&repo, &[alice.into(), bob.into()], 1)
        .await
        .unwrap();
    assert_eq!(output.clusters.len(), 1);

    // latest_clustering_run finds it
    let latest = repo.latest_clustering_run().await.unwrap().unwrap();
    assert_eq!(latest.run_id, run_id);
    assert!(latest.params_json.contains("\"min_evidence\":1"));

    // clusters_for_run reconstructs the same cluster contents and
    // shared_evidence_keys (parsed back out of evidence_json)
    let persisted = repo.clusters_for_run(&run_id).await.unwrap();
    assert_eq!(persisted.len(), 1);
    let c = &persisted[0];
    assert_eq!(c.cluster_id, alice, "cluster_id should be min(address)");
    assert_eq!(c.addresses, vec![alice.to_string(), bob.to_string()]);
    assert!(c.shared_evidence_keys.iter().any(|k| k == funder));
    assert!(c.shared_evidence_keys.iter().any(|k| k == "twitter:joseph"));
}

#[tokio::test]
async fn fan_out_cap_round_trip_through_suspected_keys() {
    let repo = fresh_repo().await;
    let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";

    // Same pattern as the persistence fan-out test: a non-CEX funder
    // paying out to 51 addresses crosses the cap and lands in
    // suspected_service_keys.
    let mut addrs = Vec::with_capacity(51);
    for i in 0..51u32 {
        let a = format!("0x{:040x}", i + 1);
        repo.insert_transfer(&t(funder, &a, (i as i64) + 1, &format!("0x{i:x}")))
            .await
            .unwrap();
        addrs.push(a);
    }

    let (run_id, output) = link_and_persist(&repo, &addrs, 1).await.unwrap();
    assert_eq!(output.skipped_service_keys.len(), 1);

    let read_back = repo.suspected_keys_for_run(&run_id).await.unwrap();
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].kind, "funded_by");
    assert_eq!(read_back[0].key, funder);
    assert_eq!(read_back[0].fan_out, 51);
}

#[tokio::test]
async fn empty_run_yields_no_clusters_or_skipped_keys() {
    let repo = fresh_repo().await;
    let lonely = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    repo.upsert_address(lonely, None).await.unwrap();

    let (run_id, _) = link_and_persist(&repo, &[lonely.into()], 1).await.unwrap();
    let clusters = repo.clusters_for_run(&run_id).await.unwrap();
    let skipped = repo.suspected_keys_for_run(&run_id).await.unwrap();

    // Even a single isolated address gets a cluster row (the singleton).
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].addresses.len(), 1);
    assert!(clusters[0].shared_evidence_keys.is_empty());
    assert!(skipped.is_empty());
}

#[tokio::test]
async fn latest_clustering_run_returns_the_most_recent() {
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    repo.upsert_address(alice, None).await.unwrap();

    let (first, _) = link_and_persist(&repo, &[alice.into()], 1).await.unwrap();
    let (second, _) = link_and_persist(&repo, &[alice.into()], 1).await.unwrap();
    assert_ne!(first, second);

    let latest = repo.latest_clustering_run().await.unwrap().unwrap();
    assert_eq!(
        latest.run_id, second,
        "latest must be the most recently started run"
    );

    // Sanity check: the older run's metadata is still queryable.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clustering_runs")
        .fetch_one(repo.pool())
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn latest_clustering_run_breaks_started_at_ties_via_run_id() {
    // Regression: SQLite's datetime('now') is second-resolution. Two
    // link_and_persist calls within the same wall-clock second land
    // on identical started_at values, so any ORDER BY started_at
    // without a tie-breaker is non-deterministic — engine row-scan
    // order decides which run "latest" returns. The fix is
    // ORDER BY started_at DESC, run_id DESC.
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    repo.upsert_address(alice, None).await.unwrap();

    let (first, _) = link_and_persist(&repo, &[alice.into()], 1).await.unwrap();
    let (second, _) = link_and_persist(&repo, &[alice.into()], 1).await.unwrap();
    assert_ne!(first, second);

    // Force both rows to share an identical started_at so the test
    // does not rely on luck of-the-clock to exercise the tie path.
    sqlx::query("UPDATE clustering_runs SET started_at = '2026-04-30 18:23:00'")
        .execute(repo.pool())
        .await
        .unwrap();

    let latest = repo.latest_clustering_run().await.unwrap().unwrap();
    assert_eq!(
        latest.run_id, second,
        "with started_at tied, the higher (later) run_id must win"
    );
}
