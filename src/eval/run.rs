//! Run ablation evaluation against gold pair labels.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::linking::pairwise::{
    fanout_table, pairwise_features, score_pair, LinkTier, LinkageParams,
};
use crate::linking::{cluster_from_attestations, FundedByMergePolicy, LinkingOutput, FAN_OUT_CAP};
use crate::storage::Repo;

use super::ablation::AblationMode;
use super::gold::{GoldLabel, GoldPair};

#[derive(Debug, Clone, Serialize)]
pub struct PairRow {
    pub address_a: String,
    pub address_b: String,
    pub gold_label: String,
    pub pairwise_tier: String,
    pub pairwise_score: f64,
    pub pairwise_positive: bool,
    pub rule_same_cluster: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Confusion {
    pub tp: u64,
    pub fp: u64,
    pub tn: u64,
    #[serde(rename = "fn")]
    pub false_negative: u64,
    pub ambiguous: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AblationReport {
    pub ablation: String,
    pub pair_rows: Vec<PairRow>,
    pub pairwise_vs_gold: Confusion,
    pub rule_merge_vs_gold: Confusion,
    /// Precision of treating top-K (by score) pairs as `same_control` predictions.
    /// Only pairs with gold label `same_control` or `different_control` enter the ranking.
    pub precision_at_k: BTreeMap<String, f64>,
    pub cluster_count: usize,
    pub skipped_service_keys: usize,
}

fn address_to_cluster_id(output: &LinkingOutput) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for c in &output.clusters {
        for a in &c.addresses {
            m.insert(a.clone(), c.cluster_id.clone());
        }
    }
    m
}

fn pairwise_positive(tier: LinkTier, deterministic_anchor: bool) -> bool {
    deterministic_anchor || tier == LinkTier::Accepted
}

/// Update confusion for pairwise-positive vs gold same/different (excludes uncertain gold).
fn bump_pairwise_confusion(c: &mut Confusion, gold: GoldLabel, pred_positive: bool) {
    match gold {
        GoldLabel::Uncertain => c.ambiguous += 1,
        GoldLabel::SameControl => {
            if pred_positive {
                c.tp += 1;
            } else {
                c.false_negative += 1;
            }
        }
        GoldLabel::DifferentControl => {
            if pred_positive {
                c.fp += 1;
            } else {
                c.tn += 1;
            }
        }
    }
}

fn bump_rule_confusion(c: &mut Confusion, gold: GoldLabel, merged: bool) {
    match gold {
        GoldLabel::Uncertain => c.ambiguous += 1,
        GoldLabel::SameControl => {
            if merged {
                c.tp += 1;
            } else {
                c.false_negative += 1;
            }
        }
        GoldLabel::DifferentControl => {
            if merged {
                c.fp += 1;
            } else {
                c.tn += 1;
            }
        }
    }
}

fn precision_at_k_scores(rows: &[(GoldPair, f64)]) -> BTreeMap<String, f64> {
    let mut ranked: Vec<(GoldPair, f64)> = rows
        .iter()
        .filter(|(g, _)| {
            g.label == GoldLabel::SameControl || g.label == GoldLabel::DifferentControl
        })
        .map(|(g, s)| (g.clone(), *s))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.address_a.cmp(&b.0.address_a))
    });

    let ks = [1usize, 2, 5, 10, 20, 50, 100];
    let mut out = BTreeMap::new();
    for k in ks {
        if k > ranked.len() || k == 0 {
            continue;
        }
        let slice = &ranked[..k];
        let same_in_top = slice
            .iter()
            .filter(|(g, _)| g.label == GoldLabel::SameControl)
            .count() as f64;
        out.insert(format!("p@{k}"), same_in_top / k as f64);
    }
    out
}

pub async fn run_ablation(
    repo: &Repo,
    gold: &[GoldPair],
    mode: AblationMode,
    min_evidence: usize,
    linkage_params: &LinkageParams,
) -> Result<AblationReport> {
    let addresses = super::gold::union_addresses(gold);
    let full = repo.attestations_for(&addresses).await?;
    let filtered = mode.filter(&full);

    let clusters = cluster_from_attestations(
        &addresses,
        &filtered,
        min_evidence,
        FAN_OUT_CAP,
        &FundedByMergePolicy::legacy_disabled(),
    )?;
    let cluster_of = address_to_cluster_id(&clusters);

    let fanout = fanout_table(&filtered);
    let mut pair_rows = Vec::with_capacity(gold.len());
    let mut pairwise_conf = Confusion::default();
    let mut rule_conf = Confusion::default();
    let mut score_pairs: Vec<(GoldPair, f64)> = Vec::new();

    for g in gold {
        let feats = pairwise_features(&g.address_a, &g.address_b, &filtered, &fanout);
        let scored = score_pair(&g.address_a, &g.address_b, &feats, &fanout, linkage_params);
        let pred_pos = pairwise_positive(scored.tier, scored.deterministic_anchor);
        bump_pairwise_confusion(&mut pairwise_conf, g.label, pred_pos);

        let merged = cluster_of
            .get(&g.address_a)
            .zip(cluster_of.get(&g.address_b))
            .map(|(ca, cb)| ca == cb)
            .unwrap_or(false);
        bump_rule_confusion(&mut rule_conf, g.label, merged);

        score_pairs.push((g.clone(), scored.score));

        pair_rows.push(PairRow {
            address_a: g.address_a.clone(),
            address_b: g.address_b.clone(),
            gold_label: g.label.as_str().to_string(),
            pairwise_tier: scored.tier.as_str().to_string(),
            pairwise_score: scored.score,
            pairwise_positive: pred_pos,
            rule_same_cluster: merged,
        });
    }

    let precision_at_k = precision_at_k_scores(&score_pairs);

    Ok(AblationReport {
        ablation: mode.as_str().to_string(),
        pair_rows,
        pairwise_vs_gold: pairwise_conf,
        rule_merge_vs_gold: rule_conf,
        precision_at_k,
        cluster_count: clusters.clusters.len(),
        skipped_service_keys: clusters.skipped_service_keys.len(),
    })
}

