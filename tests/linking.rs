use unmasking_did::alchemy::Transfer;
use unmasking_did::did::DidDocument;
use unmasking_did::linking::{cluster_by_funding, link_addresses_with_fanout, FundedByMergePolicy};
use unmasking_did::safe::SafeOwner;
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

fn conservative_policy(service_cap: usize) -> FundedByMergePolicy {
    FundedByMergePolicy {
        enabled: true,
        service_fan_out_cap: service_cap,
        min_shared_keys: 2,
        min_short_burst_hits: 2,
        short_burst_block_delta: 5_000,
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
    assert!(only.shared_evidence_keys.contains(&funder.to_string()));
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

#[tokio::test]
async fn conservative_single_shared_funder_does_not_merge() {
    let repo = fresh_repo().await;
    let funder = "0x1111111111111111111111111111111111111111";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    repo.insert_transfer(&transfer(funder, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(funder, bob, 120, "0xtx2"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(120)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into()],
        1,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 2);
}

#[tokio::test]
async fn conservative_two_shared_funders_without_burst_do_not_merge() {
    let repo = fresh_repo().await;
    let f1 = "0x1111111111111111111111111111111111111111";
    let f2 = "0x2222222222222222222222222222222222222222";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    repo.insert_transfer(&transfer(f1, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f1, bob, 200_100, "0xtx2"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, alice, 300, "0xtx3"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, bob, 300_300, "0xtx4"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(200_100)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into()],
        1,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 2);
}

#[tokio::test]
async fn conservative_two_shared_funders_with_burst_merge() {
    let repo = fresh_repo().await;
    let f1 = "0x1111111111111111111111111111111111111111";
    let f2 = "0x2222222222222222222222222222222222222222";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    repo.insert_transfer(&transfer(f1, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f1, bob, 120, "0xtx2"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, alice, 300, "0xtx3"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, bob, 320, "0xtx4"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(120)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into()],
        1,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 1);
}

#[tokio::test]
async fn conservative_zero_address_never_merges() {
    let repo = fresh_repo().await;
    let zero = "0x0000000000000000000000000000000000000000";
    let f2 = "0x2222222222222222222222222222222222222222";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    repo.insert_transfer(&transfer(zero, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(zero, bob, 120, "0xtx2"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, alice, 300, "0xtx3"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, bob, 320, "0xtx4"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(120)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into()],
        1,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 2);
}

#[tokio::test]
async fn conservative_high_fanout_key_never_merges() {
    let repo = fresh_repo().await;
    let hf = "0x9999999999999999999999999999999999999999";
    let f2 = "0x2222222222222222222222222222222222222222";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let charlie = "0xcccccccccccccccccccccccccccccccccccccccc";
    // hf shared by 3 addresses -> service-like when cap=2
    repo.insert_transfer(&transfer(hf, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(hf, bob, 120, "0xtx2"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(hf, charlie, 140, "0xtx3"))
        .await
        .unwrap();
    // second shared key only for alice+bob; without hf suppression pair would merge.
    repo.insert_transfer(&transfer(f2, alice, 300, "0xtx4"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, bob, 320, "0xtx5"))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(120)).await.unwrap();
    repo.upsert_address(charlie, Some(140)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into(), charlie.into()],
        1,
        50,
        None,
        &conservative_policy(2),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 3);
}

#[tokio::test]
async fn conservative_suppressed_funder_does_not_count_as_burst_hit() {
    let repo = fresh_repo().await;
    let service = "0x9999999999999999999999999999999999999999";
    let f1 = "0x1111111111111111111111111111111111111111";
    let f2 = "0x2222222222222222222222222222222222222222";
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let charlie = "0xcccccccccccccccccccccccccccccccccccccccc";

    // service is a short-burst shared funder for alice+bob, but it is
    // service-like at cap=2 and must not contribute to burst-hit count.
    repo.insert_transfer(&transfer(service, alice, 100, "0xtx1"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(service, bob, 120, "0xtx2"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(service, charlie, 140, "0xtx3"))
        .await
        .unwrap();

    // alice+bob still share two non-service funders, but only one is a
    // short burst. Without the regression fix, the suppressed service
    // key incorrectly supplied the second burst hit and merged them.
    repo.insert_transfer(&transfer(f1, alice, 300, "0xtx4"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f1, bob, 320, "0xtx5"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, alice, 1_000, "0xtx6"))
        .await
        .unwrap();
    repo.insert_transfer(&transfer(f2, bob, 20_000, "0xtx7"))
        .await
        .unwrap();

    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(120)).await.unwrap();
    repo.upsert_address(charlie, Some(140)).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into(), charlie.into()],
        1,
        50,
        None,
        &conservative_policy(2),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 3);
}

#[tokio::test]
async fn conservative_policy_keeps_safe_owner_behavior() {
    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let owner = "0xdddddddddddddddddddddddddddddddddddddddd";
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
        observed_block: Some(110),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[safe_a.into(), safe_b.into()],
        1,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 1);
}

#[tokio::test]
async fn conservative_policy_keeps_did_controller_strong_bypass() {
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let controller = "0xcccccccccccccccccccccccccccccccccccccccc";
    repo.upsert_did_document(&DidDocument {
        did: format!("did:ethr:{alice}"),
        subject_address: alice.into(),
        controller: controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_did_document(&DidDocument {
        did: format!("did:ethr:{bob}"),
        subject_address: bob.into(),
        controller: controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_address(alice, None).await.unwrap();
    repo.upsert_address(bob, None).await.unwrap();
    let out = link_addresses_with_fanout(
        &repo,
        &[alice.into(), bob.into()],
        5,
        50,
        None,
        &conservative_policy(50),
    )
    .await
    .unwrap();
    assert_eq!(out.clusters.len(), 1);
}
