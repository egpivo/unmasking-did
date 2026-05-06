//! Local HTTP server: graph JSON from SQLite (`GET /api/graph`) plus static files.
//!
//! Browsers cannot open `sqlite://` URLs. This module runs the same
//! [`crate::graph_export`] builders as `export-graph`, so the viewer can
//! `fetch("/api/graph")` against the current `DATABASE_URL` after `link`.

use std::path::Path;

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use tower_http::services::ServeDir;

use crate::graph_export::{
    build_graph, build_pairwise_graph, DEFAULT_FAN_OUT_CAP, DEFAULT_MAX_EVIDENCE_NODES,
    DEFAULT_MAX_IDENTIFIER_NODES,
};
use crate::linking::LinkageParams;
use crate::storage::Repo;

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

async fn health() -> &'static str {
    "ok"
}

/// Bind `0.0.0.0:port` and serve until process exit. **CWD** should be the
/// repo root so `viewer/`, `out/`, and `data/findings/` paths resolve.
pub async fn run(repo: Repo, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/api/graph", get(api_graph))
        .route("/health", get(health))
        .with_state(repo)
        .nest_service("/viewer", ServeDir::new("viewer"))
        .nest_service("/out", ServeDir::new("out"))
        .nest_service("/data/findings", ServeDir::new("data/findings"));

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    tracing::info!(%addr, "serve: GET /api/graph  static /viewer /out /data/findings");
    eprintln!(
        "unmasking-did serve: http://127.0.0.1:{port}/viewer/graph-explorer.html\n\
         API: GET http://127.0.0.1:{port}/api/graph?mode=evidence&max_evidence_nodes=120\n\
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

    use crate::storage::{connect, run_migrations};

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
}
