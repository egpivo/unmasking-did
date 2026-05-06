use anyhow::{anyhow, Context, Result};

use crate::alchemy::client::{DEFAULT_ALCHEMY_BASE_URL, DEFAULT_TRANSFER_CATEGORIES};
use crate::resolvers::{ens::DEFAULT_ENS_RESOLVER_URL, safe::DEFAULT_SAFE_TX_SERVICE_URL};

/// Default Alchemy REST prefix for Arbitrum One (`<this>/<api_key>`).
pub const DEFAULT_ARBITRUM_ALCHEMY_BASE_URL: &str = "https://arb-mainnet.g.alchemy.com/v2";

/// Resolve Phase 2 Arbitrum Alchemy API key: `ARBITRUM_ALCHEMY_API_KEY`, then `ALCHEMY_API_KEY`.
/// Returns `(key, env var name for logs)` — never log the key.
pub fn arbitrum_alchemy_api_key_from_env() -> Result<(String, &'static str)> {
    if let Ok(k) = std::env::var("ARBITRUM_ALCHEMY_API_KEY") {
        let t = k.trim();
        if !t.is_empty() {
            return Ok((t.to_string(), "ARBITRUM_ALCHEMY_API_KEY"));
        }
    }
    if let Ok(k) = std::env::var("ALCHEMY_API_KEY") {
        let t = k.trim();
        if !t.is_empty() {
            return Ok((t.to_string(), "ALCHEMY_API_KEY"));
        }
    }
    Err(anyhow!(
        "missing Alchemy API key: set ARBITRUM_ALCHEMY_API_KEY or ALCHEMY_API_KEY for Phase 2"
    ))
}

/// Arbitrum Alchemy base URL: `ARBITRUM_ALCHEMY_BASE_URL`, else [`DEFAULT_ARBITRUM_ALCHEMY_BASE_URL`].
pub fn arbitrum_alchemy_base_url_from_env() -> String {
    match std::env::var("ARBITRUM_ALCHEMY_BASE_URL") {
        Ok(s) => {
            let t = s.trim();
            if t.is_empty() {
                DEFAULT_ARBITRUM_ALCHEMY_BASE_URL.to_string()
            } else {
                t.to_string()
            }
        }
        Err(_) => DEFAULT_ARBITRUM_ALCHEMY_BASE_URL.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub alchemy_api_key: String,
    pub alchemy_base_url: String,
    pub alchemy_transfer_categories: Vec<String>,
    pub database_url: String,
    pub ens_resolver_url: String,
    pub safe_tx_service_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let alchemy_api_key = std::env::var("ALCHEMY_API_KEY")
            .context("ALCHEMY_API_KEY is required (set it in .env or the shell environment)")?;
        let alchemy_base_url = std::env::var("ALCHEMY_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_ALCHEMY_BASE_URL.to_string());
        let alchemy_transfer_categories = std::env::var("ALCHEMY_TRANSFER_CATEGORIES")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| {
                DEFAULT_TRANSFER_CATEGORIES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect()
            });
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/unmask.db".to_string());
        let ens_resolver_url = std::env::var("ENS_RESOLVER_URL")
            .unwrap_or_else(|_| DEFAULT_ENS_RESOLVER_URL.to_string());
        let safe_tx_service_url = std::env::var("SAFE_TX_SERVICE_URL")
            .unwrap_or_else(|_| DEFAULT_SAFE_TX_SERVICE_URL.to_string());
        Ok(Self {
            alchemy_api_key,
            alchemy_base_url,
            alchemy_transfer_categories,
            database_url,
            ens_resolver_url,
            safe_tx_service_url,
        })
    }

    /// Same as [`from_env`], but accepts `ARBITRUM_ALCHEMY_API_KEY` when `ALCHEMY_API_KEY`
    /// is unset — used only by `arbitrum-gov` so Arbitrum-only `.env` files work.
    pub fn from_env_for_arbitrum_gov() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let (alchemy_api_key, _) = arbitrum_alchemy_api_key_from_env()?;
        let alchemy_base_url = std::env::var("ALCHEMY_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_ALCHEMY_BASE_URL.to_string());
        let alchemy_transfer_categories = std::env::var("ALCHEMY_TRANSFER_CATEGORIES")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| {
                DEFAULT_TRANSFER_CATEGORIES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect()
            });
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/unmask.db".to_string());
        let ens_resolver_url = std::env::var("ENS_RESOLVER_URL")
            .unwrap_or_else(|_| DEFAULT_ENS_RESOLVER_URL.to_string());
        let safe_tx_service_url = std::env::var("SAFE_TX_SERVICE_URL")
            .unwrap_or_else(|_| DEFAULT_SAFE_TX_SERVICE_URL.to_string());
        Ok(Self {
            alchemy_api_key,
            alchemy_base_url,
            alchemy_transfer_categories,
            database_url,
            ens_resolver_url,
            safe_tx_service_url,
        })
    }
}
