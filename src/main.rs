use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use tracing::warn;
use unmasking_did::alchemy::AlchemyClient;
use unmasking_did::config::Config;
use unmasking_did::did::DidDocument;
use unmasking_did::ens::EnsRecord;
use unmasking_did::eval::{run_eval_suite, AblationMode};
use unmasking_did::graph_export::{
    build_graph, build_pairwise_graph, write_graph_json, DEFAULT_FAN_OUT_CAP,
    DEFAULT_MAX_EVIDENCE_NODES, DEFAULT_MAX_IDENTIFIER_NODES,
};
use unmasking_did::ingest_common::{normalize_eth_address, store_safe_owners};
use unmasking_did::linking::{
    link_and_persist_with_fanout, FundedByMergePolicy, LinkageParams, FAN_OUT_CAP,
    FUNDED_BY_BURST_BLOCK_DELTA, FUNDED_BY_MIN_SHARED_KEYS, FUNDED_BY_MIN_SHORT_BURST_HITS,
};
use unmasking_did::metrics::{gini, nakamoto_coefficient};
use unmasking_did::monitoring::lineage::{
    cluster_snapshots_from_map, compute_cluster_lineage, should_run_lineage, LineageConfig,
};
use unmasking_did::pipelines::arbitrum_governance::{
    cleanup_partial_arbitrum_gov_artifacts, run_arbitrum_gov_pipeline, ArbitrumGovPaths,
};
use unmasking_did::report::{render_dot, render_markdown, DotInputs, ReportInputs};
use unmasking_did::resolvers::{EnsResolver, SafeResolver};
use unmasking_did::safe::SafeOwner;
use unmasking_did::storage::{connect, run_migrations, DatasetRun, Repo};

