use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use unmasking_did::alchemy::AlchemyClient;
use unmasking_did::config::Config;
use unmasking_did::ens::EnsRecord;
use unmasking_did::linking::{cluster_by_funding, link_and_persist};
use unmasking_did::metrics::{gini, nakamoto_coefficient};
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
    /// Fetch on-chain transfers for an address and store them locally.
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
    /// Compute decentralization metrics over the latest clustering.
    Metrics {
        #[arg(long, default_value_t = 0.5)]
        threshold: f64,
        #[arg(long, default_value_t = 1)]
        min_evidence: usize,
    },
    /// Manually attach off-chain handles (twitter / github / telegram) to
    /// an address. Until the automated ENS resolver lands (PR 2.5), this
    /// is how `ens_records` gets populated.
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
        Command::Metrics {
            threshold,
            min_evidence,
        } => run_metrics(&repo, threshold, min_evidence).await,
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
    }
}

async fn run_ingest(cfg: &Config, repo: &Repo, address: &str) -> Result<()> {
    let address = normalize_address(address)?;
    info!(%address, "fetching transfers from Alchemy");

    let client = AlchemyClient::new(&cfg.alchemy_api_key);
    let transfers = client.get_asset_transfers(&address).await?;
    info!(count = transfers.len(), "fetched transfers");

    let inserted = repo.insert_transfers(&transfers).await?;
    let earliest_block = transfers.iter().filter_map(|t| t.block_num).min();
    repo.upsert_address(&address, earliest_block).await?;

    println!(
        "ingested address={address} fetched={fetched} new={inserted}",
        fetched = transfers.len(),
    );
    Ok(())
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

async fn run_metrics(repo: &Repo, threshold: f64, min_evidence: usize) -> Result<()> {
    let addresses = repo.known_addresses().await?;
    if addresses.is_empty() {
        return Err(anyhow!("no addresses to score — run `ingest` first"));
    }

    let clusters = cluster_by_funding(repo, &addresses, min_evidence).await?;
    let sizes: Vec<u64> = clusters.iter().map(|c| c.addresses.len() as u64).collect();

    let n_addresses = addresses.len();
    let n_entities = clusters.len();
    let report = serde_json::json!({
        "n_addresses": n_addresses,
        "n_entities": n_entities,
        "addresses_per_entity": (n_addresses as f64) / (n_entities.max(1) as f64),
        "nakamoto_coefficient": nakamoto_coefficient(&sizes, threshold),
        "nakamoto_threshold": threshold,
        "gini": gini(&sizes),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
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
}
