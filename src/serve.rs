//! Local HTTP server: graph JSON from SQLite (`GET /api/graph`) plus static files.
//!
//! Browsers cannot open `sqlite://` URLs. This module runs the same
//! [`crate::graph_export`] builders as `export-graph`, so the viewer can
//! `fetch("/api/graph")` against the current `DATABASE_URL` after `link`.

use std::path::Path;

use anyhow::{Context, Result};
use axum::{
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json as AxumJson, Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

use crate::benchmark::{run_scenario_suite, BenchmarkPolicyComparisonConfig};
use crate::graph_export::{
    build_graph, build_pairwise_graph, Graph, Limits, Link, Node, RunSummary, DEFAULT_FAN_OUT_CAP,
    DEFAULT_MAX_EVIDENCE_NODES, DEFAULT_MAX_IDENTIFIER_NODES,
};
use crate::linking::LinkageParams;
use crate::storage::{BenchmarkEvalDetailRow, BenchmarkEvalMetricsRow, Repo};

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    /// `evidence` (default) or `pairwise`.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub max_identifier_nodes: Option<usize>,
    #[serde(default)]
    pub max_evidence_nodes: Option<usize>,
    #[serde(default)]
    pub fan_out_cap: Option<usize>,
    #[serde(default)]
    pub max_pairwise_links: Option<usize>,
    /// Optional path to linkage JSON (pairwise mode only).
    #[serde(default)]
    pub linkage_params: Option<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkRunMeta {
    benchmark_run_id: String,
    scenario_suite_id: String,
    scenario_id: String,
    seed: i64,
    generator_version: String,
    policy_profile_id: String,
    policy_variant: String,
    input_snapshot_hash: String,
    code_commit: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkApiResponse {
    run: BenchmarkRunMeta,
    caveat: &'static str,
    metrics: Vec<BenchmarkEvalMetricsRow>,
    details: Vec<BenchmarkEvalDetailRow>,
}

#[derive(Debug, Serialize)]
struct BenchmarkRunListItem {
    benchmark_run_id: String,
    scenario_suite_id: String,
    scenario_id: String,
    seed: i64,
}

#[derive(Debug, Deserialize)]
struct SimulationTriggerRequest {
    scenario_id: Option<String>,
    seed: Option<u64>,
    suite_id: Option<String>,
    conservative_service_fan_out_cap: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct BenchmarkGraphQuery {
    #[serde(default)]
    policy_variant: Option<String>,
}

async fn api_graph(State(repo): State<Repo>, Query(q): Query<GraphQuery>) -> Response {
    let mode = q.mode.as_deref().unwrap_or("evidence");
    let max_id = q
        .max_identifier_nodes
        .unwrap_or(DEFAULT_MAX_IDENTIFIER_NODES);
    let max_ev = q.max_evidence_nodes.unwrap_or(DEFAULT_MAX_EVIDENCE_NODES);
    let fan = q.fan_out_cap.unwrap_or(DEFAULT_FAN_OUT_CAP);
    let max_pw = q.max_pairwise_links.unwrap_or(2000);

    let graph_res = match mode {
        "pairwise" => {
            let (params, src): (LinkageParams, String) = match &q.linkage_params {
                Some(p) => match LinkageParams::from_json_file(Path::new(p)) {
                    Ok(params) => (params, p.clone()),
                    Err(e) => {
                        return (StatusCode::BAD_REQUEST, format!("linkage params: {e:#}"))
                            .into_response();
                    }
                },
                None => match LinkageParams::bundled_default() {
                    Ok(params) => (
                        params,
                        "bundled data/linkage_params.default.json".to_string(),
                    ),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("bundled linkage params: {e:#}"),
                        )
                            .into_response();
                    }
                },
            };
            build_pairwise_graph(&repo, None, max_id, fan, max_pw, params, src.as_str()).await
        }
        "evidence" => build_graph(&repo, None, max_id, max_ev, fan).await,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unknown mode {other:?}; use evidence or pairwise"),
            )
                .into_response();
        }
    };

    match graph_res {
        Ok(graph) => (StatusCode::OK, Json(graph)).into_response(),
        Err(e) => {
            let msg = format!("{e:#}");
            let status = if msg.contains("no clustering runs") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, msg).into_response()
        }
    }
}

