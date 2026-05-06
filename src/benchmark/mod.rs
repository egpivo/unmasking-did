use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

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
        if self.entity_count == 0 {
            bail!("entity_count must be > 0");
        }
        if self.wallets_per_entity == 0 {
            bail!("wallets_per_entity must be > 0");
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
        let mut prng = DeterministicPrng::new(self.seed ^ stable_hash64(&self.spec.scenario_id));
        let mut wallets = Vec::with_capacity(self.spec.entity_count * self.spec.wallets_per_entity);

        for entity_ix in 0..self.spec.entity_count {
            let entity_id = deterministic_entity_id(entity_ix);
            let cohort = assign_cohort(entity_ix, &self.spec, &mut prng);
            for wallet_ix in 0..self.spec.wallets_per_entity {
                wallets.push(GroundTruthWallet {
                    entity_id: entity_id.clone(),
                    wallet_id: deterministic_wallet_id(entity_ix, wallet_ix),
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
}

fn assign_cohort(entity_ix: usize, spec: &ScenarioSpec, prng: &mut DeterministicPrng) -> Cohort {
    let deterministic_tie_break = ((entity_ix as u64) ^ prng.next_u64()) as f64 / u64::MAX as f64;
    if deterministic_tie_break < spec.governance_ratio {
        Cohort::Governance
    } else if deterministic_tie_break < (spec.governance_ratio + spec.control_ratio) {
        Cohort::Control
    } else {
        Cohort::NegativeControl
    }
}

pub fn deterministic_entity_id(entity_ix: usize) -> String {
    format!("ent_{entity_ix:05}")
}

pub fn deterministic_wallet_id(entity_ix: usize, wallet_ix: usize) -> String {
    let h = stable_hash64(&format!("entity:{entity_ix}:wallet:{wallet_ix}"));
    format!("0x{h:040x}")
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

#[derive(Debug, Clone)]
struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    fn new(seed: u64) -> Self {
        // Avoid all-zero stream state.
        let init = if seed == 0 { 0x9e3779b97f4a7c15 } else { seed };
        Self { state: init }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn deterministic_wallet_ids_are_stable_and_hex_like() {
        let w = deterministic_wallet_id(12, 7);
        assert!(w.starts_with("0x"));
        assert_eq!(w.len(), 42);
        assert_eq!(w, deterministic_wallet_id(12, 7));
    }
}