#[derive(Parser, Debug)]
#[command(name = "unmasking-did", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Ingest one address: fetch transfers, ENS records, and Safe
    /// ownership in a single best-effort pass. Resolver failures are
    /// logged and skipped — only the transfers ingest is mandatory.
    Ingest {
        /// 0x-prefixed Ethereum address to ingest.
        #[arg(long)]
        address: String,
    },
    /// Build clusters from previously ingested data using shared-funder evidence.
    Link {
        /// Minimum number of shared non-CEX funders required to merge two addresses.
        #[arg(long, default_value_t = 1)]
        min_evidence: usize,
        /// Optional explicit set of addresses to cluster. Defaults to every
        /// address that has been seen by `ingest`.
        #[arg(long, value_delimiter = ',')]
        addresses: Vec<String>,
        /// Monitoring identity profile; trend comparisons are allowed only
        /// within the same profile. Defaults from the selected linker policy.
        #[arg(long)]
        policy_profile_id: Option<String>,
        /// Chain label for run metadata.
        #[arg(long, default_value = "arbitrum")]
        chain: String,
        /// Cadence label for run metadata.
        #[arg(long, default_value = "monthly")]
        cadence: String,
        /// Window start block for run metadata.
        #[arg(long, default_value_t = 0)]
        window_start_block: i64,
        /// Window end block for run metadata.
        #[arg(long, default_value_t = 0)]
        window_end_block: i64,
        /// Jaccard threshold for stable lineage.
        #[arg(long, default_value_t = 0.5)]
        stable_threshold: f64,
        /// Jaccard threshold for related lineage.
        #[arg(long, default_value_t = 0.1)]
        related_threshold: f64,
        /// Enable conservative funded_by merge gating.
        #[arg(long, default_value_t = false)]
        conservative_funded_by: bool,
        /// Fan-out threshold for funded_by service-like suppression when
        /// conservative mode is enabled.
        #[arg(long, default_value_t = FAN_OUT_CAP)]
        funded_by_service_fan_out_cap: usize,
        /// Shared funded_by keys required for funded_by-only merge.
        #[arg(long, default_value_t = FUNDED_BY_MIN_SHARED_KEYS)]
        funded_by_min_shared_keys: usize,
        /// Short-burst hits required for funded_by-only merge.
        #[arg(long, default_value_t = FUNDED_BY_MIN_SHORT_BURST_HITS)]
        funded_by_min_short_burst_hits: usize,
        /// Short-burst delta in blocks for funded_by-only merge.
        #[arg(long, default_value_t = FUNDED_BY_BURST_BLOCK_DELTA)]
        funded_by_short_burst_block_delta: i64,
    },
    /// Compute decentralization metrics over the latest persisted
    /// clustering run. Run `link` first; this command does NOT
    /// re-cluster — it reads `entity_clusters` for the most recent
    /// `clustering_runs` row.
    Metrics {
        #[arg(long, default_value_t = 0.5)]
        threshold: f64,
    },
    /// Render a human-readable report (Markdown by default) over the
    /// latest persisted clustering run. Suitable for blog / Medium.
    Report {
        #[arg(long, default_value = "markdown")]
        format: String,
        #[arg(long, default_value_t = 0.5)]
        threshold: f64,
    },
    /// Manually attach off-chain handles (twitter / github / telegram)
    /// to an address. The automated resolver in `ingest` populates the
    /// same table; this command is for testing and overriding what the
    /// resolver returned.
    AddEnsRecord {
        #[arg(long)]
        address: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        twitter: Option<String>,
        #[arg(long)]
        github: Option<String>,
        #[arg(long)]
        telegram: Option<String>,
    },
    /// Manually record one Safe → owner edge. Only EOA owners
    /// participate in clustering as `safe_owner` evidence; pass
    /// `--owner-is-safe` to record a Safe-of-safe edge for audit
    /// without it influencing merges.
    AddSafeOwner {
        #[arg(long)]
        safe: String,
        #[arg(long)]
        owner: String,
        #[arg(long, default_value_t = false)]
        owner_is_safe: bool,
        #[arg(long)]
        threshold: Option<i64>,
        #[arg(long)]
        observed_block: Option<i64>,
    },
    /// Export a bounded D3-compatible finding graph (`graph.json`)
    /// from the latest persisted clustering run. Pair with the
    /// static `viewer/viewer.html` page (a single D3 v7 file) to
    /// inspect interactively. Bounded by construction: depth = 1,
    /// max identifier and evidence node counts, fan-out cap on
    /// service-like keys.
    ExportGraph {
        /// Output JSON file path.
        #[arg(long, default_value = "out/graph.json")]
        out: String,
        /// `evidence` — bipartite identifier↔evidence graph (audit / debug).  
        /// `pairwise` — identifier↔identifier scored linkage edges.
        #[arg(long, default_value = "evidence")]
        graph_mode: String,
        #[arg(long, default_value_t = DEFAULT_MAX_IDENTIFIER_NODES)]
        max_identifier_nodes: usize,
        #[arg(long, default_value_t = DEFAULT_MAX_EVIDENCE_NODES)]
        max_evidence_nodes: usize,
        #[arg(long, default_value_t = DEFAULT_FAN_OUT_CAP)]
        fan_out_cap: usize,
        /// Cap on candidate address pairs enumerated for pairwise mode
        /// (shared non-service evidence keys only).
        #[arg(long, default_value_t = 2000)]
        max_pairwise_links: usize,
        /// JSON file of [`LinkageParams`]. Defaults to bundled
        /// `data/linkage_params.default.json` when omitted (pairwise mode only).
        #[arg(long)]
        linkage_params: Option<String>,
    },
    /// Local HTTP server: latest graph JSON from SQLite (`GET /api/graph`,
    /// same as `export-graph`) plus static `viewer/`, `out/`, and
    /// `data/findings/`. Run from the repo root after `link`.
    Serve {
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Score candidate address pairs from typed evidence using the bundled
    /// or custom linkage weight JSON. Prints JSON to stdout; optional `--out`
    /// writes the same payload to a file.
    ScorePairs {
        #[arg(long, value_delimiter = ',')]
        addresses: Vec<String>,
        #[arg(long, default_value_t = 5000)]
        max_pairs: usize,
        #[arg(long)]
        linkage_params: Option<String>,
        #[arg(long)]
        out: Option<String>,
    },
    /// Arbitrum governance + control stratified seeds: bounded ingest + link +
    /// coordination report (isolated SQLite DB; does not modify seed CSVs).
    #[command(name = "arbitrum-gov")]
    ArbitrumGov {
        #[arg(long, default_value = "data/unmask_arbitrum_gov_v1.db")]
        database: String,
        #[arg(
            long,
            default_value = "data/seeds/arbitrum_gov_90d_governance_stratified500.csv"
        )]
        governance_csv: String,
        #[arg(
            long,
            default_value = "data/seeds/arbitrum_gov_90d_control_stratified500.csv"
        )]
        control_csv: String,
        #[arg(long, default_value = "out/phase1b_arbitrum_gov_seed_quality.json")]
        phase1b_json: String,
        #[arg(long, default_value = "out/arbitrum_gov_report.md")]
        report_md: String,
        #[arg(long, default_value = "out/arbitrum_gov.graph.json")]
        graph_json: String,
        #[arg(long, default_value = "out/arbitrum_gov_summary.json")]
        summary_json: String,
        #[arg(long, default_value_t = 1)]
        min_evidence: usize,
        /// Remove the SQLite file before running (recommended for a clean run).
        #[arg(long, default_value_t = false)]
        overwrite_db: bool,
        /// Optional newline-separated `0x` addresses to drop from `funded_by`
        /// evidence (bridges / CEX routers). Raw transfers remain in SQLite.
        #[arg(long)]
        funder_denylist_txt: Option<String>,
    },
    /// Gold-label evaluation: ablations × pairwise tier × rule-based merge
    /// vs `same_control` / `different_control` / `uncertain` pairs (CSV).
    /// Does not fetch chain data — reads `evidence` already in the DB.
    Eval {
        /// CSV with columns: address_a, address_b, label, rationale
        #[arg(long)]
        gold: String,
        #[arg(long, default_value_t = 2)]
        min_evidence: usize,
        /// `all` to run every preset ablation, or comma-separated modes
        /// (`safe_owner_only`, `all_evidence`, `funded_by_only`, …).
        #[arg(long, default_value = "all")]
        ablation: String,
        #[arg(long)]
        linkage_params: Option<String>,
    },
    /// Manually record one DID document's controller relationship.
    /// When `--controller` differs from `--address`, the relationship
    /// is emitted as STRONG `did_controller` evidence — a single
    /// shared-controller edge is sufficient to merge two addresses
    /// regardless of `--min-evidence`. Self-controlled DIDs (where
    /// controller equals subject) are recorded but produce no
    /// clustering edge. Until M3.5 lands an automated `did:ethr`
    /// resolver, this is how `did_documents` gets populated.
    AddDidDocument {
        /// 0x-prefixed Ethereum-style address embedded in the DID.
        #[arg(long)]
        address: String,
        /// Address authorised to update the DID document.
        #[arg(long)]
        controller: String,
        /// DID method name (e.g. `ethr`, `pkh`, `web`, `key`).
        #[arg(long, default_value = "ethr")]
        method: String,
        /// Full DID string. If omitted, derived as `did:<method>:<address>`.
        #[arg(long)]
        did: Option<String>,
        #[arg(long)]
        observed_block: Option<i64>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load `.env` when present; never overrides variables already set in the shell.
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg = match &cli.command {
        Command::ArbitrumGov { .. } => Config::from_env_for_arbitrum_gov()?,
        _ => Config::from_env()?,
    };

    let pool = connect(&cfg.database_url).await?;
    run_migrations(&pool).await?;
    let repo = Repo::new(pool);

    match cli.command {
        Command::Ingest { address } => run_ingest(&cfg, &repo, &address).await,
        Command::Link {
            min_evidence,
            addresses,
            policy_profile_id,
            chain,
            cadence,
            window_start_block,
            window_end_block,
            stable_threshold,
            related_threshold,
            conservative_funded_by,
            funded_by_service_fan_out_cap,
            funded_by_min_shared_keys,
            funded_by_min_short_burst_hits,
            funded_by_short_burst_block_delta,
        } => {
            let effective_policy_profile_id = policy_profile_id.unwrap_or_else(|| {
                if conservative_funded_by {
                    "arbitrum_gov_conservative_v1".to_string()
                } else {
                    "legacy_funded_by_v1".to_string()
                }
            });
            let funded_by_policy = FundedByMergePolicy {
                enabled: conservative_funded_by,
                service_fan_out_cap: funded_by_service_fan_out_cap,
                min_shared_keys: funded_by_min_shared_keys,
                min_short_burst_hits: funded_by_min_short_burst_hits,
                short_burst_block_delta: funded_by_short_burst_block_delta,
            };
            run_link(
                &repo,
                addresses,
                min_evidence,
                &funded_by_policy,
                &effective_policy_profile_id,
                &chain,
                &cadence,
                window_start_block,
                window_end_block,
                stable_threshold,
                related_threshold,
            )
            .await
        }
        Command::Metrics { threshold } => run_metrics(&repo, threshold).await,
        Command::Report { format, threshold } => run_report(&repo, format, threshold).await,
        Command::AddEnsRecord {
            address,
            name,
            twitter,
            github,
            telegram,
        } => run_add_ens_record(&repo, address, name, twitter, github, telegram).await,
        Command::AddSafeOwner {
            safe,
            owner,
            owner_is_safe,
            threshold,
            observed_block,
        } => run_add_safe_owner(&repo, safe, owner, owner_is_safe, threshold, observed_block).await,
        Command::AddDidDocument {
            address,
            controller,
            method,
            did,
            observed_block,
        } => run_add_did_document(&repo, address, controller, method, did, observed_block).await,
        Command::ExportGraph {
            out,
            graph_mode,
            max_identifier_nodes,
            max_evidence_nodes,
            fan_out_cap,
            max_pairwise_links,
            linkage_params,
        } => {
            run_export_graph(
                &repo,
                out,
                graph_mode,
                max_identifier_nodes,
                max_evidence_nodes,
                fan_out_cap,
                max_pairwise_links,
                linkage_params,
            )
            .await
        }
        Command::Serve { port } => unmasking_did::serve::run(repo.clone(), port).await,
        Command::ScorePairs {
            addresses,
            max_pairs,
            linkage_params,
            out,
        } => run_score_pairs(&repo, addresses, max_pairs, linkage_params, out).await,
        Command::Eval {
            gold,
            min_evidence,
            ablation,
            linkage_params,
        } => run_eval(&repo, gold, min_evidence, ablation, linkage_params).await,
        Command::ArbitrumGov {
            database,
            governance_csv,
            control_csv,
            phase1b_json,
            report_md,
            graph_json,
            summary_json,
            min_evidence,
            overwrite_db,
            funder_denylist_txt,
        } => {
            let db_url = if database.starts_with("sqlite://") {
                database
            } else {
                format!("sqlite://{database}")
            };
            let paths = ArbitrumGovPaths {
                governance_csv: governance_csv.into(),
                control_csv: control_csv.into(),
                phase1b_json: phase1b_json.into(),
                database_url: db_url,
                report_md: report_md.into(),
                graph_json: graph_json.into(),
                summary_json: summary_json.into(),
                funder_denylist_txt: funder_denylist_txt.map(PathBuf::from),
            };
            match run_arbitrum_gov_pipeline(&cfg, &paths, min_evidence, overwrite_db).await {
                Ok(summary) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!(summary))?
                    );
                    Ok(())
                }
                Err(e) => {
                    cleanup_partial_arbitrum_gov_artifacts(&paths);
                    Err(e)
                }
            }
        }
    }
}