async fn load_benchmark_payload(
    repo: &Repo,
    benchmark_run_id: &str,
) -> Result<BenchmarkApiResponse> {
    let run = repo
        .benchmark_run_by_id(benchmark_run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("benchmark run not found: {benchmark_run_id}"))?;
    let metrics = repo
        .benchmark_eval_metrics_for_run(benchmark_run_id)
        .await?;
    if metrics.is_empty() {
        return Err(anyhow::anyhow!(
            "benchmark run has no eval metrics: {benchmark_run_id}"
        ));
    }
    let details = repo
        .benchmark_eval_details_for_run(benchmark_run_id)
        .await?;
    Ok(BenchmarkApiResponse {
        run: BenchmarkRunMeta {
            benchmark_run_id: run.benchmark_run_id,
            scenario_suite_id: run.scenario_suite_id,
            scenario_id: run.scenario_id,
            seed: run.seed,
            generator_version: run.generator_version,
            policy_profile_id: run.policy_profile_id,
            policy_variant: run.policy_variant,
            input_snapshot_hash: run.input_snapshot_hash,
            code_commit: run.code_commit,
        },
        caveat: "Coordination-structure benchmark only; no maliciousness attribution.",
        metrics,
        details,
    })
}

async fn api_benchmark_latest(State(repo): State<Repo>) -> Response {
    let run_id = match repo.latest_benchmark_run_id().await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                "no benchmark runs with eval metrics found".to_string(),
            )
                .into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    match load_benchmark_payload(&repo, &run_id).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

async fn api_benchmark_run(
    State(repo): State<Repo>,
    AxumPath(benchmark_run_id): AxumPath<String>,
) -> Response {
    match load_benchmark_payload(&repo, &benchmark_run_id).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => {
            let msg = format!("{e:#}");
            let status = if msg.contains("not found") || msg.contains("no eval metrics") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, msg).into_response()
        }
    }
}

