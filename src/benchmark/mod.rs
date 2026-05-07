use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::evidence::{Attestation, EvidenceKind, Strength};
use crate::linking::{cluster_from_attestations, FundedByMergePolicy};
use crate::storage::{
    BenchmarkEvalDetailRow, BenchmarkEvalMetricsRow, BenchmarkGroundTruthEntityRow,
    BenchmarkPolicyResultRow, BenchmarkRun, BenchmarkSyntheticEvidenceRow, Repo,
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
    /// Optional service-hub style contamination. When enabled, a single
    /// high-fanout `funded_by` key touches wallets across unrelated entities.
    pub service_hub_contamination: Option<ServiceHubContaminationSpec>,
    /// Number of distinct shared `funded_by` keys emitted per non-negative
    /// entity. More keys enable conservative policy merges to use the
    /// funded-only repeated burst pathway.
    pub funded_by_keys_per_entity: usize,
    /// Number of additional shared "sink-like" `funded_by` keys emitted per
    /// entity at a later time offset. This approximates consolidation-style
    /// behavior using only the existing `funded_by` evidence kind.
    pub sink_keys_per_entity: usize,
    /// Synthetic time steps for short-burst evaluation.
    pub funded_by_wallet_time_step: i64,
    pub funded_by_key_time_step: i64,
    /// Later time offset for sink/consolidation-style keys.
    pub sink_time_offset: i64,
}