async fn run_ingest(cfg: &Config, repo: &Repo, address: &str) -> Result<()> {
    let address = normalize_eth_address(address)?;
    info!(%address, "fetching transfers from Alchemy");

    let client = AlchemyClient::with_base_url(&cfg.alchemy_base_url, &cfg.alchemy_api_key)
        .with_transfer_categories(cfg.alchemy_transfer_categories.clone());
    let transfers = client.get_asset_transfers(&address).await?;
    info!(count = transfers.len(), "fetched transfers");

    let inserted = repo.insert_transfers(&transfers).await?;
    let earliest_block = transfers.iter().filter_map(|t| t.block_num).min();
    repo.upsert_address(&address, earliest_block).await?;

    // Best-effort enrichment: ENS records and Safe ownership. Network
    // failures are logged and ignored so a flaky upstream does not
    // wedge the primary transfers ingest.
    let ens_resolver = EnsResolver::new(&cfg.ens_resolver_url);
    let ens_status = match ens_resolver.resolve(&address).await {
        Ok(Some(record)) => {
            repo.upsert_ens_record(&record).await?;
            "stored"
        }
        Ok(None) => "no record",
        Err(e) => {
            warn!(error = %e, "ENS resolve failed (continuing)");
            "skipped (resolver error)"
        }
    };

    let safe_resolver = SafeResolver::new(&cfg.safe_tx_service_url);
    let safe_status = match safe_resolver.fetch_owners(&address, earliest_block).await {
        Ok(Some(owners)) => store_safe_owners(repo, &client, owners).await?,
        Ok(None) => "not a Safe".to_string(),
        Err(e) => {
            warn!(error = %e, "Safe Tx Service fetch failed (continuing)");
            "skipped (resolver error)".to_string()
        }
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "address": address,
            "transfers_fetched": transfers.len(),
            "transfers_new": inserted,
            "ens": ens_status,
            "safe": safe_status,
        }))?
    );
    Ok(())
}

