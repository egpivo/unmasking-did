use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::ens::EnsRecord;

pub const DEFAULT_ENS_RESOLVER_URL: &str = "https://api.ensideas.com/ens/resolve";

/// HTTP wrapper around an ENS reverse-resolution + text-record service.
/// Default endpoint is the community REST shim at `api.ensideas.com`,
/// which returns the primary name plus common social text records in
/// a single GET. The endpoint is overridable via `ENS_RESOLVER_URL`
/// (no trailing slash); a different upstream just needs to return a
/// JSON shape compatible with [`EnsResolverResponse`].
#[derive(Debug, Clone)]
pub struct EnsResolver {
    http: Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
pub struct EnsResolverResponse {
    pub address: Option<String>,
    pub name: Option<String>,
    pub twitter: Option<String>,
    pub github: Option<String>,
    pub telegram: Option<String>,
}

impl EnsResolver {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into(),
        }
    }

    pub fn default_endpoint() -> Self {
        Self::new(DEFAULT_ENS_RESOLVER_URL)
    }

    pub fn endpoint(&self) -> &str {
        &self.base_url
    }

    /// Resolve the given address against the configured ENS endpoint.
    /// Returns `Ok(None)` when the upstream has no record for the
    /// address (404 or empty body); only network and parse errors
    /// surface as `Err`. Callers in best-effort paths (e.g. `ingest`)
    /// should log on `Err` and continue.
    pub async fn resolve(&self, address: &str) -> Result<Option<EnsRecord>> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            address.to_lowercase()
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("ens resolve request failed: {url}"))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = resp
            .error_for_status()
            .with_context(|| format!("ens resolve HTTP error: {url}"))?;
        let body: EnsResolverResponse = resp
            .json()
            .await
            .context("failed to decode ens resolver JSON")?;
        Ok(Some(into_record(address, body)))
    }
}

pub(crate) fn into_record(address: &str, body: EnsResolverResponse) -> EnsRecord {
    EnsRecord {
        address: address.to_lowercase(),
        name: clean(body.name),
        twitter: clean(body.twitter),
        github: clean(body.github),
        telegram: clean(body.telegram),
    }
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
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
    fn parses_full_response() {
        let body: EnsResolverResponse = serde_json::from_str(
            r#"{
                "address": "0xd8da6bf26964af9d7eed9e03e53415d37aa96045",
                "name": "vitalik.eth",
                "twitter": "VitalikButerin",
                "github": "vbuterin",
                "telegram": null
            }"#,
        )
        .unwrap();
        let r = into_record("0xD8dA6BF26964aF9D7eEd9e03E53415D37aA96045", body);
        assert_eq!(r.address, "0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
        assert_eq!(r.name.as_deref(), Some("vitalik.eth"));
        assert_eq!(r.twitter.as_deref(), Some("VitalikButerin"));
        assert_eq!(r.github.as_deref(), Some("vbuterin"));
        assert_eq!(r.telegram, None);
    }

    #[test]
    fn empty_strings_become_none() {
        let body = EnsResolverResponse {
            address: None,
            name: Some("  ".to_string()),
            twitter: Some("".to_string()),
            github: None,
            telegram: Some("@joe".to_string()),
        };
        let r = into_record("0xabc", body);
        assert_eq!(r.name, None);
        assert_eq!(r.twitter, None);
        assert_eq!(r.telegram.as_deref(), Some("@joe"));
    }

    #[tokio::test]
    async fn resolve_returns_none_on_404() {
        let base = serve_once("404 Not Found", "{}").await;
        let resolver = EnsResolver::new(base);
        let out = resolver
            .resolve("0x1111111111111111111111111111111111111111")
            .await
            .expect("resolve");
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn resolve_parses_success_payload() {
        let base = serve_once(
            "200 OK",
            r#"{"address":"0x1","name":"alice.eth","twitter":"alice","github":"alicegh","telegram":"  "}"#,
        )
        .await;
        let resolver = EnsResolver::new(base);
        let out = resolver
            .resolve("0x1111111111111111111111111111111111111111")
            .await
            .expect("resolve")
            .expect("record");
        assert_eq!(out.name.as_deref(), Some("alice.eth"));
        assert_eq!(out.twitter.as_deref(), Some("alice"));
        assert_eq!(out.github.as_deref(), Some("alicegh"));
        assert_eq!(out.telegram, None);
    }

    #[tokio::test]
    async fn resolve_errors_on_http_failure() {
        let base = serve_once("500 Internal Server Error", "{}").await;
        let resolver = EnsResolver::new(base);
        let err = resolver
            .resolve("0x1111111111111111111111111111111111111111")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ens resolve HTTP error"));
    }
}