impl Default for SyntheticEvidenceConfig {
    fn default() -> Self {
        Self {
            emit_funded_by: true,
            emit_safe_owner: true,
            emit_did_controller: true,
            emit_ens_handle: true,
            service_hub_contamination: None,
            funded_by_keys_per_entity: 3,
            sink_keys_per_entity: 1,
            funded_by_wallet_time_step: 100,
            funded_by_key_time_step: 10,
            sink_time_offset: 20_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceHubContaminationSpec {
    /// Fraction of all wallets that receive the hub `funded_by` edge.
    /// Must be in \([0, 1]\).
    pub wallet_fraction: f64,
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

    pub async fn evaluate_policy_metrics_and_persist(
        &self,
        repo: &Repo,
        benchmark_run_id: &str,
        policy_variant: &str,
    ) -> Result<BenchmarkEvalMetricsRow> {
        let dataset = self.build_ground_truth()?;
        let mut truth_by_wallet: HashMap<String, String> = HashMap::new();
        for w in &dataset.wallets {
            truth_by_wallet.insert(w.wallet_id.clone(), w.entity_id.clone());
        }
        let pred_by_wallet = repo
            .benchmark_policy_assignments(benchmark_run_id, policy_variant)
            .await?;
        let truth_wallets: BTreeSet<String> = truth_by_wallet.keys().cloned().collect();
        let pred_wallets: BTreeSet<String> = pred_by_wallet.keys().cloned().collect();
        if truth_wallets != pred_wallets {
            let missing = truth_wallets
                .difference(&pred_wallets)
                .cloned()
                .collect::<Vec<_>>();
            let extra = pred_wallets
                .difference(&truth_wallets)
                .cloned()
                .collect::<Vec<_>>();
            bail!(
                "predicted wallet set mismatch: missing={:?} extra={:?}",
                missing,
                extra
            );
        }

        let wallets = truth_by_wallet.keys().cloned().collect::<Vec<_>>();
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut fn_ = 0usize;
        let mut predicted_same = 0usize;
        let mut truth_same = 0usize;
        for i in 0..wallets.len() {
            for j in (i + 1)..wallets.len() {
                let wi = &wallets[i];
                let wj = &wallets[j];
                let same_truth = truth_by_wallet[wi] == truth_by_wallet[wj];
                let same_pred = pred_by_wallet[wi] == pred_by_wallet[wj];
                if same_truth {
                    truth_same += 1;
                }
                if same_pred {
                    predicted_same += 1;
                }
                match (same_truth, same_pred) {
                    (true, true) => tp += 1,
                    (false, true) => fp += 1,
                    (true, false) => fn_ += 1,
                    (false, false) => {}
                }
            }
        }

        let precision = if predicted_same == 0 {
            if truth_same == 0 {
                1.0
            } else {
                0.0
            }
        } else {
            tp as f64 / predicted_same as f64
        };
        let recall = if truth_same == 0 {
            if predicted_same == 0 {
                1.0
            } else {
                0.0
            }
        } else {
            tp as f64 / truth_same as f64
        };
        let f1 = if (precision + recall) == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        let over_merge_rate = if predicted_same == 0 {
            0.0
        } else {
            fp as f64 / predicted_same as f64
        };
        let under_merge_rate = if truth_same == 0 {
            0.0
        } else {
            fn_ as f64 / truth_same as f64
        };

        let mut truth_sizes: HashMap<String, usize> = HashMap::new();
        let mut pred_sizes: HashMap<String, usize> = HashMap::new();
        let mut pred_to_truth_counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let mut truth_to_pred_clusters: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for wallet in &wallets {
            let truth = truth_by_wallet[wallet].clone();
            let pred = pred_by_wallet[wallet].clone();
            *truth_sizes.entry(truth.clone()).or_insert(0) += 1;
            *pred_sizes.entry(pred.clone()).or_insert(0) += 1;
            *pred_to_truth_counts
                .entry(pred.clone())
                .or_default()
                .entry(truth.clone())
                .or_insert(0) += 1;
            truth_to_pred_clusters
                .entry(truth)
                .or_default()
                .insert(pred);
        }
        let truth_largest = truth_sizes.values().copied().max().unwrap_or(1) as f64;
        let pred_largest = pred_sizes.values().copied().max().unwrap_or(0) as f64;
        let giant_component_inflation = pred_largest / truth_largest;

        let total_wallets = wallets.len().max(1) as f64;
        let mut purity_weighted_sum = 0usize;
        for counts in pred_to_truth_counts.values() {
            let cluster_size: usize = counts.values().sum();
            let dominant_truth = counts.values().copied().max().unwrap_or(0);
            purity_weighted_sum += dominant_truth.min(cluster_size);
        }
        let cluster_purity = purity_weighted_sum as f64 / total_wallets;

        let mut frag_sum = 0usize;
        for pred_set in truth_to_pred_clusters.values() {
            frag_sum += pred_set.len();
        }
        let cluster_fragmentation = if truth_to_pred_clusters.is_empty() {
            0.0
        } else {
            frag_sum as f64 / truth_to_pred_clusters.len() as f64
        };

        let persisted_evidence = repo
            .benchmark_synthetic_evidence_rows(benchmark_run_id)
            .await?;
        let calibration_json_by_evidence_kind = Some(compute_evidence_kind_calibration_json(
            &dataset.wallets,
            &persisted_evidence,
        )?);

        let detail_rows = build_eval_detail_rows(
            benchmark_run_id,
            policy_variant,
            &truth_by_wallet,
            &pred_by_wallet,
        );

        let row = BenchmarkEvalMetricsRow {
            benchmark_run_id: benchmark_run_id.to_string(),
            policy_variant: policy_variant.to_string(),
            precision,
            recall,
            f1,
            over_merge_rate,
            under_merge_rate,
            giant_component_inflation,
            cluster_purity,
            cluster_fragmentation,
            calibration_json_by_evidence_kind,
        };
        let _ = repo
            .insert_benchmark_eval_bundle(&row, &detail_rows)
            .await?;
        Ok(row)
    }

    pub async fn render_eval_report_markdown(
        &self,
        repo: &Repo,
        benchmark_run_id: &str,
    ) -> Result<String> {
        let metrics = repo
            .benchmark_eval_metrics_for_run(benchmark_run_id)
            .await?;
        let details = repo
            .benchmark_eval_details_for_run(benchmark_run_id)
            .await?;
        if metrics.is_empty() {
            bail!("no benchmark eval metrics found for run: {benchmark_run_id}");
        }
        let mut out = String::new();
        out.push_str("# Benchmark Evaluation Report\n\n");
        out.push_str(&format!("- benchmark_run_id: `{benchmark_run_id}`\n"));
        out.push_str(
            "- caveat: coordination-structure benchmark only; no maliciousness attribution.\n\n",
        );
        out.push_str("## Policy Metrics\n\n");
        for m in &metrics {
            out.push_str(&format!("### `{}`\n", m.policy_variant));
            out.push_str(&format!("- precision: {:.4}\n", m.precision));
            out.push_str(&format!("- recall: {:.4}\n", m.recall));
            out.push_str(&format!("- f1: {:.4}\n", m.f1));
            out.push_str(&format!("- over_merge_rate: {:.4}\n", m.over_merge_rate));
            out.push_str(&format!("- under_merge_rate: {:.4}\n", m.under_merge_rate));
            out.push_str(&format!(
                "- giant_component_inflation: {:.4}\n",
                m.giant_component_inflation
            ));
            out.push_str(&format!("- cluster_purity: {:.4}\n", m.cluster_purity));
            out.push_str(&format!(
                "- cluster_fragmentation: {:.4}\n\n",
                m.cluster_fragmentation
            ));
        }

        out.push_str("## Entity-Level Diagnostics\n\n");
        for d in &details {
            out.push_str(&format!(
                "- policy=`{}` truth_entity=`{}` split_count={} merge_intrusion_count={} dominant_error_kind=`{}`\n",
                d.policy_variant,
                d.truth_entity_id,
                d.split_count,
                d.merge_intrusion_count,
                d.dominant_error_kind.clone().unwrap_or_else(|| "unknown".to_string())
            ));
        }
        Ok(out)
    }

    pub async fn render_eval_report_json(
        &self,
        repo: &Repo,
        benchmark_run_id: &str,
    ) -> Result<serde_json::Value> {
        let metrics = repo
            .benchmark_eval_metrics_for_run(benchmark_run_id)
            .await?;
        let details = repo
            .benchmark_eval_details_for_run(benchmark_run_id)
            .await?;
        if metrics.is_empty() {
            bail!("no benchmark eval metrics found for run: {benchmark_run_id}");
        }
        Ok(serde_json::json!({
            "benchmark_run_id": benchmark_run_id,
            "caveat": "Coordination-structure benchmark only; no maliciousness attribution.",
            "metrics": metrics,
            "details": details,
        }))
    }
}

fn build_eval_detail_rows(
    benchmark_run_id: &str,
    policy_variant: &str,
    truth_by_wallet: &HashMap<String, String>,
    pred_by_wallet: &HashMap<String, String>,
) -> Vec<BenchmarkEvalDetailRow> {
    let mut truth_to_wallets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut pred_to_truth_counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (wallet, truth) in truth_by_wallet {
        let pred = pred_by_wallet.get(wallet).cloned().unwrap_or_default();
        truth_to_wallets
            .entry(truth.clone())
            .or_default()
            .push(wallet.clone());
        *pred_to_truth_counts
            .entry(pred)
            .or_default()
            .entry(truth.clone())
            .or_insert(0) += 1;
    }

    let mut out = Vec::new();
    for (truth_entity_id, wallets) in truth_to_wallets {
        let mut cluster_counts: BTreeMap<String, usize> = BTreeMap::new();
        for wallet in &wallets {
            let pred = pred_by_wallet.get(wallet).cloned().unwrap_or_default();
            *cluster_counts.entry(pred).or_insert(0) += 1;
        }
        let matched_pred_cluster_id = cluster_counts
            .iter()
            .max_by_key(|(_, n)| *n)
            .map(|(cluster, _)| cluster.clone());
        let split_count = cluster_counts.len() as i64;

        let merge_intrusion_count = matched_pred_cluster_id
            .as_ref()
            .and_then(|cluster| pred_to_truth_counts.get(cluster))
            .map(|by_truth| {
                by_truth
                    .iter()
                    .filter(|(truth, _)| *truth != &truth_entity_id)
                    .map(|(_, count)| *count as i64)
                    .sum::<i64>()
            })
            .unwrap_or(0);

        let dominant_error_kind = if split_count > 1 && merge_intrusion_count > 0 {
            Some("mixed".to_string())
        } else if split_count > 1 {
            Some("under_merge".to_string())
        } else if merge_intrusion_count > 0 {
            Some("over_merge".to_string())
        } else {
            Some("none".to_string())
        };

        out.push(BenchmarkEvalDetailRow {
            benchmark_run_id: benchmark_run_id.to_string(),
            policy_variant: policy_variant.to_string(),
            truth_entity_id: truth_entity_id.clone(),
            matched_pred_cluster_id,
            split_count,
            merge_intrusion_count,
            dominant_error_kind,
            detail_json: Some(
                serde_json::json!({
                    "truth_wallet_count": wallets.len(),
                    "pred_cluster_counts": cluster_counts,
                })
                .to_string(),
            ),
        });
    }
    out
}

fn compute_evidence_kind_calibration_json(
    wallets: &[GroundTruthWallet],
    evidence_rows: &[BenchmarkSyntheticEvidenceRow],
) -> Result<String> {
    let mut truth_by_wallet = HashMap::new();
    for w in wallets {
        truth_by_wallet.insert(w.wallet_id.clone(), w.entity_id.clone());
    }

    let mut by_kind_key: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for row in evidence_rows {
        if !truth_by_wallet.contains_key(&row.subject_wallet_id) {
            bail!(
                "calibration evidence contains unknown subject_wallet_id: {}",
                row.subject_wallet_id
            );
        }
        by_kind_key
            .entry(row.evidence_kind.clone())
            .or_default()
            .entry(row.counterparty_id.clone())
            .or_default()
            .insert(row.subject_wallet_id.clone());
    }

    let mut out = serde_json::Map::new();
    for (kind, key_groups) in by_kind_key {
        let mut pair_count = 0usize;
        let mut same_truth_pairs = 0usize;
        for wallets_set in key_groups.values() {
            let ws = wallets_set.iter().cloned().collect::<Vec<_>>();
            for i in 0..ws.len() {
                for j in (i + 1)..ws.len() {
                    pair_count += 1;
                    if truth_by_wallet.get(&ws[i]) == truth_by_wallet.get(&ws[j]) {
                        same_truth_pairs += 1;
                    }
                }
            }
        }
        let precision = if pair_count == 0 {
            0.0
        } else {
            same_truth_pairs as f64 / pair_count as f64
        };
        out.insert(
            kind,
            serde_json::json!({
                "pair_count": pair_count,
                "same_truth_pairs": same_truth_pairs,
                "precision": precision,
            }),
        );
    }
    Ok(serde_json::Value::Object(out).to_string())
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
    let mut by_entity: BTreeMap<&str, Vec<&GroundTruthWallet>> = BTreeMap::new();
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
        let base_time = base_bucket * 1_000_000;

        let mut funder_keys: Vec<String> =
            Vec::with_capacity(config.funded_by_keys_per_entity.max(1));
        for k in 0..config.funded_by_keys_per_entity {
            funder_keys.push(format!(
                "0x{:040x}",
                stable_hash64(&format!(
                    "funder_key:{}:{}:{}:{}",
                    dataset.scenario_id, dataset.seed, entity_id, k
                ))
            ));
        }
        let mut sink_keys: Vec<String> = Vec::with_capacity(config.sink_keys_per_entity.max(1));
        for s in 0..config.sink_keys_per_entity {
            sink_keys.push(format!(
                "0x{:040x}",
                stable_hash64(&format!(
                    "sink_key:{}:{}:{}:{}",
                    dataset.scenario_id, dataset.seed, entity_id, s
                ))
            ));
        }
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
                for (k, funder_key) in funder_keys.iter().enumerate() {
                    rows.push(make_evidence_row(EvidenceRowInput {
                        dataset,
                        subject_wallet_id: wallet.wallet_id.as_str(),
                        counterparty_id: funder_key.as_str(),
                        evidence_kind: "funded_by",
                        strength_hint: "medium",
                        bucket: base_bucket,
                        sequence_index: base_time
                            + (ix as i64) * config.funded_by_wallet_time_step
                            + (k as i64) * config.funded_by_key_time_step,
                        entity_id,
                    }));
                }
                for (s, sink_key) in sink_keys.iter().enumerate() {
                    rows.push(make_evidence_row(EvidenceRowInput {
                        dataset,
                        subject_wallet_id: wallet.wallet_id.as_str(),
                        counterparty_id: sink_key.as_str(),
                        evidence_kind: "funded_by",
                        strength_hint: "medium",
                        bucket: base_bucket + 1,
                        sequence_index: base_time
                            + config.sink_time_offset
                            + (ix as i64) * config.funded_by_wallet_time_step
                            + (s as i64) * config.funded_by_key_time_step,
                        entity_id,
                    }));
                }
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

    if config.emit_funded_by {
        if let Some(spec) = config.service_hub_contamination.as_ref() {
            if spec.wallet_fraction.is_finite()
                && spec.wallet_fraction > 0.0
                && spec.wallet_fraction <= 1.0
            {
                let mut all_wallets = dataset
                    .wallets
                    .iter()
                    .map(|w| (w.wallet_id.as_str(), w.entity_id.as_str()))
                    .collect::<Vec<_>>();
                all_wallets.sort_by_key(|(wallet_id, _)| {
                    stable_hash64(&format!(
                        "hub_select:{}:{}:{}",
                        dataset.scenario_id, dataset.seed, wallet_id
                    ))
                });
                let n = ((all_wallets.len() as f64) * spec.wallet_fraction).round() as usize;
                let n = n.min(all_wallets.len());
                let hub_funder = format!(
                    "0x{:040x}",
                    stable_hash64(&format!(
                        "service_hub_funder:{}:{}",
                        dataset.scenario_id, dataset.seed
                    ))
                );
                for (ix, (wallet_id, entity_id)) in all_wallets.into_iter().take(n).enumerate() {
                    rows.push(make_evidence_row(EvidenceRowInput {
                        dataset,
                        subject_wallet_id: wallet_id,
                        counterparty_id: hub_funder.as_str(),
                        evidence_kind: "funded_by",
                        strength_hint: "medium",
                        bucket: 0,
                        sequence_index: 10_000 + ix as i64,
                        entity_id,
                    }));
                }
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
            .collect::<BTreeSet<_>>();
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
            service_hub_contamination: None,
            ..SyntheticEvidenceConfig::default()
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

    #[tokio::test]
    async fn evaluator_persists_minimal_pairwise_metrics() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 3,
            wallets_per_entity: 2,
            governance_ratio: 0.33,
            control_ratio: 0.33,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 88);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-eval-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 88,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-88".to_string(),
            code_commit: "commit-88".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist snapshot");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-eval-1",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let metrics = builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-eval-1", "naive_funded_by")
            .await
            .expect("eval metrics");
        assert!((0.0..=1.0).contains(&metrics.precision));
        assert!((0.0..=1.0).contains(&metrics.recall));
        assert!((0.0..=1.0).contains(&metrics.f1));
        assert!((0.0..=1.0).contains(&metrics.over_merge_rate));
        assert!((0.0..=1.0).contains(&metrics.under_merge_rate));
        assert!(metrics.cluster_fragmentation >= 0.0);
        assert!(metrics.giant_component_inflation >= 0.0);

        let row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_eval_metrics
             WHERE benchmark_run_id = ?1 AND policy_variant = 'naive_funded_by'",
        )
        .bind("bench-eval-1")
        .fetch_one(repo.pool())
        .await
        .expect("metrics row count");
        assert_eq!(row_count, 1);

        let detail_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM benchmark_eval_details
             WHERE benchmark_run_id = ?1 AND policy_variant = 'naive_funded_by'",
        )
        .bind("bench-eval-1")
        .fetch_one(repo.pool())
        .await
        .expect("details row count");
        assert_eq!(detail_count, 3);

        assert!(
            metrics
                .calibration_json_by_evidence_kind
                .as_deref()
                .unwrap_or("")
                .contains("funded_by"),
            "calibration JSON should include evidence-kind stats"
        );
    }

    #[tokio::test]
    async fn report_helpers_render_markdown_and_json() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 2,
            wallets_per_entity: 2,
            governance_ratio: 0.5,
            control_ratio: 0.5,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 101);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-report-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 101,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-101".to_string(),
            code_commit: "commit-101".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-report-1",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("compare");
        builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-report-1", "naive_funded_by")
            .await
            .expect("eval naive");
        builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-report-1", "conservative_funded_by")
            .await
            .expect("eval conservative");

        let md = builder
            .render_eval_report_markdown(&repo, "bench-report-1")
            .await
            .expect("md report");
        assert!(md.contains("Benchmark Evaluation Report"));
        assert!(md.contains("naive_funded_by"));
        assert!(md.contains("conservative_funded_by"));

        let json = builder
            .render_eval_report_json(&repo, "bench-report-1")
            .await
            .expect("json report");
        assert_eq!(
            json.get("benchmark_run_id")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "bench-report-1"
        );
        let metrics_len = json
            .get("metrics")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(metrics_len, 2);
    }

