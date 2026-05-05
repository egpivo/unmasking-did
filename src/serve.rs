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
