use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub alchemy_api_key: String,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let alchemy_api_key = std::env::var("ALCHEMY_API_KEY")
            .context("ALCHEMY_API_KEY is required (set it in .env or the shell environment)")?;
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/unmask.db".to_string());
        Ok(Self {
            alchemy_api_key,
            database_url,
        })
    }
}
