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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use crate::storage::{connect, run_migrations};

    static TEST_DB_SEQ: AtomicU64 = AtomicU64::new(0);

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

    async fn test_repo() -> Repo {
        let seq = TEST_DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let db_url = format!("sqlite://data/test_ingest_common_{seq}_{ts}.db");
        let pool = connect(&db_url).await.expect("connect");
        run_migrations(&pool).await.expect("migrations");
        Repo::new(pool)
    }

    #[test]
    fn normalize_eth_address_rejects_non_hex() {
        let err = normalize_eth_address("0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").unwrap_err();
        assert!(err.to_string().contains("non-hex"));
    }

    #[tokio::test]
    async fn store_safe_owners_marks_eoa_when_get_code_is_empty() {
        let repo = test_repo().await;
        let base = serve_once("200 OK", r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#).await;
        let client = AlchemyClient::with_base_url(base, "k1");
        let owners = vec![SafeOwner {
            safe_address: "0x1111111111111111111111111111111111111111".to_string(),
            owner_address: "0x2222222222222222222222222222222222222222".to_string(),
            owner_is_safe: false,
            threshold: Some(2),
            observed_block: Some(10),
            source: "test".to_string(),
        }];
        let summary = store_safe_owners(&repo, &client, owners)
            .await
            .expect("store owners");
        assert!(summary.contains("1 EOA"));
        let stored = repo
            .safe_owners_of("0x1111111111111111111111111111111111111111")
            .await
            .expect("safe owners");
        assert_eq!(stored.len(), 1);
        assert!(!stored[0].owner_is_safe);
    }

    #[tokio::test]
    async fn store_safe_owners_treats_probe_failure_as_contract() {
        let repo = test_repo().await;
        let base = serve_once("500 Internal Server Error", "{}").await;
        let client = AlchemyClient::with_base_url(base, "k1");
        let owners = vec![SafeOwner {
            safe_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            owner_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            owner_is_safe: false,
            threshold: Some(1),
            observed_block: Some(20),
            source: "test".to_string(),
        }];
        let summary = store_safe_owners(&repo, &client, owners)
            .await
            .expect("store owners");
        assert!(summary.contains("1 contract"));
        assert!(summary.contains("1 unverified"));
        let stored = repo
            .safe_owners_of("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .await
            .expect("safe owners");
        assert_eq!(stored.len(), 1);
        assert!(stored[0].owner_is_safe);
    }
}
