//! Pairwise entity-linkage features and an interpretable linear score.
//!
//! Typed rows in `evidence` remain the auditable observation layer. This
//! module aggregates them into **address–address** features, applies a
//! small hand-tunable weight vector (Fellegi–Sunter–shaped: log-linear
//! intuition, not a trained EM model yet), and assigns a three-way tier:
//! accepted / uncertain / rejected.
//!
//! Strong cryptographic/structural signals (`did_controller`) still act
//! as a **deterministic anchor**: any pair sharing a controller key is
//! always tier `accepted`, independent of the scalar score thresholds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::evidence::{Attestation, EvidenceKind};

use super::features::FAN_OUT_CAP;

/// Tunable linkage weights and thresholds. Load from JSON; defaults match
/// `data/linkage_params.default.json` in the repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkageParams {
    /// Score ≥ this ⇒ `accepted` (unless only weak channels contributed —
    /// we still require at least one structural channel for non-anchor
    /// accepts; see [`score_pair`].)
    pub t_high: f64,
    /// Score > this (and < `t_high`) ⇒ `uncertain`.
    pub t_low: f64,
    pub w_did_controller: f64,
    /// `|A ∩ B| / |A ∪ B|` on Safe owner keys. Often understates **shared
    /// control** when a hub multisig (large owner set) overlaps a smaller
    /// committee; prefer [`Self::w_safe_owner_min_overlap`] for governance
    /// shapes.
    pub w_safe_owner_jaccard: f64,
    /// `|A ∩ B| / max(1, min(|A|, |B|))` — overlap relative to the **smaller**
    /// signer set (“committee ⊆ treasury” semantics). When `> 0`, this term
    /// is used **instead of** the Jaccard term (not stacked with it).
    #[serde(default)]
    pub w_safe_owner_min_overlap: f64,
    /// Linear bonus per **shared** Safe-owner key (intersection size). Stacks
    /// with min-overlap / Jaccard when `> 0`.
    #[serde(default)]
    pub w_shared_safe_owner_count: f64,
    pub w_shared_ens_handle: f64,
    pub w_shared_funder: f64,
    /// Inside `ln(offset + fan_out)` when down-weighting busy funders.
    pub funder_fanout_ln_offset: f64,
    /// For mapping score → display probability via a logistic curve.
    pub link_probability_mid: f64,
    pub link_probability_scale: f64,
}

impl Default for LinkageParams {
    fn default() -> Self {
        Self {
            t_high: 6.0,
            t_low: 2.0,
            w_did_controller: 12.0,
            w_safe_owner_jaccard: 0.0,
            w_safe_owner_min_overlap: 8.0,
            w_shared_safe_owner_count: 0.0,
            w_shared_ens_handle: 5.0,
            w_shared_funder: 3.0,
            funder_fanout_ln_offset: 2.0,
            link_probability_mid: 4.0,
            link_probability_scale: 2.0,
        }
    }
}

impl LinkageParams {
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let p: Self = serde_json::from_slice(bytes).context("parse linkage params JSON")?;
        Ok(p)
    }

    pub fn from_json_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read linkage params file {}", path.display()))?;
        Self::from_json_slice(&bytes)
    }

    pub fn bundled_default() -> Result<Self> {
        const BYTES: &[u8] = include_bytes!("../../data/linkage_params.default.json");
        Self::from_json_slice(BYTES)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkTier {
    Accepted,
    Uncertain,
    Rejected,
}

impl LinkTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Uncertain => "uncertain",
            Self::Rejected => "rejected",
        }
    }
}

/// Raw counts / overlaps for one unordered address pair.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PairwiseFeatures {
    pub shared_did_controller_keys: usize,
    pub shared_safe_owner_keys: usize,
    pub safe_owner_union: usize,
    /// `|A ∩ B| / |A ∪ B|` on `safe_owner` keys.
    pub safe_owner_jaccard: f64,
    /// `|A ∩ B| / max(1, min(|A|, |B|))` — asymmetric overlap for hub vs
    /// smaller multisig (shared control, not generic set similarity).
    pub safe_owner_min_overlap: f64,
    pub safe_owner_count_a: usize,
    pub safe_owner_count_b: usize,
    pub shared_ens_handle_keys: usize,
    pub shared_funder_keys: usize,
    /// Lexicographically sorted shared `funded_by` keys (non-service only).
    pub shared_funder_keys_list: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairwiseScore {
    pub address_a: String,
    pub address_b: String,
    pub features: PairwiseFeatures,
    pub contributions: BTreeMap<String, f64>,
    pub score: f64,
    pub link_probability: f64,
    pub tier: LinkTier,
    /// True when `shared_did_controller_keys > 0` (STRONG channel).
    pub deterministic_anchor: bool,
    /// True when at least one `funded_by` / `ens_handle` / `safe_owner`
    /// channel contributed positively to the score.
    pub has_structural_support: bool,
}

fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Map score to a display-friendly probability using a logistic around the
/// mid-point between the two thresholds by default.
pub fn score_to_link_probability(score: f64, params: &LinkageParams) -> f64 {
    let x = (score - params.link_probability_mid) / params.link_probability_scale.max(1e-6);
    logistic(x).clamp(0.0, 1.0)
}

pub fn fanout_table(attestations: &[Attestation]) -> HashMap<(EvidenceKind, String), usize> {
    let mut m: HashMap<(EvidenceKind, String), usize> = HashMap::new();
    for a in attestations {
        *m.entry((a.kind, a.key.to_lowercase())).or_insert(0) += 1;
    }
    m
}

fn service_keys(
    fanout: &HashMap<(EvidenceKind, String), usize>,
) -> HashSet<(EvidenceKind, String)> {
    fanout
        .iter()
        .filter(|(_, &c)| c > FAN_OUT_CAP)
        .map(|(k, _)| k.clone())
        .collect()
}

fn keys_of_kind(
    addr: &str,
    kind: EvidenceKind,
    attestations: &[Attestation],
    service: &HashSet<(EvidenceKind, String)>,
) -> HashSet<String> {
    attestations
        .iter()
        .filter(|a| a.address == addr && a.kind == kind)
        .filter(|a| !service.contains(&(a.kind, a.key.to_lowercase())))
        .map(|a| a.key.to_lowercase())
        .collect()
}

pub fn pairwise_features(
    address_a: &str,
    address_b: &str,
    attestations: &[Attestation],
    fanout: &HashMap<(EvidenceKind, String), usize>,
) -> PairwiseFeatures {
    let service = service_keys(fanout);
    let dc_a = keys_of_kind(
        address_a,
        EvidenceKind::DidController,
        attestations,
        &service,
    );
    let dc_b = keys_of_kind(
        address_b,
        EvidenceKind::DidController,
        attestations,
        &service,
    );
    let shared_dc = dc_a.intersection(&dc_b).count();

    let so_a = keys_of_kind(address_a, EvidenceKind::SafeOwner, attestations, &service);
    let so_b = keys_of_kind(address_b, EvidenceKind::SafeOwner, attestations, &service);
    let inter_so = so_a.intersection(&so_b).count();
    let union_so = so_a.union(&so_b).count();
    let jaccard = if union_so == 0 {
        0.0
    } else {
        inter_so as f64 / union_so as f64
    };
    let na = so_a.len();
    let nb = so_b.len();
    let min_side = na.min(nb).max(1);
    let min_overlap = inter_so as f64 / min_side as f64;

    let ens_a = keys_of_kind(address_a, EvidenceKind::EnsHandle, attestations, &service);
    let ens_b = keys_of_kind(address_b, EvidenceKind::EnsHandle, attestations, &service);
    let shared_ens = ens_a.intersection(&ens_b).count();

    let fb_a = keys_of_kind(address_a, EvidenceKind::FundedBy, attestations, &service);
    let fb_b = keys_of_kind(address_b, EvidenceKind::FundedBy, attestations, &service);
    let mut shared_fb: Vec<String> = fb_a.intersection(&fb_b).cloned().collect();
    shared_fb.sort();

    PairwiseFeatures {
        shared_did_controller_keys: shared_dc,
        shared_safe_owner_keys: inter_so,
        safe_owner_union: union_so,
        safe_owner_jaccard: jaccard,
        safe_owner_min_overlap: min_overlap,
        safe_owner_count_a: na,
        safe_owner_count_b: nb,
        shared_ens_handle_keys: shared_ens,
        shared_funder_keys: shared_fb.len(),
        shared_funder_keys_list: shared_fb,
    }
}

