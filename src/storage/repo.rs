use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone)]
pub struct DatasetRun {
    pub run_id: String,
    pub chain: String,
    pub run_type: String,
    pub parent_run_id: Option<String>,
    pub window_start_block: i64,
    pub window_end_block: i64,
    pub window_start_ts: Option<String>,
    pub window_end_ts: Option<String>,
    pub cadence: String,
    pub seed_spec_json: String,
    pub params_json: String,
    pub input_snapshot_hash: String,
    pub code_commit: String,
    pub policy_profile_id: String,
    pub stable_threshold: f64,
    pub related_threshold: f64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DatasetRunSummary {
    pub run_id: String,
    pub chain: String,
    pub run_type: String,
    pub parent_run_id: Option<String>,
    pub window_start_block: i64,
    pub window_end_block: i64,
    pub cadence: String,
    pub input_snapshot_hash: String,
    pub code_commit: String,
    pub policy_profile_id: String,
    pub stable_threshold: f64,
    pub related_threshold: f64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RunInputRow {
    pub input_type: String,
    pub input_ref: String,
    pub source: String,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunMetricsRow {
    pub run_id: String,
    pub num_seed_inputs: i64,
    pub num_seed_addresses: i64,
    pub num_addresses_total: i64,
    pub num_transfers: i64,
    pub num_evidence_rows: i64,
    pub num_clusters: i64,
    pub num_multi_address_clusters: i64,
    pub top_cluster_size: i64,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClusterMetricsRow {
    pub run_id: String,
    pub cluster_id: String,
    pub num_addresses: i64,
    pub num_identifiers: i64,
    pub num_evidence_rows: i64,
    pub num_unique_funders: Option<i64>,
    pub top_funder: Option<String>,
    pub top_funder_share: Option<f64>,
    pub first_funder_shared_count: Option<i64>,
    pub funding_block_min: Option<i64>,
    pub funding_block_max: Option<i64>,
    pub funding_block_span: Option<i64>,
    pub funding_burst_label: Option<String>,
    pub shared_safe_owner_count: Option<i64>,
    pub control_link_density: Option<f64>,
    pub num_unique_sinks: Option<i64>,
    pub top_sink: Option<String>,
    pub top_sink_share: Option<f64>,
    pub possible_consolidation: Option<bool>,
    pub coordination_tier: String,
    pub coordination_reasons_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClusterLineageRow {
    pub run_id_current: Option<String>,
    pub cluster_id_current: Option<String>,
    pub run_id_previous: Option<String>,
    pub cluster_id_previous: Option<String>,
    pub overlap_count: i64,
    pub jaccard: f64,
    pub transition_label: String,
}

#[derive(Debug, Clone)]
pub struct GraphExportArtifact {
    pub run_id: String,
    pub artifact_type: String,
    pub path: String,
    pub sha256: String,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BenchmarkRun {
    pub benchmark_run_id: String,
    pub scenario_suite_id: String,
    pub scenario_id: String,
    pub seed: i64,
    pub generator_version: String,
    pub policy_profile_id: String,
    pub policy_variant: String,
    pub input_snapshot_hash: String,
    pub code_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkGroundTruthEntityRow {
    pub benchmark_run_id: String,
    pub entity_id: String,
    pub wallet_id: String,
    pub cohort: String,
    pub role_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkSyntheticEvidenceRow {
    pub benchmark_run_id: String,
    pub evidence_id: String,
    pub subject_wallet_id: String,
    pub counterparty_id: String,
    pub evidence_kind: String,
    pub strength_hint: String,
    pub event_time_bucket: Option<String>,
    pub sequence_index: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BenchmarkPolicyResultRow {
    pub benchmark_run_id: String,
    pub policy_variant: String,
    pub pred_cluster_id: String,
    pub wallet_id: String,
    pub link_explanation_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkEvalMetricsRow {
    pub benchmark_run_id: String,
    pub policy_variant: String,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub over_merge_rate: f64,
    pub under_merge_rate: f64,
    pub giant_component_inflation: f64,
    pub cluster_purity: f64,
    pub cluster_fragmentation: f64,
    pub calibration_json_by_evidence_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkEvalDetailRow {
    pub benchmark_run_id: String,
    pub policy_variant: String,
    pub truth_entity_id: String,
    pub matched_pred_cluster_id: Option<String>,
    pub split_count: i64,
    pub merge_intrusion_count: i64,
    pub dominant_error_kind: Option<String>,
    pub detail_json: Option<String>,
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
        // Stable order for the extractor's "first-seen wins"
        // dedup: rows are sorted by observed_block ascending, then
        // by `did` lexicographically. The earliest observation of
        // a (subject, controller) pair therefore drives the
        // attestation's source / observed_block fields, regardless
        // of the order rows happened to be inserted.
        let sql = format!(
            "SELECT did, subject_address, controller, method,
                    document_json, observed_block, source
             FROM did_documents
             WHERE subject_address IN ({placeholders})
             ORDER BY observed_block ASC, did ASC"
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
        sqlx::query("INSERT INTO clustering_runs (run_id, params_json) VALUES (?1, ?2)")
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

    pub async fn start_dataset_run(&self, run: &DatasetRun) -> Result<()> {
        sqlx::query(
            "INSERT INTO dataset_runs
                (run_id, chain, run_type, parent_run_id, window_start_block, window_end_block,
                 window_start_ts, window_end_ts, cadence, seed_spec_json, params_json,
                 input_snapshot_hash, code_commit, policy_profile_id, stable_threshold, related_threshold, notes)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )
        .bind(&run.run_id)
        .bind(&run.chain)
        .bind(&run.run_type)
        .bind(run.parent_run_id.as_deref())
        .bind(run.window_start_block)
        .bind(run.window_end_block)
        .bind(run.window_start_ts.as_deref())
        .bind(run.window_end_ts.as_deref())
        .bind(&run.cadence)
        .bind(&run.seed_spec_json)
        .bind(&run.params_json)
        .bind(&run.input_snapshot_hash)
        .bind(&run.code_commit)
        .bind(&run.policy_profile_id)
        .bind(run.stable_threshold)
        .bind(run.related_threshold)
        .bind(run.notes.as_deref())
        .execute(&self.pool)
        .await
        .context("start_dataset_run failed")?;
        Ok(())
    }

    pub async fn start_benchmark_run(&self, run: &BenchmarkRun) -> Result<()> {
        sqlx::query(
            "INSERT INTO benchmark_runs
                (benchmark_run_id, scenario_suite_id, scenario_id, seed, generator_version,
                 policy_profile_id, policy_variant, input_snapshot_hash, code_commit)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&run.benchmark_run_id)
        .bind(&run.scenario_suite_id)
        .bind(&run.scenario_id)
        .bind(run.seed)
        .bind(&run.generator_version)
        .bind(&run.policy_profile_id)
        .bind(&run.policy_variant)
        .bind(&run.input_snapshot_hash)
        .bind(&run.code_commit)
        .execute(&self.pool)
        .await
        .context("start_benchmark_run failed")?;
        Ok(())
    }

    pub async fn insert_benchmark_snapshot(
        &self,
        run: &BenchmarkRun,
        truth_rows: &[BenchmarkGroundTruthEntityRow],
        evidence_rows: &[BenchmarkSyntheticEvidenceRow],
    ) -> Result<()> {
        if truth_rows
            .iter()
            .any(|r| r.benchmark_run_id != run.benchmark_run_id)
        {
            bail!("insert_benchmark_snapshot: truth row run_id mismatch");
        }
        if evidence_rows
            .iter()
            .any(|r| r.benchmark_run_id != run.benchmark_run_id)
        {
            bail!("insert_benchmark_snapshot: evidence row run_id mismatch");
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO benchmark_runs
                (benchmark_run_id, scenario_suite_id, scenario_id, seed, generator_version,
                 policy_profile_id, policy_variant, input_snapshot_hash, code_commit)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&run.benchmark_run_id)
        .bind(&run.scenario_suite_id)
        .bind(&run.scenario_id)
        .bind(run.seed)
        .bind(&run.generator_version)
        .bind(&run.policy_profile_id)
        .bind(&run.policy_variant)
        .bind(&run.input_snapshot_hash)
        .bind(&run.code_commit)
        .execute(&mut *tx)
        .await
        .context("insert_benchmark_snapshot: insert benchmark_runs failed")?;

        for row in truth_rows {
            sqlx::query(
                "INSERT INTO benchmark_ground_truth_entities
                    (benchmark_run_id, entity_id, wallet_id, cohort, role_tag)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.entity_id)
            .bind(row.wallet_id.to_lowercase())
            .bind(&row.cohort)
            .bind(row.role_tag.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_snapshot: insert truth row failed")?;
        }

        for row in evidence_rows {
            sqlx::query(
                "INSERT INTO benchmark_synthetic_evidence
                    (benchmark_run_id, evidence_id, subject_wallet_id, counterparty_id,
                     evidence_kind, strength_hint, event_time_bucket, sequence_index, metadata_json)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.evidence_id)
            .bind(row.subject_wallet_id.to_lowercase())
            .bind(row.counterparty_id.to_lowercase())
            .bind(&row.evidence_kind)
            .bind(&row.strength_hint)
            .bind(row.event_time_bucket.as_deref())
            .bind(row.sequence_index)
            .bind(row.metadata_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_snapshot: insert evidence row failed")?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_benchmark_ground_truth_rows(
        &self,
        rows: &[BenchmarkGroundTruthEntityRow],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO benchmark_ground_truth_entities
                    (benchmark_run_id, entity_id, wallet_id, cohort, role_tag)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.entity_id)
            .bind(row.wallet_id.to_lowercase())
            .bind(&row.cohort)
            .bind(row.role_tag.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_ground_truth_rows row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_benchmark_synthetic_evidence_rows(
        &self,
        rows: &[BenchmarkSyntheticEvidenceRow],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO benchmark_synthetic_evidence
                    (benchmark_run_id, evidence_id, subject_wallet_id, counterparty_id,
                     evidence_kind, strength_hint, event_time_bucket, sequence_index, metadata_json)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.evidence_id)
            .bind(row.subject_wallet_id.to_lowercase())
            .bind(row.counterparty_id.to_lowercase())
            .bind(&row.evidence_kind)
            .bind(&row.strength_hint)
            .bind(row.event_time_bucket.as_deref())
            .bind(row.sequence_index)
            .bind(row.metadata_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_synthetic_evidence_rows row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_benchmark_policy_results(
        &self,
        rows: &[BenchmarkPolicyResultRow],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO benchmark_policy_results
                    (benchmark_run_id, policy_variant, pred_cluster_id, wallet_id, link_explanation_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.policy_variant)
            .bind(&row.pred_cluster_id)
            .bind(row.wallet_id.to_lowercase())
            .bind(row.link_explanation_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_policy_results row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_benchmark_eval_metrics(&self, row: &BenchmarkEvalMetricsRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO benchmark_eval_metrics
                (benchmark_run_id, policy_variant, precision, recall, f1, over_merge_rate,
                 under_merge_rate, giant_component_inflation, cluster_purity, cluster_fragmentation,
                 calibration_json_by_evidence_kind)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(&row.benchmark_run_id)
        .bind(&row.policy_variant)
        .bind(row.precision)
        .bind(row.recall)
        .bind(row.f1)
        .bind(row.over_merge_rate)
        .bind(row.under_merge_rate)
        .bind(row.giant_component_inflation)
        .bind(row.cluster_purity)
        .bind(row.cluster_fragmentation)
        .bind(row.calibration_json_by_evidence_kind.as_deref())
        .execute(&self.pool)
        .await
        .context("insert_benchmark_eval_metrics failed")?;
        Ok(())
    }

    pub async fn insert_benchmark_eval_details(
        &self,
        rows: &[BenchmarkEvalDetailRow],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO benchmark_eval_details
                    (benchmark_run_id, policy_variant, truth_entity_id, matched_pred_cluster_id,
                     split_count, merge_intrusion_count, dominant_error_kind, detail_json)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.policy_variant)
            .bind(&row.truth_entity_id)
            .bind(row.matched_pred_cluster_id.as_deref())
            .bind(row.split_count)
            .bind(row.merge_intrusion_count)
            .bind(row.dominant_error_kind.as_deref())
            .bind(row.detail_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_eval_details row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_benchmark_eval_bundle(
        &self,
        metrics: &BenchmarkEvalMetricsRow,
        details: &[BenchmarkEvalDetailRow],
    ) -> Result<usize> {
        if details.iter().any(|d| {
            d.benchmark_run_id != metrics.benchmark_run_id
                || d.policy_variant != metrics.policy_variant
        }) {
            bail!("insert_benchmark_eval_bundle: detail row run/policy mismatch");
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO benchmark_eval_metrics
                (benchmark_run_id, policy_variant, precision, recall, f1, over_merge_rate,
                 under_merge_rate, giant_component_inflation, cluster_purity, cluster_fragmentation,
                 calibration_json_by_evidence_kind)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(&metrics.benchmark_run_id)
        .bind(&metrics.policy_variant)
        .bind(metrics.precision)
        .bind(metrics.recall)
        .bind(metrics.f1)
        .bind(metrics.over_merge_rate)
        .bind(metrics.under_merge_rate)
        .bind(metrics.giant_component_inflation)
        .bind(metrics.cluster_purity)
        .bind(metrics.cluster_fragmentation)
        .bind(metrics.calibration_json_by_evidence_kind.as_deref())
        .execute(&mut *tx)
        .await
        .context("insert_benchmark_eval_bundle: insert metrics failed")?;

        let mut n = 0usize;
        for row in details {
            let res = sqlx::query(
                "INSERT INTO benchmark_eval_details
                    (benchmark_run_id, policy_variant, truth_entity_id, matched_pred_cluster_id,
                     split_count, merge_intrusion_count, dominant_error_kind, detail_json)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&row.benchmark_run_id)
            .bind(&row.policy_variant)
            .bind(&row.truth_entity_id)
            .bind(row.matched_pred_cluster_id.as_deref())
            .bind(row.split_count)
            .bind(row.merge_intrusion_count)
            .bind(row.dominant_error_kind.as_deref())
            .bind(row.detail_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_benchmark_eval_bundle: insert detail row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn benchmark_policy_assignments(
        &self,
        benchmark_run_id: &str,
        policy_variant: &str,
    ) -> Result<HashMap<String, String>> {
        let rows = sqlx::query(
            "SELECT wallet_id, pred_cluster_id
             FROM benchmark_policy_results
             WHERE benchmark_run_id = ?1 AND policy_variant = ?2",
        )
        .bind(benchmark_run_id)
        .bind(policy_variant)
        .fetch_all(&self.pool)
        .await
        .context("benchmark_policy_assignments query failed")?;
        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let wallet: String = row.get("wallet_id");
            let cluster: String = row.get("pred_cluster_id");
            out.insert(wallet, cluster);
        }
        Ok(out)
    }

    pub async fn benchmark_eval_metrics_for_run(
        &self,
        benchmark_run_id: &str,
    ) -> Result<Vec<BenchmarkEvalMetricsRow>> {
        let rows = sqlx::query(
            "SELECT benchmark_run_id, policy_variant, precision, recall, f1, over_merge_rate,
                    under_merge_rate, giant_component_inflation, cluster_purity, cluster_fragmentation,
                    calibration_json_by_evidence_kind
             FROM benchmark_eval_metrics
             WHERE benchmark_run_id = ?1
             ORDER BY policy_variant ASC",
        )
        .bind(benchmark_run_id)
        .fetch_all(&self.pool)
        .await
        .context("benchmark_eval_metrics_for_run query failed")?;
        Ok(rows
            .into_iter()
            .map(|row| BenchmarkEvalMetricsRow {
                benchmark_run_id: row.get("benchmark_run_id"),
                policy_variant: row.get("policy_variant"),
                precision: row.get("precision"),
                recall: row.get("recall"),
                f1: row.get("f1"),
                over_merge_rate: row.get("over_merge_rate"),
                under_merge_rate: row.get("under_merge_rate"),
                giant_component_inflation: row.get("giant_component_inflation"),
                cluster_purity: row.get("cluster_purity"),
                cluster_fragmentation: row.get("cluster_fragmentation"),
                calibration_json_by_evidence_kind: row.get("calibration_json_by_evidence_kind"),
            })
            .collect())
    }

    pub async fn benchmark_eval_details_for_run(
        &self,
        benchmark_run_id: &str,
    ) -> Result<Vec<BenchmarkEvalDetailRow>> {
        let rows = sqlx::query(
            "SELECT benchmark_run_id, policy_variant, truth_entity_id, matched_pred_cluster_id,
                    split_count, merge_intrusion_count, dominant_error_kind, detail_json
             FROM benchmark_eval_details
             WHERE benchmark_run_id = ?1
             ORDER BY policy_variant ASC, truth_entity_id ASC",
        )
        .bind(benchmark_run_id)
        .fetch_all(&self.pool)
        .await
        .context("benchmark_eval_details_for_run query failed")?;
        Ok(rows
            .into_iter()
            .map(|row| BenchmarkEvalDetailRow {
                benchmark_run_id: row.get("benchmark_run_id"),
                policy_variant: row.get("policy_variant"),
                truth_entity_id: row.get("truth_entity_id"),
                matched_pred_cluster_id: row.get("matched_pred_cluster_id"),
                split_count: row.get("split_count"),
                merge_intrusion_count: row.get("merge_intrusion_count"),
                dominant_error_kind: row.get("dominant_error_kind"),
                detail_json: row.get("detail_json"),
            })
            .collect())
    }

    pub async fn latest_benchmark_run_id(&self) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT r.benchmark_run_id
             FROM benchmark_runs r
             WHERE EXISTS (
                SELECT 1
                FROM benchmark_eval_metrics m
                WHERE m.benchmark_run_id = r.benchmark_run_id
             )
             ORDER BY r.rowid DESC
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .context("latest_benchmark_run_id query failed")?;
        Ok(row.map(|r| r.get("benchmark_run_id")))
    }

    pub async fn benchmark_run_by_id(
        &self,
        benchmark_run_id: &str,
    ) -> Result<Option<BenchmarkRun>> {
        let row = sqlx::query(
            "SELECT benchmark_run_id, scenario_suite_id, scenario_id, seed, generator_version,
                    policy_profile_id, policy_variant, input_snapshot_hash, code_commit
             FROM benchmark_runs
             WHERE benchmark_run_id = ?1",
        )
        .bind(benchmark_run_id)
        .fetch_optional(&self.pool)
        .await
        .context("benchmark_run_by_id query failed")?;
        Ok(row.map(|r| BenchmarkRun {
            benchmark_run_id: r.get("benchmark_run_id"),
            scenario_suite_id: r.get("scenario_suite_id"),
            scenario_id: r.get("scenario_id"),
            seed: r.get("seed"),
            generator_version: r.get("generator_version"),
            policy_profile_id: r.get("policy_profile_id"),
            policy_variant: r.get("policy_variant"),
            input_snapshot_hash: r.get("input_snapshot_hash"),
            code_commit: r.get("code_commit"),
        }))
    }

    pub async fn recent_benchmark_runs(&self, limit: usize) -> Result<Vec<BenchmarkRun>> {
        let safe_limit = limit.max(1) as i64;
        let rows = sqlx::query(
            "SELECT benchmark_run_id, scenario_suite_id, scenario_id, seed, generator_version,
                    policy_profile_id, policy_variant, input_snapshot_hash, code_commit
             FROM benchmark_runs
             WHERE EXISTS (
                SELECT 1
                FROM benchmark_eval_metrics m
                WHERE m.benchmark_run_id = benchmark_runs.benchmark_run_id
             )
             ORDER BY rowid DESC
             LIMIT ?1",
        )
        .bind(safe_limit)
        .fetch_all(&self.pool)
        .await
        .context("recent_benchmark_runs query failed")?;
        Ok(rows
            .into_iter()
            .map(|r| BenchmarkRun {
                benchmark_run_id: r.get("benchmark_run_id"),
                scenario_suite_id: r.get("scenario_suite_id"),
                scenario_id: r.get("scenario_id"),
                seed: r.get("seed"),
                generator_version: r.get("generator_version"),
                policy_profile_id: r.get("policy_profile_id"),
                policy_variant: r.get("policy_variant"),
                input_snapshot_hash: r.get("input_snapshot_hash"),
                code_commit: r.get("code_commit"),
            })
            .collect())
    }

    pub async fn benchmark_synthetic_evidence_rows(
        &self,
        benchmark_run_id: &str,
    ) -> Result<Vec<BenchmarkSyntheticEvidenceRow>> {
        let rows = sqlx::query(
            "SELECT benchmark_run_id, evidence_id, subject_wallet_id, counterparty_id,
                    evidence_kind, strength_hint, event_time_bucket, sequence_index, metadata_json
             FROM benchmark_synthetic_evidence
             WHERE benchmark_run_id = ?1
             ORDER BY evidence_id ASC",
        )
        .bind(benchmark_run_id)
        .fetch_all(&self.pool)
        .await
        .context("benchmark_synthetic_evidence_rows query failed")?;
        Ok(rows
            .into_iter()
            .map(|row| BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: row.get("benchmark_run_id"),
                evidence_id: row.get("evidence_id"),
                subject_wallet_id: row.get("subject_wallet_id"),
                counterparty_id: row.get("counterparty_id"),
                evidence_kind: row.get("evidence_kind"),
                strength_hint: row.get("strength_hint"),
                event_time_bucket: row.get("event_time_bucket"),
                sequence_index: row.get("sequence_index"),
                metadata_json: row.get("metadata_json"),
            })
            .collect())
    }

    pub async fn insert_run_inputs(&self, run_id: &str, inputs: &[RunInputRow]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for i in inputs {
            let res = sqlx::query(
                "INSERT INTO run_inputs
                    (run_id, input_type, input_ref, source, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(run_id)
            .bind(&i.input_type)
            .bind(&i.input_ref)
            .bind(&i.source)
            .bind(i.metadata_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_run_inputs row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn upsert_run_metrics(&self, row: &RunMetricsRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO run_metrics
                (run_id, num_seed_inputs, num_seed_addresses, num_addresses_total,
                 num_transfers, num_evidence_rows, num_clusters, num_multi_address_clusters,
                 top_cluster_size, metadata_json)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(run_id) DO UPDATE SET
                num_seed_inputs = excluded.num_seed_inputs,
                num_seed_addresses = excluded.num_seed_addresses,
                num_addresses_total = excluded.num_addresses_total,
                num_transfers = excluded.num_transfers,
                num_evidence_rows = excluded.num_evidence_rows,
                num_clusters = excluded.num_clusters,
                num_multi_address_clusters = excluded.num_multi_address_clusters,
                top_cluster_size = excluded.top_cluster_size,
                metadata_json = excluded.metadata_json,
                computed_at = datetime('now')",
        )
        .bind(&row.run_id)
        .bind(row.num_seed_inputs)
        .bind(row.num_seed_addresses)
        .bind(row.num_addresses_total)
        .bind(row.num_transfers)
        .bind(row.num_evidence_rows)
        .bind(row.num_clusters)
        .bind(row.num_multi_address_clusters)
        .bind(row.top_cluster_size)
        .bind(row.metadata_json.as_deref())
        .execute(&self.pool)
        .await
        .context("upsert_run_metrics failed")?;
        Ok(())
    }

    pub async fn upsert_cluster_metrics(&self, row: &ClusterMetricsRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO cluster_metrics
                (run_id, cluster_id, num_addresses, num_identifiers, num_evidence_rows,
                 num_unique_funders, top_funder, top_funder_share, first_funder_shared_count,
                 funding_block_min, funding_block_max, funding_block_span, funding_burst_label,
                 shared_safe_owner_count, control_link_density, num_unique_sinks, top_sink,
                 top_sink_share, possible_consolidation, coordination_tier,
                 coordination_reasons_json)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?21)
             ON CONFLICT(run_id, cluster_id) DO UPDATE SET
                num_addresses = excluded.num_addresses,
                num_identifiers = excluded.num_identifiers,
                num_evidence_rows = excluded.num_evidence_rows,
                num_unique_funders = excluded.num_unique_funders,
                top_funder = excluded.top_funder,
                top_funder_share = excluded.top_funder_share,
                first_funder_shared_count = excluded.first_funder_shared_count,
                funding_block_min = excluded.funding_block_min,
                funding_block_max = excluded.funding_block_max,
                funding_block_span = excluded.funding_block_span,
                funding_burst_label = excluded.funding_burst_label,
                shared_safe_owner_count = excluded.shared_safe_owner_count,
                control_link_density = excluded.control_link_density,
                num_unique_sinks = excluded.num_unique_sinks,
                top_sink = excluded.top_sink,
                top_sink_share = excluded.top_sink_share,
                possible_consolidation = excluded.possible_consolidation,
                coordination_tier = excluded.coordination_tier,
                coordination_reasons_json = excluded.coordination_reasons_json,
                computed_at = datetime('now')",
        )
        .bind(&row.run_id)
        .bind(&row.cluster_id)
        .bind(row.num_addresses)
        .bind(row.num_identifiers)
        .bind(row.num_evidence_rows)
        .bind(row.num_unique_funders)
        .bind(row.top_funder.as_deref())
        .bind(row.top_funder_share)
        .bind(row.first_funder_shared_count)
        .bind(row.funding_block_min)
        .bind(row.funding_block_max)
        .bind(row.funding_block_span)
        .bind(row.funding_burst_label.as_deref())
        .bind(row.shared_safe_owner_count)
        .bind(row.control_link_density)
        .bind(row.num_unique_sinks)
        .bind(row.top_sink.as_deref())
        .bind(row.top_sink_share)
        .bind(
            row.possible_consolidation
                .map(|v| if v { 1i64 } else { 0i64 }),
        )
        .bind(&row.coordination_tier)
        .bind(row.coordination_reasons_json.as_deref())
        .execute(&self.pool)
        .await
        .context("upsert_cluster_metrics failed")?;
        Ok(())
    }

    pub async fn insert_cluster_lineage_rows(&self, rows: &[ClusterLineageRow]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO cluster_lineage
                    (run_id_current, cluster_id_current, run_id_previous, cluster_id_previous,
                     overlap_count, jaccard, transition_label)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(row.run_id_current.as_deref())
            .bind(row.cluster_id_current.as_deref())
            .bind(row.run_id_previous.as_deref())
            .bind(row.cluster_id_previous.as_deref())
            .bind(row.overlap_count)
            .bind(row.jaccard)
            .bind(&row.transition_label)
            .execute(&mut *tx)
            .await
            .context("insert_cluster_lineage_rows row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn insert_graph_export_artifacts(
        &self,
        rows: &[GraphExportArtifact],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for row in rows {
            let res = sqlx::query(
                "INSERT INTO graph_exports
                    (run_id, artifact_type, path, sha256, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&row.run_id)
            .bind(&row.artifact_type)
            .bind(&row.path)
            .bind(&row.sha256)
            .bind(row.metadata_json.as_deref())
            .execute(&mut *tx)
            .await
            .context("insert_graph_export_artifacts row failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn latest_dataset_run_for_chain(
        &self,
        chain: &str,
    ) -> Result<Option<DatasetRunSummary>> {
        let row = sqlx::query(
            "SELECT run_id, chain, run_type, parent_run_id, window_start_block, window_end_block,
                    cadence, input_snapshot_hash, code_commit, policy_profile_id, stable_threshold, related_threshold, created_at
             FROM dataset_runs
             WHERE chain = ?1
             ORDER BY created_at DESC, run_id DESC
             LIMIT 1",
        )
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .context("latest_dataset_run_for_chain query failed")?;
        Ok(row.map(|r| DatasetRunSummary {
            run_id: r.get("run_id"),
            chain: r.get("chain"),
            run_type: r.get("run_type"),
            parent_run_id: r.get("parent_run_id"),
            window_start_block: r.get("window_start_block"),
            window_end_block: r.get("window_end_block"),
            cadence: r.get("cadence"),
            input_snapshot_hash: r.get("input_snapshot_hash"),
            code_commit: r.get("code_commit"),
            policy_profile_id: r.get("policy_profile_id"),
            stable_threshold: r.get("stable_threshold"),
            related_threshold: r.get("related_threshold"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn latest_dataset_run_for_chain_profile(
        &self,
        chain: &str,
        policy_profile_id: &str,
    ) -> Result<Option<DatasetRunSummary>> {
        let row = sqlx::query(
            "SELECT run_id, chain, run_type, parent_run_id, window_start_block, window_end_block,
                    cadence, input_snapshot_hash, code_commit, policy_profile_id, stable_threshold, related_threshold, created_at
             FROM dataset_runs
             WHERE chain = ?1 AND policy_profile_id = ?2
             ORDER BY created_at DESC, run_id DESC
             LIMIT 1",
        )
        .bind(chain)
        .bind(policy_profile_id)
        .fetch_optional(&self.pool)
        .await
        .context("latest_dataset_run_for_chain_profile query failed")?;
        Ok(row.map(|r| DatasetRunSummary {
            run_id: r.get("run_id"),
            chain: r.get("chain"),
            run_type: r.get("run_type"),
            parent_run_id: r.get("parent_run_id"),
            window_start_block: r.get("window_start_block"),
            window_end_block: r.get("window_end_block"),
            cadence: r.get("cadence"),
            input_snapshot_hash: r.get("input_snapshot_hash"),
            code_commit: r.get("code_commit"),
            policy_profile_id: r.get("policy_profile_id"),
            stable_threshold: r.get("stable_threshold"),
            related_threshold: r.get("related_threshold"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn clusters_for_run_map(&self, run_id: &str) -> Result<HashMap<String, Vec<String>>> {
        let rows = sqlx::query(
            "SELECT cluster_id, address
             FROM entity_clusters
             WHERE cluster_run_id = ?1
             ORDER BY cluster_id ASC, address ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("clusters_for_run_map query failed")?;
        let mut by: HashMap<String, Vec<String>> = HashMap::new();
        for r in rows {
            let cid: String = r.get("cluster_id");
            let addr: String = r.get("address");
            by.entry(cid).or_default().push(addr);
        }
        Ok(by)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::query_scalar;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DB_SEQ: AtomicU64 = AtomicU64::new(0);

    fn sample_dataset_run(run_id: &str, profile: &str) -> DatasetRun {
        DatasetRun {
            run_id: run_id.to_string(),
            chain: "arbitrum".to_string(),
            run_type: "monitor".to_string(),
            parent_run_id: None,
            window_start_block: 100,
            window_end_block: 200,
            window_start_ts: None,
            window_end_ts: None,
            cadence: "monthly".to_string(),
            seed_spec_json: r#"{"address_count":2}"#.to_string(),
            params_json: r#"{"min_evidence":1}"#.to_string(),
            input_snapshot_hash: "hash".to_string(),
            code_commit: "commit".to_string(),
            policy_profile_id: profile.to_string(),
            stable_threshold: 0.9,
            related_threshold: 0.5,
            notes: None,
        }
    }

    async fn test_repo() -> Repo {
        let seq = TEST_DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let db_url = format!("sqlite://data/test_repo_unit_{seq}_{ts}.db");
        let pool = connect(&db_url).await.expect("connect");
        run_migrations(&pool).await.expect("migrations");
        Repo::new(pool)
    }

    #[test]
    fn parse_shared_evidence_keys_handles_missing_invalid_and_valid_payload() {
        assert!(parse_shared_evidence_keys(None).is_empty());
        assert!(parse_shared_evidence_keys(Some("not-json")).is_empty());
        assert!(parse_shared_evidence_keys(Some(r#"{"foo":1}"#)).is_empty());
        assert_eq!(
            parse_shared_evidence_keys(Some(r#"{"shared_evidence_keys":["a","b","c"]}"#)),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[tokio::test]
    async fn incoming_funders_groups_by_funder_and_uses_earliest_block() {
        let repo = test_repo().await;
        let to = "0x1111111111111111111111111111111111111111";
        let funder_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let funder_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        repo.insert_transfer(&Transfer {
            from_addr: funder_a.to_string(),
            to_addr: to.to_string(),
            value: None,
            block_num: Some(42),
            tx_hash: Some("0x1".to_string()),
            asset: Some("ETH".to_string()),
        })
        .await
        .expect("insert transfer 1");
        repo.insert_transfer(&Transfer {
            from_addr: funder_a.to_string(),
            to_addr: to.to_string(),
            value: None,
            block_num: Some(10),
            tx_hash: Some("0x2".to_string()),
            asset: Some("ETH".to_string()),
        })
        .await
        .expect("insert transfer 2");
        repo.insert_transfer(&Transfer {
            from_addr: funder_b.to_string(),
            to_addr: to.to_string(),
            value: None,
            block_num: None,
            tx_hash: Some("0x3".to_string()),
            asset: Some("ETH".to_string()),
        })
        .await
        .expect("insert transfer 3");

        let rows = repo.incoming_funders(to).await.expect("incoming funders");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (funder_b.to_string(), 0));
        assert_eq!(rows[1], (funder_a.to_string(), 10));
    }

    #[tokio::test]
    async fn replace_attestations_for_kind_is_scoped_by_kind_and_address() {
        let repo = test_repo().await;
        let a1 = "0x1111111111111111111111111111111111111111".to_string();
        let a2 = "0x2222222222222222222222222222222222222222".to_string();

        let inserted = repo
            .insert_attestations(&[
                Attestation {
                    address: a1.clone(),
                    kind: EvidenceKind::FundedBy,
                    key: "0xfunder".to_string(),
                    strength: Strength::Medium,
                    source: "test".to_string(),
                    observed_block: 1,
                    payload_json: None,
                },
                Attestation {
                    address: a1.clone(),
                    kind: EvidenceKind::SafeOwner,
                    key: "0xowner".to_string(),
                    strength: Strength::Strong,
                    source: "test".to_string(),
                    observed_block: 2,
                    payload_json: None,
                },
            ])
            .await
            .expect("seed attestations");
        assert_eq!(inserted, 2);

        let replaced = repo
            .replace_attestations_for_kind(
                std::slice::from_ref(&a1),
                EvidenceKind::FundedBy,
                &[Attestation {
                    address: a2.clone(),
                    kind: EvidenceKind::FundedBy,
                    key: "0xfunder2".to_string(),
                    strength: Strength::Medium,
                    source: "test".to_string(),
                    observed_block: 3,
                    payload_json: None,
                }],
            )
            .await
            .expect("replace funded_by");
        assert_eq!(replaced, 1);

        let all = repo
            .attestations_for(&[a1, a2])
            .await
            .expect("attestations");
        assert_eq!(
            all.len(),
            2,
            "safe_owner should remain, funded_by should be replaced"
        );
        assert!(all.iter().any(|a| a.kind == EvidenceKind::SafeOwner));
        assert!(all.iter().any(|a| a.key == "0xfunder2"));
    }

    #[tokio::test]
    async fn dataset_runs_and_aux_tables_round_trip() {
        let repo = test_repo().await;
        repo.start_dataset_run(&sample_dataset_run("run-1", "p1"))
            .await
            .expect("run-1");
        repo.start_dataset_run(&sample_dataset_run("run-2", "p2"))
            .await
            .expect("run-2");
        repo.start_dataset_run(&sample_dataset_run("run-0", "p1"))
            .await
            .expect("run-0");

        let latest_chain = repo
            .latest_dataset_run_for_chain("arbitrum")
            .await
            .expect("latest chain")
            .expect("some run");
        assert_eq!(latest_chain.run_id, "run-2");

        let latest_profile = repo
            .latest_dataset_run_for_chain_profile("arbitrum", "p1")
            .await
            .expect("latest profile")
            .expect("some run");
        assert_eq!(latest_profile.run_id, "run-1");

        let none = repo
            .latest_dataset_run_for_chain_profile("arbitrum", "missing")
            .await
            .expect("query missing");
        assert!(none.is_none());

        let inserted_inputs = repo
            .insert_run_inputs(
                "run-1",
                &[RunInputRow {
                    input_type: "seed".to_string(),
                    input_ref: "addr-list".to_string(),
                    source: "unit-test".to_string(),
                    metadata_json: Some(r#"{"n":2}"#.to_string()),
                }],
            )
            .await
            .expect("insert run input");
        assert_eq!(inserted_inputs, 1);

        repo.upsert_run_metrics(&RunMetricsRow {
            run_id: "run-1".to_string(),
            num_seed_inputs: 1,
            num_seed_addresses: 2,
            num_addresses_total: 2,
            num_transfers: 3,
            num_evidence_rows: 4,
            num_clusters: 2,
            num_multi_address_clusters: 1,
            top_cluster_size: 2,
            metadata_json: None,
        })
        .await
        .expect("upsert run metrics");

        repo.upsert_cluster_metrics(&ClusterMetricsRow {
            run_id: "run-1".to_string(),
            cluster_id: "c1".to_string(),
            num_addresses: 2,
            num_identifiers: 2,
            num_evidence_rows: 3,
            num_unique_funders: Some(1),
            top_funder: Some("0xf".to_string()),
            top_funder_share: Some(0.9),
            first_funder_shared_count: Some(2),
            funding_block_min: Some(10),
            funding_block_max: Some(12),
            funding_block_span: Some(2),
            funding_burst_label: Some("short".to_string()),
            shared_safe_owner_count: Some(1),
            control_link_density: Some(0.8),
            num_unique_sinks: Some(1),
            top_sink: Some("0xs".to_string()),
            top_sink_share: Some(0.9),
            possible_consolidation: Some(true),
            coordination_tier: "medium".to_string(),
            coordination_reasons_json: Some(r#"["funded_by"]"#.to_string()),
        })
        .await
        .expect("upsert cluster metrics");

        let lineage_n = repo
            .insert_cluster_lineage_rows(&[ClusterLineageRow {
                run_id_current: Some("run-1".to_string()),
                cluster_id_current: Some("c1".to_string()),
                run_id_previous: Some("run-0".to_string()),
                cluster_id_previous: Some("c0".to_string()),
                overlap_count: 2,
                jaccard: 0.5,
                transition_label: "related".to_string(),
            }])
            .await
            .expect("insert lineage");
        assert_eq!(lineage_n, 1);

        let artifacts_n = repo
            .insert_graph_export_artifacts(&[GraphExportArtifact {
                run_id: "run-1".to_string(),
                artifact_type: "graph_json".to_string(),
                path: "out/x.json".to_string(),
                sha256: "abc".to_string(),
                metadata_json: None,
            }])
            .await
            .expect("insert artifacts");
        assert_eq!(artifacts_n, 1);
    }

    #[tokio::test]
    async fn clusters_for_run_map_groups_addresses() {
        let repo = test_repo().await;
        repo.start_clustering_run("r1", "{}")
            .await
            .expect("start clustering run");
        repo.insert_cluster("r1", "c1", &["0xa".to_string(), "0xb".to_string()], "{}")
            .await
            .expect("insert c1");
        repo.insert_cluster("r1", "c2", &["0xc".to_string()], "{}")
            .await
            .expect("insert c2");

        let map = repo.clusters_for_run_map("r1").await.expect("map");
        assert_eq!(map.get("c1").map(Vec::len), Some(2));
        assert_eq!(map.get("c2").map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn benchmark_tables_round_trip_and_append_only_constraints() {
        let repo = test_repo().await;
        let run = BenchmarkRun {
            benchmark_run_id: "bench-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 42,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-1".to_string(),
            code_commit: "commit-1".to_string(),
        };
        repo.start_benchmark_run(&run)
            .await
            .expect("start benchmark run");

        let n_truth = repo
            .insert_benchmark_ground_truth_rows(&[
                BenchmarkGroundTruthEntityRow {
                    benchmark_run_id: run.benchmark_run_id.clone(),
                    entity_id: "e1".to_string(),
                    wallet_id: "0xaaa".to_string(),
                    cohort: "governance".to_string(),
                    role_tag: None,
                },
                BenchmarkGroundTruthEntityRow {
                    benchmark_run_id: run.benchmark_run_id.clone(),
                    entity_id: "e1".to_string(),
                    wallet_id: "0xaab".to_string(),
                    cohort: "governance".to_string(),
                    role_tag: Some("coordinator".to_string()),
                },
            ])
            .await
            .expect("insert benchmark truth");
        assert_eq!(n_truth, 2);

        let n_evidence = repo
            .insert_benchmark_synthetic_evidence_rows(&[BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                evidence_id: "ev-1".to_string(),
                subject_wallet_id: "0xaaa".to_string(),
                counterparty_id: "0xfunder".to_string(),
                evidence_kind: "funded_by".to_string(),
                strength_hint: "medium".to_string(),
                event_time_bucket: Some("t0".to_string()),
                sequence_index: Some(1),
                metadata_json: Some(r#"{"scenario":"S1"}"#.to_string()),
            }])
            .await
            .expect("insert benchmark evidence");
        assert_eq!(n_evidence, 1);

        let n_results = repo
            .insert_benchmark_policy_results(&[BenchmarkPolicyResultRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                policy_variant: "naive_funded_by".to_string(),
                pred_cluster_id: "c1".to_string(),
                wallet_id: "0xaaa".to_string(),
                link_explanation_json: Some(r#"{"k":"funded_by"}"#.to_string()),
            }])
            .await
            .expect("insert benchmark policy result");
        assert_eq!(n_results, 1);

        repo.insert_benchmark_eval_metrics(&BenchmarkEvalMetricsRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            policy_variant: "naive_funded_by".to_string(),
            precision: 0.8,
            recall: 0.7,
            f1: 0.746,
            over_merge_rate: 0.1,
            under_merge_rate: 0.2,
            giant_component_inflation: 1.2,
            cluster_purity: 0.9,
            cluster_fragmentation: 1.1,
            calibration_json_by_evidence_kind: Some(r#"{"funded_by":0.6}"#.to_string()),
        })
        .await
        .expect("insert benchmark metrics");

        let n_details = repo
            .insert_benchmark_eval_details(&[BenchmarkEvalDetailRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                policy_variant: "naive_funded_by".to_string(),
                truth_entity_id: "e1".to_string(),
                matched_pred_cluster_id: Some("c1".to_string()),
                split_count: 0,
                merge_intrusion_count: 1,
                dominant_error_kind: Some("funded_by".to_string()),
                detail_json: Some(r#"{"note":"service_hub_risk"}"#.to_string()),
            }])
            .await
            .expect("insert benchmark details");
        assert_eq!(n_details, 1);

        let dup_result = repo
            .insert_benchmark_policy_results(&[BenchmarkPolicyResultRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                policy_variant: "naive_funded_by".to_string(),
                pred_cluster_id: "c2".to_string(),
                wallet_id: "0xaaa".to_string(),
                link_explanation_json: Some(r#"{"k":"updated"}"#.to_string()),
            }])
            .await;
        assert!(
            dup_result.is_err(),
            "duplicate policy results must fail to preserve append-only contract"
        );

        let dup_metrics = repo
            .insert_benchmark_eval_metrics(&BenchmarkEvalMetricsRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                policy_variant: "naive_funded_by".to_string(),
                precision: 0.1,
                recall: 0.1,
                f1: 0.1,
                over_merge_rate: 0.9,
                under_merge_rate: 0.9,
                giant_component_inflation: 9.0,
                cluster_purity: 0.1,
                cluster_fragmentation: 9.0,
                calibration_json_by_evidence_kind: None,
            })
            .await;
        assert!(
            dup_metrics.is_err(),
            "duplicate metrics rows must fail to preserve append-only contract"
        );
    }

    #[tokio::test]
    async fn benchmark_snapshot_insert_is_atomic_on_failure() {
        let repo = test_repo().await;
        let run = BenchmarkRun {
            benchmark_run_id: "bench-atomic-fail".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S5_service_hub_contaminated".to_string(),
            seed: 5,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-atomic".to_string(),
            code_commit: "commit-atomic".to_string(),
        };
        let truth_rows = vec![BenchmarkGroundTruthEntityRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            entity_id: "e1".to_string(),
            wallet_id: "0xabc".to_string(),
            cohort: "governance".to_string(),
            role_tag: None,
        }];
        let evidence_rows = vec![
            BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                evidence_id: "dup-1".to_string(),
                subject_wallet_id: "0xabc".to_string(),
                counterparty_id: "0xfunder".to_string(),
                evidence_kind: "funded_by".to_string(),
                strength_hint: "medium".to_string(),
                event_time_bucket: Some("t0".to_string()),
                sequence_index: Some(1),
                metadata_json: None,
            },
            BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: run.benchmark_run_id.clone(),
                evidence_id: "dup-1".to_string(),
                subject_wallet_id: "0xabc".to_string(),
                counterparty_id: "0xfunder".to_string(),
                evidence_kind: "funded_by".to_string(),
                strength_hint: "medium".to_string(),
                event_time_bucket: Some("t0".to_string()),
                sequence_index: Some(2),
                metadata_json: None,
            },
        ];

        let res = repo
            .insert_benchmark_snapshot(&run, &truth_rows, &evidence_rows)
            .await;
        assert!(res.is_err(), "duplicate evidence_id should fail");

        let run_count: i64 =
            query_scalar("SELECT COUNT(*) FROM benchmark_runs WHERE benchmark_run_id = ?1")
                .bind("bench-atomic-fail")
                .fetch_one(repo.pool())
                .await
                .expect("count benchmark run");
        assert_eq!(run_count, 0, "failed snapshot insert must rollback run row");
    }

    #[tokio::test]
    async fn benchmark_snapshot_rejects_mismatched_row_run_ids() {
        let repo = test_repo().await;
        let run = BenchmarkRun {
            benchmark_run_id: "bench-run-main".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1".to_string(),
            seed: 1,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap".to_string(),
            code_commit: "commit".to_string(),
        };
        let truth_rows = vec![BenchmarkGroundTruthEntityRow {
            benchmark_run_id: "bench-run-other".to_string(),
            entity_id: "e1".to_string(),
            wallet_id: "0xabc".to_string(),
            cohort: "governance".to_string(),
            role_tag: None,
        }];
        let evidence_rows = vec![BenchmarkSyntheticEvidenceRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            evidence_id: "ev-1".to_string(),
            subject_wallet_id: "0xabc".to_string(),
            counterparty_id: "0xf".to_string(),
            evidence_kind: "funded_by".to_string(),
            strength_hint: "medium".to_string(),
            event_time_bucket: Some("t0".to_string()),
            sequence_index: Some(1),
            metadata_json: None,
        }];
        let err = repo
            .insert_benchmark_snapshot(&run, &truth_rows, &evidence_rows)
            .await
            .expect_err("mismatch should fail");
        assert!(err.to_string().contains("run_id mismatch"));
    }

    #[tokio::test]
    async fn benchmark_eval_bundle_is_atomic_on_detail_failure() {
        let repo = test_repo().await;
        let run = BenchmarkRun {
            benchmark_run_id: "bench-bundle-atomic".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1".to_string(),
            seed: 1,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap".to_string(),
            code_commit: "commit".to_string(),
        };
        repo.start_benchmark_run(&run)
            .await
            .expect("start benchmark run");

        let metrics = BenchmarkEvalMetricsRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            policy_variant: "naive_funded_by".to_string(),
            precision: 0.5,
            recall: 0.5,
            f1: 0.5,
            over_merge_rate: 0.5,
            under_merge_rate: 0.5,
            giant_component_inflation: 1.0,
            cluster_purity: 0.5,
            cluster_fragmentation: 1.0,
            calibration_json_by_evidence_kind: None,
        };
        let bad_details = vec![BenchmarkEvalDetailRow {
            benchmark_run_id: run.benchmark_run_id.clone(),
            policy_variant: "conservative_funded_by".to_string(),
            truth_entity_id: "e1".to_string(),
            matched_pred_cluster_id: Some("c1".to_string()),
            split_count: 1,
            merge_intrusion_count: 0,
            dominant_error_kind: Some("none".to_string()),
            detail_json: None,
        }];
        let err = repo
            .insert_benchmark_eval_bundle(&metrics, &bad_details)
            .await
            .expect_err("mismatched policy in details should fail");
        assert!(err.to_string().contains("detail row run/policy mismatch"));

        let metrics_count: i64 = query_scalar(
            "SELECT COUNT(*) FROM benchmark_eval_metrics
             WHERE benchmark_run_id = ?1 AND policy_variant = 'naive_funded_by'",
        )
        .bind("bench-bundle-atomic")
        .fetch_one(repo.pool())
        .await
        .expect("metrics count");
        assert_eq!(
            metrics_count, 0,
            "bundle failure should not persist metrics"
        );
    }
}
