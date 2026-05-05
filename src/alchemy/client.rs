use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transfers::{parse_transfers, Transfer};

pub const DEFAULT_ALCHEMY_BASE_URL: &str = "https://eth-mainnet.g.alchemy.com/v2";
const PAGE_SIZE_HEX: &str = "0x3e8"; // 1000, the Alchemy max per request
/// Hard cap on `alchemy_getAssetTransfers` pagination per address ingest.
/// Hot contracts (e.g. ENS registrar controllers) can otherwise page for hours.
const MAX_TRANSFER_PAGES: usize = 60;

/// Result of a bounded `alchemy_getAssetTransfers` pull for Phase-2 style
/// one-hop caches (windowed, capped pages / rows / distinct peers).
#[derive(Debug, Clone, Default)]
pub struct BoundedTransferFetch {
    pub transfers: Vec<Transfer>,
    pub alchemy_calls: u64,
    pub pages_fetched: usize,
    pub stopped_early_distinct_peers: bool,
    pub stopped_early_row_cap: bool,
    pub stopped_early_page_cap: bool,
}

fn hex_block_u64(n: u64) -> String {
    format!("0x{:x}", n)
}

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
            .call(
                "eth_getCode",
                &[serde_json::json!(address), serde_json::json!("latest")],
            )
            .await?;
        let code = result
            .as_str()
            .ok_or_else(|| anyhow!("eth_getCode response is not a string"))?;
        // `0x` (no bytecode) → EOA. Anything longer → contract.
        Ok(code.len() > 2)
    }

    pub async fn get_asset_transfers(&self, to_address: &str) -> Result<Vec<Transfer>> {
        Ok(self
            .get_asset_transfers_bounded(
                to_address,
                None,
                None,
                Some("toAddress"),
                MAX_TRANSFER_PAGES,
                usize::MAX,
                None,
            )
            .await?
            .transfers)
    }

    /// Windowed, capped `alchemy_getAssetTransfers` for a single address.
    ///
    /// `direction`: `"toAddress"` (incoming), `"fromAddress"` (outgoing), or
    /// run both when `None` (merges and de-duplicates by tx/from/to/asset/value).
    ///
    /// Stops when any cap trips: `max_pages` per direction leg, `max_rows`
    /// total rows appended, or distinct counterparty addresses ≥
    /// `early_stop_distinct_peers` (evaluated after each page).
    pub async fn get_asset_transfers_bounded(
        &self,
        address: &str,
        from_block: Option<u64>,
        to_block: Option<u64>,
        direction: Option<&str>,
        max_pages: usize,
        max_rows: usize,
        early_stop_distinct_peers: Option<usize>,
    ) -> Result<BoundedTransferFetch> {
        let from_hex = from_block
            .map(hex_block_u64)
            .unwrap_or_else(|| "0x0".to_string());
        let to_hex = to_block
            .map(hex_block_u64)
            .unwrap_or_else(|| "latest".to_string());

        let legs: Vec<&str> = match direction {
            Some("toAddress") => vec!["toAddress"],
            Some("fromAddress") => vec!["fromAddress"],
            None => vec!["toAddress", "fromAddress"],
            Some(other) => {
                return Err(anyhow!(
                    "invalid direction {other:?}: use toAddress, fromAddress, or None for both"
                ));
            }
        };

        let mut out: Vec<Transfer> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut distinct_peers: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut alchemy_calls: u64 = 0;
        let mut stopped_early_distinct_peers = false;
        let mut stopped_early_row_cap = false;
        let mut stopped_early_page_cap = false;
        let mut pages_total: usize = 0;

        for leg in legs {
            let mut page_key: Option<String> = None;
            let mut pages_leg: usize = 0;
            loop {
                if out.len() >= max_rows {
                    stopped_early_row_cap = true;
                    break;
                }
                pages_leg += 1;
                pages_total += 1;
                if pages_leg > max_pages {
                    stopped_early_page_cap = true;
                    break;
                }

                let mut params = serde_json::json!({
                    "fromBlock": from_hex.clone(),
                    "toBlock": to_hex.clone(),
                    "category": self.transfer_categories,
                    "withMetadata": false,
                    "excludeZeroValue": true,
                    "maxCount": PAGE_SIZE_HEX,
                });
                match leg {
                    "toAddress" => {
                        params["toAddress"] = Value::String(address.to_string());
                    }
                    "fromAddress" => {
                        params["fromAddress"] = Value::String(address.to_string());
                    }
                    _ => unreachable!(),
                }
                if let Some(ref pk) = page_key {
                    params["pageKey"] = Value::String(pk.clone());
                }

                alchemy_calls += 1;
                let result = self.call("alchemy_getAssetTransfers", &[params]).await?;
                let transfers = result
                    .get("transfers")
                    .ok_or_else(|| anyhow!("alchemy response missing `transfers` field"))?;
                let batch = parse_transfers(transfers)
                    .context("failed to parse Alchemy transfers payload")?;
                for t in batch {
                    let key = format!(
                        "{}|{}|{}|{}|{}",
                        t.tx_hash.as_deref().unwrap_or(""),
                        t.from_addr,
                        t.to_addr,
                        t.asset.as_deref().unwrap_or(""),
                        t.value.as_deref().unwrap_or("")
                    );
                    if seen.insert(key) {
                        let peer = if leg == "toAddress" {
                            t.from_addr.clone()
                        } else {
                            t.to_addr.clone()
                        };
                        distinct_peers.insert(peer);
                        out.push(t);
                    }
                    if out.len() >= max_rows {
                        stopped_early_row_cap = true;
                        break;
                    }
                }

                if let Some(limit) = early_stop_distinct_peers {
                    if distinct_peers.len() >= limit {
                        stopped_early_distinct_peers = true;
                        break;
                    }
                }

                page_key = result
                    .get("pageKey")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if page_key.is_none() {
                    break;
                }
            }
        }

        Ok(BoundedTransferFetch {
            transfers: out,
            alchemy_calls,
            pages_fetched: pages_total,
            stopped_early_distinct_peers,
            stopped_early_row_cap,
            stopped_early_page_cap,
        })
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
