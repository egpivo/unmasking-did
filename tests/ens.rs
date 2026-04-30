//! ENS-handle evidence: M2 first slice.
//!
//! Verifies the new evidence kind end-to-end:
//!   * `extract_ens_handle` emits one MEDIUM attestation per non-empty
//!     off-chain handle (twitter / github / telegram) and ignores
//!     `name` (ENS primary names are unique per address).
//!   * Two addresses sharing the same twitter handle get merged.
//!   * The same twitter handle differing only in casing / `@`-prefix
//!     still merges (handle normalization is part of the contract).
//!   * ENS evidence and funded_by evidence stack: a pair with one ENS
//!     edge and one funder edge merges at `min_evidence = 2`.

use unmasking_did::ens::EnsRecord;
use unmasking_did::evidence::{extract_ens_handle, EvidenceKind, Strength};
use unmasking_did::linking::link_addresses;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn record(addr: &str, name: Option<&str>, twitter: Option<&str>, github: Option<&str>) -> EnsRecord {
    EnsRecord {
        address: addr.to_string(),
        name: name.map(str::to_string),
        twitter: twitter.map(str::to_string),
        github: github.map(str::to_string),
        telegram: None,
    }
}

#[tokio::test]
async fn extract_ens_handle_skips_name_and_emits_per_service() {
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    repo.upsert_ens_record(&record(
        alice,
        Some("alice.eth"),
        Some("@alice"),
        Some("alice-dev"),
    ))
    .await
    .unwrap();

    let atts = extract_ens_handle(&repo, &[alice.into()]).await.unwrap();

    assert_eq!(atts.len(), 2, "expected one attestation per non-empty handle, no `name` row");
    assert!(atts.iter().all(|a| a.kind == EvidenceKind::EnsHandle));
    assert!(atts.iter().all(|a| a.strength == Strength::Medium));
    let keys: Vec<&str> = atts.iter().map(|a| a.key.as_str()).collect();
    assert!(keys.contains(&"twitter:alice"));
    assert!(keys.contains(&"github:alice-dev"));
}

#[tokio::test]
async fn shared_twitter_handle_merges_addresses() {
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    // Same twitter handle, different ENS names. Casing / `@` should
    // not affect normalization.
    repo.upsert_ens_record(&record(alice, Some("alice.eth"), Some("@joseph"), None))
        .await
        .unwrap();
    repo.upsert_ens_record(&record(bob, Some("bob.eth"), Some("Joseph"), None))
        .await
        .unwrap();
    repo.upsert_address(alice, None).await.unwrap();
    repo.upsert_address(bob, None).await.unwrap();

    let out = link_addresses(&repo, &[alice.into(), bob.into()], 1)
        .await
        .unwrap();
    assert_eq!(out.clusters.len(), 1, "shared twitter handle should merge");
    assert_eq!(out.clusters[0].addresses.len(), 2);
    assert!(out.clusters[0]
        .shared_evidence_keys
        .iter()
        .any(|k| k == "twitter:joseph"));
}

#[tokio::test]
async fn ens_handle_and_funder_stack_to_meet_min_evidence_2() {
    use unmasking_did::alchemy::Transfer;

    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";

    // One funder edge.
    let t = |from, to, block, tx| Transfer {
        from_addr: String::from(from),
        to_addr: String::from(to),
        value: Some("1".into()),
        block_num: Some(block),
        tx_hash: Some(String::from(tx)),
        asset: Some("ETH".into()),
    };
    repo.insert_transfer(&t(funder, alice, 100, "0x1")).await.unwrap();
    repo.insert_transfer(&t(funder, bob, 101, "0x2")).await.unwrap();
    // One twitter edge.
    repo.upsert_ens_record(&record(alice, None, Some("@joseph"), None))
        .await
        .unwrap();
    repo.upsert_ens_record(&record(bob, None, Some("@joseph"), None))
        .await
        .unwrap();
    repo.upsert_address(alice, Some(100)).await.unwrap();
    repo.upsert_address(bob, Some(101)).await.unwrap();

    // With min_evidence = 2 and only one funder edge, the funded_by
    // signal alone is insufficient — but combined with the ens_handle
    // edge, the pair has 2 medium edges and merges.
    let out = link_addresses(&repo, &[alice.into(), bob.into()], 2)
        .await
        .unwrap();
    assert_eq!(out.clusters.len(), 1, "stacked evidence should clear min_evidence=2");
    assert_eq!(out.clusters[0].addresses.len(), 2);
}