async fn run_link(
    repo: &Repo,
    addresses: Vec<String>,
    min_evidence: usize,
    funded_by_policy: &FundedByMergePolicy,
    policy_profile_id: &str,
    chain: &str,
    cadence: &str,
    window_start_block: i64,
    window_end_block: i64,
    stable_threshold: f64,
    related_threshold: f64,
) -> Result<()> {
    let addresses = if addresses.is_empty() {
        repo.known_addresses().await?
    } else {
        addresses
            .into_iter()
            .map(|a| normalize_eth_address(&a))
            .collect::<Result<Vec<_>>>()?
    };

    if addresses.is_empty() {
        return Err(anyhow!(
            "no addresses to cluster — run `ingest` first or pass --addresses"
        ));
    }

    let (run_id, output) = link_and_persist_with_fanout(
        repo,
        &addresses,
        min_evidence,
        FAN_OUT_CAP,
        None,
        funded_by_policy,
    )
    .await?;

    let prev_same_profile = repo
        .latest_dataset_run_for_chain_profile(chain, policy_profile_id)
        .await?;
    let code_commit = std::env::var("GIT_COMMIT").unwrap_or_else(|_| "unknown".to_string());
    let input_snapshot_hash =
        std::env::var("INPUT_SNAPSHOT_HASH").unwrap_or_else(|_| "unknown".to_string());
    let seed_spec_json = serde_json::json!({
        "address_count": addresses.len(),
        "selection": if addresses.is_empty() { "known_addresses" } else { "explicit_or_known_addresses" },
    })
    .to_string();
    let params_json = serde_json::json!({
        "min_evidence": min_evidence,
        "fan_out_cap": FAN_OUT_CAP,
        "funded_by_policy": funded_by_policy,
    })
    .to_string();
    let ds_run = DatasetRun {
        run_id: run_id.clone(),
        chain: chain.to_string(),
        run_type: "monitor".to_string(),
        parent_run_id: prev_same_profile.as_ref().map(|r| r.run_id.clone()),
        window_start_block,
        window_end_block,
        window_start_ts: None,
        window_end_ts: None,
        cadence: cadence.to_string(),
        seed_spec_json,
        params_json,
        input_snapshot_hash,
        code_commit,
        policy_profile_id: policy_profile_id.to_string(),
        stable_threshold,
        related_threshold,
        notes: None,
    };
    repo.start_dataset_run(&ds_run).await?;

    if window_start_block == 0 && window_end_block == 0 {
        warn!("Lineage skipped because monitoring window is not set.");
    } else if let Some(prev) = prev_same_profile {
        if should_run_lineage(
            chain,
            policy_profile_id,
            window_start_block,
            window_end_block,
            &prev.chain,
            &prev.policy_profile_id,
            prev.window_start_block,
            prev.window_end_block,
        ) {
            let current_map = repo.clusters_for_run_map(&run_id).await?;
            let previous_map = repo.clusters_for_run_map(&prev.run_id).await?;
            let lineage = compute_cluster_lineage(
                &run_id,
                &prev.run_id,
                &cluster_snapshots_from_map(&current_map),
                &cluster_snapshots_from_map(&previous_map),
                &LineageConfig {
                    stable_threshold,
                    related_threshold,
                },
            );
            let _ = repo.insert_cluster_lineage_rows(&lineage).await?;
        } else {
            warn!(
                "Lineage skipped because previous run window is not set or profile/chain mismatch."
            );
        }
    }

    let report = serde_json::json!({
        "run_id": run_id,
        "clusters": output.clusters,
        "skipped_service_keys": output.skipped_service_keys,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_metrics(repo: &Repo, threshold: f64) -> Result<()> {
    let run = repo
        .latest_clustering_run()
        .await?
        .ok_or_else(|| anyhow!("no clustering runs found — run `link` first"))?;
    let clusters = repo.clusters_for_run(&run.run_id).await?;
    let sizes: Vec<u64> = clusters.iter().map(|c| c.addresses.len() as u64).collect();
    let n_addresses: u64 = sizes.iter().sum();
    let n_clusters = clusters.len();

    // `n_clusters` (renamed from `n_entities`) avoids the overclaim
    // that a connected component in the evidence graph IS a real-world
    // entity. The number is "how many clusters the evidence model
    // produced," not "how many people / orgs are behind these
    // identifiers."
    let report = serde_json::json!({
        "run_id": run.run_id,
        "n_addresses": n_addresses,
        "n_clusters": n_clusters,
        "addresses_per_cluster": (n_addresses as f64) / (n_clusters.max(1) as f64),
        "nakamoto_coefficient": nakamoto_coefficient(&sizes, threshold),
        "nakamoto_threshold": threshold,
        "gini": gini(&sizes),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_report(repo: &Repo, format: String, threshold: f64) -> Result<()> {
    let run = repo
        .latest_clustering_run()
        .await?
        .ok_or_else(|| anyhow!("no clustering runs found — run `link` first"))?;
    let clusters = repo.clusters_for_run(&run.run_id).await?;
    let skipped = repo.suspected_keys_for_run(&run.run_id).await?;
    let sizes: Vec<u64> = clusters.iter().map(|c| c.addresses.len() as u64).collect();
    let nakamoto = nakamoto_coefficient(&sizes, threshold);
    let gini_value = gini(&sizes);

    // Re-read evidence for every address in every cluster. The
    // markdown renderer uses this to label clusters by their dominant
    // evidence kind (controller-level / shared-owner / …). The DOT
    // renderer uses the same data to draw labelled edges. Important
    // caveat: this reflects the *current* state of `evidence`, not a
    // snapshot at `run.run_id` — if the user has touched the cache
    // since `link` ran, the rendered visualization may diverge from
    // the persisted cluster shape. Persisting per-pair edges per run
    // would fix that and is on the M3.5+ backlog.
    let cluster_addresses: Vec<String> = clusters
        .iter()
        .flat_map(|c| c.addresses.iter().cloned())
        .collect();
    let attestations = repo.attestations_for(&cluster_addresses).await?;

    match format.as_str() {
        "markdown" | "md" => {
            let inputs = ReportInputs {
                run: &run,
                clusters: &clusters,
                skipped: &skipped,
                attestations: &attestations,
                nakamoto,
                gini: gini_value,
                nakamoto_threshold: threshold,
            };
            print!("{}", render_markdown(&inputs));
        }
        "json" => {
            let parsed_params: serde_json::Value = serde_json::from_str(&run.params_json)
                .unwrap_or(serde_json::json!(run.params_json));
            let body = serde_json::json!({
                "run_id": run.run_id,
                "started_at": run.started_at,
                "params": parsed_params,
                "n_addresses": sizes.iter().sum::<u64>(),
                "n_clusters": clusters.len(),
                "nakamoto_coefficient": nakamoto,
                "nakamoto_threshold": threshold,
                "gini": gini_value,
                "clusters": clusters,
                "skipped_service_keys": skipped,
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        "dot" => {
            let inputs = DotInputs {
                run: &run,
                clusters: &clusters,
                skipped: &skipped,
                attestations: &attestations,
            };
            print!("{}", render_dot(&inputs));
        }
        other => {
            return Err(anyhow!(
                "unknown --format {other:?}; expected `markdown`, `json`, or `dot`"
            ));
        }
    }
    Ok(())
}

async fn run_add_ens_record(
    repo: &Repo,
    address: String,
    name: Option<String>,
    twitter: Option<String>,
    github: Option<String>,
    telegram: Option<String>,
) -> Result<()> {
    let address = normalize_eth_address(&address)?;
    if name.is_none() && twitter.is_none() && github.is_none() && telegram.is_none() {
        return Err(anyhow!(
            "at least one of --name / --twitter / --github / --telegram must be provided"
        ));
    }
    let record = EnsRecord {
        address: address.clone(),
        name,
        twitter,
        github,
        telegram,
    };
    repo.upsert_ens_record(&record).await?;
    repo.upsert_address(&address, None).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "stored": record,
        }))?
    );
    Ok(())
}

async fn run_add_safe_owner(
    repo: &Repo,
    safe: String,
    owner: String,
    owner_is_safe: bool,
    threshold: Option<i64>,
    observed_block: Option<i64>,
) -> Result<()> {
    let safe_address = normalize_eth_address(&safe)?;
    let owner_address = normalize_eth_address(&owner)?;
    let record = SafeOwner {
        safe_address: safe_address.clone(),
        owner_address: owner_address.clone(),
        owner_is_safe,
        threshold,
        observed_block,
        source: "manual".to_string(),
    };
    repo.upsert_safe_owner(&record).await?;
    // Only the Safe is a clustering subject. The owner is an evidence
    // value (queried via `safe_owners.owner_address` at extract time)
    // — adding it to `addresses` would inflate `n_addresses`, create a
    // singleton owner cluster on every link run, and conflate the
    // "subject" and "evidence value" roles.
    repo.upsert_address(&safe_address, observed_block).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "stored": record,
        }))?
    );
    Ok(())
}

async fn run_export_graph(
    repo: &Repo,
    out: String,
    graph_mode: String,
    max_identifier_nodes: usize,
    max_evidence_nodes: usize,
    fan_out_cap: usize,
    max_pairwise_links: usize,
    linkage_params: Option<String>,
) -> Result<()> {
    let graph = match graph_mode.as_str() {
        "pairwise" => {
            let params = if let Some(ref p) = linkage_params {
                LinkageParams::from_json_file(Path::new(p))?
            } else {
                LinkageParams::bundled_default()?
            };
            let src = linkage_params
                .clone()
                .unwrap_or_else(|| "bundled data/linkage_params.default.json".to_string());
            build_pairwise_graph(
                repo,
                None,
                max_identifier_nodes,
                fan_out_cap,
                max_pairwise_links,
                params,
                &src,
            )
            .await?
        }
        "evidence" => {
            build_graph(
                repo,
                None,
                max_identifier_nodes,
                max_evidence_nodes,
                fan_out_cap,
            )
            .await?
        }
        other => {
            return Err(anyhow!(
                "unknown --graph-mode {other:?}; expected `evidence` or `pairwise`"
            ));
        }
    };
    let path = std::path::PathBuf::from(&out);
    write_graph_json(&graph, &path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "wrote": out,
            "graph_mode": graph.graph_mode,
            "run_id": graph.run.run_id,
            "nodes": graph.nodes.len(),
            "links": graph.links.len(),
            "limits": graph.limits,
        }))?
    );
    Ok(())
}

