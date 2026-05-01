use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use tracing::warn;
use unmasking_did::alchemy::AlchemyClient;
use unmasking_did::config::Config;
use unmasking_did::did::DidDocument;
use unmasking_did::ens::EnsRecord;
use unmasking_did::linking::link_and_persist;
use unmasking_did::metrics::{gini, nakamoto_coefficient};
use unmasking_did::report::{render_dot, render_markdown, DotInputs, ReportInputs};
use unmasking_did::resolvers::{EnsResolver, SafeResolver};
use unmasking_did::safe::SafeOwner;
use unmasking_did::storage::{connect, run_migrations, Repo};

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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg = Config::from_env()?;

    let pool = connect(&cfg.database_url).await?;
    run_migrations(&pool).await?;
    let repo = Repo::new(pool);

    match cli.command {
        Command::Ingest { address } => run_ingest(&cfg, &repo, &address).await,
        Command::Link {
            min_evidence,
            addresses,
        } => run_link(&repo, addresses, min_evidence).await,
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
    }
}

async fn run_ingest(cfg: &Config, repo: &Repo, address: &str) -> Result<()> {
    let address = normalize_address(address)?;
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

async fn store_safe_owners(
    repo: &Repo,
    client: &AlchemyClient,
    mut owners: Vec<SafeOwner>,
) -> Result<String> {
    // Refine `owner_is_safe` by probing each owner's bytecode. An
    // address with code is treated as a contract owner (likely Safe)
    // and excluded from `safe_owner` evidence per project policy.
    let mut eoa = 0usize;
    let mut contract = 0usize;
    let mut unverified = 0usize;
    for owner in &mut owners {
        let probe = client.is_contract(&owner.owner_address).await;
        if let Err(ref e) = probe {
            warn!(
                owner = %owner.owner_address,
                error = %e,
                "is_contract probe failed; treating owner as contract (conservative — re-run ingest with a working RPC to refine)"
            );
            unverified += 1;
        }
        owner.owner_is_safe = classify_owner_probe(probe);
        if owner.owner_is_safe {
            contract += 1;
        } else {
            eoa += 1;
        }
        repo.upsert_safe_owner(owner).await?;
    }
    Ok(format!(
        "stored {total} owners ({eoa} EOA, {contract} contract, {unverified} unverified→treated as contract)",
        total = eoa + contract,
    ))
}

/// Map an `is_contract` probe result to the `owner_is_safe` flag.
///
/// On probe failure, prefer the false-negative (under-cluster) over the
/// false-positive (false merge): an unverified owner is marked as
/// contract-like and dropped from `safe_owner` evidence until a future
/// ingest with a working RPC can refine. The previous behavior — treat
/// probe failure as EOA — would have let a flaky RPC silently merge
/// Safes through what was actually another Safe.
fn classify_owner_probe(probe: Result<bool>) -> bool {
    match probe {
        Ok(is_contract) => is_contract,
        Err(_) => true,
    }
}

async fn run_link(repo: &Repo, addresses: Vec<String>, min_evidence: usize) -> Result<()> {
    let addresses = if addresses.is_empty() {
        repo.known_addresses().await?
    } else {
        addresses
            .into_iter()
            .map(|a| normalize_address(&a))
            .collect::<Result<Vec<_>>>()?
    };

    if addresses.is_empty() {
        return Err(anyhow!(
            "no addresses to cluster — run `ingest` first or pass --addresses"
        ));
    }

    let (run_id, output) = link_and_persist(repo, &addresses, min_evidence).await?;
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
            let parsed_params: serde_json::Value =
                serde_json::from_str(&run.params_json).unwrap_or(serde_json::json!(run.params_json));
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
    let address = normalize_address(&address)?;
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
    let safe_address = normalize_address(&safe)?;
    let owner_address = normalize_address(&owner)?;
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

async fn run_add_did_document(
    repo: &Repo,
    address: String,
    controller: String,
    method: String,
    did: Option<String>,
    observed_block: Option<i64>,
) -> Result<()> {
    let subject = normalize_address(&address)?;
    let controller = normalize_address(&controller)?;
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

fn normalize_address(addr: &str) -> Result<String> {
    let trimmed = addr.trim();
    if !trimmed.starts_with("0x") || trimmed.len() != 42 {
        return Err(anyhow!(
            "address must be a 0x-prefixed 40-hex-character string: {trimmed}"
        ));
    }
    let hex = &trimmed[2..];
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("address contains non-hex characters: {trimmed}"));
    }
    Ok(trimmed.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_valid_address() {
        let a = normalize_address("0xAbCdef0123456789AbCdef0123456789aBcDef01").unwrap();
        assert_eq!(a, "0xabcdef0123456789abcdef0123456789abcdef01");
    }

    #[test]
    fn normalize_rejects_short_address() {
        assert!(normalize_address("0x1234").is_err());
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
}
