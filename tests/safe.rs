//! Safe-owner evidence: M2 second slice.
//!
//! Verifies the third evidence kind end-to-end:
//!   * Two Safes sharing an EOA owner get merged.
//!   * Two Safes sharing a Safe-as-owner do NOT merge — only EOA
//!     owners qualify per the project's evidence taxonomy, since
//!     Safe-of-safe ownership tells us nothing about human-level
//!     control on its own.
//!   * `extract_safe_owner` filters out `owner_is_safe = true`
//!     attestations before they reach the evidence table.

use unmasking_did::evidence::{extract_safe_owner, EvidenceKind, Strength};
use unmasking_did::linking::link_addresses;
use unmasking_did::safe::SafeOwner;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn edge(safe: &str, owner: &str, owner_is_safe: bool) -> SafeOwner {
    SafeOwner {
        safe_address: safe.to_string(),
        owner_address: owner.to_string(),
        owner_is_safe,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".to_string(),
    }
}

#[tokio::test]
async fn shared_eoa_owner_merges_two_safes() {
    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let eoa = "0xcccccccccccccccccccccccccccccccccccccccc";

    repo.upsert_safe_owner(&edge(safe_a, eoa, false)).await.unwrap();
    repo.upsert_safe_owner(&edge(safe_b, eoa, false)).await.unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    let out = link_addresses(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();
    assert_eq!(out.clusters.len(), 1, "shared EOA owner should merge two Safes");
    assert_eq!(out.clusters[0].addresses.len(), 2);
    assert!(out.clusters[0].shared_evidence_keys.iter().any(|k| k == eoa));
}

#[tokio::test]
async fn shared_safe_as_owner_does_not_merge() {
    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    // Both Safes are owned by the same other Safe — but that's not
    // human-level shared control evidence.
    let parent_safe = "0xdddddddddddddddddddddddddddddddddddddddd";

    repo.upsert_safe_owner(&edge(safe_a, parent_safe, true)).await.unwrap();
    repo.upsert_safe_owner(&edge(safe_b, parent_safe, true)).await.unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    let out = link_addresses(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();
    assert_eq!(
        out.clusters.len(),
        2,
        "Safe-of-safe ownership must not merge entities on its own"
    );
}

#[tokio::test]
async fn extract_safe_owner_filters_safe_owners() {
    let repo = fresh_repo().await;
    let safe = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let eoa1 = "0xcccccccccccccccccccccccccccccccccccccccc";
    let eoa2 = "0xdddddddddddddddddddddddddddddddddddddddd";
    let parent_safe = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    repo.upsert_safe_owner(&edge(safe, eoa1, false)).await.unwrap();
    repo.upsert_safe_owner(&edge(safe, eoa2, false)).await.unwrap();
    repo.upsert_safe_owner(&edge(safe, parent_safe, true)).await.unwrap();

    let atts = extract_safe_owner(&repo, &[safe.into()]).await.unwrap();
    assert_eq!(atts.len(), 2, "Safe-as-owner edges should be excluded");
    assert!(atts.iter().all(|a| a.kind == EvidenceKind::SafeOwner));
    assert!(atts.iter().all(|a| a.strength == Strength::Medium));
    let keys: Vec<&str> = atts.iter().map(|a| a.key.as_str()).collect();
    assert!(keys.contains(&eoa1));
    assert!(keys.contains(&eoa2));
    assert!(!keys.contains(&parent_safe));
}
