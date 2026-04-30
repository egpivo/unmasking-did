use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transfers::{parse_transfers, Transfer};

pub const DEFAULT_ALCHEMY_BASE_URL: &str = "https://eth-mainnet.g.alchemy.com/v2";
const PAGE_SIZE_HEX: &str = "0x3e8"; // 1000, the Alchemy max per request

/// Default `category` filter for `alchemy_getAssetTransfers`. Valid on
/// Ethereum mainnet and Polygon; the `internal` category is rejected on
/// every other chain Alchemy hosts. Override via `ALCHEMY_TRANSFER_CATEGORIES`
/// when running against L2s — e.g. Scroll needs `external,erc20`.
pub const DEFAULT_TRANSFER_CATEGORIES: &[&str] = &["external", "internal", "erc20"];

#[derive(Debug, Clone)]
pub struct AlchemyClient {
    http: Client,
    url: String,
    transfer_categories: Vec<String>,
}

#[derive(Serialize)]
struct JsonRpcRequest<'a, P: Serialize> {
    id: u64,
    jsonrpc: &'static str,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl AlchemyClient {
    /// Construct a client targeting Ethereum mainnet (the historical
    /// default). For other networks — Scroll, Optimism, Polygon, etc.
    /// — use [`AlchemyClient::with_base_url`] and an Alchemy app
    /// provisioned for the matching chain.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(DEFAULT_ALCHEMY_BASE_URL, api_key)
    }

    /// Construct a client against an arbitrary Alchemy base URL. The
    /// `base_url` is the prefix up to (but not including) the API key
    /// segment — e.g. `https://scroll-mainnet.g.alchemy.com/v2`. A
    /// trailing slash is tolerated and stripped.
    pub fn with_base_url(base_url: impl AsRef<str>, api_key: impl Into<String>) -> Self {
        let base = base_url.as_ref().trim_end_matches('/');
        let url = format!("{}/{}", base, api_key.into());
        Self {
            http: Client::new(),
            url,
            transfer_categories: DEFAULT_TRANSFER_CATEGORIES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    /// Override the `category` filter passed to `alchemy_getAssetTransfers`.
    /// Required when running against chains that reject `internal`
    /// (any chain other than Ethereum mainnet and Polygon).
    pub fn with_transfer_categories(mut self, categories: Vec<String>) -> Self {
        self.transfer_categories = categories;
        self
    }

    /// Returns `true` when `address` has non-empty bytecode at the
    /// latest block — i.e. it is a contract account, not an EOA.
    /// Used by the ingest pipeline to decide whether a Safe owner
    /// should be flagged `owner_is_safe = true` (treating any contract
    /// owner as non-EOA, which is the conservative reading of the
    /// project's "EOA owners only" rule).
    pub async fn is_contract(&self, address: &str) -> Result<bool> {
        let result = self
            .call("eth_getCode", &[serde_json::json!(address), serde_json::json!("latest")])
            .await?;
        let code = result
            .as_str()
            .ok_or_else(|| anyhow!("eth_getCode response is not a string"))?;
        // `0x` (no bytecode) → EOA. Anything longer → contract.
        Ok(code.len() > 2)
    }

    pub async fn get_asset_transfers(&self, to_address: &str) -> Result<Vec<Transfer>> {
        let mut all = Vec::new();
        let mut page_key: Option<String> = None;
        loop {
            let mut params = serde_json::json!({
                "fromBlock": "0x0",
                "toBlock": "latest",
                "toAddress": to_address,
                "category": self.transfer_categories,
                "withMetadata": false,
                "excludeZeroValue": true,
                "maxCount": PAGE_SIZE_HEX,
            });
            if let Some(ref pk) = page_key {
                params["pageKey"] = Value::String(pk.clone());
            }

            let result = self.call("alchemy_getAssetTransfers", &[params]).await?;
            let transfers = result
                .get("transfers")
                .ok_or_else(|| anyhow!("alchemy response missing `transfers` field"))?;
            let mut batch = parse_transfers(transfers)
                .context("failed to parse Alchemy transfers payload")?;
            all.append(&mut batch);

            page_key = result
                .get("pageKey")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if page_key.is_none() {
                break;
            }
        }
        Ok(all)
    }

    async fn call<P: Serialize>(&self, method: &str, params: &P) -> Result<Value> {
        let req = JsonRpcRequest {
            id: 1,
            jsonrpc: "2.0",
            method,
            params,
        };

        // Manual error handling instead of `error_for_status` / `with_context`
        // on raw reqwest errors: those would Display the full request URL,
        // which embeds the API key (`/v2/<KEY>`). Anything that ends up in a
        // log file or a shared error report would leak the credential.
        // `reqwest::Error::without_url` returns the same error with the URL
        // stripped — that's what we surface to the caller.
        let resp = self
            .http
            .post(&self.url)
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow!("alchemy {method} request failed: {}", e.without_url()))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!(
                "alchemy returned HTTP {status} for method {method}"
            ));
        }

        let resp: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("alchemy {method} JSON decode failed: {}", e.without_url()))?;

        if let Some(err) = resp.error {
            return Err(anyhow!(
                "alchemy JSON-RPC error {}: {}",
                err.code,
                err.message
            ));
        }
        resp.result
            .ok_or_else(|| anyhow!("alchemy response missing both `result` and `error`"))
    }
}
