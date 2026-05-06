use std::str::FromStr;

use alloy::primitives::Address;
use anyhow::{anyhow, Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::safe::SafeOwner;

pub const DEFAULT_SAFE_TX_SERVICE_URL: &str = "https://safe-transaction-mainnet.safe.global";

/// HTTP wrapper around the Safe Transaction Service. A 404 from the
/// `/api/v1/safes/{address}/` endpoint is the canonical signal that
/// the address is *not* a Safe — `fetch_owners` returns `Ok(None)`
/// in that case so callers can branch without parsing the error body.
#[derive(Debug, Clone)]
pub struct SafeResolver {
    http: Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct SafeInfoResponse {
    #[serde(default)]
    owners: Vec<String>,
    #[serde(default)]
    threshold: Option<i64>,
}

impl SafeResolver {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into(),
        }
    }

    pub fn default_endpoint() -> Self {
        Self::new(DEFAULT_SAFE_TX_SERVICE_URL)
    }

    pub fn endpoint(&self) -> &str {
        &self.base_url
    }

    /// Fetch the owners of `safe_address`. `Ok(None)` means the address
    /// is not a Safe (404 from the upstream); any other failure path
    /// returns `Err`. Owners come back with `owner_is_safe = false` by
    /// default — the caller is responsible for refining EOA-ness via
    /// `eth_getCode` (the resolver itself doesn't do RPC calls).
    pub async fn fetch_owners(
        &self,
        safe_address: &str,
        observed_block: Option<i64>,
    ) -> Result<Option<Vec<SafeOwner>>> {
        // The current Safe Tx Service deployment (now hosted at
        // api.safe.global with chain-specific path prefixes) rejects
        // lowercase addresses with HTTP 422 "Checksum address
        // validation failed". We normalize lowercase everywhere
        // internally for indexing, so we have to convert back to the
        // EIP-55 checksum form at the network boundary.
        let parsed = Address::from_str(safe_address)
            .with_context(|| format!("invalid Safe address: {safe_address}"))?;
        let url = format!(
            "{}/api/v1/safes/{}/",
            self.base_url.trim_end_matches('/'),
            parsed
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("safe tx service request failed for {safe_address}"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            // Surface the status code so failures (4xx / 5xx / unfollowed
            // redirects) are diagnosable from logs without leaking the
            // request URL.
            return Err(anyhow!(
                "safe tx service returned HTTP {status} for {safe_address}"
            ));
        }
        let body: SafeInfoResponse = resp
            .json()
            .await
            .context("failed to decode safe tx service JSON")?;

        Ok(Some(into_owners(safe_address, body, observed_block)))
    }
}

fn into_owners(
    safe_address: &str,
    body: SafeInfoResponse,
    observed_block: Option<i64>,
) -> Vec<SafeOwner> {
    body.owners
        .into_iter()
        .map(|owner| SafeOwner {
            safe_address: safe_address.to_lowercase(),
            owner_address: owner.to_lowercase(),
            owner_is_safe: false,
            threshold: body.threshold,
            observed_block,
            source: "safe-tx-service".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_once(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let status_line = status.to_string();
        let body_text = body.to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0_u8; 2048];
            let _ = stream.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_text.len(),
                body_text
            );
            stream
                .write_all(resp.as_bytes())
                .await
                .expect("write resp");
        });
        format!("http://{}", addr)
    }

    #[test]
    fn parses_safe_info_response() {
        let body: SafeInfoResponse = serde_json::from_str(
            r#"{
                "address": "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
                "nonce": 5,
                "threshold": 2,
                "owners": [
                    "0xC0C0c0c0C0c0c0C0c0c0c0c0C0c0c0c0c0c0C0c0",
                    "0xeFeFeFeFeFEFeFeFeFEFeFeFeFEFeFeFeFEFeFEf"
                ],
                "masterCopy": "0x..."
            }"#,
        )
        .unwrap();
        let owners = into_owners(
            "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
            body,
            Some(123),
        );
        assert_eq!(owners.len(), 2);
        assert!(owners.iter().all(|o| !o.owner_is_safe));
        assert_eq!(owners[0].threshold, Some(2));
        assert_eq!(owners[0].observed_block, Some(123));
        assert_eq!(owners[0].source, "safe-tx-service");
        assert_eq!(
            owners[0].owner_address, "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0",
            "owner addresses must be lowercased before storage"
        );
    }

    #[test]
    fn missing_threshold_is_none() {
        let body: SafeInfoResponse =
            serde_json::from_str(r#"{ "owners": ["0xabc1234500000000000000000000000000000000"] }"#)
                .unwrap();
        let owners = into_owners("0xa1a1", body, None);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].threshold, None);
        assert_eq!(owners[0].observed_block, None);
    }

    #[tokio::test]
    async fn fetch_owners_returns_none_on_404() {
        let base = serve_once("404 Not Found", "{}").await;
        let resolver = SafeResolver::new(base);
        let out = resolver
            .fetch_owners("0x1111111111111111111111111111111111111111", Some(10))
            .await
            .expect("fetch");
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn fetch_owners_errors_on_http_failure() {
        let base = serve_once("500 Internal Server Error", "{}").await;
        let resolver = SafeResolver::new(base);
        let err = resolver
            .fetch_owners("0x1111111111111111111111111111111111111111", Some(10))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("returned HTTP 500"));
    }

    #[tokio::test]
    async fn fetch_owners_parses_owner_list_on_success() {
        let base = serve_once(
            "200 OK",
            r#"{"threshold":2,"owners":["0xC0C0c0c0C0c0c0C0c0c0c0c0C0c0c0c0c0c0C0c0"]}"#,
        )
        .await;
        let resolver = SafeResolver::new(base);
        let out = resolver
            .fetch_owners("0x1111111111111111111111111111111111111111", Some(99))
            .await
            .expect("fetch")
            .expect("owners");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].threshold, Some(2));
        assert_eq!(out[0].observed_block, Some(99));
        assert_eq!(
            out[0].owner_address,
            "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0"
        );
    }
}
