//! Arbitrum governance + control cohort pipeline (strict guardrails).
//!
//! Ingests exactly the stratified seed CSVs (no expansion), bounded one-hop
//! transfer caching via Alchemy, then runs the existing evidence extractors
//! and deterministic merge rules. Invoked from the CLI as `arbitrum-gov`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::alchemy::client::AlchemyClient;
use crate::config::{
    arbitrum_alchemy_api_key_from_env, arbitrum_alchemy_base_url_from_env, Config,
};
use crate::graph_export::{build_graph, write_graph_json};
use crate::ingest_common::{normalize_eth_address, store_safe_owners};
use crate::linking::{
    link_and_persist_with_fanout, ClusterReport, FundedByMergePolicy, LinkingOutput, SkippedKey,
    FAN_OUT_CAP,
};
use crate::monitoring::lineage::{
    cluster_snapshots_from_map, compute_cluster_lineage, should_run_lineage, LineageConfig,
};
use crate::resolvers::{EnsResolver, SafeResolver};
use crate::storage::{connect, run_migrations, DatasetRun, Repo};

/// Frozen Arbitrum window (matches Phase 1 / 1b run spec).
pub const WINDOW_START_BLOCK: u64 = 428_203_933;
pub const WINDOW_END_BLOCK: u64 = 459_307_198;

/// Safe Transaction Service host for Arbitrum One Safes.
pub const DEFAULT_ARBITRUM_SAFE_TX_SERVICE_URL: &str =
    "https://safe-transaction-arbitrum.safe.global";

/// L2-safe categories (`internal` is rejected outside ETH/MATIC on Alchemy).
const ARBITRUM_TRANSFER_CATEGORIES: &[&str] = &["external", "erc20"];

/// Merge-time `(kind, key)` fan-out cap for service-like suppression (Arbitrum gov cohort spec).
pub const ARBITRUM_GOV_LINK_FANOUT_CAP: usize = 1000;

const DEFAULT_GOV_CSV: &str = "data/seeds/arbitrum_gov_90d_governance_stratified500.csv";
const DEFAULT_CTL_CSV: &str = "data/seeds/arbitrum_gov_90d_control_stratified500.csv";
const DEFAULT_PHASE1B_JSON: &str = "out/phase1b_arbitrum_gov_seed_quality.json";
const DEFAULT_DB: &str = "data/unmask_arbitrum_gov_v1.db";
const DEFAULT_REPORT: &str = "out/arbitrum_gov_report.md";
const DEFAULT_GRAPH: &str = "out/arbitrum_gov.graph.json";
const DEFAULT_SUMMARY_JSON: &str = "out/arbitrum_gov_summary.json";

/// Total `alchemy_getAssetTransfers` rows cached per seed (incoming + outgoing).
const MAX_TRANSFER_ROWS_PER_SEED: usize = 250;
const MAX_PAGES_PER_DIRECTION: usize = 30;
const EARLY_STOP_DISTINCT_PEERS: usize = 220;

const MAX_DB_BYTES: u64 = 450_000_000;
const PAGINATION_BIAS_WARN_FRAC: f64 = 0.20;
const SERVICE_DOMINANCE_MIN_CLUSTER: usize = 950;
const POLICY_PROFILE_ID: &str = "arbitrum_gov_conservative_v1";
const STABLE_THRESHOLD: f64 = 0.5;
const RELATED_THRESHOLD: f64 = 0.1;
const SKIP_WINDOW_NOT_SET: &str = "Lineage skipped because monitoring window is not set.";
const SKIP_PREV_WINDOW_NOT_SET: &str =
    "Lineage skipped because previous comparable run has no monitoring window.";
const SKIP_NO_PREV: &str =
    "No prior same-profile run available; lineage not computed for this run.";

#[derive(Debug, Clone)]
pub struct ArbitrumGovPaths {
    pub governance_csv: PathBuf,
    pub control_csv: PathBuf,
    pub phase1b_json: PathBuf,
    pub database_url: String,
    pub report_md: PathBuf,
    pub graph_json: PathBuf,
    pub summary_json: PathBuf,
    pub funder_denylist_txt: Option<PathBuf>,
}

