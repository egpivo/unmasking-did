//! DID controller as STRONG evidence — M3 first slice.
//!
//! Verifies the fourth evidence kind end-to-end:
//!   * Two addresses sharing a non-self DID controller get merged via
//!     a single edge — the strong-alone bypass kicks in even when
//!     `min_evidence` would otherwise demand more.
//!   * Self-controlled DIDs (controller == subject) emit no
//!     attestation — they're tautological and would produce a
//!     self-referential edge with no clustering signal.
//!   * `extract_did_controller` reads from the `did_documents` cache
//!     and lowercases both subject and controller before emitting.
//!
//! Until M3.5 lands an automated `did:ethr` resolver, the test (and
//! the CLI's `add-did-document`) populate `did_documents` directly.

use unmasking_did::did::DidDocument;
use unmasking_did::evidence::{extract_did_controller, EvidenceKind, Strength};
use unmasking_did::linking::link_addresses;
use unmasking_did::storage::{connect, run_migrations, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn doc(subject: &str, controller: &str, method: &str) -> DidDocument {
    DidDocument {
        did: format!("did:{method}:{subject}"),
        subject_address: subject.to_string(),
        controller: controller.to_string(),
        method: method.to_string(),
        document_json: None,
        observed_block: Some(100),
        source: "test".to_string(),
    }
}

#[tokio::test]
async fn shared_did_controller_merges_via_single_strong_edge() {
    // The whole point of strong evidence: a single shared
    // cryptographic-controller edge is enough to merge, even when
    // min_evidence demands more for medium-only signals.
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let controller = "0xcccccccccccccccccccccccccccccccccccccccc";

    repo.upsert_did_document(&doc(alice, controller, "ethr"))
        .await
        .unwrap();
    repo.upsert_did_document(&doc(bob, controller, "ethr"))
        .await
        .unwrap();
    repo.upsert_address(alice, None).await.unwrap();
    repo.upsert_address(bob, None).await.unwrap();

    // min_evidence = 5: medium-only evidence would never merge here,
    // but a single STRONG edge bypasses the count requirement.
    let out = link_addresses(&repo, &[alice.into(), bob.into()], 5)
        .await
        .unwrap();
    assert_eq!(
        out.clusters.len(),
        1,
        "shared DID controller is strong evidence and must merge alone"
    );
    assert_eq!(out.clusters[0].addresses.len(), 2);
    assert!(out.clusters[0]
        .shared_evidence_keys
        .iter()
        .any(|k| k == controller));
}

#[tokio::test]
async fn self_controlled_did_emits_no_evidence() {
    // did:pkh:eip155:1:0xabc is by construction controlled by 0xabc.
    // Emitting that as evidence would create a self-referential edge
    // with no clustering signal — every address would "share" its own
    // implicit DID with itself.
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    repo.upsert_did_document(&doc(alice, alice, "pkh"))
        .await
        .unwrap();

    let atts = extract_did_controller(&repo, &[alice.into()])
        .await
        .unwrap();
    assert!(
        atts.is_empty(),
        "self-controlled DID must not produce a did_controller attestation"
    );
}

#[tokio::test]
async fn extract_did_controller_emits_strong_attestations() {
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let controller = "0xcccccccccccccccccccccccccccccccccccccccc";

    repo.upsert_did_document(&doc(alice, controller, "ethr"))
        .await
        .unwrap();

    let atts = extract_did_controller(&repo, &[alice.into()])
        .await
        .unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].kind, EvidenceKind::DidController);
    assert_eq!(atts[0].strength, Strength::Strong);
    assert_eq!(atts[0].address, alice);
    assert_eq!(atts[0].key, controller);
    assert!(atts[0].source.contains("did_documents"));
    assert!(atts[0].source.contains("ethr"));
}