pub fn score_pair(
    address_a: &str,
    address_b: &str,
    features: &PairwiseFeatures,
    fanout: &HashMap<(EvidenceKind, String), usize>,
    params: &LinkageParams,
) -> PairwiseScore {
    let mut contributions = BTreeMap::new();
    let mut score = 0.0_f64;

    let deterministic_anchor = features.shared_did_controller_keys > 0;
    if deterministic_anchor {
        let w = params.w_did_controller * features.shared_did_controller_keys as f64;
        contributions.insert("did_controller_shared".to_string(), w);
        score += w;
    }

    if params.w_safe_owner_min_overlap > 0.0 && features.safe_owner_min_overlap > 0.0 {
        let w = params.w_safe_owner_min_overlap * features.safe_owner_min_overlap;
        contributions.insert("safe_owner_min_overlap".to_string(), w);
        score += w;
    } else if params.w_safe_owner_jaccard > 0.0 && features.safe_owner_jaccard > 0.0 {
        let w = params.w_safe_owner_jaccard * features.safe_owner_jaccard;
        contributions.insert("safe_owner_jaccard".to_string(), w);
        score += w;
    }

    if params.w_shared_safe_owner_count > 0.0 && features.shared_safe_owner_keys > 0 {
        let w = params.w_shared_safe_owner_count * features.shared_safe_owner_keys as f64;
        contributions.insert("shared_safe_owner_count".to_string(), w);
        score += w;
    }

    if features.shared_ens_handle_keys > 0 {
        let w = params.w_shared_ens_handle;
        contributions.insert("ens_handle_shared".to_string(), w);
        score += w;
    }

    if !features.shared_funder_keys_list.is_empty() {
        let off = params.funder_fanout_ln_offset.max(1e-6);
        let mut funder_total = 0.0_f64;
        for key in &features.shared_funder_keys_list {
            let n = fanout
                .get(&(EvidenceKind::FundedBy, key.clone()))
                .copied()
                .unwrap_or(1) as f64;
            let denom = (off + n).ln().max(1e-6);
            funder_total += params.w_shared_funder / denom;
        }
        contributions.insert("funded_by_shared_downweighted".to_string(), funder_total);
        score += funder_total;
    }

    let has_structural_support = deterministic_anchor
        || features.shared_funder_keys > 0
        || features.shared_ens_handle_keys > 0
        || features.shared_safe_owner_keys > 0;

    let link_probability = score_to_link_probability(score, params);

    let tier = if deterministic_anchor || (score >= params.t_high && has_structural_support) {
        LinkTier::Accepted
    } else if score > params.t_low {
        LinkTier::Uncertain
    } else {
        LinkTier::Rejected
    };

    PairwiseScore {
        address_a: address_a.to_string(),
        address_b: address_b.to_string(),
        features: features.clone(),
        contributions,
        score,
        link_probability,
        tier,
        deterministic_anchor,
        has_structural_support,
    }
}

/// Candidate pairs that share at least one non-service `(kind, key)` with
/// run-level fan-out ≤ [`FAN_OUT_CAP`], capped for tractability.
pub fn candidate_address_pairs(
    addresses: &[String],
    attestations: &[Attestation],
    max_pairs: usize,
) -> Vec<(String, String)> {
    let fanout = fanout_table(attestations);
    let service = service_keys(&fanout);

    let mut by_key: HashMap<(EvidenceKind, String), Vec<String>> = HashMap::new();
    for a in attestations {
        let k = (a.kind, a.key.to_lowercase());
        if service.contains(&k) {
            continue;
        }
        if fanout.get(&k).copied().unwrap_or(0) <= 1 {
            continue;
        }
        by_key.entry(k).or_default().push(a.address.to_lowercase());
    }

    for addrs in by_key.values_mut() {
        addrs.sort();
        addrs.dedup();
    }

    let addr_set: HashSet<String> = addresses.iter().map(|s| s.to_lowercase()).collect();
    let mut pairs: BTreeMap<(String, String), ()> = BTreeMap::new();

    for addrs in by_key.values() {
        if addrs.len() < 2 {
            continue;
        }
        for i in 0..addrs.len() {
            for j in (i + 1)..addrs.len() {
                let x = &addrs[i];
                let y = &addrs[j];
                if !addr_set.contains(x) || !addr_set.contains(y) {
                    continue;
                }
                let (a, b) = if x < y {
                    (x.clone(), y.clone())
                } else {
                    (y.clone(), x.clone())
                };
                pairs.insert((a, b), ());
                if pairs.len() >= max_pairs {
                    return pairs.into_keys().collect();
                }
            }
        }
    }

    pairs.into_keys().collect()
}