    #[tokio::test]
    async fn evaluator_rejects_extra_predicted_wallets() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 2,
            wallets_per_entity: 1,
            governance_ratio: 0.5,
            control_ratio: 0.5,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 501);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-extra-wallet".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 501,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-501".to_string(),
            code_commit: "commit-501".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-extra-wallet",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let _ = repo
            .insert_benchmark_policy_results(&[BenchmarkPolicyResultRow {
                benchmark_run_id: "bench-extra-wallet".to_string(),
                policy_variant: "naive_funded_by".to_string(),
                pred_cluster_id: "cluster_extra".to_string(),
                wallet_id: "0x9999999999999999999999999999999999999999".to_string(),
                link_explanation_json: None,
            }])
            .await
            .expect("insert extra");
        let err = builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-extra-wallet", "naive_funded_by")
            .await
            .expect_err("extra predicted wallet should fail");
        assert!(err.to_string().contains("predicted wallet set mismatch"));
    }

    #[tokio::test]
    async fn evaluator_no_positive_pairs_convention_returns_ones() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S9_negative_control_only".to_string(),
            entity_count: 3,
            wallets_per_entity: 1,
            governance_ratio: 0.0,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 777);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-no-positives".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S9_negative_control_only".to_string(),
            seed: 777,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-777".to_string(),
            code_commit: "commit-777".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-no-positives",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let metrics = builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-no-positives", "naive_funded_by")
            .await
            .expect("evaluate");
        assert!((metrics.precision - 1.0).abs() < 1e-9);
        assert!((metrics.recall - 1.0).abs() < 1e-9);
        assert!((metrics.f1 - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn evaluator_calibration_uses_persisted_evidence_rows() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 2,
            wallets_per_entity: 2,
            governance_ratio: 0.5,
            control_ratio: 0.5,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 909);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-calibration-source".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 909,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-909".to_string(),
            code_commit: "commit-909".to_string(),
        };
        builder
            .persist_snapshot(
                &repo,
                &run,
                &SyntheticEvidenceConfig {
                    emit_funded_by: true,
                    emit_safe_owner: false,
                    emit_did_controller: false,
                    emit_ens_handle: false,
                    service_hub_contamination: None,
                    ..SyntheticEvidenceConfig::default()
                },
            )
            .await
            .expect("persist with funded_only");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-calibration-source",
                &SyntheticEvidenceConfig {
                    emit_funded_by: true,
                    emit_safe_owner: false,
                    emit_did_controller: false,
                    emit_ens_handle: false,
                    service_hub_contamination: None,
                    ..SyntheticEvidenceConfig::default()
                },
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let metrics = builder
            .evaluate_policy_metrics_and_persist(
                &repo,
                "bench-calibration-source",
                "naive_funded_by",
            )
            .await
            .expect("evaluate");
        let calibration = metrics
            .calibration_json_by_evidence_kind
            .unwrap_or_default();
        assert!(calibration.contains("funded_by"));
        assert!(!calibration.contains("safe_owner"));
        assert!(!calibration.contains("did_controller"));
        assert!(!calibration.contains("ens_handle"));
    }

    #[tokio::test]
    async fn evaluator_calibration_rejects_unknown_evidence_wallet() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S1_clean_shared_funder".to_string(),
            entity_count: 2,
            wallets_per_entity: 1,
            governance_ratio: 0.5,
            control_ratio: 0.5,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 1001);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-calibration-unknown-wallet".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S1_clean_shared_funder".to_string(),
            seed: 1001,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-1001".to_string(),
            code_commit: "commit-1001".to_string(),
        };
        builder
            .persist_snapshot(&repo, &run, &SyntheticEvidenceConfig::default())
            .await
            .expect("persist");
        let _ = repo
            .insert_benchmark_synthetic_evidence_rows(&[BenchmarkSyntheticEvidenceRow {
                benchmark_run_id: "bench-calibration-unknown-wallet".to_string(),
                evidence_id: "ev-unknown-wallet".to_string(),
                subject_wallet_id: "0x9999999999999999999999999999999999999999".to_string(),
                counterparty_id: "0xfunder".to_string(),
                evidence_kind: "funded_by".to_string(),
                strength_hint: "medium".to_string(),
                event_time_bucket: Some("t0".to_string()),
                sequence_index: Some(1),
                metadata_json: None,
            }])
            .await
            .expect("inject unknown evidence");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-calibration-unknown-wallet",
                &SyntheticEvidenceConfig::default(),
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let err = builder
            .evaluate_policy_metrics_and_persist(
                &repo,
                "bench-calibration-unknown-wallet",
                "naive_funded_by",
            )
            .await
            .expect_err("unknown wallet in evidence should fail calibration");
        assert!(err.to_string().contains("unknown subject_wallet_id"));
    }

    #[tokio::test]
    async fn service_hub_contamination_makes_conservative_less_overmerged() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S5_service_hub_contaminated".to_string(),
            entity_count: 12,
            wallets_per_entity: 2,
            governance_ratio: 0.25,
            control_ratio: 0.25,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 4242);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-hub-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S5_service_hub_contaminated".to_string(),
            seed: 4242,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-4242".to_string(),
            code_commit: "commit-4242".to_string(),
        };
        let cfg = SyntheticEvidenceConfig {
            emit_funded_by: true,
            emit_safe_owner: false,
            emit_did_controller: false,
            emit_ens_handle: false,
            service_hub_contamination: Some(ServiceHubContaminationSpec {
                wallet_fraction: 0.8,
            }),
            ..SyntheticEvidenceConfig::default()
        };
        builder
            .persist_snapshot(&repo, &run, &cfg)
            .await
            .expect("persist");
        builder
            .run_policy_comparison_and_persist(
                &repo,
                "bench-hub-1",
                &cfg,
                &BenchmarkPolicyComparisonConfig::default(),
            )
            .await
            .expect("policy compare");
        let naive = builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-hub-1", "naive_funded_by")
            .await
            .expect("eval naive");
        let conservative = builder
            .evaluate_policy_metrics_and_persist(&repo, "bench-hub-1", "conservative_funded_by")
            .await
            .expect("eval conservative");

        assert!(
            conservative.over_merge_rate <= naive.over_merge_rate,
            "conservative should not over-merge more than naive under service-hub contamination"
        );
        assert!(
            conservative.precision >= naive.precision,
            "conservative precision should be at least naive under service-hub contamination"
        );
    }

    #[tokio::test]
    async fn conservative_funded_only_merges_via_repeated_funder_keys_and_bursts() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S3_short_burst".to_string(),
            entity_count: 2,
            wallets_per_entity: 2,
            governance_ratio: 1.0,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 333);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-funded-only-1".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S3_short_burst".to_string(),
            seed: 333,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-333".to_string(),
            code_commit: "commit-333".to_string(),
        };

        let cfg = SyntheticEvidenceConfig {
            emit_funded_by: true,
            emit_safe_owner: false,
            emit_did_controller: false,
            emit_ens_handle: false,
            service_hub_contamination: None,
            funded_by_keys_per_entity: 2,
            sink_keys_per_entity: 0,
            funded_by_wallet_time_step: 100,
            funded_by_key_time_step: 10,
            sink_time_offset: 20_000,
        };

        builder
            .persist_snapshot(&repo, &run, &cfg)
            .await
            .expect("persist");

        builder
            .run_policy_comparison_and_persist(
                &repo,
                &run.benchmark_run_id,
                &cfg,
                &BenchmarkPolicyComparisonConfig {
                    min_evidence: 1,
                    fan_out_cap: 50,
                    conservative_service_fan_out_cap: 50,
                    conservative_min_shared_keys: 2,
                    conservative_min_short_burst_hits: 2,
                    conservative_short_burst_block_delta: 5_000,
                },
            )
            .await
            .expect("policy compare");

        let cluster_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT pred_cluster_id)
             FROM benchmark_policy_results
             WHERE benchmark_run_id = ?1 AND policy_variant = 'conservative_funded_by'",
        )
        .bind(&run.benchmark_run_id)
        .fetch_one(repo.pool())
        .await
        .expect("cluster count");

        assert_eq!(
            cluster_count, 2,
            "wallets within each truth entity should merge"
        );
    }

    #[tokio::test]
    async fn conservative_funded_only_merges_via_sink_keys() {
        let repo = test_repo().await;
        let spec = ScenarioSpec {
            scenario_id: "S4_common_sink".to_string(),
            entity_count: 2,
            wallets_per_entity: 2,
            governance_ratio: 1.0,
            control_ratio: 0.0,
        };
        let builder = SyntheticDatasetBuilder::new(spec, 444);
        let run = BenchmarkRun {
            benchmark_run_id: "bench-funded-only-2".to_string(),
            scenario_suite_id: "suite-v0".to_string(),
            scenario_id: "S4_common_sink".to_string(),
            seed: 444,
            generator_version: "v0".to_string(),
            policy_profile_id: "arbitrum_gov_conservative_v1".to_string(),
            policy_variant: "naive_funded_by".to_string(),
            input_snapshot_hash: "snap-444".to_string(),
            code_commit: "commit-444".to_string(),
        };

        let cfg = SyntheticEvidenceConfig {
            emit_funded_by: true,
            emit_safe_owner: false,
            emit_did_controller: false,
            emit_ens_handle: false,
            service_hub_contamination: None,
            funded_by_keys_per_entity: 1,
            sink_keys_per_entity: 2,
            funded_by_wallet_time_step: 100,
            funded_by_key_time_step: 10,
            sink_time_offset: 20_000,
        };

        builder
            .persist_snapshot(&repo, &run, &cfg)
            .await
            .expect("persist");

        builder
            .run_policy_comparison_and_persist(
                &repo,
                &run.benchmark_run_id,
                &cfg,
                &BenchmarkPolicyComparisonConfig {
                    min_evidence: 1,
                    fan_out_cap: 50,
                    conservative_service_fan_out_cap: 50,
                    conservative_min_shared_keys: 2,
                    conservative_min_short_burst_hits: 2,
                    conservative_short_burst_block_delta: 5_000,
                },
            )
            .await
            .expect("policy compare");

        let cluster_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT pred_cluster_id)
             FROM benchmark_policy_results
             WHERE benchmark_run_id = ?1 AND policy_variant = 'conservative_funded_by'",
        )
        .bind(&run.benchmark_run_id)
        .fetch_one(repo.pool())
        .await
        .expect("cluster count");

        assert_eq!(
            cluster_count, 2,
            "wallets within each truth entity should merge via sink keys"
        );
    }
}