pub async fn run_eval_suite(
    repo: &Repo,
    gold_path: &Path,
    modes: &[AblationMode],
    min_evidence: usize,
    linkage_params: &LinkageParams,
) -> Result<EvalSuiteReport> {
    let gold = super::gold::load_gold_pairs(gold_path)?;
    let mut ablations = Vec::new();
    for m in modes {
        ablations.push(run_ablation(repo, &gold, *m, min_evidence, linkage_params).await?);
    }
    Ok(EvalSuiteReport {
        task: "labeled_pair_control_cluster_recovery",
        gold_path: gold_path.display().to_string(),
        gold_pair_count: gold.len(),
        min_evidence,
        ablations,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalSuiteReport {
    /// Fixed evaluation contract (not “same human”).
    pub task: &'static str,
    pub gold_path: String,
    pub gold_pair_count: usize,
    pub min_evidence: usize,
    pub ablations: Vec<AblationReport>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::eval::ablation::AblationMode;
    use crate::linking::{link_and_persist, LinkageParams};
    use crate::safe::SafeOwner;
    use crate::storage::{connect, run_migrations, Repo};

    fn safe_row(safe: &str, owner: &str) -> SafeOwner {
        SafeOwner {
            safe_address: safe.to_string(),
            owner_address: owner.to_string(),
            owner_is_safe: false,
            threshold: Some(2),
            observed_block: Some(100),
            source: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn run_ablation_tracks_confusion_and_precision_at_k() {
        let pool = connect("sqlite::memory:").await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let repo = Repo::new(pool);

        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        let lonely = "0xdddddddddddddddddddddddddddddddddddddddd";
        let other = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

        repo.upsert_safe_owner(&safe_row(safe_a, owner))
            .await
            .expect("so1");
        repo.upsert_safe_owner(&safe_row(safe_b, owner))
            .await
            .expect("so2");
        repo.upsert_address(safe_a, None).await.expect("a");
        repo.upsert_address(safe_b, None).await.expect("b");
        repo.upsert_address(lonely, None).await.expect("l");
        repo.upsert_address(other, None).await.expect("o");

        link_and_persist(&repo, &[safe_a.into(), safe_b.into()], 1)
            .await
            .expect("link populates evidence");

        let gold = vec![
            GoldPair {
                address_a: safe_a.to_string(),
                address_b: safe_b.to_string(),
                label: GoldLabel::SameControl,
                rationale: "shared owner".to_string(),
            },
            GoldPair {
                address_a: lonely.to_string(),
                address_b: other.to_string(),
                label: GoldLabel::DifferentControl,
                rationale: "no evidence".to_string(),
            },
            GoldPair {
                address_a: safe_a.to_string(),
                address_b: lonely.to_string(),
                label: GoldLabel::Uncertain,
                rationale: "mixed".to_string(),
            },
        ];

        let params = LinkageParams::bundled_default().expect("params");
        let report = run_ablation(&repo, &gold, AblationMode::SafeOwnerOnly, 1, &params)
            .await
            .expect("ablation");

        assert_eq!(report.pair_rows.len(), 3);
        assert!(report.precision_at_k.contains_key("p@1"));
        assert!(
            report.pairwise_vs_gold.ambiguous >= 1,
            "uncertain gold should bump ambiguous"
        );
        assert!(
            report.rule_merge_vs_gold.tp >= 1,
            "same_control should merge on safe_owner"
        );
    }

    #[tokio::test]
    async fn run_eval_suite_runs_multiple_modes() {
        let pool = connect("sqlite::memory:").await.expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let repo = Repo::new(pool);

        let safe_a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let safe_b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let owner = "0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
        repo.upsert_safe_owner(&safe_row(safe_a, owner))
            .await
            .expect("so1");
        repo.upsert_safe_owner(&safe_row(safe_b, owner))
            .await
            .expect("so2");
        repo.upsert_address(safe_a, None).await.expect("a");
        repo.upsert_address(safe_b, None).await.expect("b");

        let gold_path =
            std::env::temp_dir().join(format!("unmasking_eval_suite_{}.csv", std::process::id()));
        std::fs::write(
            &gold_path,
            format!("address_a,address_b,label,rationale\n{safe_a},{safe_b},same_control,x\n"),
        )
        .expect("gold");

        let params = LinkageParams::bundled_default().expect("params");
        let suite = run_eval_suite(
            &repo,
            Path::new(&gold_path),
            &[AblationMode::SafeOwnerOnly, AblationMode::FundedByOnly],
            1,
            &params,
        )
        .await
        .expect("suite");
        let _ = std::fs::remove_file(&gold_path);

        assert_eq!(suite.gold_pair_count, 1);
        assert_eq!(suite.ablations.len(), 2);
        assert_eq!(suite.ablations[0].ablation, "safe_owner_only");
        assert_eq!(suite.ablations[1].ablation, "funded_by_only");
    }
}
