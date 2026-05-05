//! Shared helpers for `ingest` and Phase-2 style bounded pipelines.

use anyhow::{anyhow, Result};
use tracing::warn;

use crate::alchemy::client::AlchemyClient;
use crate::safe::SafeOwner;
use crate::storage::Repo;

/// Normalize a user-supplied `0x` + 40 hex address to lowercase checksummed form
/// is not applied — internal storage is lowercase hex.
pub fn normalize_eth_address(addr: &str) -> Result<String> {
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

/// Map an `is_contract` probe result to the `owner_is_safe` flag.
pub fn classify_owner_probe(probe: Result<bool>) -> bool {
    match probe {
        Ok(is_contract) => is_contract,
        Err(_) => true,
    }
}

pub async fn store_safe_owners(
    repo: &Repo,
    client: &AlchemyClient,
    mut owners: Vec<SafeOwner>,
) -> Result<String> {
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
