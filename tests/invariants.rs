//! Lock the M1 evidence-graph invariants:
//!   * Strong evidence may merge two addresses on its own.
//!   * Weak-only evidence never merges, regardless of count.
//!   * `(kind, key)` groups exceeding the fan-out cap are skipped and
//!      surfaced as suspected service keys.
//!   * `cluster_id` is deterministic across runs (= min address).

use unmasking_did::evidence::{Attestation, EvidenceKind, Strength};
use unmasking_did::linking::cluster_from_evidence;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn att(addr: &str, kind: EvidenceKind, key: &str, strength: Strength, tag: &str) -> Attestation {
    Attestation {
        address: addr.to_string(),
        kind,
        key: key.to_string(),
        strength,
        source: format!("test:{tag}"),
        observed_block: 0,
        payload_json: None,
    }
}

#[tokio::test]
async fn strong_evidence_merges_alone() {
    let repo = fresh_repo().await;
    let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let ctrl = "0xcccccccccccccccccccccccccccccccccccccccc";

    repo.insert_attestations(&[
        att(a, EvidenceKind::DidController, ctrl, Strength::Strong, "1"),
        att(b, EvidenceKind::DidController, ctrl, Strength::Strong, "2"),
    ])
    .await
    .unwrap();

    // min_evidence=2 would normally require 2 shared edges, but a single
    // strong edge bypasses the count requirement.
    let out = cluster_from_evidence(&repo, &[a.into(), b.into()], 2)
        .await
        .unwrap();
    assert_eq!(out.clusters.len(), 1);
    assert_eq!(out.clusters[0].addresses.len(), 2);
}

#[tokio::test]
async fn weak_only_does_not_merge_even_with_many_edges() {
    let repo = fresh_repo().await;
    let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    // Five distinct weak signals between A and B. Skill says weak alone
    // can never merge entities — bulk shouldn't change that.
    let weak_pairs = ["t1", "t2", "t3", "t4", "t5"];
    let mut atts = Vec::new();
    for (i, k) in weak_pairs.iter().enumerate() {
        atts.push(att(a, EvidenceKind::FundedBy, k, Strength::Weak, &format!("a{i}")));
        atts.push(att(b, EvidenceKind::FundedBy, k, Strength::Weak, &format!("b{i}")));
    }
    repo.insert_attestations(&atts).await.unwrap();

    let out = cluster_from_evidence(&repo, &[a.into(), b.into()], 1)
        .await
        .unwrap();
    assert_eq!(
        out.clusters.len(),
        2,
        "weak-only evidence must not merge entities"
    );
}

#[tokio::test]
async fn fan_out_cap_skips_service_like_keys() {
    let repo = fresh_repo().await;

    // 51 addresses all sharing the same medium-strength funder. With
    // FAN_OUT_CAP = 50, the key is treated as service-like, no edges
    // are generated, and every address ends up as its own cluster.
    let funder = "0xfeefeefeefeefeefeefeefeefeefeefeefeefee0";
    let mut addrs = Vec::with_capacity(51);
    let mut atts = Vec::with_capacity(51);
    for i in 0..51u32 {
        let a = format!("0x{:040x}", i + 1);
        atts.push(att(&a, EvidenceKind::FundedBy, funder, Strength::Medium, "x"));
        addrs.push(a);
    }
    repo.insert_attestations(&atts).await.unwrap();

    let out = cluster_from_evidence(&repo, &addrs, 1).await.unwrap();
    assert_eq!(out.clusters.len(), 51, "fan-out cap should suppress merges");
    assert_eq!(out.skipped_service_keys.len(), 1);
    assert_eq!(out.skipped_service_keys[0].fan_out, 51);
    assert_eq!(out.skipped_service_keys[0].key, funder);
}

#[tokio::test]
async fn cluster_id_is_min_address_and_stable() {
    let repo = fresh_repo().await;
    let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";

    repo.insert_attestations(&[
        att(a, EvidenceKind::FundedBy, funder, Strength::Medium, "1"),
        att(b, EvidenceKind::FundedBy, funder, Strength::Medium, "2"),
    ])
    .await
    .unwrap();

    // Run twice with different input orderings — cluster_id must match.
    let r1 = cluster_from_evidence(&repo, &[a.into(), b.into()], 1)
        .await
        .unwrap();
    let r2 = cluster_from_evidence(&repo, &[b.into(), a.into()], 1)
        .await
        .unwrap();
    assert_eq!(r1.clusters[0].cluster_id, a, "cluster_id must equal min(address)");
    assert_eq!(
        r1.clusters[0].cluster_id, r2.clusters[0].cluster_id,
        "cluster_id must be stable across input orderings"
    );
}