async fn run_eval(
    repo: &Repo,
    gold: String,
    min_evidence: usize,
    ablation: String,
    linkage_params: Option<String>,
) -> Result<()> {
    let params = if let Some(ref p) = linkage_params {
        LinkageParams::from_json_file(Path::new(p))?
    } else {
        LinkageParams::bundled_default()?
    };
    let modes = AblationMode::parse_list(&ablation)?;
    let report = run_eval_suite(repo, Path::new(&gold), &modes, min_evidence, &params).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_score_pairs(
    repo: &Repo,
    addresses: Vec<String>,
    max_pairs: usize,
    linkage_params: Option<String>,
    out: Option<String>,
) -> Result<()> {
    use unmasking_did::linking::{candidate_address_pairs, score_address_pairs};

    let addresses = if addresses.is_empty() {
        repo.known_addresses().await?
    } else {
        addresses
            .into_iter()
            .map(|a| normalize_eth_address(&a))
            .collect::<Result<Vec<_>>>()?
    };
    if addresses.is_empty() {
        return Err(anyhow!(
            "no addresses — run `ingest` first or pass --addresses"
        ));
    }

    let params = if let Some(ref p) = linkage_params {
        LinkageParams::from_json_file(Path::new(p))?
    } else {
        LinkageParams::bundled_default()?
    };
    let params_source = linkage_params
        .clone()
        .unwrap_or_else(|| "bundled data/linkage_params.default.json".to_string());

    let attestations = repo.attestations_for(&addresses).await?;
    let pairs = candidate_address_pairs(&addresses, &attestations, max_pairs);
    let scored = score_address_pairs(&pairs, &attestations, &params);

    let body = serde_json::json!({
        "linkage_params_source": params_source,
        "address_count": addresses.len(),
        "candidate_pair_count": pairs.len(),
        "pairs": scored,
    });
    let text = serde_json::to_string_pretty(&body)?;
    if let Some(path) = out {
        let p = Path::new(&path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        std::fs::write(p, &text).with_context(|| format!("write {}", p.display()))?;
    }
    println!("{text}");
    Ok(())
}

async fn run_add_did_document(
    repo: &Repo,
    address: String,
    controller: String,
    method: String,
    did: Option<String>,
    observed_block: Option<i64>,
) -> Result<()> {
    let subject = normalize_eth_address(&address)?;
    let controller = normalize_eth_address(&controller)?;
    let did = did.unwrap_or_else(|| format!("did:{method}:{subject}"));
    let doc = DidDocument {
        did,
        subject_address: subject.clone(),
        controller,
        method,
        document_json: None,
        observed_block,
        source: "manual".to_string(),
    };
    repo.upsert_did_document(&doc).await?;
    // Only the SUBJECT address enters `addresses` as a clustering
    // subject. The controller is an evidence value and should not
    // become a phantom singleton cluster on every link run, mirroring
    // the equivalent guard in run_add_safe_owner.
    repo.upsert_address(&subject, observed_block).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "stored": doc,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use unmasking_did::ingest_common::classify_owner_probe;
    use unmasking_did::storage::{connect, run_migrations};

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DB_SEQ: AtomicU64 = AtomicU64::new(0);

    async fn test_repo() -> Repo {
        let seq = TEST_DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let db_url = format!("sqlite://data/test_main_{seq}_{ts}.db");
        let pool = connect(&db_url).await.expect("connect");
        run_migrations(&pool).await.expect("migrations");
        Repo::new(pool)
    }

    #[test]
    fn normalize_lowercases_valid_address() {
        let a = normalize_eth_address("0xAbCdef0123456789AbCdef0123456789aBcDef01").unwrap();
        assert_eq!(a, "0xabcdef0123456789abcdef0123456789abcdef01");
    }

    #[test]
    fn normalize_rejects_short_address() {
        assert!(normalize_eth_address("0x1234").is_err());
    }

    #[test]
    fn classify_owner_probe_passes_through_success() {
        assert!(classify_owner_probe(Ok(true)));
        assert!(!classify_owner_probe(Ok(false)));
    }

    #[test]
    fn classify_owner_probe_treats_failure_as_contract() {
        // Regression: a flaky RPC must not silently turn an unverified
        // owner into evidence-eligible "EOA" and merge Safes through it.
        let probe: Result<bool> = Err(anyhow!("simulated RPC failure"));
        assert!(
            classify_owner_probe(probe),
            "probe failure must be classified as contract (conservative under-cluster)"
        );
    }

    #[test]
    fn cli_link_defaults_parse() {
        let cli = Cli::try_parse_from(["unmasking-did", "link"]).expect("parse link");
        let Command::Link {
            min_evidence,
            stable_threshold,
            related_threshold,
            conservative_funded_by,
            funded_by_service_fan_out_cap,
            funded_by_min_shared_keys,
            funded_by_min_short_burst_hits,
            funded_by_short_burst_block_delta,
            ..
        } = cli.command
        else {
            panic!("expected link command")
        };
        assert_eq!(min_evidence, 1);
        assert_eq!(stable_threshold, 0.5);
        assert_eq!(related_threshold, 0.1);
        assert!(!conservative_funded_by);
        assert_eq!(funded_by_service_fan_out_cap, FAN_OUT_CAP);
        assert_eq!(funded_by_min_shared_keys, FUNDED_BY_MIN_SHARED_KEYS);
        assert_eq!(
            funded_by_min_short_burst_hits,
            FUNDED_BY_MIN_SHORT_BURST_HITS
        );
        assert_eq!(
            funded_by_short_burst_block_delta,
            FUNDED_BY_BURST_BLOCK_DELTA
        );
    }

    #[test]
    fn cli_link_conservative_flags_override_defaults() {
        let cli = Cli::try_parse_from([
            "unmasking-did",
            "link",
            "--conservative-funded-by",
            "--funded-by-service-fan-out-cap",
            "77",
            "--funded-by-min-shared-keys",
            "3",
            "--funded-by-min-short-burst-hits",
            "4",
            "--funded-by-short-burst-block-delta",
            "6000",
        ])
        .expect("parse conservative link");
        let Command::Link {
            conservative_funded_by,
            funded_by_service_fan_out_cap,
            funded_by_min_shared_keys,
            funded_by_min_short_burst_hits,
            funded_by_short_burst_block_delta,
            ..
        } = cli.command
        else {
            panic!("expected link command")
        };
        assert!(conservative_funded_by);
        assert_eq!(funded_by_service_fan_out_cap, 77);
        assert_eq!(funded_by_min_shared_keys, 3);
        assert_eq!(funded_by_min_short_burst_hits, 4);
        assert_eq!(funded_by_short_burst_block_delta, 6000);
    }

    #[test]
    fn cli_export_graph_defaults_parse() {
        let cli = Cli::try_parse_from(["unmasking-did", "export-graph"]).expect("parse export-graph");
        let Command::ExportGraph {
            graph_mode,
            max_identifier_nodes,
            max_evidence_nodes,
            fan_out_cap,
            max_pairwise_links,
            linkage_params,
            ..
        } = cli.command
        else {
            panic!("expected export-graph command")
        };
        assert_eq!(graph_mode, "evidence");
        assert_eq!(max_identifier_nodes, DEFAULT_MAX_IDENTIFIER_NODES);
        assert_eq!(max_evidence_nodes, DEFAULT_MAX_EVIDENCE_NODES);
        assert_eq!(fan_out_cap, DEFAULT_FAN_OUT_CAP);
        assert_eq!(max_pairwise_links, 2000);
        assert!(linkage_params.is_none());
    }

    #[test]
    fn cli_arbitrum_gov_defaults_parse() {
        let cli = Cli::try_parse_from(["unmasking-did", "arbitrum-gov"])
            .expect("parse arbitrum-gov");
        let Command::ArbitrumGov {
            database,
            governance_csv,
            control_csv,
            min_evidence,
            overwrite_db,
            ..
        } = cli.command
        else {
            panic!("expected arbitrum-gov command")
        };
        assert_eq!(database, "data/unmask_arbitrum_gov_v1.db");
        assert_eq!(
            governance_csv,
            "data/seeds/arbitrum_gov_90d_governance_stratified500.csv"
        );
        assert_eq!(
            control_csv,
            "data/seeds/arbitrum_gov_90d_control_stratified500.csv"
        );
        assert_eq!(min_evidence, 1);
        assert!(!overwrite_db);
    }

    #[tokio::test]
    async fn run_add_ens_record_requires_at_least_one_field() {
        let repo = test_repo().await;
        let err = run_add_ens_record(&repo, "0x1111111111111111111111111111111111111111".to_string(), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at least one of"));
    }

    #[tokio::test]
    async fn run_add_safe_owner_and_did_document_store_records() {
        let repo = test_repo().await;
        run_add_safe_owner(
            &repo,
            "0x1111111111111111111111111111111111111111".to_string(),
            "0x2222222222222222222222222222222222222222".to_string(),
            false,
            Some(2),
            Some(123),
        )
        .await
        .expect("safe owner");
        let owners = repo
            .safe_owners_of("0x1111111111111111111111111111111111111111")
            .await
            .expect("safe owners");
        assert_eq!(owners.len(), 1);

        run_add_did_document(
            &repo,
            "0x3333333333333333333333333333333333333333".to_string(),
            "0x4444444444444444444444444444444444444444".to_string(),
            "ethr".to_string(),
            None,
            Some(99),
        )
        .await
        .expect("did doc");
        let docs = repo
            .did_documents_for(&["0x3333333333333333333333333333333333333333".to_string()])
            .await
            .expect("did docs");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].controller, "0x4444444444444444444444444444444444444444");
    }

    #[tokio::test]
    async fn run_export_graph_rejects_unknown_mode() {
        let repo = test_repo().await;
        let err = run_export_graph(
            &repo,
            "out/test_graph.json".to_string(),
            "unknown".to_string(),
            10,
            10,
            10,
            10,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("unknown --graph-mode"));
    }

    #[tokio::test]
    async fn run_score_pairs_errors_when_no_addresses_known() {
        let repo = test_repo().await;
        let err = run_score_pairs(&repo, vec![], 10, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no addresses"));
    }

    #[tokio::test]
    async fn mini_pipeline_covers_link_metrics_report_and_exports() {
        let repo = test_repo().await;
        let a1 = "0x1111111111111111111111111111111111111111".to_string();
        let a2 = "0x2222222222222222222222222222222222222222".to_string();
        let shared_owner = "0x3333333333333333333333333333333333333333".to_string();

        run_add_safe_owner(&repo, a1.clone(), shared_owner.clone(), false, Some(2), Some(10))
            .await
            .expect("add safe owner a1");
        run_add_safe_owner(&repo, a2.clone(), shared_owner, false, Some(2), Some(11))
            .await
            .expect("add safe owner a2");

        let policy = FundedByMergePolicy {
            enabled: true,
            service_fan_out_cap: 50,
            min_shared_keys: 2,
            min_short_burst_hits: 2,
            short_burst_block_delta: 5_000,
        };
        run_link(
            &repo,
            vec![a1.clone(), a2.clone()],
            1,
            &policy,
            "test_profile",
            "arbitrum",
            "monthly",
            100,
            200,
            0.9,
            0.5,
        )
        .await
        .expect("run_link");

        run_metrics(&repo, 0.5).await.expect("run_metrics");
        run_report(&repo, "json".to_string(), 0.5)
            .await
            .expect("run_report json");
        run_report(&repo, "dot".to_string(), 0.5)
            .await
            .expect("run_report dot");
        run_report(&repo, "markdown".to_string(), 0.5)
            .await
            .expect("run_report markdown");

        let out_evidence = "out/test_main_evidence_graph.json".to_string();
        run_export_graph(&repo, out_evidence, "evidence".to_string(), 100, 100, 50, 100, None)
            .await
            .expect("export evidence graph");

        let out_pairwise = "out/test_main_pairwise_graph.json".to_string();
        run_export_graph(&repo, out_pairwise, "pairwise".to_string(), 100, 100, 50, 100, None)
            .await
            .expect("export pairwise graph");

        let scored_out = "out/test_main_scored_pairs.json".to_string();
        run_score_pairs(&repo, vec![a1, a2], 100, None, Some(scored_out))
            .await
            .expect("score pairs");
    }

    #[tokio::test]
    async fn run_report_rejects_unknown_format() {
        let repo = test_repo().await;
        let a1 = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let a2 = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
        let owner = "0xcccccccccccccccccccccccccccccccccccccccc".to_string();
        run_add_safe_owner(&repo, a1.clone(), owner.clone(), false, Some(2), Some(10))
            .await
            .expect("owner1");
        run_add_safe_owner(&repo, a2.clone(), owner, false, Some(2), Some(11))
            .await
            .expect("owner2");

        let policy = FundedByMergePolicy {
            enabled: true,
            service_fan_out_cap: 50,
            min_shared_keys: 2,
            min_short_burst_hits: 2,
            short_burst_block_delta: 5_000,
        };
        run_link(
            &repo,
            vec![a1, a2],
            1,
            &policy,
            "test_profile",
            "arbitrum",
            "monthly",
            100,
            200,
            0.9,
            0.5,
        )
        .await
        .expect("run_link");

        let err = run_report(&repo, "xml".to_string(), 0.5).await.unwrap_err();
        assert!(err.to_string().contains("unknown --format"));
    }

    #[tokio::test]
    async fn run_eval_rejects_invalid_ablation_mode() {
        let repo = test_repo().await;
        let err = run_eval(
            &repo,
            "tests/fixtures/does-not-matter.csv".to_string(),
            1,
            "not-a-mode".to_string(),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("unknown ablation mode"));
    }
}