pub fn score_address_pairs(
    pairs: &[(String, String)],
    attestations: &[Attestation],
    params: &LinkageParams,
) -> Vec<PairwiseScore> {
    let fanout = fanout_table(attestations);
    let mut out = Vec::with_capacity(pairs.len());
    for (a, b) in pairs {
        let f = pairwise_features(a, b, attestations, &fanout);
        out.push(score_pair(a, b, &f, &fanout, params));
    }
    out.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.address_a.cmp(&y.address_a))
            .then_with(|| x.address_b.cmp(&y.address_b))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Strength;

    fn att(addr: &str, kind: EvidenceKind, key: &str) -> Attestation {
        Attestation {
            address: addr.to_string(),
            kind,
            key: key.to_string(),
            strength: Strength::Medium,
            source: "test".to_string(),
            observed_block: 1,
            payload_json: None,
        }
    }

    #[test]
    fn deterministic_anchor_overrides_thresholds() {
        let a = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let ckey = "0xctrl0000000000000000000000000000000000";
        let atts = vec![
            att(a, EvidenceKind::DidController, ckey),
            att(b, EvidenceKind::DidController, ckey),
        ];
        let fanout = fanout_table(&atts);
        let f = pairwise_features(a, b, &atts, &fanout);
        let params = LinkageParams {
            t_high: 1.0e9,
            t_low: 1.0e9,
            ..Default::default()
        };
        let s = score_pair(a, b, &f, &fanout, &params);
        assert!(s.deterministic_anchor);
        assert_eq!(s.tier, LinkTier::Accepted);
    }

    #[test]
    fn min_overlap_and_funder_boost_score() {
        let s1 = "0x0000000000000000000000000000000000000aa1";
        let s2 = "0x0000000000000000000000000000000000000aa2";
        let o = "0x00000000000000000000000000000000000000e0";
        let funder = "0x0000000000000000000000000000000000000f00";
        let atts = vec![
            att(s1, EvidenceKind::SafeOwner, o),
            att(s2, EvidenceKind::SafeOwner, o),
            att(s1, EvidenceKind::FundedBy, funder),
            att(s2, EvidenceKind::FundedBy, funder),
        ];
        let fanout = fanout_table(&atts);
        let f = pairwise_features(s1, s2, &atts, &fanout);
        let params = LinkageParams::default();
        let sc = score_pair(s1, s2, &f, &fanout, &params);
        assert!(sc.score > 0.0);
        assert!(sc.features.safe_owner_min_overlap > 0.0);
        assert!(sc.contributions.contains_key("safe_owner_min_overlap"));
    }

    #[test]
    fn treasury_hub_vs_committee_min_overlap_is_full() {
        let treasury = "0x0000000000000000000000000000000000000t01";
        let committee = "0x0000000000000000000000000000000000000c01";
        let o1 = "0x0000000000000000000000000000000000000001";
        let o2 = "0x0000000000000000000000000000000000000002";
        let o3 = "0x0000000000000000000000000000000000000003";
        let o4 = "0x0000000000000000000000000000000000000004";
        let o5 = "0x0000000000000000000000000000000000000005";
        let mut atts = Vec::new();
        for o in [o1, o2, o3, o4, o5] {
            atts.push(att(treasury, EvidenceKind::SafeOwner, o));
        }
        for o in [o1, o2, o3] {
            atts.push(att(committee, EvidenceKind::SafeOwner, o));
        }
        let fanout = fanout_table(&atts);
        let f = pairwise_features(treasury, committee, &atts, &fanout);
        assert_eq!(f.shared_safe_owner_keys, 3);
        assert!((f.safe_owner_jaccard - 0.6).abs() < 1e-9, "jaccard=3/5");
        assert!(
            (f.safe_owner_min_overlap - 1.0).abs() < 1e-9,
            "min-overlap=3/3"
        );
        let params = LinkageParams::default();
        let sc = score_pair(treasury, committee, &f, &fanout, &params);
        assert_eq!(sc.tier, LinkTier::Accepted);
        assert!(sc.score >= params.t_high);
    }

    #[test]
    fn legacy_jaccard_only_params_still_parse() {
        let json = br#"{"t_high":6,"t_low":2,"w_did_controller":12,"w_safe_owner_jaccard":8,"w_shared_ens_handle":5,"w_shared_funder":3,"funder_fanout_ln_offset":2,"link_probability_mid":4,"link_probability_scale":2}"#;
        let p = LinkageParams::from_json_slice(json).unwrap();
        assert_eq!(p.w_safe_owner_min_overlap, 0.0);
        assert_eq!(p.w_shared_safe_owner_count, 0.0);
        assert_eq!(p.w_safe_owner_jaccard, 8.0);
    }

    #[test]
    fn bundled_default_matches_struct_default() {
        let bundled = LinkageParams::bundled_default().expect("bundled JSON");
        let dflt = LinkageParams::default();
        assert_eq!(bundled.t_high, dflt.t_high);
        assert_eq!(bundled.t_low, dflt.t_low);
        assert_eq!(bundled.w_did_controller, dflt.w_did_controller);
        assert_eq!(bundled.w_safe_owner_jaccard, dflt.w_safe_owner_jaccard);
        assert_eq!(
            bundled.w_safe_owner_min_overlap,
            dflt.w_safe_owner_min_overlap
        );
        assert_eq!(
            bundled.w_shared_safe_owner_count,
            dflt.w_shared_safe_owner_count
        );
        assert_eq!(bundled.w_shared_ens_handle, dflt.w_shared_ens_handle);
        assert_eq!(bundled.w_shared_funder, dflt.w_shared_funder);
        assert_eq!(
            bundled.funder_fanout_ln_offset,
            dflt.funder_fanout_ln_offset
        );
        assert_eq!(bundled.link_probability_mid, dflt.link_probability_mid);
        assert_eq!(bundled.link_probability_scale, dflt.link_probability_scale);
    }

    #[test]
    fn from_json_slice_rejects_invalid_json() {
        let err = LinkageParams::from_json_slice(b"not-json").unwrap_err();
        assert!(err.to_string().contains("parse linkage params"));
    }

    #[test]
    fn score_to_link_probability_clamps_to_unit_interval() {
        let p = LinkageParams::default();
        assert!((score_to_link_probability(-1.0e9, &p) - 0.0).abs() < 1e-9);
        assert!((score_to_link_probability(1.0e9, &p) - 1.0).abs() < 1e-9);
        let mid = score_to_link_probability(p.link_probability_mid, &p);
        assert!(
            mid > 0.45 && mid < 0.55,
            "logistic at midpoint ~0.5, got {mid}"
        );
    }

    #[test]
    fn link_tier_as_str_roundtrip() {
        assert_eq!(LinkTier::Accepted.as_str(), "accepted");
        assert_eq!(LinkTier::Uncertain.as_str(), "uncertain");
        assert_eq!(LinkTier::Rejected.as_str(), "rejected");
    }

    #[test]
    fn candidate_address_pairs_emits_shared_funder_pair() {
        let a = "0x0000000000000000000000000000000000000aa1";
        let b = "0x0000000000000000000000000000000000000aa2";
        let funder = "0x0000000000000000000000000000000000000f00";
        let addresses = vec![a.to_string(), b.to_string()];
        let atts = vec![
            att(a, EvidenceKind::FundedBy, funder),
            att(b, EvidenceKind::FundedBy, funder),
        ];
        let pairs = candidate_address_pairs(&addresses, &atts, 100);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0.to_lowercase(), a.to_lowercase());
        assert_eq!(pairs[0].1.to_lowercase(), b.to_lowercase());
    }

    #[test]
    fn score_address_pairs_sorts_deterministically() {
        let a = "0x0000000000000000000000000000000000000aa1";
        let b = "0x0000000000000000000000000000000000000aa2";
        let c = "0x0000000000000000000000000000000000000aa3";
        let funder = "0x0000000000000000000000000000000000000f00";
        let atts = vec![
            att(a, EvidenceKind::FundedBy, funder),
            att(b, EvidenceKind::FundedBy, funder),
            att(c, EvidenceKind::FundedBy, funder),
        ];
        let pairs = vec![
            (b.to_string(), c.to_string()),
            (a.to_string(), b.to_string()),
        ];
        let params = LinkageParams::default();
        let scored = score_address_pairs(&pairs, &atts, &params);
        assert_eq!(scored.len(), 2);
        assert!(scored[0].score >= scored[1].score);
        assert!(scored[0].address_a < scored[0].address_b);
    }
}
