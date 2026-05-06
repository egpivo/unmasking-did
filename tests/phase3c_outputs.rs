use serde::Deserialize;
use unmasking_did::alchemy::Transfer;
use unmasking_did::graph_export::build_graph;
use unmasking_did::linking::link_and_persist;
use unmasking_did::pipelines::arbitrum_governance::{
    ArbitrumGovSummary, ClusterSummary, LineageCounts, LineageSummary, PaginationCapHits,
    SeedCounts,
};
use unmasking_did::storage::{connect, run_migrations, Repo};

#[derive(Debug, Deserialize)]
struct LegacyArbitrumGovSummary {
    run_id: String,
    n_clusters: usize,
    n_addresses_clustered: usize,
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

#[test]
fn summary_json_contains_lineage_and_monitoring_metadata() {
    let summary = ArbitrumGovSummary {
        database_url: "sqlite://data/unmask_arbitrum_gov_v1.db".to_string(),
        alchemy_base_url_used: "https://arb-mainnet.g.alchemy.com/v2".to_string(),
        arbitrum_alchemy_key_source: "ARBITRUM_ALCHEMY_API_KEY".to_string(),
        safe_tx_service_url_used: "https://safe-transaction-arbitrum.safe.global".to_string(),
        input_snapshot_hash: "hash".to_string(),
        policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
        stable_threshold: 0.5,
        related_threshold: 0.1,
        chain_notes: "note".to_string(),
        seed_counts: SeedCounts {
            governance: 500,
            control: 500,
            total: 1000,
        },
        alchemy_calls: 1,
        is_contract_calls: 0,
        transfers_rows_inserted: 1,
        pagination_cap_hits: PaginationCapHits::default(),
        pagination_bias_risk: false,
        db_size_bytes: 1,
        db_size_stopped: false,
        link_fanout_cap: 1000,
        min_evidence: 1,
        run_id: "run".to_string(),
        n_clusters: 2,
        n_addresses_clustered: 3,
        top_clusters: vec![ClusterSummary {
            cluster_id: "0x1".to_string(),
            size: 2,
            coordination_tier: "tier".to_string(),
            shared_evidence_keys: vec!["k".to_string()],
            governance_count: 1,
            control_count: 1,
        }],
        lineage: LineageSummary {
            enabled: true,
            skip_reason: None,
            previous_run_id: Some("prev".to_string()),
            counts: LineageCounts {
                stable: 1,
                related: 2,
                new: 3,
                disappeared: 4,
                total_rows: 10,
            },
        },
        anomalies: vec![],
    };
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&summary).expect("serialize summary"))
            .expect("parse summary json");
    assert_eq!(
        v.get("policy_profile_id").and_then(|x| x.as_str()),
        Some("arbitrum_gov_conservative_v1")
    );
    assert_eq!(
        v.get("stable_threshold").and_then(|x| x.as_f64()),
        Some(0.5)
    );
    assert_eq!(
        v.get("related_threshold").and_then(|x| x.as_f64()),
        Some(0.1)
    );
    assert_eq!(
        v.get("lineage")
            .and_then(|l| l.get("counts"))
            .and_then(|c| c.get("total_rows"))
            .and_then(|x| x.as_u64()),
        Some(10)
    );
}

#[test]
fn summary_json_remains_backward_compatible_for_legacy_parsers() {
    let summary = ArbitrumGovSummary {
        database_url: "sqlite://data/unmask_arbitrum_gov_v1.db".to_string(),
        alchemy_base_url_used: "https://arb-mainnet.g.alchemy.com/v2".to_string(),
        arbitrum_alchemy_key_source: "ARBITRUM_ALCHEMY_API_KEY".to_string(),
        safe_tx_service_url_used: "https://safe-transaction-arbitrum.safe.global".to_string(),
        input_snapshot_hash: "hash".to_string(),
        policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
        stable_threshold: 0.5,
        related_threshold: 0.1,
        chain_notes: "note".to_string(),
        seed_counts: SeedCounts {
            governance: 500,
            control: 500,
            total: 1000,
        },
        alchemy_calls: 1,
        is_contract_calls: 0,
        transfers_rows_inserted: 1,
        pagination_cap_hits: PaginationCapHits::default(),
        pagination_bias_risk: false,
        db_size_bytes: 1,
        db_size_stopped: false,
        link_fanout_cap: 1000,
        min_evidence: 1,
        run_id: "run-legacy".to_string(),
        n_clusters: 2,
        n_addresses_clustered: 3,
        top_clusters: vec![],
        lineage: LineageSummary {
            enabled: false,
            skip_reason: Some(
                "No prior same-profile run available; lineage not computed for this run."
                    .to_string(),
            ),
            previous_run_id: None,
            counts: LineageCounts {
                stable: 0,
                related: 0,
                new: 0,
                disappeared: 0,
                total_rows: 0,
            },
        },
        anomalies: vec![],
    };
    let json = serde_json::to_string(&summary).expect("serialize summary");
    let legacy: LegacyArbitrumGovSummary =
        serde_json::from_str(&json).expect("legacy parser should still work");
    assert_eq!(legacy.run_id, "run-legacy");
    assert_eq!(legacy.n_clusters, 2);
    assert_eq!(legacy.n_addresses_clustered, 3);
}

#[tokio::test]
async fn graph_output_remains_bounded() {
    let pool = connect("sqlite::memory:").await.expect("connect memory db");
    run_migrations(&pool).await.expect("run migrations");
    let repo = Repo::new(pool);
    let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";
    let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    repo.insert_transfer(&t(funder, a, 100, "0x1"))
        .await
        .expect("insert transfer");
    repo.insert_transfer(&t(funder, b, 101, "0x2"))
        .await
        .expect("insert transfer");
    repo.upsert_address(a, Some(100))
        .await
        .expect("upsert address");
    repo.upsert_address(b, Some(101))
        .await
        .expect("upsert address");
    let (run_id, _) = link_and_persist(&repo, &[a.to_string(), b.to_string()], 1)
        .await
        .expect("link and persist");
    let graph = build_graph(&repo, Some(&run_id), 1, 1, 50)
        .await
        .expect("build graph");
    assert!(
        graph.nodes.len() <= 2,
        "identifier + evidence caps remain bounded"
    );
    assert_eq!(graph.limits.max_identifier_nodes, 1);
    assert_eq!(graph.limits.max_evidence_nodes, 1);
}
