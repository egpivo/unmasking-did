use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transfers::{parse_transfers, Transfer};

const MAINNET_BASE: &str = "https://eth-mainnet.g.alchemy.com/v2/";
const PAGE_SIZE_HEX: &str = "0x3e8"; // 1000, the Alchemy max per request
const TRANSFER_CATEGORIES: &[&str] = &["external", "internal", "erc20"];

#[derive(Debug, Clone)]
pub struct AlchemyClient {
    http: Client,
    url: String,
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
    pub fn new(api_key: impl Into<String>) -> Self {
        let url = format!("{}{}", MAINNET_BASE, api_key.into());
        Self {
            http: Client::new(),
            url,
        }
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
                "category": TRANSFER_CATEGORIES,
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
        let resp: JsonRpcResponse = self
            .http
            .post(&self.url)
            .json(&req)
            .send()
            .await
            .with_context(|| format!("alchemy request failed for method {method}"))?
            .error_for_status()
            .with_context(|| format!("alchemy returned HTTP error for method {method}"))?
            .json()
            .await
            .context("failed to decode alchemy JSON-RPC response")?;

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
