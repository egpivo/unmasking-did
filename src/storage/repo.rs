use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

use crate::alchemy::Transfer;
use crate::ens::EnsRecord;
use crate::evidence::{Attestation, EvidenceKind, Strength};

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("invalid SQLite URL: {database_url}"))?
        .create_if_missing(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("failed to connect to SQLite")
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run sqlx migrations")?;
    Ok(())
}

#[derive(Clone)]
pub struct Repo {
    pool: SqlitePool,
}

impl Repo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn upsert_address(&self, address: &str, first_seen_block: Option<i64>) -> Result<()> {
        sqlx::query(
            "INSERT INTO addresses (address, first_seen_block) VALUES (?1, ?2)
             ON CONFLICT(address) DO UPDATE SET
               first_seen_block = CASE
                   WHEN excluded.first_seen_block IS NULL THEN addresses.first_seen_block
                   WHEN addresses.first_seen_block IS NULL THEN excluded.first_seen_block
                   WHEN excluded.first_seen_block < addresses.first_seen_block
                       THEN excluded.first_seen_block
                   ELSE addresses.first_seen_block
               END",
        )
        .bind(address.to_lowercase())
        .bind(first_seen_block)
        .execute(&self.pool)
        .await
        .context("upsert_address failed")?;
        Ok(())
    }

    pub async fn insert_transfer(&self, t: &Transfer) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO transfers
                (from_addr, to_addr, value, block_num, tx_hash, asset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(t.from_addr.to_lowercase())
        .bind(t.to_addr.to_lowercase())
        .bind(t.value.as_deref())
        .bind(t.block_num)
        .bind(t.tx_hash.as_deref())
        .bind(t.asset.as_deref())
        .execute(&self.pool)
        .await
        .context("insert_transfer failed")?;
        Ok(())
    }

    pub async fn insert_transfers(&self, ts: &[Transfer]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for t in ts {
            let res = sqlx::query(
                "INSERT OR IGNORE INTO transfers
                    (from_addr, to_addr, value, block_num, tx_hash, asset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(t.from_addr.to_lowercase())
            .bind(t.to_addr.to_lowercase())
            .bind(t.value.as_deref())
            .bind(t.block_num)
            .bind(t.tx_hash.as_deref())
            .bind(t.asset.as_deref())
            .execute(&mut *tx)
            .await
            .context("batch insert_transfer failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn incoming_funders(&self, address: &str) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT from_addr, COALESCE(MIN(block_num), 0) AS first_block
             FROM transfers
             WHERE to_addr = ?1
             GROUP BY from_addr
             ORDER BY first_block ASC",
        )
        .bind(address.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .context("incoming_funders query failed")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let from: String = r.get("from_addr");
                let block: i64 = r.get("first_block");
                (from, block)
            })
            .collect())
    }

    pub async fn known_addresses(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT address FROM addresses")
            .fetch_all(&self.pool)
            .await
            .context("known_addresses query failed")?;
        Ok(rows.into_iter().map(|r| r.get::<String, _>(0)).collect())
    }

    pub async fn insert_attestations(&self, atts: &[Attestation]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for a in atts {
            let res = sqlx::query(
                "INSERT OR IGNORE INTO evidence
                    (address, kind, key, strength, source, observed_block, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(a.address.to_lowercase())
            .bind(a.kind.as_str())
            .bind(a.key.to_lowercase())
            .bind(a.strength.as_int())
            .bind(&a.source)
            .bind(a.observed_block)
            .bind(a.payload_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_attestation failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    /// Read attestations for the given address set. Used by the cluster
    /// builder to materialize edges without re-deriving from raw caches.
    pub async fn attestations_for(&self, addresses: &[String]) -> Result<Vec<Attestation>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=addresses.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT address, kind, key, strength, source, observed_block, payload_json
             FROM evidence
             WHERE address IN ({placeholders})
             ORDER BY id ASC"
        );
        let mut q = sqlx::query(&sql);
        for a in addresses {
            q = q.bind(a.to_lowercase());
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .context("attestations_for query failed")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let kind_s: String = r.get("kind");
            let kind = EvidenceKind::parse(&kind_s)
                .ok_or_else(|| anyhow::anyhow!("unknown evidence kind in DB: {kind_s}"))?;
            let strength_i: i64 = r.get("strength");
            let strength = Strength::from_int(strength_i)
                .ok_or_else(|| anyhow::anyhow!("invalid strength in DB: {strength_i}"))?;
            out.push(Attestation {
                address: r.get("address"),
                kind,
                key: r.get("key"),
                strength,
                source: r.get("source"),
                observed_block: r.get("observed_block"),
                payload_json: r.get("payload_json"),
            });
        }
        Ok(out)
    }

    pub async fn record_suspected_service_key(
        &self,
        cluster_run_id: &str,
        kind: EvidenceKind,
        key: &str,
        fan_out: usize,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO suspected_service_keys
                (cluster_run_id, kind, key, fan_out)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(cluster_run_id)
        .bind(kind.as_str())
        .bind(key.to_lowercase())
        .bind(fan_out as i64)
        .execute(&self.pool)
        .await
        .context("record_suspected_service_key failed")?;
        Ok(())
    }

    pub async fn upsert_ens_record(&self, record: &EnsRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO ens_records (address, name, twitter, github, telegram)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(address) DO UPDATE SET
                name     = excluded.name,
                twitter  = excluded.twitter,
                github   = excluded.github,
                telegram = excluded.telegram",
        )
        .bind(record.address.to_lowercase())
        .bind(record.name.as_deref())
        .bind(record.twitter.as_deref())
        .bind(record.github.as_deref())
        .bind(record.telegram.as_deref())
        .execute(&self.pool)
        .await
        .context("upsert_ens_record failed")?;
        Ok(())
    }

    pub async fn ens_records_for(&self, addresses: &[String]) -> Result<Vec<EnsRecord>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=addresses.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT address, name, twitter, github, telegram
             FROM ens_records
             WHERE address IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql);
        for a in addresses {
            q = q.bind(a.to_lowercase());
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .context("ens_records_for query failed")?;
        Ok(rows
            .into_iter()
            .map(|r| EnsRecord {
                address: r.get("address"),
                name: r.get("name"),
                twitter: r.get("twitter"),
                github: r.get("github"),
                telegram: r.get("telegram"),
            })
            .collect())
    }

    pub async fn start_clustering_run(&self, run_id: &str, params_json: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO clustering_runs (run_id, params_json) VALUES (?1, ?2)",
        )
        .bind(run_id)
        .bind(params_json)
        .execute(&self.pool)
        .await
        .context("start_clustering_run failed")?;
        Ok(())
    }

    /// Persist one cluster's membership rows into `entity_clusters`.
    /// `evidence_json` is stored once per address row and carries the
    /// per-cluster evidence trail (shared funders for M1; richer in M2+).
    pub async fn insert_cluster(
        &self,
        run_id: &str,
        cluster_id: &str,
        addresses: &[String],
        evidence_json: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for addr in addresses {
            sqlx::query(
                "INSERT INTO entity_clusters
                    (cluster_run_id, cluster_id, address, evidence_json)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(run_id)
            .bind(cluster_id)
            .bind(addr.to_lowercase())
            .bind(evidence_json)
            .execute(&mut *tx)
            .await
            .context("insert_cluster row failed")?;
        }
        tx.commit().await?;
        Ok(())
    }
}