/// Remove Arbitrum governance cohort artifacts after a failed run so a partial SQLite file or
/// stale outputs are not mistaken for a completed pipeline.
pub fn cleanup_partial_arbitrum_gov_artifacts(paths: &ArbitrumGovPaths) {
    if let Some(p) = paths.database_url.strip_prefix("sqlite://") {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(&paths.report_md);
    let _ = std::fs::remove_file(&paths.graph_json);
    let _ = std::fs::remove_file(&paths.summary_json);
}

impl Default for ArbitrumGovPaths {
    fn default() -> Self {
        Self {
            governance_csv: PathBuf::from(DEFAULT_GOV_CSV),
            control_csv: PathBuf::from(DEFAULT_CTL_CSV),
            phase1b_json: PathBuf::from(DEFAULT_PHASE1B_JSON),
            database_url: format!("sqlite://{DEFAULT_DB}"),
            report_md: PathBuf::from(DEFAULT_REPORT),
            graph_json: PathBuf::from(DEFAULT_GRAPH),
            summary_json: PathBuf::from(DEFAULT_SUMMARY_JSON),
            funder_denylist_txt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArbitrumGovSummary {
    pub database_url: String,
    pub alchemy_base_url_used: String,
    /// Which env var supplied the key (`ARBITRUM_ALCHEMY_API_KEY` or `ALCHEMY_API_KEY`); key is never stored.
    pub arbitrum_alchemy_key_source: String,
    pub safe_tx_service_url_used: String,
    pub input_snapshot_hash: String,
    pub policy_profile_id: String,
    pub stable_threshold: f64,
    pub related_threshold: f64,
    pub chain_notes: String,
    pub seed_counts: SeedCounts,
    pub alchemy_calls: u64,
    pub is_contract_calls: u64,
    pub transfers_rows_inserted: usize,
    pub pagination_cap_hits: PaginationCapHits,
    pub pagination_bias_risk: bool,
    pub db_size_bytes: u64,
    pub db_size_stopped: bool,
    pub link_fanout_cap: usize,
    pub min_evidence: usize,
    pub run_id: String,
    pub n_clusters: usize,
    pub n_addresses_clustered: usize,
    pub top_clusters: Vec<ClusterSummary>,
    pub lineage: LineageSummary,
    pub anomalies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeedCounts {
    pub governance: usize,
    pub control: usize,
    pub total: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct PaginationCapHits {
    pub row_cap: usize,
    pub page_cap: usize,
    pub distinct_peer_cap: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterSummary {
    pub cluster_id: String,
    pub size: usize,
    pub coordination_tier: String,
    pub shared_evidence_keys: Vec<String>,
    pub governance_count: usize,
    pub control_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineageSummary {
    pub enabled: bool,
    pub skip_reason: Option<String>,
    pub previous_run_id: Option<String>,
    pub counts: LineageCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineageCounts {
    pub stable: usize,
    pub related: usize,
    pub new: usize,
    pub disappeared: usize,
    pub total_rows: usize,
}

#[derive(Debug, Clone)]
struct SeedRow {
    address: String,
    first_seen_block: i64,
    role: &'static str,
}

#[allow(clippy::too_many_arguments)]
fn build_lineage_summary(
    current_run_id: &str,
    current_chain: &str,
    current_profile: &str,
    current_window_start: i64,
    current_window_end: i64,
    previous_run: Option<&crate::storage::repo::DatasetRunSummary>,
    current_clusters: Option<&HashMap<String, Vec<String>>>,
    previous_clusters: Option<&HashMap<String, Vec<String>>>,
) -> Result<(LineageSummary, Vec<crate::storage::repo::ClusterLineageRow>)> {
    let mut lineage = LineageSummary {
        enabled: false,
        skip_reason: None,
        previous_run_id: previous_run.map(|p| p.run_id.clone()),
        counts: LineageCounts {
            stable: 0,
            related: 0,
            new: 0,
            disappeared: 0,
            total_rows: 0,
        },
    };
    if current_window_start == 0 && current_window_end == 0 {
        lineage.skip_reason = Some(SKIP_WINDOW_NOT_SET.to_string());
        return Ok((lineage, Vec::new()));
    }
    let Some(prev) = previous_run else {
        lineage.skip_reason = Some(SKIP_NO_PREV.to_string());
        return Ok((lineage, Vec::new()));
    };
    if prev.window_start_block == 0 && prev.window_end_block == 0 {
        lineage.skip_reason = Some(SKIP_PREV_WINDOW_NOT_SET.to_string());
        return Ok((lineage, Vec::new()));
    }
    if !should_run_lineage(
        current_chain,
        current_profile,
        current_window_start,
        current_window_end,
        &prev.chain,
        &prev.policy_profile_id,
        prev.window_start_block,
        prev.window_end_block,
    ) {
        lineage.skip_reason = Some(SKIP_PREV_WINDOW_NOT_SET.to_string());
        return Ok((lineage, Vec::new()));
    }
    let current_map = current_clusters
        .ok_or_else(|| anyhow!("current cluster map is required when lineage is enabled"))?;
    let previous_map = previous_clusters
        .ok_or_else(|| anyhow!("previous cluster map is required when lineage is enabled"))?;
    let rows = compute_cluster_lineage(
        current_run_id,
        &prev.run_id,
        &cluster_snapshots_from_map(current_map),
        &cluster_snapshots_from_map(previous_map),
        &LineageConfig {
            stable_threshold: STABLE_THRESHOLD,
            related_threshold: RELATED_THRESHOLD,
        },
    );
    lineage.enabled = true;
    lineage.counts.total_rows = rows.len();
    for row in &rows {
        match row.transition_label.as_str() {
            "stable" => lineage.counts.stable += 1,
            "related" => lineage.counts.related += 1,
            "new" => lineage.counts.new += 1,
            "disappeared" => lineage.counts.disappeared += 1,
            _ => {}
        }
    }
    Ok((lineage, rows))
}

fn read_snapshot_hash(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read phase1b json: {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&raw).context("parse phase1b json")?;
    v.get("input_snapshot_hash")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{} missing input_snapshot_hash", path.display()))
}

fn load_optional_funder_deny(path: &Path) -> Result<HashSet<String>> {
    let mut s = HashSet::new();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read funder denylist {}", path.display()))?;
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        s.insert(normalize_eth_address(t)?);
    }
    Ok(s)
}

fn parse_seed_csv(path: &Path, seed_type: &'static str) -> Result<Vec<SeedRow>> {
    let mut rdr = csv::Reader::from_path(path)
        .with_context(|| format!("open seed csv {}", path.display()))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let addr = rec
            .get(0)
            .ok_or_else(|| anyhow!("{}: missing address column", path.display()))?;
        let blk = rec
            .get(1)
            .ok_or_else(|| anyhow!("{}: missing first_seen_block", path.display()))?
            .parse::<i64>()
            .with_context(|| format!("{}: bad block", path.display()))?;
        out.push(SeedRow {
            address: normalize_eth_address(addr)?,
            first_seen_block: blk,
            role: seed_type,
        });
    }
    Ok(out)
}

fn coordination_tier(c: &ClusterReport, gov: &HashSet<String>, ctl: &HashSet<String>) -> String {
    let g = c.addresses.iter().filter(|a| gov.contains(*a)).count();
    let ct = c.addresses.iter().filter(|a| ctl.contains(*a)).count();
    if c.addresses.len() == 1 {
        if ct == 1 {
            return "singleton_negative_control_reference".to_string();
        }
        if g == 1 {
            return "singleton_governance_seed".to_string();
        }
        return "singleton".to_string();
    }
    if ct == c.addresses.len() {
        return "multi_identifier_coordination_control_only".to_string();
    }
    if g == c.addresses.len() {
        return "multi_identifier_coordination_governance_only".to_string();
    }
    if g > 0 && ct > 0 {
        return "multi_identifier_coordination_mixed_governance_and_control".to_string();
    }
    "multi_identifier_coordination_candidate".to_string()
}

fn cluster_summary(
    c: &ClusterReport,
    gov: &HashSet<String>,
    ctl: &HashSet<String>,
) -> ClusterSummary {
    let governance_count = c.addresses.iter().filter(|a| gov.contains(*a)).count();
    let control_count = c.addresses.iter().filter(|a| ctl.contains(*a)).count();
    ClusterSummary {
        cluster_id: c.cluster_id.clone(),
        size: c.addresses.len(),
        coordination_tier: coordination_tier(c, gov, ctl),
        shared_evidence_keys: c.shared_evidence_keys.clone(),
        governance_count,
        control_count,
    }
}

fn skipped_key_set(skipped: &[SkippedKey]) -> HashSet<(String, String)> {
    skipped
        .iter()
        .map(|s| (s.kind.clone(), s.key.to_lowercase()))
        .collect()
}

fn cluster_evidence_dominated_by_skipped(
    c: &ClusterReport,
    skipped: &HashSet<(String, String)>,
) -> bool {
    if c.shared_evidence_keys.is_empty() {
        return false;
    }
    c.shared_evidence_keys.iter().all(|k| {
        skipped.contains(&("funded_by".to_string(), k.to_lowercase()))
            || skipped.contains(&("ens_handle".to_string(), k.to_lowercase()))
            || skipped.contains(&("safe_owner".to_string(), k.to_lowercase()))
            || skipped.contains(&("did_controller".to_string(), k.to_lowercase()))
    })
}

/// Run the Arbitrum governance + control cohort pipeline end-to-end. Arbitrum Alchemy URL + API key are resolved only via
/// [`crate::config::arbitrum_alchemy_api_key_from_env`] / [`crate::config::arbitrum_alchemy_base_url_from_env`].
/// `cfg` supplies ENS resolver URL and other defaults; isolated DB uses `paths.database_url`.
pub async fn run_arbitrum_gov_pipeline(
    cfg: &Config,
    paths: &ArbitrumGovPaths,
    min_evidence: usize,
    overwrite_db: bool,
) -> Result<ArbitrumGovSummary> {
    let (alchemy_api_key, key_source_static) = arbitrum_alchemy_api_key_from_env()?;
    let arb_base = arbitrum_alchemy_base_url_from_env();
    let key_source = key_source_static.to_string();
    info!(
        "Using Arbitrum Alchemy endpoint: {} (key source: {})",
        arb_base.trim_end_matches('/'),
        key_source_static
    );

    let safe_tx_base = std::env::var("ARBITRUM_SAFE_TX_SERVICE_URL")
        .unwrap_or_else(|_| DEFAULT_ARBITRUM_SAFE_TX_SERVICE_URL.to_string());
    let ab = arb_base.to_lowercase();
    if !(ab.contains("arbitrum") || ab.contains("arb-mainnet") || ab.contains("arb-sepolia")) {
        warn!(
            %arb_base,
            "ARBITRUM_ALCHEMY_BASE_URL (or default) should target Arbitrum on Alchemy"
        );
    }

    let input_snapshot_hash = read_snapshot_hash(&paths.phase1b_json)?;
    let gov_rows = parse_seed_csv(&paths.governance_csv, "governance")?;
    let ctl_rows = parse_seed_csv(&paths.control_csv, "control")?;
    if gov_rows.len() != 500 {
        return Err(anyhow!(
            "expected 500 governance seeds, got {}",
            gov_rows.len()
        ));
    }
    if ctl_rows.len() != 500 {
        return Err(anyhow!(
            "expected 500 control seeds, got {}",
            ctl_rows.len()
        ));
    }

    let mut seeds: Vec<SeedRow> = Vec::new();
    seeds.extend(gov_rows);
    seeds.extend(ctl_rows);
    seeds.sort_by(|a, b| a.address.cmp(&b.address));

    let gov_set: HashSet<String> = seeds
        .iter()
        .filter(|s| s.role == "governance")
        .map(|s| s.address.clone())
        .collect();
    let ctl_set: HashSet<String> = seeds
        .iter()
        .filter(|s| s.role == "control")
        .map(|s| s.address.clone())
        .collect();

    let extra_deny: Option<HashSet<String>> = match &paths.funder_denylist_txt {
        Some(p) => Some(load_optional_funder_deny(p)?),
        None => {
            let default_deny = PathBuf::from("data/arbitrum_gov_funder_denylist.txt");
            if default_deny.is_file() {
                Some(load_optional_funder_deny(&default_deny)?)
            } else {
                None
            }
        }
    };

    if let Some(ref url) = paths.database_url.strip_prefix("sqlite://") {
        let p = Path::new(url);
        if overwrite_db && p.exists() {
            std::fs::remove_file(p).with_context(|| format!("remove {}", p.display()))?;
        }
    }

    let pool = connect(&paths.database_url).await?;
    run_migrations(&pool).await?;
    let repo = Repo::new(pool);

    let client = AlchemyClient::with_base_url(&arb_base, &alchemy_api_key)
        .with_transfer_categories(
            ARBITRUM_TRANSFER_CATEGORIES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );

    let mut alchemy_calls: u64 = 0;
    let mut is_contract_calls: u64 = 0;
    let mut transfers_inserted: usize = 0;
    let mut cap_hits = PaginationCapHits::default();
    let win_lo = WINDOW_START_BLOCK as i64;
    let win_hi = WINDOW_END_BLOCK as i64;

    for s in &seeds {
        info!(address = %s.address, role = s.role, "arbitrum_gov ingest");

        let inc = client
            .get_asset_transfers_bounded(
                &s.address,
                Some(WINDOW_START_BLOCK),
                Some(WINDOW_END_BLOCK),
                Some("toAddress"),
                MAX_PAGES_PER_DIRECTION,
                MAX_TRANSFER_ROWS_PER_SEED,
                Some(EARLY_STOP_DISTINCT_PEERS),
            )
            .await?;
        alchemy_calls += inc.alchemy_calls;
        if inc.stopped_early_row_cap {
            cap_hits.row_cap += 1;
        }
        if inc.stopped_early_page_cap {
            cap_hits.page_cap += 1;
        }
        if inc.stopped_early_distinct_peers {
            cap_hits.distinct_peer_cap += 1;
        }

        let remaining = MAX_TRANSFER_ROWS_PER_SEED.saturating_sub(inc.transfers.len());
        let outg = if remaining == 0 {
            inc.transfers
        } else {
            let out = client
                .get_asset_transfers_bounded(
                    &s.address,
                    Some(WINDOW_START_BLOCK),
                    Some(WINDOW_END_BLOCK),
                    Some("fromAddress"),
                    MAX_PAGES_PER_DIRECTION,
                    remaining,
                    Some(EARLY_STOP_DISTINCT_PEERS),
                )
                .await?;
            alchemy_calls += out.alchemy_calls;
            if out.stopped_early_row_cap {
                cap_hits.row_cap += 1;
            }
            if out.stopped_early_page_cap {
                cap_hits.page_cap += 1;
            }
            if out.stopped_early_distinct_peers {
                cap_hits.distinct_peer_cap += 1;
            }
            let mut merged = inc.transfers;
            merged.extend(out.transfers);
            merged
        };

        let filtered: Vec<_> = outg
            .into_iter()
            .filter(|t| {
                let Some(b) = t.block_num else {
                    return false;
                };
                b >= win_lo && b <= win_hi
            })
            .collect();

        transfers_inserted += repo.insert_transfers(&filtered).await?;
        repo.upsert_address(&s.address, Some(s.first_seen_block))
            .await?;

        let ens_resolver = EnsResolver::new(&cfg.ens_resolver_url);
        match ens_resolver.resolve(&s.address).await {
            Ok(Some(record)) => {
                let _ = repo.upsert_ens_record(&record).await;
            }
            Ok(None) => {}
            Err(e) => warn!(error = %e, address = %s.address, "ENS resolve skipped"),
        }

        let safe_resolver = SafeResolver::new(&safe_tx_base);
        match safe_resolver
            .fetch_owners(&s.address, Some(s.first_seen_block))
            .await
        {
            Ok(Some(owners)) => {
                is_contract_calls += owners.len() as u64;
                let _ = store_safe_owners(&repo, &client, owners).await?;
            }
            Ok(None) => {}
            Err(e) => warn!(error = %e, address = %s.address, "Safe fetch skipped"),
        }

        let db_path = paths.database_url.strip_prefix("sqlite://").map(Path::new);
        if let Some(p) = db_path {
            if let Ok(meta) = std::fs::metadata(p) {
                if meta.len() > MAX_DB_BYTES {
                    return Err(anyhow!(
                        "SQLite DB {} exceeds size guard ({} bytes); stopping before link",
                        p.display(),
                        MAX_DB_BYTES
                    ));
                }
            }
        }
    }

    let db_path = paths
        .database_url
        .strip_prefix("sqlite://")
        .map(Path::new)
        .ok_or_else(|| anyhow!("expected sqlite:// URL"))?;
    let db_size_bytes = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

    let addresses: Vec<String> = seeds.iter().map(|s| s.address.clone()).collect();
    let previous_same_profile = repo
        .latest_dataset_run_for_chain_profile("arbitrum", POLICY_PROFILE_ID)
        .await?;
    let (run_id, output) = link_and_persist_with_fanout(
        &repo,
        &addresses,
        min_evidence,
        ARBITRUM_GOV_LINK_FANOUT_CAP,
        extra_deny.as_ref(),
        &FundedByMergePolicy::legacy_disabled(),
    )
    .await?;

    let skipped_set = skipped_key_set(&output.skipped_service_keys);
    for c in &output.clusters {
        if c.addresses.len() >= SERVICE_DOMINANCE_MIN_CLUSTER
            && cluster_evidence_dominated_by_skipped(c, &skipped_set)
        {
            return Err(anyhow!(
                "halt: cluster {} (size {}) appears dominated by service-like evidence keys (skipped_service_keys)",
                c.cluster_id,
                c.addresses.len()
            ));
        }
    }

    let n_addresses: usize = output.clusters.iter().map(|c| c.addresses.len()).sum();
    let n_clusters = output.clusters.len();

    let mut ranked: Vec<ClusterSummary> = output
        .clusters
        .iter()
        .map(|c| cluster_summary(c, &gov_set, &ctl_set))
        .collect();
    ranked.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| a.cluster_id.cmp(&b.cluster_id))
    });
    let top_clusters: Vec<ClusterSummary> = ranked.iter().take(3).cloned().collect();

    let capped_addrs = cap_hits.row_cap + cap_hits.page_cap + cap_hits.distinct_peer_cap;
    let pagination_bias_risk =
        (capped_addrs as f64) / (seeds.len() as f64) > PAGINATION_BIAS_WARN_FRAC;

    let mut anomalies: Vec<String> = Vec::new();
    if pagination_bias_risk {
        anomalies.push(format!(
            ">{}% of seed addresses hit a transfer pagination / fan-out stop (row/page/distinct-peer); partial transfer caches may bias funded_by / sink summaries",
            (PAGINATION_BIAS_WARN_FRAC * 100.0) as u32
        ));
    }
    let graph = build_graph(&repo, Some(&run_id), 120, 240, ARBITRUM_GOV_LINK_FANOUT_CAP).await?;
    if let Some(parent) = paths.graph_json.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    write_graph_json(&graph, &paths.graph_json)?;

    let ds_run = DatasetRun {
        run_id: run_id.clone(),
        chain: "arbitrum".to_string(),
        run_type: "monitor".to_string(),
        parent_run_id: previous_same_profile.as_ref().map(|r| r.run_id.clone()),
        window_start_block: WINDOW_START_BLOCK as i64,
        window_end_block: WINDOW_END_BLOCK as i64,
        window_start_ts: None,
        window_end_ts: None,
        cadence: "monthly".to_string(),
        seed_spec_json: serde_json::json!({
            "governance_csv": paths.governance_csv,
            "control_csv": paths.control_csv,
            "governance_count": gov_set.len(),
            "control_count": ctl_set.len()
        })
        .to_string(),
        params_json: serde_json::json!({
            "min_evidence": min_evidence,
            "link_fanout_cap": ARBITRUM_GOV_LINK_FANOUT_CAP,
            "funded_by_policy": FundedByMergePolicy::legacy_disabled()
        })
        .to_string(),
        input_snapshot_hash: input_snapshot_hash.clone(),
        code_commit: std::env::var("GIT_COMMIT").unwrap_or_else(|_| "unknown".to_string()),
        policy_profile_id: POLICY_PROFILE_ID.to_string(),
        stable_threshold: STABLE_THRESHOLD,
        related_threshold: RELATED_THRESHOLD,
        notes: None,
    };
    repo.start_dataset_run(&ds_run).await?;

    let current_cluster_map = repo.clusters_for_run_map(&run_id).await?;
    let previous_cluster_map = if let Some(prev) = &previous_same_profile {
        Some(repo.clusters_for_run_map(&prev.run_id).await?)
    } else {
        None
    };
    let (lineage, lineage_rows) = build_lineage_summary(
        &run_id,
        "arbitrum",
        POLICY_PROFILE_ID,
        WINDOW_START_BLOCK as i64,
        WINDOW_END_BLOCK as i64,
        previous_same_profile.as_ref(),
        Some(&current_cluster_map),
        previous_cluster_map.as_ref(),
    )?;
    if let Some(reason) = &lineage.skip_reason {
        warn!("{reason}");
    }
    if !lineage_rows.is_empty() {
        repo.insert_cluster_lineage_rows(&lineage_rows).await?;
    }

    let report_md = render_arbitrum_gov_markdown(
        paths,
        &arb_base,
        &key_source,
        &safe_tx_base,
        &input_snapshot_hash,
        &run_id,
        min_evidence,
        ARBITRUM_GOV_LINK_FANOUT_CAP,
        &output,
        &gov_set,
        &ctl_set,
        alchemy_calls,
        is_contract_calls,
        transfers_inserted,
        &cap_hits,
        pagination_bias_risk,
        db_size_bytes,
        &top_clusters,
        POLICY_PROFILE_ID,
        STABLE_THRESHOLD,
        RELATED_THRESHOLD,
        &lineage,
        &anomalies,
    );
    if let Some(parent) = paths.report_md.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&paths.report_md, report_md)?;

    let summary = ArbitrumGovSummary {
        database_url: paths.database_url.clone(),
        alchemy_base_url_used: arb_base.clone(),
        arbitrum_alchemy_key_source: key_source.clone(),
        safe_tx_service_url_used: safe_tx_base.clone(),
        input_snapshot_hash: input_snapshot_hash.clone(),
        policy_profile_id: POLICY_PROFILE_ID.to_string(),
        stable_threshold: STABLE_THRESHOLD,
        related_threshold: RELATED_THRESHOLD,
        chain_notes: "Arbitrum One; coordination and shared-public-signal framing only (no intent or real-world identity attribution).".to_string(),
        seed_counts: SeedCounts {
            governance: gov_set.len(),
            control: ctl_set.len(),
            total: seeds.len(),
        },
        alchemy_calls,
        is_contract_calls,
        transfers_rows_inserted: transfers_inserted,
        pagination_cap_hits: cap_hits,
        pagination_bias_risk,
        db_size_bytes,
        db_size_stopped: false,
        link_fanout_cap: ARBITRUM_GOV_LINK_FANOUT_CAP,
        min_evidence,
        run_id: run_id.clone(),
        n_clusters,
        n_addresses_clustered: n_addresses,
        top_clusters: top_clusters.clone(),
        lineage,
        anomalies: anomalies.clone(),
    };

    let summary_json = serde_json::to_string_pretty(&summary)?;
    if let Some(parent) = paths.summary_json.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&paths.summary_json, summary_json)?;

    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn render_arbitrum_gov_markdown(
    paths: &ArbitrumGovPaths,
    alchemy_base: &str,
    alchemy_key_source: &str,
    safe_tx_base: &str,
    input_snapshot_hash: &str,
    run_id: &str,
    min_evidence: usize,
    link_fanout: usize,
    output: &LinkingOutput,
    gov: &HashSet<String>,
    ctl: &HashSet<String>,
    alchemy_calls: u64,
    is_contract_calls: u64,
    transfers_inserted: usize,
    cap_hits: &PaginationCapHits,
    pagination_bias_risk: bool,
    db_size_bytes: u64,
    top_clusters: &[ClusterSummary],
    policy_profile_id: &str,
    stable_threshold: f64,
    related_threshold: f64,
    lineage: &LineageSummary,
    anomalies: &[String],
) -> String {
    let mut s = String::new();
    s.push_str("# Arbitrum governance / control cohort (research)\n\n");
    s.push_str("This report describes **on-chain coordination signals** among a fixed seed set. ");
    s.push_str("It does **not** assert malicious behavior, duplicate-identity farming, or same real-world human control.\n\n");
    s.push_str("**Disclaimers**\n\n");
    s.push_str("- **Governance participation is not malicious behavior.** Voting and delegation are ordinary protocol use.\n");
    s.push_str("- **Shared funder or sink activity does not imply the same human.** Public infrastructure (bridges, custodial exchanges, batch payers) creates dense shared-graph structure.\n");
    s.push_str("- **Coordination tier** names describe evidence topology only — not intent, ethics, or wrongdoing.\n\n");

    s.push_str("## Reproducibility\n\n");
    s.push_str(&format!(
        "- **Arbitrum Alchemy**: base `{}`, API key from **{}** (key value not recorded)\n",
        alchemy_base.trim_end_matches('/'),
        alchemy_key_source
    ));
    s.push_str(&format!(
        "- **input_snapshot_hash** (Phase 1b): `{input_snapshot_hash}`\n"
    ));
    s.push_str(&format!("- **clustering run_id**: `{run_id}`\n"));
    s.push_str(&format!("- **SQLite**: `{}`\n", paths.database_url));
    s.push_str(&format!(
        "- **Graph JSON**: `{}`\n",
        paths.graph_json.display()
    ));
    s.push_str(&format!(
        "- **Seeds**: `{}` + `{}` (do not modify; 1000 addresses total)\n",
        paths.governance_csv.display(),
        paths.control_csv.display()
    ));
    s.push_str(&format!(
        "- **Block window**: `{}` → `{}`\n",
        WINDOW_START_BLOCK, WINDOW_END_BLOCK
    ));
    s.push_str("- **Transfer categories (this pipeline)**: `external`, `erc20` only — ignores generic `ALCHEMY_BASE_URL` / `ALCHEMY_TRANSFER_CATEGORIES` from `.env` meant for other chains.\n");
    s.push_str(&format!(
        "- **Safe Transaction Service (this pipeline)**: `{}` (override with `ARBITRUM_SAFE_TX_SERVICE_URL`).\n\n",
        safe_tx_base.trim_end_matches('/')
    ));

    s.push_str("## Sink / outgoing concentration (audit SQL)\n\n");
    s.push_str("Outgoing “sink” concentration is **not** a separate evidence kind; inspect raw `transfers` where `from_addr` is a seed, e.g.:\n\n");
    s.push_str("```sql\nSELECT to_addr, COUNT(*) AS n FROM transfers\n");
    s.push_str("WHERE from_addr = LOWER(:seed) AND block_num BETWEEN ");
    s.push_str(&format!(
        "{} AND {}\n",
        WINDOW_START_BLOCK, WINDOW_END_BLOCK
    ));
    s.push_str("GROUP BY to_addr ORDER BY n DESC LIMIT 15;\n```\n\n");

    s.push_str("## Ingest guardrails\n\n");
    s.push_str("- **One-hop only**: transfers cached for seed addresses inside the frozen window; no recursive neighbor ingest.\n");
    s.push_str(&format!(
        "- **Per-seed transfer cap**: ≤{MAX_TRANSFER_ROWS_PER_SEED} rows combined (incoming then outgoing), with early stop when distinct counterparties ≥ {EARLY_STOP_DISTINCT_PEERS}.\n"
    ));
    s.push_str(&format!(
        "- **Alchemy `alchemy_getAssetTransfers` calls** (approx): **{alchemy_calls}**; **`eth_getCode` probes** (Safe owner refinement): **{is_contract_calls}**\n"
    ));
    s.push_str(&format!(
        "- **Transfer rows inserted** (deduped): **{transfers_inserted}**\n"
    ));
    s.push_str(&format!(
        "- **Pagination stop counts** (seeds): row_cap={}, page_cap={}, distinct_peer_cap={}\n",
        cap_hits.row_cap, cap_hits.page_cap, cap_hits.distinct_peer_cap
    ));
    if pagination_bias_risk {
        s.push_str("- **Bias risk**: a large fraction of seeds hit pagination / fan-out stops — treat funding / sink summaries as **lower confidence**.\n");
    }
    s.push_str(&format!("- **DB file size**: {} bytes\n\n", db_size_bytes));

    s.push_str("## Linking parameters\n\n");
    s.push_str(&format!(
        "- **min_evidence**: {min_evidence}\n- **link fan-out cap** (service-like `(kind,key)` suppression): **{link_fanout}** (default rule linker uses {} for generic runs)\n\n",
        FAN_OUT_CAP
    ));

    s.push_str("## Lineage\n\n");
    s.push_str(&format!(
        "- **policy_profile_id**: `{policy_profile_id}`\n- **stable_threshold**: `{stable_threshold}`\n- **related_threshold**: `{related_threshold}`\n"
    ));
    if let Some(prev) = &lineage.previous_run_id {
        s.push_str(&format!("- **previous_run_id**: `{prev}`\n"));
    } else {
        s.push_str("- **previous_run_id**: `none`\n");
    }
    if lineage.enabled {
        s.push_str("- **lineage_enabled**: `true`\n");
    } else {
        s.push_str("- **lineage_enabled**: `false`\n");
        if let Some(reason) = &lineage.skip_reason {
            s.push_str(&format!("- **lineage_note**: {reason}\n"));
        }
    }
    s.push_str(&format!(
        "- **lineage_counts**: stable={}, related={}, new={}, disappeared={}, total_rows={}\n\n",
        lineage.counts.stable,
        lineage.counts.related,
        lineage.counts.new,
        lineage.counts.disappeared,
        lineage.counts.total_rows
    ));

    s.push_str("## Coordination clusters (illustrative)\n\n");
    let pos = output
        .clusters
        .iter()
        .find(|c| c.addresses.len() > 1 && c.addresses.iter().any(|a| gov.contains(a)));
    let neg = output
        .clusters
        .iter()
        .find(|c| c.addresses.len() > 1 && c.addresses.iter().all(|a| ctl.contains(a)));
    if let Some(c) = pos {
        s.push_str("### Positive coordination example (governance-involved, multi-address)\n\n");
        s.push_str(&format!("- **cluster_id**: `{}`\n", c.cluster_id));
        s.push_str(&format!("- **size**: {}\n", c.addresses.len()));
        s.push_str(&format!(
            "- **shared_evidence_keys**: `{:?}`\n\n",
            c.shared_evidence_keys
        ));
    } else {
        s.push_str("### Positive coordination example\n\n_No multi-address cluster contained a governance seed in this run._\n\n");
    }
    if let Some(c) = neg {
        s.push_str("### Negative-control example (control-only multi-address)\n\n");
        s.push_str(&format!("- **cluster_id**: `{}`\n", c.cluster_id));
        s.push_str(&format!("- **size**: {}\n", c.addresses.len()));
        s.push_str(&format!(
            "- **shared_evidence_keys**: `{:?}`\n\n",
            c.shared_evidence_keys
        ));
    } else {
        s.push_str("### Negative-control example\n\n_No multi-address cluster contained only control seeds in this run._\n\n");
    }

    s.push_str("## Top clusters (by size)\n\n");
    for c in top_clusters {
        s.push_str(&format!(
            "- `{}` — size **{}**, tier **{}**, gov **{}** / control **{}**, keys {:?}\n",
            c.cluster_id,
            c.size,
            c.coordination_tier,
            c.governance_count,
            c.control_count,
            c.shared_evidence_keys
        ));
    }
    s.push('\n');

    s.push_str("## Skipped service-like keys (audit)\n\n");
    let lim = output.skipped_service_keys.len().min(40);
    for sk in output.skipped_service_keys.iter().take(lim) {
        s.push_str(&format!(
            "- `{}` / `{}` — fan-out **{}**\n",
            sk.kind, sk.key, sk.fan_out
        ));
    }
    if output.skipped_service_keys.len() > lim {
        s.push_str(&format!(
            "\n_…{} more entries in `suspected_service_keys` table._\n",
            output.skipped_service_keys.len() - lim
        ));
    }
    s.push('\n');

    if !anomalies.is_empty() {
        s.push_str("## Anomalies / cautions\n\n");
        for a in anomalies {
            s.push_str(&format!("- {a}\n"));
        }
        s.push('\n');
    }

    s.push_str("## Optional D3 viewer\n\n");
    s.push_str("After `cargo run -- serve`, open `viewer/graph-explorer.html` and load the graph JSON path above (same bounded evidence graph as `export-graph`).\n");

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linking::ClusterReport;
    use crate::storage::repo::DatasetRunSummary;
    use std::collections::{HashMap, HashSet};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn prev_summary(window_start: i64, window_end: i64) -> DatasetRunSummary {
        DatasetRunSummary {
            run_id: "prev-run".to_string(),
            chain: "arbitrum".to_string(),
            run_type: "monitor".to_string(),
            parent_run_id: None,
            window_start_block: window_start,
            window_end_block: window_end,
            cadence: "monthly".to_string(),
            input_snapshot_hash: "x".to_string(),
            code_commit: "c".to_string(),
            policy_profile_id: POLICY_PROFILE_ID.to_string(),
            stable_threshold: STABLE_THRESHOLD,
            related_threshold: RELATED_THRESHOLD,
            created_at: "now".to_string(),
        }
    }

    #[test]
    fn lineage_skips_when_current_window_missing() {
        let (lineage, rows) = build_lineage_summary(
            "run",
            "arbitrum",
            POLICY_PROFILE_ID,
            0,
            0,
            Some(&prev_summary(100, 200)),
            Some(&HashMap::new()),
            Some(&HashMap::new()),
        )
        .expect("lineage helper should not fail");
        assert!(!lineage.enabled);
        assert_eq!(lineage.skip_reason.as_deref(), Some(SKIP_WINDOW_NOT_SET));
        assert!(rows.is_empty());
    }

    #[test]
    fn lineage_skips_when_previous_window_missing() {
        let mut cur = HashMap::new();
        cur.insert("c1".to_string(), vec!["0x1".to_string()]);
        let mut prev = HashMap::new();
        prev.insert("p1".to_string(), vec!["0x1".to_string()]);
        let (lineage, rows) = build_lineage_summary(
            "run",
            "arbitrum",
            POLICY_PROFILE_ID,
            100,
            200,
            Some(&prev_summary(0, 0)),
            Some(&cur),
            Some(&prev),
        )
        .expect("lineage helper should not fail");
        assert!(!lineage.enabled);
        assert_eq!(
            lineage.skip_reason.as_deref(),
            Some(SKIP_PREV_WINDOW_NOT_SET)
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn markdown_includes_lineage_section() {
        let output = LinkingOutput {
            clusters: vec![ClusterReport {
                cluster_id: "0x1".to_string(),
                addresses: vec!["0x1".to_string(), "0x2".to_string()],
                shared_evidence_keys: vec!["k".to_string()],
            }],
            skipped_service_keys: vec![],
        };
        let top = vec![ClusterSummary {
            cluster_id: "0x1".to_string(),
            size: 2,
            coordination_tier: "multi_identifier_coordination_governance_only".to_string(),
            shared_evidence_keys: vec!["k".to_string()],
            governance_count: 2,
            control_count: 0,
        }];
        let md = render_arbitrum_gov_markdown(
            &ArbitrumGovPaths::default(),
            "https://arb-mainnet.g.alchemy.com/v2",
            "ARBITRUM_ALCHEMY_API_KEY",
            "https://safe-transaction-arbitrum.safe.global",
            "hash",
            "run",
            1,
            ARBITRUM_GOV_LINK_FANOUT_CAP,
            &output,
            &HashSet::new(),
            &HashSet::new(),
            0,
            0,
            0,
            &PaginationCapHits::default(),
            false,
            0,
            &top,
            POLICY_PROFILE_ID,
            STABLE_THRESHOLD,
            RELATED_THRESHOLD,
            &LineageSummary {
                enabled: false,
                skip_reason: Some(SKIP_NO_PREV.to_string()),
                previous_run_id: None,
                counts: LineageCounts {
                    stable: 0,
                    related: 0,
                    new: 0,
                    disappeared: 0,
                    total_rows: 0,
                },
            },
            &[],
        );
        assert!(md.contains("## Lineage"));
        assert!(md.contains(POLICY_PROFILE_ID));
        assert!(md.contains(SKIP_NO_PREV));
    }

    #[test]
    fn explicit_windows_allow_lineage_computation() {
        let mut cur = HashMap::new();
        cur.insert("c1".to_string(), vec!["0x1".to_string(), "0x2".to_string()]);
        let mut prev = HashMap::new();
        prev.insert("p1".to_string(), vec!["0x1".to_string(), "0x2".to_string()]);
        let (lineage, rows) = build_lineage_summary(
            "run",
            "arbitrum",
            POLICY_PROFILE_ID,
            100,
            200,
            Some(&prev_summary(1, 99)),
            Some(&cur),
            Some(&prev),
        )
        .expect("lineage helper should not fail");
        assert!(lineage.enabled);
        assert_eq!(lineage.counts.stable, 1);
        assert_eq!(lineage.counts.total_rows, 1);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn coordination_tier_covers_singleton_and_mixed_cases() {
        let mut gov = HashSet::new();
        gov.insert("0xg".to_string());
        let mut ctl = HashSet::new();
        ctl.insert("0xc".to_string());

        let singleton_g = ClusterReport {
            cluster_id: "0xg".to_string(),
            addresses: vec!["0xg".to_string()],
            shared_evidence_keys: vec![],
        };
        assert_eq!(
            coordination_tier(&singleton_g, &gov, &ctl),
            "singleton_governance_seed"
        );

        let mixed = ClusterReport {
            cluster_id: "0xm".to_string(),
            addresses: vec!["0xg".to_string(), "0xc".to_string()],
            shared_evidence_keys: vec!["k".to_string()],
        };
        assert_eq!(
            coordination_tier(&mixed, &gov, &ctl),
            "multi_identifier_coordination_mixed_governance_and_control"
        );
    }

    #[test]
    fn coordination_tier_singleton_control_generic_and_multi_pure_cohorts() {
        let mut gov = HashSet::new();
        gov.insert("0xg".to_string());
        let mut ctl = HashSet::new();
        ctl.insert("0xc".to_string());

        let singleton_c = ClusterReport {
            cluster_id: "a".to_string(),
            addresses: vec!["0xc".to_string()],
            shared_evidence_keys: vec![],
        };
        assert_eq!(
            coordination_tier(&singleton_c, &gov, &ctl),
            "singleton_negative_control_reference"
        );

        let singleton_other = ClusterReport {
            cluster_id: "b".to_string(),
            addresses: vec!["0xunknown0000000000000000000000000000000001".to_string()],
            shared_evidence_keys: vec![],
        };
        assert_eq!(coordination_tier(&singleton_other, &gov, &ctl), "singleton");

        let mut gov2 = HashSet::new();
        gov2.extend([
            "0xa100000000000000000000000000000000000001".to_string(),
            "0xa200000000000000000000000000000000000002".to_string(),
        ]);
        let mut ctl2 = HashSet::new();
        ctl2.extend([
            "0xc100000000000000000000000000000000000001".to_string(),
            "0xc200000000000000000000000000000000000002".to_string(),
        ]);

        let ctl_only = ClusterReport {
            cluster_id: "x".to_string(),
            addresses: vec![
                "0xc100000000000000000000000000000000000001".to_string(),
                "0xc200000000000000000000000000000000000002".to_string(),
            ],
            shared_evidence_keys: vec!["k".to_string()],
        };
        assert_eq!(
            coordination_tier(&ctl_only, &gov2, &ctl2),
            "multi_identifier_coordination_control_only"
        );

        let gov_only = ClusterReport {
            cluster_id: "y".to_string(),
            addresses: vec![
                "0xa100000000000000000000000000000000000001".to_string(),
                "0xa200000000000000000000000000000000000002".to_string(),
            ],
            shared_evidence_keys: vec!["k".to_string()],
        };
        assert_eq!(
            coordination_tier(&gov_only, &gov2, &ctl2),
            "multi_identifier_coordination_governance_only"
        );

        let candidate = ClusterReport {
            cluster_id: "z".to_string(),
            addresses: vec![
                "0xe100000000000000000000000000000000000001".to_string(),
                "0xe200000000000000000000000000000000000002".to_string(),
            ],
            shared_evidence_keys: vec!["k".to_string()],
        };
        assert_eq!(
            coordination_tier(&candidate, &gov2, &ctl2),
            "multi_identifier_coordination_candidate"
        );
    }

    #[test]
    fn lineage_skips_when_policy_profile_mismatched() {
        let mut prev = prev_summary(100, 200);
        prev.policy_profile_id = "other_profile".to_string();
        let mut cur = HashMap::new();
        cur.insert("c1".to_string(), vec!["0x1".to_string()]);
        let mut prev_map = HashMap::new();
        prev_map.insert("p1".to_string(), vec!["0x1".to_string()]);
        let (lineage, rows) = build_lineage_summary(
            "run",
            "arbitrum",
            POLICY_PROFILE_ID,
            100,
            200,
            Some(&prev),
            Some(&cur),
            Some(&prev_map),
        )
        .expect("lineage helper should not fail");
        assert!(!lineage.enabled);
        assert_eq!(
            lineage.skip_reason.as_deref(),
            Some(SKIP_PREV_WINDOW_NOT_SET)
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn skipped_key_set_and_dominated_by_skipped_detection() {
        let skipped = vec![SkippedKey {
            kind: "funded_by".to_string(),
            key: "0xabc".to_string(),
            fan_out: 10,
        }];
        let set = skipped_key_set(&skipped);
        let c = ClusterReport {
            cluster_id: "0x1".to_string(),
            addresses: vec!["0xa".to_string(), "0xb".to_string()],
            shared_evidence_keys: vec!["0xAbC".to_string()],
        };
        assert!(cluster_evidence_dominated_by_skipped(&c, &set));
    }

    #[test]
    fn parse_seed_csv_reads_rows_and_normalizes_addresses() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arb_gov_seed_{ts}.csv"));
        std::fs::write(
            &path,
            "address,first_seen_block,seed_type\n0xAbCdef0123456789AbCdef0123456789aBcDef01,123,governance\n",
        )
        .expect("write csv");
        let rows = parse_seed_csv(&path, "governance").expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "governance");
        assert_eq!(rows[0].first_seen_block, 123);
        assert_eq!(
            rows[0].address,
            "0xabcdef0123456789abcdef0123456789abcdef01"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_seed_csv_rejects_missing_first_seen_block() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arb_gov_seed_bad_{ts}.csv"));
        std::fs::write(
            &path,
            "address,seed_type\n0xabcdef0123456789abcdef0123456789abcdef01,governance\n",
        )
        .expect("write csv");
        let err = parse_seed_csv(&path, "governance").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing first_seen_block")
                || msg.contains("missing address column")
                || msg.contains("bad block"),
            "unexpected parse error: {msg}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_snapshot_hash_success_and_missing_key() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let ok = std::env::temp_dir().join(format!("arb_gov_snapshot_ok_{ts}.json"));
        std::fs::write(&ok, r#"{"input_snapshot_hash":"abc123"}"#).expect("write json");
        let h = read_snapshot_hash(&ok).expect("snapshot hash");
        assert_eq!(h, "abc123");

        let bad = std::env::temp_dir().join(format!("arb_gov_snapshot_bad_{ts}.json"));
        std::fs::write(&bad, r#"{"other":"x"}"#).expect("write json");
        let err = read_snapshot_hash(&bad).unwrap_err();
        assert!(err.to_string().contains("missing input_snapshot_hash"));

        let _ = std::fs::remove_file(&ok);
        let _ = std::fs::remove_file(&bad);
    }

    #[test]
    fn read_snapshot_hash_rejects_invalid_json() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let p = std::env::temp_dir().join(format!("arb_gov_snap_badjson_{ts}.txt"));
        std::fs::write(&p, "not json {{{").expect("write");
        let err = read_snapshot_hash(&p).unwrap_err();
        assert!(err.to_string().contains("parse phase1b json"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_seed_csv_allows_header_only() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arb_gov_empty_csv_{ts}.csv"));
        std::fs::write(&path, "address,first_seen_block,seed_type\n").expect("write csv");
        let rows = parse_seed_csv(&path, "governance").expect("parse");
        assert!(rows.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_optional_funder_deny_skips_comments_and_normalizes() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let p = std::env::temp_dir().join(format!("arb_gov_deny_{ts}.txt"));
        std::fs::write(
            &p,
            "# comment\n\n0xAbCdef0123456789AbCdef0123456789aBcDef01\n",
        )
        .expect("write txt");
        let deny = load_optional_funder_deny(&p).expect("deny");
        assert_eq!(deny.len(), 1);
        assert!(deny.contains("0xabcdef0123456789abcdef0123456789abcdef01"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_optional_funder_deny_rejects_invalid_address() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let p = std::env::temp_dir().join(format!("arb_gov_deny_bad_{ts}.txt"));
        std::fs::write(&p, "not-an-address\n").expect("write txt");
        let err = load_optional_funder_deny(&p).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("0x-prefixed") || msg.contains("non-hex"),
            "unexpected denylist error: {msg}"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn arbitrum_gov_paths_default_uses_expected_files() {
        let p = ArbitrumGovPaths::default();
        assert!(p.database_url.starts_with("sqlite://"));
        assert!(p.governance_csv.ends_with(DEFAULT_GOV_CSV));
        assert!(p.control_csv.ends_with(DEFAULT_CTL_CSV));
        assert!(p.report_md.ends_with(DEFAULT_REPORT));
        assert!(p.graph_json.ends_with(DEFAULT_GRAPH));
        assert!(p.summary_json.ends_with(DEFAULT_SUMMARY_JSON));
        assert!(p.funder_denylist_txt.is_none());
    }

    #[test]
    fn cluster_evidence_dominated_by_skipped_false_for_empty_or_non_skipped() {
        let empty = ClusterReport {
            cluster_id: "0x1".to_string(),
            addresses: vec!["0xa".to_string()],
            shared_evidence_keys: vec![],
        };
        assert!(!cluster_evidence_dominated_by_skipped(
            &empty,
            &HashSet::new()
        ));

        let c = ClusterReport {
            cluster_id: "0x2".to_string(),
            addresses: vec!["0xa".to_string(), "0xb".to_string()],
            shared_evidence_keys: vec!["0xkey".to_string()],
        };
        let skipped = HashSet::from([("funded_by".to_string(), "0xother".to_string())]);
        assert!(!cluster_evidence_dominated_by_skipped(&c, &skipped));
    }

    #[test]
    fn build_lineage_summary_requires_maps_when_enabled() {
        let prev = prev_summary(1, 2);
        let err = build_lineage_summary(
            "run",
            "arbitrum",
            POLICY_PROFILE_ID,
            100,
            200,
            Some(&prev),
            None,
            None,
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("current cluster map is required when lineage is enabled"));
    }

    #[test]
    fn cleanup_partial_removes_sqlite_db_and_output_artifacts() {
        let tmp = std::env::temp_dir().join(format!(
            "arb_gov_cleanup_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let db = tmp.join("partial.db");
        std::fs::write(&db, b"x").expect("db");
        let report = tmp.join("r.md");
        std::fs::write(&report, b"x").expect("report");
        let graph = tmp.join("g.json");
        std::fs::write(&graph, b"x").expect("graph");
        let summary = tmp.join("s.json");
        std::fs::write(&summary, b"x").expect("summary");
        let paths = ArbitrumGovPaths {
            governance_csv: PathBuf::from(""),
            control_csv: PathBuf::from(""),
            phase1b_json: PathBuf::from(""),
            database_url: format!("sqlite://{}", db.display()),
            report_md: report.clone(),
            graph_json: graph.clone(),
            summary_json: summary.clone(),
            funder_denylist_txt: None,
        };
        cleanup_partial_arbitrum_gov_artifacts(&paths);
        assert!(!db.exists());
        assert!(!report.exists());
        assert!(!graph.exists());
        assert!(!summary.exists());
    }
}
