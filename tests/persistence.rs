//! End-to-end check that `link_and_persist` writes a full audit trail:
//!   * one row in `clustering_runs` per call (with parameters JSON)
//!   * one row in `entity_clusters` per (cluster, address)
//!   * one row in `suspected_service_keys` per fan-out-cap hit, tied
//!     back to the same run via foreign key

use sqlx::Row;
use unmasking_did::alchemy::Transfer;
use unmasking_did::linking::link_and_persist;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn t(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
    Transfer {
        from_addr: from.to_string(),
        to_addr: to.to_string(),
        value: Some("1".to_string()),
        block_num: Some(block),
        tx_hash: Some(tx.to_string()),
        asset: Some("ETH".to_string()),
    }
}

#[tokio::test]
async fn link_and_persist_writes_full_audit_trail() {
    let repo = fresh_repo().await;
    let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    repo.insert_transfer(&t(funder, alice, 100, "0x1"))
        .await
        .unwrap();
    repo.insert_transfer(&t(funder, bob, 101, "0x2"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(101)).await.unwrap();

    let (run_id, output) = link_and_persist(&repo, &[alice.into(), bob.into()], 1)
        .await
        .unwrap();
    assert_eq!(output.clusters.len(), 1);
    assert_eq!(output.skipped_service_keys.len(), 0);

    let pool = repo.pool();

    let run_row = sqlx::query("SELECT run_id, params_json FROM clustering_runs WHERE run_id = ?1")
        .bind(&run_id)
        .fetch_one(pool)
        .await
        .unwrap();
    let stored_run_id: String = run_row.get("run_id");
    let params: String = run_row.get("params_json");
    assert_eq!(stored_run_id, run_id);
    assert!(params.contains("\"min_evidence\":1"));
    assert!(params.contains("\"fan_out_cap\":50"));

    let cluster_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM entity_clusters WHERE cluster_run_id = ?1")
            .bind(&run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        cluster_rows, 2,
        "expected one row per address in the cluster"
    );

    let evidence_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evidence")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        evidence_rows, 2,
        "expected one funded_by attestation per address"
    );
}

#[tokio::test]
async fn fan_out_cap_persists_suspected_service_keys() {
    let repo = fresh_repo().await;
    let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";

    // Seed via the real source-of-truth path (transfers), so
    // `link_and_persist`'s extract step rebuilds the scenario
    // deterministically. After the P1 fix, link_addresses replaces
    // attestations for the input set on every run — direct evidence
    // pre-seeding no longer survives that wipe.
    let mut addrs = Vec::with_capacity(51);
    for i in 0..51u32 {
        let a = format!("0x{:040x}", i + 1);
        repo.insert_transfer(&t(funder, &a, (i as i64) + 1, &format!("0x{i:x}")))
            .await
            .unwrap();
        addrs.push(a);
    }

    let (run_id, output) = link_and_persist(&repo, &addrs, 1).await.unwrap();
    assert_eq!(output.clusters.len(), 51);
    assert_eq!(output.skipped_service_keys.len(), 1);

    let pool = repo.pool();
    let row = sqlx::query(
        "SELECT key, fan_out FROM suspected_service_keys
         WHERE cluster_run_id = ?1 AND kind = 'funded_by'",
    )
    .bind(&run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let stored_key: String = row.get("key");
    let stored_fan_out: i64 = row.get("fan_out");
    assert_eq!(stored_key, funder);
    assert_eq!(stored_fan_out, 51);
}
