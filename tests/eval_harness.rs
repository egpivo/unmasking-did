//! Integration test for `eval` gold harness against real DB evidence.

use unmasking_did::eval::{run_eval_suite, AblationMode};
use unmasking_did::linking::LinkageParams;
use unmasking_did::safe::SafeOwner;
use unmasking_did::storage::{connect, run_migrations, Repo};

fn safe_owner(safe: &str, owner: &str) -> SafeOwner {
    SafeOwner {
        safe_address: safe.to_string(),
        owner_address: owner.to_string(),
        owner_is_safe: false,
        threshold: Some(2),
        observed_block: Some(100),
        source: "test".to_string(),
    }
}

#[tokio::test]
async fn eval_ablation_detects_shared_safe_owner_as_same_control() {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = Repo::new(pool);

    let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let owner = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
    repo.upsert_safe_owner(&safe_owner(safe_a, owner))
        .await
        .unwrap();
    repo.upsert_safe_owner(&safe_owner(safe_b, owner))
        .await
        .unwrap();
    repo.upsert_address(safe_a, None).await.unwrap();
    repo.upsert_address(safe_b, None).await.unwrap();

    unmasking_did::linking::link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
        .await
        .unwrap();

    let gold_path = std::env::temp_dir().join(format!(
        "unmasking_did_eval_gold_{}.csv",
        std::process::id()
    ));
    std::fs::write(
        &gold_path,
        format!(
            "address_a,address_b,label,rationale\n{safe_a},{safe_b},same_control,shared EOA owner\n"
        ),
    )
    .unwrap();

    let params = LinkageParams::bundled_default().unwrap();
    let modes = vec![AblationMode::SafeOwnerOnly, AblationMode::FundedByOnly];
    let suite = run_eval_suite(&repo, &gold_path, &modes, 1, &params)
        .await
        .unwrap();
    let _ = std::fs::remove_file(&gold_path);

    assert_eq!(suite.gold_pair_count, 1);
    let so = suite
        .ablations
        .iter()
        .find(|a| a.ablation == "safe_owner_only")
        .unwrap();
    assert!(
        so.pair_rows[0].rule_same_cluster,
        "rule linker should merge on shared safe_owner with min_evidence=1"
    );
    assert!(
        so.pair_rows[0].pairwise_score > 0.0,
        "pairwise score should reflect jaccard signal"
    );

    let fb = suite
        .ablations
        .iter()
        .find(|a| a.ablation == "funded_by_only")
        .unwrap();
    assert!(
        !fb.pair_rows[0].rule_same_cluster,
        "funded_by-only ablation must not merge two safes with only shared owner evidence"
    );
}
