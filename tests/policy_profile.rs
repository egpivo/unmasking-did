use unmasking_did::storage::{connect, run_migrations, ClusterLineageRow, DatasetRun, Repo};

async fn fresh_repo() -> Repo {
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    Repo::new(pool)
}

fn ds(run_id: &str, profile: &str) -> DatasetRun {
    DatasetRun {
        run_id: run_id.to_string(),
        chain: "arbitrum".to_string(),
        run_type: "monitor".to_string(),
        parent_run_id: None,
        window_start_block: 1,
        window_end_block: 2,
        window_start_ts: None,
        window_end_ts: None,
        cadence: "monthly".to_string(),
        seed_spec_json: "{}".to_string(),
        params_json: "{}".to_string(),
        input_snapshot_hash: "h".to_string(),
        code_commit: "c".to_string(),
        policy_profile_id: profile.to_string(),
        stable_threshold: 0.5,
        related_threshold: 0.1,
        notes: None,
    }
}

#[tokio::test]
async fn latest_run_query_is_profile_scoped() {
    let repo = fresh_repo().await;
    repo.start_dataset_run(&ds("run_a_1", "profile_a"))
        .await
        .unwrap();
    repo.start_dataset_run(&ds("run_b_1", "profile_b"))
        .await
        .unwrap();

    let latest_a = repo
        .latest_dataset_run_for_chain_profile("arbitrum", "profile_a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_a.run_id, "run_a_1");

    let latest_b = repo
        .latest_dataset_run_for_chain_profile("arbitrum", "profile_b")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_b.run_id, "run_b_1");
}

#[tokio::test]
async fn lineage_rows_support_new_and_disappeared_statuses() {
    let repo = fresh_repo().await;
    repo.start_dataset_run(&ds("prev", "profile_a"))
        .await
        .unwrap();
    repo.start_dataset_run(&ds("curr", "profile_a"))
        .await
        .unwrap();
    let rows = vec![
        ClusterLineageRow {
            run_id_current: Some("curr".to_string()),
            cluster_id_current: Some("c1".to_string()),
            run_id_previous: Some("prev".to_string()),
            cluster_id_previous: None,
            overlap_count: 0,
            jaccard: 0.0,
            transition_label: "new".to_string(),
        },
        ClusterLineageRow {
            run_id_current: Some("curr".to_string()),
            cluster_id_current: None,
            run_id_previous: Some("prev".to_string()),
            cluster_id_previous: Some("p1".to_string()),
            overlap_count: 0,
            jaccard: 0.0,
            transition_label: "disappeared".to_string(),
        },
    ];
    let inserted = repo.insert_cluster_lineage_rows(&rows).await.unwrap();
    assert_eq!(inserted, 2);
}