async fn api_benchmark_runs_recent(State(repo): State<Repo>) -> Response {
    match repo.recent_benchmark_runs(50).await {
        Ok(runs) => {
            let out = runs
                .into_iter()
                .map(|r| BenchmarkRunListItem {
                    benchmark_run_id: r.benchmark_run_id,
                    scenario_suite_id: r.scenario_suite_id,
                    scenario_id: r.scenario_id,
                    seed: r.seed,
                })
                .collect::<Vec<_>>();
            (StatusCode::OK, Json(out)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

async fn api_benchmark_graph(
    State(repo): State<Repo>,
    AxumPath(benchmark_run_id): AxumPath<String>,
    Query(q): Query<BenchmarkGraphQuery>,
) -> Response {
    let policy_variant = q
        .policy_variant
        .as_deref()
        .unwrap_or("conservative_funded_by");
    let run = match repo.benchmark_run_by_id(&benchmark_run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("benchmark run not found: {benchmark_run_id}"),
            )
                .into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    let assignments = match repo
        .benchmark_policy_assignments(&benchmark_run_id, policy_variant)
        .await
    {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "no policy assignments for run={} policy_variant={}",
                    benchmark_run_id, policy_variant
                ),
            )
                .into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    let evidence_rows = match repo
        .benchmark_synthetic_evidence_rows(&benchmark_run_id)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };

    let mut nodes = Vec::<Node>::new();
    for (wallet, cluster_id) in &assignments {
        nodes.push(Node {
            id: wallet.clone(),
            kind: "identifier",
            node_type: "address".to_string(),
            label: wallet.clone(),
            value: wallet.clone(),
            cluster_id: Some(cluster_id.clone()),
            strength: None,
        });
    }

    let mut evidence_node_set = std::collections::BTreeSet::<String>::new();
    let mut links = Vec::<Link>::new();
    for row in evidence_rows {
        if !assignments.contains_key(&row.subject_wallet_id) {
            continue;
        }
        let evidence_node_id = format!("{}:{}", row.evidence_kind, row.counterparty_id);
        if evidence_node_set.insert(evidence_node_id.clone()) {
            let strength = match row.strength_hint.as_str() {
                "strong" => "strong",
                "weak" => "weak",
                _ => "medium",
            };
            nodes.push(Node {
                id: evidence_node_id.clone(),
                kind: "evidence",
                node_type: row.evidence_kind.clone(),
                label: row.counterparty_id.clone(),
                value: row.counterparty_id.clone(),
                cluster_id: None,
                strength: Some(strength),
            });
        }
        let strength = match row.strength_hint.as_str() {
            "strong" => "strong",
            "weak" => "weak",
            _ => "medium",
        };
        links.push(Link {
            source: row.subject_wallet_id.clone(),
            target: evidence_node_id,
            link_type: row.evidence_kind.clone(),
            strength: Some(strength),
            tier: None,
            score: None,
            link_probability: None,
            contributions: None,
            deterministic_anchor: None,
            first_seen_block: row.sequence_index,
            last_seen_block: row.sequence_index,
            attestation_count: Some(1),
        });
    }

    let graph = Graph {
        graph_mode: "benchmark".to_string(),
        run: RunSummary {
            run_id: run.benchmark_run_id,
            started_at: "benchmark".to_string(),
            params: serde_json::json!({
                "scenario_suite_id": run.scenario_suite_id,
                "scenario_id": run.scenario_id,
                "seed": run.seed,
                "policy_variant": policy_variant,
            }),
        },
        nodes,
        links,
        limits: Limits {
            max_identifier_nodes: usize::MAX,
            max_evidence_nodes: usize::MAX,
            depth: 1,
            fan_out_cap: 0,
            truncated_identifiers: false,
            truncated_evidence: false,
            applied_filters: vec!["benchmark_synthetic_evidence".to_string()],
            max_pairwise_links: None,
            linkage_params_source: None,
        },
        evidence_events: Vec::new(),
    };
    (StatusCode::OK, Json(graph)).into_response()
}

async fn api_benchmark_run_simulation(
    State(repo): State<Repo>,
    AxumJson(req): AxumJson<SimulationTriggerRequest>,
) -> Response {
    let scenario_id = req
        .scenario_id
        .unwrap_or_else(|| "S5_service_hub_contaminated".to_string());
    let seed = req.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    });
    let suite_id = req.suite_id.unwrap_or_else(|| "ui_simulation".to_string());
    let run_suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let policy_cfg = BenchmarkPolicyComparisonConfig {
        // Keep this high so conservative service cap is the gating path.
        fan_out_cap: 10_000,
        conservative_service_fan_out_cap: req.conservative_service_fan_out_cap.unwrap_or(15),
        ..BenchmarkPolicyComparisonConfig::default()
    };
    let scenarios = vec![scenario_id];
    let seeds = vec![seed];
    let suite = match run_scenario_suite(
        &repo,
        &suite_id,
        &scenarios,
        &seeds,
        &policy_cfg,
        Some(&run_suffix),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    let Some(case) = suite.cases.first() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "simulation produced no benchmark case".to_string(),
        )
            .into_response();
    };
    match load_benchmark_payload(&repo, &case.benchmark_run_id).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

async fn health() -> &'static str {
    "ok"
}

