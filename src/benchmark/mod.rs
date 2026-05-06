use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::{cluster_from_attestations, FundedByMergePolicy};
use crate::storage::{
    BenchmarkGroundTruthEntityRow, BenchmarkPolicyResultRow, BenchmarkRun,
    BenchmarkSyntheticEvidenceRow, Repo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cohort {
    Governance,
    Control,
    NegativeControl,
}

impl Cohort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Cohort::Governance => "governance",
            Cohort::Control => "control",
            Cohort::NegativeControl => "negative_control",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub scenario_id: String,
    pub entity_count: usize,
    pub wallets_per_entity: usize,
    pub governance_ratio: f64,
    pub control_ratio: f64,
}

impl ScenarioSpec {
    pub fn validate(&self) -> Result<()> {
        if self.scenario_id.trim().is_empty() {
            bail!("scenario_id must not be empty");
        }
        if self.entity_count == 0 {
            bail!("entity_count must be > 0");
        }
        if self.wallets_per_entity == 0 {
            bail!("wallets_per_entity must be > 0");
        }
        if !self.governance_ratio.is_finite() || !self.control_ratio.is_finite() {
            bail!("cohort ratios must be finite numbers");
        }
        if self.governance_ratio < 0.0
            || self.control_ratio < 0.0
            || (self.governance_ratio + self.control_ratio) > 1.0
        {
            bail!("invalid cohort ratios; governance + control must be in [0, 1]");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundTruthWallet {
    pub entity_id: String,
    pub wallet_id: String,
    pub cohort: Cohort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticDataset {
    pub scenario_id: String,
    pub seed: u64,
    pub wallets: Vec<GroundTruthWallet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticEvidenceConfig {
    pub emit_funded_by: bool,
    pub emit_safe_owner: bool,
    pub emit_did_controller: bool,
    pub emit_ens_handle: bool,
}

impl Default for SyntheticEvidenceConfig {
    fn default() -> Self {
        Self {
            emit_funded_by: true,
            emit_safe_owner: true,
            emit_did_controller: true,
            emit_ens_handle: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticEvidenceRow {
    pub evidence_id: String,
    pub subject_wallet_id: String,
    pub counterparty_id: String,
    pub evidence_kind: String,
    pub strength_hint: String,
    pub event_time_bucket: String,
    pub sequence_index: i64,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkPolicyComparisonConfig {
    pub min_evidence: usize,
    pub fan_out_cap: usize,
    pub conservative_service_fan_out_cap: usize,
    pub conservative_min_shared_keys: usize,
    pub conservative_min_short_burst_hits: usize,
    pub conservative_short_burst_block_delta: i64,
}

impl Default for BenchmarkPolicyComparisonConfig {
    fn default() -> Self {
        Self {
            min_evidence: 1,
            fan_out_cap: 50,
            conservative_service_fan_out_cap: 50,
            conservative_min_shared_keys: 2,
            conservative_min_short_burst_hits: 2,
            conservative_short_burst_block_delta: 5_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyntheticDatasetBuilder {
    spec: ScenarioSpec,
    seed: u64,
}

impl SyntheticDatasetBuilder {
    pub fn new(spec: ScenarioSpec, seed: u64) -> Self {
        Self { spec, seed }
    }

    pub fn build_ground_truth(&self) -> Result<SyntheticDataset> {
        self.spec.validate()?;
        let mut wallets = Vec::with_capacity(self.spec.entity_count * self.spec.wallets_per_entity);
        let cohorts_by_entity = deterministic_cohort_assignment(
            &self.spec.scenario_id,
            self.seed,
            self.spec.entity_count,
            self.spec.governance_ratio,
            self.spec.control_ratio,
        );

        for (entity_ix, cohort) in cohorts_by_entity
            .iter()
            .copied()
            .enumerate()
            .take(self.spec.entity_count)
        {
            let entity_id = deterministic_entity_id(entity_ix);
            for wallet_ix in 0..self.spec.wallets_per_entity {
                wallets.push(GroundTruthWallet {
                    entity_id: entity_id.clone(),
                    wallet_id: deterministic_wallet_id_scoped(
                        &self.spec.scenario_id,
                        self.seed,
                        entity_ix,
                        wallet_ix,
                    ),
                    cohort,
                });
            }
        }

        Ok(SyntheticDataset {
            scenario_id: self.spec.scenario_id.clone(),
            seed: self.seed,
            wallets,
        })
    }

    pub fn build_evidence_rows(
        &self,
        config: &SyntheticEvidenceConfig,
    ) -> Result<Vec<SyntheticEvidenceRow>> {
        let dataset = self.build_ground_truth()?;
        Ok(emit_synthetic_evidence(&dataset, config))
    }

    pub fn build_storage_ground_truth_rows(
        &self,
        benchmark_run_id: &str,
    ) -> Result<Vec<BenchmarkGroundTruthEntityRow>> {
        let dataset = self.build_ground_truth()?;
        Ok(dataset
            .wallets
            .iter()
            .map(|w| BenchmarkGroundTruthEntityRow {
                benchmark_run_id: benchmark_run_id.to_string(),
                entity_id: w.entity_id.clone(),
                wallet_id: w.wallet_id.clone(),
                cohort: w.cohort.as_str().to_string(),
                role_tag: None,
            })
            .collect())
    }

    pub fn build_storage_evidence_rows(
        &self,
        benchmark_run_id: &str,
        config: &SyntheticEvidenceConfig,
    ) -> Result<Vec<BenchmarkSyntheticEvidenceRow>> {
        let rows = self.build_evidence_rows(config)?;
        Ok(rows
            .iter()
            .map(|r| BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: benchmark_run_id.to_string(),
                evidence_id: r.evidence_id.clone(),
                subject_wallet_id: r.subject_wallet_id.clone(),
                counterparty_id: r.counterparty_id.clone(),
                evidence_kind: r.evidence_kind.clone(),
                strength_hint: r.strength_hint.clone(),
                event_time_bucket: Some(r.event_time_bucket.clone()),
                sequence_index: Some(r.sequence_index),
                metadata_json: r.metadata_json.clone(),
            })
            .collect())
    }

    pub async fn persist_snapshot(
        &self,
        repo: &Repo,
        run: &BenchmarkRun,
        config: &SyntheticEvidenceConfig,
    ) -> Result<()> {
        if run.scenario_id != self.spec.scenario_id {
            bail!(
                "benchmark run scenario_id mismatch: run={}, builder={}",
                run.scenario_id,
                self.spec.scenario_id
            );
        }
        if run.seed < 0 || run.seed as u64 != self.seed {
            bail!(
                "benchmark run seed mismatch: run={}, builder={}",
                run.seed,
                self.seed
            );
        }
        let truth_rows = self.build_storage_ground_truth_rows(&run.benchmark_run_id)?;
        let evidence_rows = self.build_storage_evidence_rows(&run.benchmark_run_id, config)?;
        repo.insert_benchmark_snapshot(run, &truth_rows, &evidence_rows)
            .await
    }

    pub async fn run_policy_comparison_and_persist(
        &self,
        repo: &Repo,
        benchmark_run_id: &str,
        config: &SyntheticEvidenceConfig,
        policy_cfg: &BenchmarkPolicyComparisonConfig,
    ) -> Result<()> {
        let dataset = self.build_ground_truth()?;
        let addresses = dataset
            .wallets
            .iter()
            .map(|w| w.wallet_id.clone())
            .collect::<Vec<_>>();
        let synthetic = self.build_evidence_rows(config)?;
        let attestations = synthetic_rows_to_attestations(&synthetic)?;

        let naive = cluster_from_attestations(
            &addresses,
            &attestations,
            policy_cfg.min_evidence,
            policy_cfg.fan_out_cap,
            &FundedByMergePolicy::legacy_disabled(),
        )?;
        let conservative_policy = FundedByMergePolicy {
            enabled: true,
            service_fan_out_cap: policy_cfg.conservative_service_fan_out_cap,
            min_shared_keys: policy_cfg.conservative_min_shared_keys,
            min_short_burst_hits: policy_cfg.conservative_min_short_burst_hits,
            short_burst_block_delta: policy_cfg.conservative_short_burst_block_delta,
        };
        let conservative = cluster_from_attestations(
            &addresses,
            &attestations,
            policy_cfg.min_evidence,
            policy_cfg.fan_out_cap,
            &conservative_policy,
        )?;

        let mut rows = policy_output_to_rows(benchmark_run_id, "naive_funded_by", &naive.clusters);
        rows.extend(policy_output_to_rows(
            benchmark_run_id,
            "conservative_funded_by",
            &conservative.clusters,
        ));
        let _ = repo.insert_benchmark_policy_results(&rows).await?;
        Ok(())
    }
}

fn synthetic_rows_to_attestations(rows: &[SyntheticEvidenceRow]) -> Result<Vec<Attestation>> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let kind = EvidenceKind::parse(&r.evidence_kind).ok_or_else(|| {
            anyhow::anyhow!("unknown synthetic evidence kind: {}", r.evidence_kind)
        })?;
        let strength = match r.strength_hint.as_str() {
            "strong" => Strength::Strong,
            "medium" => Strength::Medium,
            "weak" => Strength::Weak,
            other => {
                return Err(anyhow::anyhow!(
                    "unknown synthetic strength_hint: {}",
                    other
                ))
            }
        };
        out.push(Attestation {
            address: r.subject_wallet_id.clone(),
            kind,
            key: r.counterparty_id.clone(),
            strength,
            source: "benchmark_synthetic".to_string(),
            observed_block: r.sequence_index,
            payload_json: r.metadata_json.clone(),
        });
    }
    Ok(out)
}

fn policy_output_to_rows(
    benchmark_run_id: &str,
    policy_variant: &str,
    clusters: &[crate::linking::ClusterReport],
) -> Vec<BenchmarkPolicyResultRow> {
    let mut rows = Vec::new();
    for cluster in clusters {
        for wallet in &cluster.addresses {
            rows.push(BenchmarkPolicyResultRow {
                benchmark_run_id: benchmark_run_id.to_string(),
                policy_variant: policy_variant.to_string(),
                pred_cluster_id: cluster.cluster_id.clone(),
                wallet_id: wallet.clone(),
                link_explanation_json: Some(
                    serde_json::json!({
                        "shared_evidence_keys": cluster.shared_evidence_keys,
                    })
                    .to_string(),
                ),
            });
        }
    }
    rows
}

struct EvidenceRowInput<'a> {
    dataset: &'a SyntheticDataset,
    subject_wallet_id: &'a str,
    counterparty_id: &'a str,
    evidence_kind: &'a str,
    strength_hint: &'a str,
    bucket: i64,
    sequence_index: i64,
    entity_id: &'a str,
}

pub fn emit_synthetic_evidence(
    dataset: &SyntheticDataset,
    config: &SyntheticEvidenceConfig,
) -> Vec<SyntheticEvidenceRow> {
    let mut rows = Vec::new();
    let mut by_entity: std::collections::BTreeMap<&str, Vec<&GroundTruthWallet>> =
        std::collections::BTreeMap::new();
    for wallet in &dataset.wallets {
        by_entity
            .entry(wallet.entity_id.as_str())
            .or_default()
            .push(wallet);
    }

    for (entity_id, wallets) in by_entity {
        let base_bucket = (stable_hash64(&format!(
            "bucket:{}:{}:{}",
            dataset.scenario_id, dataset.seed, entity_id
        )) % 8) as i64;

        let funder = format!(
            "0x{:040x}",
            stable_hash64(&format!(
                "funder:{}:{}:{}",
                dataset.scenario_id, dataset.seed, entity_id
            ))
        );
        let safe_owner = format!(
            "0x{:040x}",
            stable_hash64(&format!(
                "safe_owner:{}:{}:{}",
                dataset.scenario_id, dataset.seed, entity_id
            ))
        );
        let did_controller = format!(
            "0x{:040x}",
            stable_hash64(&format!(
                "did_controller:{}:{}:{}",
                dataset.scenario_id, dataset.seed, entity_id
            ))
        );
        let ens_handle = format!(
            "ens_{}",
            stable_hash64(&format!(
                "ens:{}:{}:{}",
                dataset.scenario_id, dataset.seed, entity_id
            ))
        );

        for (ix, wallet) in wallets.iter().enumerate() {
            if config.emit_funded_by {
                rows.push(make_evidence_row(EvidenceRowInput {
                    dataset,
                    subject_wallet_id: wallet.wallet_id.as_str(),
                    counterparty_id: funder.as_str(),
                    evidence_kind: "funded_by",
                    strength_hint: "medium",
                    bucket: base_bucket,
                    sequence_index: ix as i64,
                    entity_id,
                }));
            }
            if config.emit_safe_owner {
                rows.push(make_evidence_row(EvidenceRowInput {
                    dataset,
                    subject_wallet_id: wallet.wallet_id.as_str(),
                    counterparty_id: safe_owner.as_str(),
                    evidence_kind: "safe_owner",
                    strength_hint: "medium",
                    bucket: base_bucket + 1,
                    sequence_index: ix as i64,
                    entity_id,
                }));
            }
            if config.emit_did_controller {
                rows.push(make_evidence_row(EvidenceRowInput {
                    dataset,
                    subject_wallet_id: wallet.wallet_id.as_str(),
                    counterparty_id: did_controller.as_str(),
                    evidence_kind: "did_controller",
                    strength_hint: "strong",
                    bucket: base_bucket + 2,
                    sequence_index: ix as i64,
                    entity_id,
                }));
            }
            if config.emit_ens_handle {
                rows.push(make_evidence_row(EvidenceRowInput {
                    dataset,
                    subject_wallet_id: wallet.wallet_id.as_str(),
                    counterparty_id: ens_handle.as_str(),
                    evidence_kind: "ens_handle",
                    strength_hint: "medium",
                    bucket: base_bucket + 3,
                    sequence_index: ix as i64,
                    entity_id,
                }));
            }
        }
    }
    rows
}

fn make_evidence_row(input: EvidenceRowInput<'_>) -> SyntheticEvidenceRow {
    let evidence_id = format!(
        "ev_{:016x}",
        stable_hash64(&format!(
            "{}:{}:{}:{}:{}:{}",
            input.dataset.scenario_id,
            input.dataset.seed,
            input.subject_wallet_id,
            input.counterparty_id,
            input.evidence_kind,
            input.sequence_index
        ))
    );
    SyntheticEvidenceRow {
        evidence_id,
        subject_wallet_id: input.subject_wallet_id.to_string(),
        counterparty_id: input.counterparty_id.to_string(),
        evidence_kind: input.evidence_kind.to_string(),
        strength_hint: input.strength_hint.to_string(),
        event_time_bucket: format!("t{}", input.bucket),
        sequence_index: input.sequence_index,
        metadata_json: Some(
            serde_json::json!({
                "scenario_id": input.dataset.scenario_id,
                "entity_id": input.entity_id,
            })
            .to_string(),
        ),
    }
}

fn deterministic_cohort_assignment(
    scenario_id: &str,
    seed: u64,
    entity_count: usize,
    governance_ratio: f64,
    control_ratio: f64,
) -> Vec<Cohort> {
    let governance_target = (governance_ratio * entity_count as f64).round() as usize;
    let control_target = (control_ratio * entity_count as f64).round() as usize;
    let governance_count = governance_target.min(entity_count);
    let control_count = control_target.min(entity_count.saturating_sub(governance_count));

    let mut entity_order = (0..entity_count).collect::<Vec<_>>();
    entity_order.sort_by_key(|ix| stable_hash64(&format!("cohort:{scenario_id}:{seed}:{ix}")));

    let mut cohorts = vec![Cohort::NegativeControl; entity_count];
    for ix in entity_order.iter().take(governance_count) {
        cohorts[*ix] = Cohort::Governance;
    }
    for ix in entity_order
        .iter()
        .skip(governance_count)
        .take(control_count)
    {
        cohorts[*ix] = Cohort::Control;
    }
    cohorts
}

pub fn deterministic_entity_id(entity_ix: usize) -> String {
    format!("ent_{entity_ix:05}")
}

pub fn deterministic_wallet_id(entity_ix: usize, wallet_ix: usize) -> String {
    deterministic_wallet_id_scoped("default", 0, entity_ix, wallet_ix)
}

pub fn deterministic_wallet_id_scoped(
    scenario_id: &str,
    seed: u64,
    entity_ix: usize,
    wallet_ix: usize,
) -> String {
    let h = stable_hash64(&format!("entity:{entity_ix}:wallet:{wallet_ix}"));
    let scoped = stable_hash64(&format!("wallet:{scenario_id}:{seed}:{h:016x}"));
    format!("0x{scoped:040x}")
}

fn stable_hash64(input: &str) -> u64 {
    // FNV-1a 64-bit for deterministic cross-platform hashing.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{connect, run_migrations, BenchmarkRun, Repo};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DB_SEQ: AtomicU64 = AtomicU64::new(0);

    async fn test_repo() -> Repo {
        let seq = TEST_DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let db_url = format!("sqlite://data/test_benchmark_mod_{seq}_{ts}.db");
        let pool = connect(&db_url).await.expect("connect");
        run_migrations(&pool).await.expect("migrations");
        Repo::new(pool)
    }

    #[test]
    fn scenario_spec_validation_rejects_invalid_inputs() {
        let bad = ScenarioSpec {
            scenario_id: "S1".to_string(),
            entity_count: 0,
            wallets_per_entity: 1,
            governance_ratio: 0.5,
            control_ratio: 0.4,
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn builder_is_deterministic_for_same_seed_and_spec() {
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 10,
            wallets_per_entity: 3,
            governance_ratio: 0.3,
            control_ratio: 0.5,
        };
        let a = SyntheticDatasetBuilder::new(spec.clone(), 42)
            .build_ground_truth()
            .expect("dataset A");
        let b = SyntheticDatasetBuilder::new(spec, 42)
            .build_ground_truth()
            .expect("dataset B");
        assert_eq!(a, b);
    }

    #[test]
    fn builder_changes_output_for_different_seed() {
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 10,
            wallets_per_entity: 3,
            governance_ratio: 0.3,
            control_ratio: 0.5,
        };
        let a = SyntheticDatasetBuilder::new(spec.clone(), 42)
            .build_ground_truth()
            .expect("dataset A");
        let b = SyntheticDatasetBuilder::new(spec, 43)
            .build_ground_truth()
            .expect("dataset B");
        assert_ne!(a, b);
    }

    #[test]
    fn builder_assigns_exact_cohort_counts_by_ratio_rounding() {
        let spec = ScenarioSpec {
            scenario_id: "S10_mixed_governance_control".to_string(),
            entity_count: 10,
            wallets_per_entity: 1,
            governance_ratio: 0.3,
            control_ratio: 0.4,
        };
        let ds = SyntheticDatasetBuilder::new(spec, 7)
            .build_ground_truth()
            .expect("dataset");
        let governance = ds
            .wallets
            .iter()
            .filter(|w| w.cohort == Cohort::Governance)
            .count();
        let control = ds
            .wallets
            .iter()
            .filter(|w| w.cohort == Cohort::Control)
            .count();
        let negative = ds
            .wallets
            .iter()
            .filter(|w| w.cohort == Cohort::NegativeControl)
            .count();
        assert_eq!(governance, 3);
        assert_eq!(control, 4);
        assert_eq!(negative, 3);
    }

    #[test]
    fn validate_rejects_nan_ratios_and_empty_scenario_id() {
        let bad_nan = ScenarioSpec {
            scenario_id: "S1".to_string(),
            entity_count: 1,
            wallets_per_entity: 1,
            governance_ratio: f64::NAN,
            control_ratio: 0.0,
        };
        assert!(bad_nan.validate().is_err());

        let bad_empty = ScenarioSpec {
            scenario_id: " ".to_string(),
            entity_count: 1,
            wallets_per_entity: 1,
            governance_ratio: 0.0,
            control_ratio: 0.0,
        };
        assert!(bad_empty.validate().is_err());
    }

    #[test]
    fn deterministic_wallet_ids_are_stable_and_hex_like() {
        let w = deterministic_wallet_id(12, 7);
        assert!(w.starts_with("0x"));
        assert_eq!(w.len(), 42);
        assert_eq!(w, deterministic_wallet_id(12, 7));
    }

    #[test]
    fn evidence_emission_is_deterministic_and_contains_expected_kinds() {
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 4,
            wallets_per_entity: 2,
            governance_ratio: 0.5,
            control_ratio: 0.25,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 99);
        let cfg = SyntheticEvidenceConfig::default();
        let a = builder.build_evidence_rows(&cfg).expect("rows a");
        let b = builder.build_evidence_rows(&cfg).expect("rows b");
        assert_eq!(a, b);

        let kinds = a
            .iter()
            .map(|r| r.evidence_kind.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(kinds.contains("funded_by"));
        assert!(kinds.contains("safe_owner"));
        assert!(kinds.contains("did_controller"));
        assert!(kinds.contains("ens_handle"));
        assert!(a
            .iter()
            .filter(|r| r.evidence_kind == "safe_owner")
            .all(|r| r.strength_hint == "medium"));
    }

    #[test]
    fn evidence_toggle_works() {
        let spec = ScenarioSpec {
            scenario_id: "S9_negative_control_only".to_string(),
            entity_count: 2,
            wallets_per_entity: 1,
            governance_ratio: 0.0,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 1);
        let cfg = SyntheticEvidenceConfig {
            emit_funded_by: true,
            emit_safe_owner: false,
            emit_did_controller: false,
            emit_ens_handle: false,
        };
        let rows = builder.build_evidence_rows(&cfg).expect("rows");
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.evidence_kind == "funded_by"));
    }

    #[test]
    fn storage_bridge_rows_are_deterministic_and_shaped() {
        let spec = ScenarioSpec {
            scenario_id: "S5_service_hub_contaminated".to_string(),
            entity_count: 3,
            wallets_per_entity: 2,
            governance_ratio: 0.33,
            control_ratio: 0.33,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 2026);
        let cfg = SyntheticEvidenceConfig::default();

        let truth_a = builder
            .build_storage_ground_truth_rows("bench-run-1")
            .expect("truth a");
        let truth_b = builder
            .build_storage_ground_truth_rows("bench-run-1")
            .expect("truth b");
        assert_eq!(truth_a.len(), 6);
        assert_eq!(truth_a, truth_b);
        assert!(truth_a.iter().all(|r| r.benchmark_run_id == "bench-run-1"));

        let ev_a = builder
            .build_storage_evidence_rows("bench-run-1", &cfg)
            .expect("ev a");
        let ev_b = builder
            .build_storage_evidence_rows("bench-run-1", &cfg)
            .expect("ev b");
        assert_eq!(ev_a, ev_b);
        assert!(!ev_a.is_empty());
        assert!(ev_a.iter().all(|r| r.benchmark_run_id == "bench-run-1"));
    }

    #[tokio::test]
    async fn persist_snapshot_writes_run_truth_and_evidence() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 2,
            wallets_per_entity: 2,
            governance_ratio: 0.5,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 11);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-snapshot-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 11,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-11".to_string(),
            code_commit: "commit-11".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist snapshot");

        let run_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM benchmark_runs WHERE benchmark_run_id = ?1")
                .bind("bench-snapshot-1")
                .fetch_one(repo.pool())
                .await
                .expect("run count");
        let truth_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_ground_truth_entities WHERE benchmark_run_id = ?1",
        )
        .bind("bench-snapshot-1")
        .fetch_one(repo.pool())
        .await
        .expect("truth count");
        let evidence_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_synthetic_evidence WHERE benchmark_run_id = ?1",
        )
        .bind("bench-snapshot-1")
        .fetch_one(repo.pool())
        .await
        .expect("evidence count");
        assert_eq!(run_count, 1);
        assert_eq!(truth_count, 4);
        assert!(evidence_count >= 4);
    }

    #[tokio::test]
    async fn persist_snapshot_rejects_run_metadata_mismatch() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 1,
            wallets_per_entity: 1,
            governance_ratio: 1.0,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 11);
        let bad_run = BenchmarkRun {
            benchmark_run_id: "bench-snapshot-mismatch".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S5_service_hub_contaminated".to_string(),
            seed: 999,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-bad".to_string(),
            code_commit: "commit-bad".to_string(),
        };
        let err = builder
            .persist_snapshot(&repo, &bad_run, &SyntheticEvidenceConfig::default())
            .await
            .expect_err("mismatch should error");
        assert!(err.to_string().contains("mismatch"));
    }

    #[tokio::test]
    async fn policy_comparison_persists_both_variants() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 3,
            wallets_per_entity: 2,
            governance_ratio: 0.33,
            control_ratio: 0.33,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 77);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-policy-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 77,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-77".to_string(),
            code_commit: "commit-77".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist snapshot");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-policy-1",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("run policy comparison");

        let naive_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_policy_results
             WHERE benchmark_run_id = ?1 AND policy_variant = 'naive_funded_by'",
        )
        .bind("bench-policy-1")
        .fetch_one(repo.pool())
        .await
        .expect("naive count");
        let conservative_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_policy_results
             WHERE benchmark_run_id = ?1 AND policy_variant = 'conservative_funded_by'",
        )
        .bind("bench-policy-1")
        .fetch_one(repo.pool())
        .await
        .expect("conservative count");
        assert_eq!(naive_count, 6);
        assert_eq!(conservative_count, 6);
    }
}
