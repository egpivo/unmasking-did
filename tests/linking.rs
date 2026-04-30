use unmasking_did::alchemy::Transfer;
use unmasking_did::linking::cluster_by_funding;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");
    run_migrations(&pool).await.expect("run migrations");
    Repo::new(pool)
}

fn transfer(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
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
async fn shared_non_cex_funder_merges_addresses() {
    let repo = fresh_repo().await;

    let funder = "0x1111111111111111111111111111111111111111";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    repo.insert_transfer(&transfer(funder, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(funder, bob, 101, "0xtx2"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(101)).await.unwrap();

    let clusters = cluster_by_funding(&repo, &[alice.to_string(), bob.to_string()], 1)
        .await
        .unwrap();

    assert_eq!(clusters.len(), 1, "expected a single merged cluster");
    let only = &clusters[0];
    assert_eq!(only.addresses.len(), 2);
    assert!(only.addresses.contains(&alice.to_string()));
    assert!(only.addresses.contains(&bob.to_string()));
    assert!(only.shared_funders.contains(&funder.to_string()));
}

#[tokio::test]
async fn shared_cex_funder_does_not_merge() {
    let repo = fresh_repo().await;

    // Binance hot wallet (in the hardcoded blacklist).
    let cex = "0x28c6c06298d514db089934071355e5743bf21d60";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    repo.insert_transfer(&transfer(cex, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(cex, bob, 101, "0xtx2"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(101)).await.unwrap();

    let clusters = cluster_by_funding(&repo, &[alice.to_string(), bob.to_string()], 1)
        .await
        .unwrap();

    assert_eq!(
        clusters.len(),
        2,
        "CEX funding should not merge unrelated addresses"
    );
}