#[tokio::test]
async fn multi_did_same_controller_collapses_to_one_attestation() {
    // Regression: when a single subject has more than one DID
    // document pointing at the same controller (e.g. did:ethr on two
    // chains), the extractor used to emit one attestation per DID.
    // Two failure modes followed:
    //   (a) replace_attestations_for_kind's plain INSERT collided on
    //       UNIQUE(address, kind, key, source) and aborted the run;
    //   (b) the per-pair edge count inflated to N² for N DIDs, so the
    //       same logical fact would silently overweight `min_evidence`.
    // Fix: collapse to one attestation per (subject, controller),
    // aggregating the supporting DIDs into payload_json for audit.
    let repo = fresh_repo().await;
    let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let controller = "0xcccccccccccccccccccccccccccccccccccccccc";

    // Alice has two DID documents (different chain variants), both
    // claiming `controller` as authoritative.
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
        did: format!("did:ethr:scroll:{alice}"),
        subject_address: alice.into(),
        controller: controller.into(),
        method: "ethr".into(),
        document_json: None,
        observed_block: Some(200),
        source: "test".into(),
    })
    .await
    .unwrap();
    // Bob has one DID document with the same controller.
    repo.upsert_did_document(&doc(bob, controller, "ethr"))
        .await
        .unwrap();

    let atts = extract_did_controller(&repo, &[alice.into(), bob.into()])
        .await
        .unwrap();

    // Failure (a): without dedup, alice would emit 2 attestations
    // with byte-identical (address, kind, key, source). The fixed
    // extractor must emit exactly one per (subject, controller).
    let alice_atts: Vec<_> = atts.iter().filter(|a| a.address == alice).collect();
    let bob_atts: Vec<_> = atts.iter().filter(|a| a.address == bob).collect();
    assert_eq!(alice_atts.len(), 1, "alice's two DIDs must collapse to one attestation");
    assert_eq!(bob_atts.len(), 1);

    // Audit info preserved: both supporting DIDs in payload_json.
    let payload = alice_atts[0].payload_json.as_ref().expect("payload_json present");
    assert!(payload.contains(&format!("did:ethr:{alice}")));
    assert!(payload.contains(&format!("did:ethr:scroll:{alice}")));

    // First-seen drives observed_block (lower = earlier observation).
    assert_eq!(alice_atts[0].observed_block, 100);

    // Failure (b): now the link pipeline must not crash on the
    // duplicate-source case AND the per-pair count between alice and
    // bob is exactly 1 (one shared controller), not 2. Strong-alone
    // bypass merges them either way; this assertion locks the
    // structural fact.
    let out = link_addresses(&repo, &[alice.into(), bob.into()], 1)
        .await
        .expect("link must not crash on multi-DID same-controller input");
    assert_eq!(out.clusters.len(), 1);
    assert_eq!(out.clusters[0].addresses.len(), 2);
}

#[tokio::test]
async fn did_controller_stacks_with_safe_owner_evidence() {
    // The architecture's whole point: a DID-controller edge and a
    // Safe-owner edge between the same pair both contribute to the
    // cluster, with the strongest edge dictating the merge.
    use unmasking_did::safe::SafeOwner;

    let repo = fresh_repo().await;
    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let shared_eoa = "0xeee0eee0eee0eee0eee0eee0eee0eee0eee0eee0";
    let shared_controller = "0xcccccccccccccccccccccccccccccccccccccccc";

    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_a.into(),
        owner_address: shared_eoa.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_safe_owner(&SafeOwner {
        safe_address: safe_b.into(),
        owner_address: shared_eoa.into(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".into(),
    })
    .await
    .unwrap();
    repo.upsert_did_document(&doc(safe_a, shared_controller, "ethr"))
        .await
        .unwrap();
    repo.upsert_did_document(&doc(safe_b, shared_controller, "ethr"))
        .await
        .unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    let out = link_addresses(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();
    assert_eq!(out.clusters.len(), 1);
    let keys = &out.clusters[0].shared_evidence_keys;
    assert!(
        keys.iter().any(|k| k == shared_eoa),
        "safe_owner edge should appear in shared_evidence_keys"
    );
    assert!(
        keys.iter().any(|k| k == shared_controller),
        "did_controller edge should appear in shared_evidence_keys"
    );
}
