use anyhow::{Context, Result};

use crate::resolvers::{ens::DEFAULT_ENS_RESOLVER_URL, safe::DEFAULT_SAFE_TX_SERVICE_URL};

#[derive(Debug, Clone)]
pub struct Config {
    pub alchemy_api_key: String,
    pub database_url: String,
    pub ens_resolver_url: String,
    pub safe_tx_service_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let alchemy_api_key = std::env::var("ALCHEMY_API_KEY")
            .context("ALCHEMY_API_KEY is required (set it in .env or the shell environment)")?;
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/unmask.db".to_string());
        let ens_resolver_url = std::env::var("ENS_RESOLVER_URL")
            .unwrap_or_else(|_| DEFAULT_ENS_RESOLVER_URL.to_string());
        let safe_tx_service_url = std::env::var("SAFE_TX_SERVICE_URL")
            .unwrap_or_else(|_| DEFAULT_SAFE_TX_SERVICE_URL.to_string());
        Ok(Self {
            alchemy_api_key,
            database_url,
            ens_resolver_url,
            safe_tx_service_url,
        })
    }
}
