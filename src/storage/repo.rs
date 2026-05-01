use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

use std::collections::HashMap;

use crate::alchemy::Transfer;
use crate::did::DidDocument;
use crate::ens::EnsRecord;
use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::{ClusterReport, SkippedKey};
use crate::safe::SafeOwner;

/// Header of a single `clustering_runs` row, used by `report` and
/// `metrics` to show which run their numbers came from.
#[derive(Debug, Clone)]
pub struct ClusteringRunSummary {
    pub run_id: String,
    pub params_json: String,
    pub started_at: String,
}

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

    /// Replace every attestation row of the given `kind` whose `address`
    /// is in the input set, with the supplied `new_attestations`.
    /// Used by `link_addresses` to refresh derived evidence (one call
    /// per derived kind) without disturbing rows of other kinds — most
    /// importantly, future strong evidence (`did_controller`) or any
    /// attestations injected by callers outside the link pipeline.
    ///
    /// All entries in `new_attestations` MUST have `kind == kind`; the
    /// function panics on mismatch since that would silently violate
    /// the scoping contract.
    pub async fn replace_attestations_for_kind(
        &self,
        addresses: &[String],
        kind: EvidenceKind,
        new_attestations: &[Attestation],
    ) -> Result<usize> {
        for a in new_attestations {
            assert!(
                a.kind == kind,
                "replace_attestations_for_kind: attestation kind {:?} does not match scope {:?}",
                a.kind,
                kind
            );
        }

        let mut tx = self.pool.begin().await?;

        if !addresses.is_empty() {
            let placeholders = (1..=addresses.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let kind_param_idx = addresses.len() + 1;
            let sql = format!(
                "DELETE FROM evidence
                 WHERE address IN ({placeholders}) AND kind = ?{kind_param_idx}"
            );
            let mut q = sqlx::query(&sql);
            for a in addresses {
                q = q.bind(a.to_lowercase());
            }
            q = q.bind(kind.as_str());
            q.execute(&mut *tx)
                .await
                .context("replace_attestations_for_kind: delete failed")?;
        }

        let mut inserted = 0usize;
        for a in new_attestations {
            let res = sqlx::query(
                "INSERT INTO evidence
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
            .context("replace_attestations_for_kind: insert failed")?;
            inserted += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(inserted)
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

    pub async fn upsert_safe_owner(&self, owner: &SafeOwner) -> Result<()> {
        sqlx::query(
            "INSERT INTO safe_owners
                (safe_address, owner_address, owner_is_safe, threshold, observed_block, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(safe_address, owner_address) DO UPDATE SET
                owner_is_safe  = excluded.owner_is_safe,
                threshold      = excluded.threshold,
                observed_block = excluded.observed_block,
                source         = excluded.source",
        )
        .bind(owner.safe_address.to_lowercase())
        .bind(owner.owner_address.to_lowercase())
        .bind(if owner.owner_is_safe { 1i64 } else { 0i64 })
        .bind(owner.threshold)
        .bind(owner.observed_block)
        .bind(&owner.source)
        .execute(&self.pool)
        .await
        .context("upsert_safe_owner failed")?;
        Ok(())
    }

    /// Return every recorded owner of `safe_address`, including those
    /// flagged as Safes themselves. The extractor is responsible for
    /// dropping non-EOA owners before emitting evidence.
    pub async fn safe_owners_of(&self, safe_address: &str) -> Result<Vec<SafeOwner>> {
        let rows = sqlx::query(
            "SELECT safe_address, owner_address, owner_is_safe, threshold, observed_block, source
             FROM safe_owners
             WHERE safe_address = ?1",
        )
        .bind(safe_address.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .context("safe_owners_of query failed")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let is_safe_int: i64 = r.get("owner_is_safe");
                SafeOwner {
                    safe_address: r.get("safe_address"),
                    owner_address: r.get("owner_address"),
                    owner_is_safe: is_safe_int != 0,
                    threshold: r.get("threshold"),
                    observed_block: r.get("observed_block"),
                    source: r.get("source"),
                }
            })
            .collect())
    }

    /// Return the most recent `clustering_runs` row, if any. `report`
    /// and `metrics` use this as the default run to render.
    ///
    /// Tie-breaker on `started_at`: SQLite's `datetime('now')` is
    /// only second-resolution, so two `link_and_persist` calls inside
    /// the same wall-clock second produce identical `started_at` values.
    /// `run_id` is `format!("run-{micros}")` with microsecond
    /// timestamps and a fixed-width digit count for the foreseeable
    /// future, so descending lexicographic order on `run_id` is
    /// equivalent to descending order by start time within a tied
    /// second. Without this tie-breaker, the result would depend on
    /// the engine's row-scan order — i.e. effectively non-deterministic.
    pub async fn latest_clustering_run(&self) -> Result<Option<ClusteringRunSummary>> {
        let row = sqlx::query(
            "SELECT run_id, params_json, started_at FROM clustering_runs
             ORDER BY started_at DESC, run_id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .context("latest_clustering_run query failed")?;
        Ok(row.map(|r| ClusteringRunSummary {
            run_id: r.get("run_id"),
            params_json: r.get("params_json"),
            started_at: r.get("started_at"),
        }))
    }

    /// Read every cluster persisted by the given `run_id` and reassemble
    /// each into a `ClusterReport` (the same shape `link_addresses`
    /// returned). `evidence_json` is parsed back into
    /// `shared_evidence_keys`; rows that fail to parse degrade to an
    /// empty key list rather than failing the whole query.
    pub async fn clusters_for_run(&self, run_id: &str) -> Result<Vec<ClusterReport>> {
        let rows = sqlx::query(
            "SELECT cluster_id, address, evidence_json
             FROM entity_clusters
             WHERE cluster_run_id = ?1
             ORDER BY cluster_id, address",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("clusters_for_run query failed")?;

        let mut by_cluster: HashMap<String, ClusterReport> = HashMap::new();
        for r in rows {
            let cluster_id: String = r.get("cluster_id");
            let address: String = r.get("address");
            let evidence_json: Option<String> = r.get("evidence_json");

            let entry = by_cluster
                .entry(cluster_id.clone())
                .or_insert_with(|| ClusterReport {
                    cluster_id: cluster_id.clone(),
                    addresses: Vec::new(),
                    shared_evidence_keys: parse_shared_evidence_keys(evidence_json.as_deref()),
                });
            entry.addresses.push(address);
        }

        let mut out: Vec<ClusterReport> = by_cluster.into_values().collect();
        for c in &mut out {
            c.addresses.sort();
        }
        out.sort_by(|a, b| {
            b.addresses
                .len()
                .cmp(&a.addresses.len())
                .then_with(|| a.cluster_id.cmp(&b.cluster_id))
        });
        Ok(out)
    }

    pub async fn suspected_keys_for_run(&self, run_id: &str) -> Result<Vec<SkippedKey>> {
        let rows = sqlx::query(
            "SELECT kind, key, fan_out FROM suspected_service_keys
             WHERE cluster_run_id = ?1
             ORDER BY fan_out DESC, kind, key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("suspected_keys_for_run query failed")?;
        Ok(rows
            .into_iter()
            .map(|r| SkippedKey {
                kind: r.get("kind"),
                key: r.get("key"),
                fan_out: r.get::<i64, _>("fan_out") as usize,
            })
            .collect())
    }

    pub async fn upsert_did_document(&self, doc: &DidDocument) -> Result<()> {
        sqlx::query(
            "INSERT INTO did_documents
                (did, subject_address, controller, method, document_json, observed_block, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(did) DO UPDATE SET
                subject_address = excluded.subject_address,
                controller      = excluded.controller,
                method          = excluded.method,
                document_json   = excluded.document_json,
                observed_block  = excluded.observed_block,
                source          = excluded.source",
        )
        .bind(&doc.did)
        .bind(doc.subject_address.to_lowercase())
        .bind(doc.controller.to_lowercase())
        .bind(&doc.method)
        .bind(doc.document_json.as_deref())
        .bind(doc.observed_block)
        .bind(&doc.source)
        .execute(&self.pool)
        .await
        .context("upsert_did_document failed")?;
        Ok(())
    }

    /// Return every DID document whose `subject_address` is in the
    /// given set. The extractor will then decide whether each row is
    /// evidence-eligible (controller != subject) or trivial.
    pub async fn did_documents_for(&self, addresses: &[String]) -> Result<Vec<DidDocument>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=addresses.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT did, subject_address, controller, method,
                    document_json, observed_block, source
             FROM did_documents
             WHERE subject_address IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql);
        for a in addresses {
            q = q.bind(a.to_lowercase());
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .context("did_documents_for query failed")?;
        Ok(rows
            .into_iter()
            .map(|r| DidDocument {
                did: r.get("did"),
                subject_address: r.get("subject_address"),
                controller: r.get("controller"),
                method: r.get("method"),
                document_json: r.get("document_json"),
                observed_block: r.get("observed_block"),
                source: r.get("source"),
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

fn parse_shared_evidence_keys(evidence_json: Option<&str>) -> Vec<String> {
    let Some(s) = evidence_json else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(s) else {
        return Vec::new();
    };
    value
        .get("shared_evidence_keys")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
}
