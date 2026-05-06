//! Serialised env tests for Arbitrum Phase 2 key resolution (other tests may
//! mutate env vars; keep these behind a global lock).

use std::sync::Mutex;

use unmasking_did::config::{
    arbitrum_alchemy_api_key_from_env, arbitrum_alchemy_base_url_from_env, Config,
    DEFAULT_ARBITRUM_ALCHEMY_BASE_URL,
};

static ARB_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvSnapshot {
    arbitrum_key: Option<String>,
    alchemy_key: Option<String>,
    arbitrum_base: Option<String>,
    alchemy_base: Option<String>,
    transfer_categories: Option<String>,
    database_url: Option<String>,
    ens_resolver_url: Option<String>,
    safe_tx_service_url: Option<String>,
}

impl EnvSnapshot {
    fn capture() -> Self {
        Self {
            arbitrum_key: std::env::var("ARBITRUM_ALCHEMY_API_KEY").ok(),
            alchemy_key: std::env::var("ALCHEMY_API_KEY").ok(),
            arbitrum_base: std::env::var("ARBITRUM_ALCHEMY_BASE_URL").ok(),
            alchemy_base: std::env::var("ALCHEMY_BASE_URL").ok(),
            transfer_categories: std::env::var("ALCHEMY_TRANSFER_CATEGORIES").ok(),
            database_url: std::env::var("DATABASE_URL").ok(),
            ens_resolver_url: std::env::var("ENS_RESOLVER_URL").ok(),
            safe_tx_service_url: std::env::var("SAFE_TX_SERVICE_URL").ok(),
        }
    }

    fn restore(self) {
        set_or_remove("ARBITRUM_ALCHEMY_API_KEY", self.arbitrum_key);
        set_or_remove("ALCHEMY_API_KEY", self.alchemy_key);
        set_or_remove("ARBITRUM_ALCHEMY_BASE_URL", self.arbitrum_base);
        set_or_remove("ALCHEMY_BASE_URL", self.alchemy_base);
        set_or_remove("ALCHEMY_TRANSFER_CATEGORIES", self.transfer_categories);
        set_or_remove("DATABASE_URL", self.database_url);
        set_or_remove("ENS_RESOLVER_URL", self.ens_resolver_url);
        set_or_remove("SAFE_TX_SERVICE_URL", self.safe_tx_service_url);
    }
}

fn set_or_remove(key: &str, val: Option<String>) {
    match val {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

#[test]
fn arbitrum_alchemy_key_prefers_arbitrum_env_var() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::remove_var("ARBITRUM_ALCHEMY_API_KEY");
    std::env::remove_var("ALCHEMY_API_KEY");
    std::env::set_var("ARBITRUM_ALCHEMY_API_KEY", "from_arb_var");
    std::env::set_var("ALCHEMY_API_KEY", "from_alchemy_var");
    let (key, src) = arbitrum_alchemy_api_key_from_env().expect("key");
    assert_eq!(key, "from_arb_var");
    assert_eq!(src, "ARBITRUM_ALCHEMY_API_KEY");
    snap.restore();
}

#[test]
fn arbitrum_alchemy_key_falls_back_to_alchemy_api_key() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::remove_var("ARBITRUM_ALCHEMY_API_KEY");
    std::env::set_var("ALCHEMY_API_KEY", "fallback_only");
    let (key, src) = arbitrum_alchemy_api_key_from_env().expect("key");
    assert_eq!(key, "fallback_only");
    assert_eq!(src, "ALCHEMY_API_KEY");
    snap.restore();
}

#[test]
fn arbitrum_alchemy_key_errors_when_both_missing() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::remove_var("ARBITRUM_ALCHEMY_API_KEY");
    std::env::remove_var("ALCHEMY_API_KEY");
    let err = arbitrum_alchemy_api_key_from_env().unwrap_err();
    assert!(err.to_string().contains("missing Alchemy API key"));
    snap.restore();
}

#[test]
fn arbitrum_alchemy_base_url_trims_and_defaults() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::remove_var("ARBITRUM_ALCHEMY_BASE_URL");
    let url = arbitrum_alchemy_base_url_from_env();
    assert_eq!(url, DEFAULT_ARBITRUM_ALCHEMY_BASE_URL);

    std::env::set_var("ARBITRUM_ALCHEMY_BASE_URL", "  https://custom.example/v2  ");
    let url = arbitrum_alchemy_base_url_from_env();
    assert_eq!(url, "https://custom.example/v2");
    snap.restore();
}

#[test]
fn config_from_env_requires_alchemy_api_key() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    let cwd = std::env::current_dir().expect("cwd");
    let tmp = std::env::temp_dir().join("unmasking_did_cfg_missing_key");
    std::fs::create_dir_all(&tmp).expect("mkdir temp");
    std::env::set_current_dir(&tmp).expect("chdir temp");
    std::env::remove_var("ALCHEMY_API_KEY");
    let err = Config::from_env().unwrap_err();
    assert!(err.to_string().contains("ALCHEMY_API_KEY is required"));
    std::env::set_current_dir(cwd).expect("restore cwd");
    snap.restore();
}

#[test]
fn config_from_env_parses_transfer_categories_and_overrides() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::set_var("ALCHEMY_API_KEY", "k1");
    std::env::set_var("ALCHEMY_BASE_URL", "https://eth-mainnet.g.alchemy.com/v2");
    std::env::set_var("ALCHEMY_TRANSFER_CATEGORIES", " external, erc20 ,,");
    std::env::set_var("DATABASE_URL", "sqlite://data/test_config.db");
    std::env::set_var("ENS_RESOLVER_URL", "https://ens.example");
    std::env::set_var("SAFE_TX_SERVICE_URL", "https://safe.example");

    let cfg = Config::from_env().expect("config");
    assert_eq!(cfg.alchemy_api_key, "k1");
    assert_eq!(cfg.alchemy_base_url, "https://eth-mainnet.g.alchemy.com/v2");
    assert_eq!(
        cfg.alchemy_transfer_categories,
        vec!["external".to_string(), "erc20".to_string()]
    );
    assert_eq!(cfg.database_url, "sqlite://data/test_config.db");
    assert_eq!(cfg.ens_resolver_url, "https://ens.example");
    assert_eq!(cfg.safe_tx_service_url, "https://safe.example");
    snap.restore();
}

#[test]
fn config_from_env_for_arbitrum_gov_uses_arbitrum_fallback_key() {
    let _guard = ARB_ENV_LOCK.lock().expect("arb env lock");
    let snap = EnvSnapshot::capture();
    std::env::remove_var("ALCHEMY_API_KEY");
    std::env::set_var("ARBITRUM_ALCHEMY_API_KEY", "arb_only");
    std::env::set_var("DATABASE_URL", "sqlite://data/test_arb_gov_cfg.db");

    let cfg = Config::from_env_for_arbitrum_gov().expect("arbitrum gov config");
    assert_eq!(cfg.alchemy_api_key, "arb_only");
    assert_eq!(cfg.database_url, "sqlite://data/test_arb_gov_cfg.db");
    snap.restore();
}