/// Bind `0.0.0.0:port` and serve until process exit. **CWD** should be the
/// repo root so `viewer/`, `out/`, and `data/findings/` paths resolve.
pub async fn run(repo: Repo, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/api/graph", get(api_graph))
        .route("/api/benchmark/latest", get(api_benchmark_latest))
        .route(
            "/api/benchmark/run/:benchmark_run_id",
            get(api_benchmark_run),
        )
        .route(
            "/api/benchmark/run/:benchmark_run_id/graph",
            get(api_benchmark_graph),
        )
        .route("/api/benchmark/runs/recent", get(api_benchmark_runs_recent))
        .route(
            "/api/benchmark/run-simulation",
            post(api_benchmark_run_simulation),
        )
        .route("/health", get(health))
        .with_state(repo)
        .nest_service("/viewer", ServeDir::new("viewer"))
        .nest_service("/out", ServeDir::new("out"))
        .nest_service("/data/findings", ServeDir::new("data/findings"));

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    tracing::info!(
        %addr,
        "serve: GET /api/graph + /api/benchmark/latest + /api/benchmark/run/{{id}} + /api/benchmark/runs/recent + POST /api/benchmark/run-simulation  static /viewer /out /data/findings"
    );
    eprintln!(
        "unmasking-did serve: http://127.0.0.1:{port}/viewer/graph-explorer.html\n\
         API: GET http://127.0.0.1:{port}/api/graph?mode=evidence&max_evidence_nodes=120\n\
         API: GET http://127.0.0.1:{port}/api/benchmark/latest\n\
         API: GET http://127.0.0.1:{port}/api/benchmark/runs/recent\n\
         API: POST http://127.0.0.1:{port}/api/benchmark/run-simulation\n\
         (uses DATABASE_URL; run `link` first.)"
    );
    axum::serve(listener, app.into_make_service())
        .await
        .context("axum serve")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::alchemy::Transfer;
    use crate::linking::link_and_persist;
    use crate::storage::{
        connect, run_migrations, BenchmarkEvalDetailRow, BenchmarkEvalMetricsRow, BenchmarkRun,
    };

    static TEST_DB_SEQ: AtomicU64 = AtomicU64::new(1);

    async fn test_repo() -> Repo {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let seq = TEST_DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let db_url = format!("sqlite://data/test_serve_{ts}_{seq}.db");
        let pool = connect(&db_url).await.expect("connect");
        run_migrations(&pool).await.expect("migrations");
        Repo::new(pool)
    }

    async fn body_string(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    fn xfer(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
        Transfer {
            from_addr: from.to_string(),
            to_addr: to.to_string(),
            value: Some("1".to_string()),
            block_num: Some(block),
            tx_hash: Some(tx.to_string()),
            asset: Some("ETH".to_string()),
        }
    }

    async fn repo_with_linked_pair() -> Repo {
        let repo = test_repo().await;
        let funder = "0xff11ff11ff11ff11ff11ff11ff11ff11ff11ff11";
        let alice = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        repo.insert_transfer(&xfer(funder, alice, 100, "0x1"))
            .await
            .expect("xfer1");
        repo.insert_transfer(&xfer(funder, bob, 101, "0x2"))
            .await
            .expect("xfer2");
        repo.upsert_address(alice, Some(100)).await.expect("addr1");
        repo.upsert_address(bob, Some(101)).await.expect("addr2");
        link_and_persist(&repo, &[alice.into(), bob.into()], 1)
            .await
            .expect("link");
        repo
    }

    async fn repo_with_benchmark_eval() -> Repo {
        let repo = test_repo().await;
        let run = BenchmarkRun {
            benchmark_run_id: "bench-serve-1".to_string(),
            scenario_suite_id: "suite-serve".to_string(),
            scenario_id: "S5_service_hub_contaminated".to_string(),
            seed: 4242,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-serve".to_string(),
            code_commit: "commit-serve".to_string(),
        };
        repo.start_benchmark_run(&run)
            .await
            .expect("start benchmark run");
        repo.insert_benchmark_eval_metrics(&BenchmarkEvalMetricsRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            policy_variant: "naive_funded_by".to_string(),
            precision: 0.1,
            recall: 1.0,
            f1: 0.18,
            over_merge_rate: 0.9,
            under_merge_rate: 0.0,
            giant_component_inflation: 5.0,
            cluster_purity: 0.2,
            cluster_fragmentation: 1.0,
            calibration_json_by_evidence_kind: None,
        })
        .await
        .expect("insert metrics");
        repo.insert_benchmark_eval_details(&[BenchmarkEvalDetailRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            policy_variant: "naive_funded_by".to_string(),
            truth_entity_id: "ent_00000".to_string(),
            matched_pred_cluster_id: Some("cluster_1".to_string()),
            split_count: 1,
            merge_intrusion_count: 3,
            dominant_error_kind: Some("over_merge".to_string()),
            detail_json: Some("{\"truth_wallet_count\":2}".to_string()),
        }])
        .await
        .expect("insert details");
        repo
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        assert_eq!(health().await, "ok");
    }

    #[tokio::test]
    async fn api_graph_rejects_unknown_mode() {
        let repo = test_repo().await;
        let resp = api_graph(
            State(repo),
            Query(GraphQuery {
                mode: Some("unknown".to_string()),
                max_identifier_nodes: None,
                max_evidence_nodes: None,
                fan_out_cap: None,
                max_pairwise_links: None,
                linkage_params: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_string(resp).await;
        assert!(body.contains("unknown mode"));
    }

    #[tokio::test]
    async fn api_graph_pairwise_rejects_bad_linkage_params_path() {
        let repo = test_repo().await;
        let resp = api_graph(
            State(repo),
            Query(GraphQuery {
                mode: Some("pairwise".to_string()),
                max_identifier_nodes: Some(10),
                max_evidence_nodes: None,
                fan_out_cap: Some(50),
                max_pairwise_links: Some(20),
                linkage_params: Some("does/not/exist.json".to_string()),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_string(resp).await;
        assert!(body.contains("linkage params"));
    }

    #[tokio::test]
    async fn api_graph_returns_not_found_when_no_clustering_runs() {
        let repo = test_repo().await;
        let resp = api_graph(
            State(repo),
            Query(GraphQuery {
                mode: Some("evidence".to_string()),
                max_identifier_nodes: Some(20),
                max_evidence_nodes: Some(20),
                fan_out_cap: Some(50),
                max_pairwise_links: None,
                linkage_params: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("no clustering runs"));
    }

    #[tokio::test]
    async fn api_graph_evidence_returns_ok_after_linking() {
        let repo = repo_with_linked_pair().await;
        let resp = api_graph(
            State(repo),
            Query(GraphQuery {
                mode: None,
                max_identifier_nodes: Some(50),
                max_evidence_nodes: Some(50),
                fan_out_cap: Some(50),
                max_pairwise_links: None,
                linkage_params: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"graph_mode\""));
        assert!(body.contains("evidence"));
    }

    #[tokio::test]
    async fn api_graph_pairwise_returns_ok_after_linking() {
        let repo = repo_with_linked_pair().await;
        let resp = api_graph(
            State(repo),
            Query(GraphQuery {
                mode: Some("pairwise".to_string()),
                max_identifier_nodes: Some(50),
                max_evidence_nodes: None,
                fan_out_cap: Some(50),
                max_pairwise_links: Some(500),
                linkage_params: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"graph_mode\""));
        assert!(body.contains("pairwise"));
    }

    #[tokio::test]
    async fn api_benchmark_latest_returns_ok_when_data_exists() {
        let repo = repo_with_benchmark_eval().await;
        let resp = api_benchmark_latest(State(repo)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"benchmark_run_id\":\"bench-serve-1\""));
        assert!(body.contains("\"metrics\""));
    }

    #[tokio::test]
    async fn api_benchmark_run_returns_not_found_when_missing() {
        let repo = test_repo().await;
        let resp = api_benchmark_run(State(repo), AxumPath("missing-run".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("not found"));
    }

    #[tokio::test]
    async fn api_benchmark_runs_recent_returns_rows() {
        let repo = repo_with_benchmark_eval().await;
        let resp = api_benchmark_runs_recent(State(repo)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"benchmark_run_id\":\"bench-serve-1\""));
    }

    #[tokio::test]
    async fn api_benchmark_run_simulation_triggers_and_returns_payload() {
        let repo = test_repo().await;
        let req = SimulationTriggerRequest {
            scenario_id: Some("S9_negative_control_only".to_string()),
            seed: Some(7),
            suite_id: Some("suite-test-ui".to_string()),
            conservative_service_fan_out_cap: Some(15),
        };
        let resp = api_benchmark_run_simulation(State(repo), AxumJson(req)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"run\""));
        assert!(body.contains("\"metrics\""));
    }

    #[tokio::test]
    async fn api_benchmark_graph_returns_graph_json() {
        let repo = repo_with_benchmark_eval().await;
        // Add minimal policy assignment + synthetic evidence so graph endpoint has data.
        sqlx::query(
            "INSERT INTO benchmark_policy_results
             (benchmark_run_id, policy_variant, pred_cluster_id, wallet_id, link_explanation_json)
             VALUES (?1, 'naive_funded_by', 'cluster_1', '0xabc', NULL)",
        )
        .bind("bench-serve-1")
        .execute(repo.pool())
        .await
        .expect("insert policy row");
        sqlx::query(
            "INSERT INTO benchmark_synthetic_evidence
             (benchmark_run_id, evidence_id, subject_wallet_id, counterparty_id, evidence_kind, strength_hint, event_time_bucket, sequence_index, metadata_json)
             VALUES (?1, 'e1', '0xabc', '0xfunder', 'funded_by', 'medium', 't0', 1, NULL)",
        )
        .bind("bench-serve-1")
        .execute(repo.pool())
        .await
        .expect("insert synthetic evidence");

        let resp = api_benchmark_graph(
            State(repo),
            AxumPath("bench-serve-1".to_string()),
            Query(BenchmarkGraphQuery {
                policy_variant: Some("naive_funded_by".to_string()),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"graph_mode\":\"benchmark\""));
        assert!(body.contains("\"nodes\""));
        assert!(body.contains("\"links\""));
    }
}
