//! Gold pair labels for control-cluster evaluation.
//!
//! CSV columns: `address_a`, `address_b`, `label`, `rationale`
//!
//! `label` is one of: `same_control`, `different_control`, `uncertain`

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldLabel {
    SameControl,
    DifferentControl,
    Uncertain,
}

impl GoldLabel {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "same_control" => Ok(Self::SameControl),
            "different_control" => Ok(Self::DifferentControl),
            "uncertain" => Ok(Self::Uncertain),
            other => Err(anyhow!("unknown gold label: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameControl => "same_control",
            Self::DifferentControl => "different_control",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoldPair {
    pub address_a: String,
    pub address_b: String,
    pub label: GoldLabel,
    pub rationale: String,
}

pub fn normalize_eth_address(addr: &str) -> Result<String> {
    let trimmed = addr.trim();
    if !trimmed.starts_with("0x") || trimmed.len() != 42 {
        return Err(anyhow!(
            "address must be 0x-prefixed 40 hex chars: {trimmed}"
        ));
    }
    let hex = &trimmed[2..];
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("address has non-hex: {trimmed}"));
    }
    Ok(trimmed.to_lowercase())
}

/// Load gold pairs from a UTF-8 CSV with header row.
pub fn load_gold_pairs(path: &Path) -> Result<Vec<GoldPair>> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("read gold file {}", path.display()))?;
    let mut rdr = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(data.as_bytes());

    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.context("csv record")?;
        let address_a = normalize_eth_address(rec.get(0).unwrap_or(""))?;
        let address_b = normalize_eth_address(rec.get(1).unwrap_or(""))?;
        let label = GoldLabel::parse(rec.get(2).unwrap_or(""))?;
        let rationale = rec.get(3).unwrap_or("").to_string();
        if address_a == address_b {
            continue;
        }
        let (a, b) = if address_a <= address_b {
            (address_a, address_b)
        } else {
            (address_b, address_a)
        };
        out.push(GoldPair {
            address_a: a,
            address_b: b,
            label,
            rationale,
        });
    }
    Ok(out)
}

pub fn union_addresses(gold: &[GoldPair]) -> Vec<String> {
    let mut s: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for g in gold {
        s.insert(g.address_a.clone());
        s.insert(g.address_b.clone());
    }
    s.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gold_label_parse_and_as_str() {
        assert_eq!(
            GoldLabel::parse(" SAME_CONTROL ").unwrap(),
            GoldLabel::SameControl
        );
        assert_eq!(
            GoldLabel::parse("different_control").unwrap(),
            GoldLabel::DifferentControl
        );
        assert_eq!(GoldLabel::parse("uncertain").unwrap(), GoldLabel::Uncertain);
        assert!(GoldLabel::parse("nope")
            .unwrap_err()
            .to_string()
            .contains("unknown gold"));
        assert_eq!(GoldLabel::SameControl.as_str(), "same_control");
    }

    #[test]
    fn normalize_eth_address_errors() {
        assert!(normalize_eth_address("0xabc")
            .unwrap_err()
            .to_string()
            .contains("40 hex"));
        assert!(
            normalize_eth_address("0xgggggggggggggggggggggggggggggggggggggggg")
                .unwrap_err()
                .to_string()
                .contains("non-hex")
        );
    }

    #[test]
    fn load_gold_pairs_sorts_and_skips_self_pairs() {
        let path = std::env::temp_dir().join(format!(
            "unmasking_did_gold_sort_{}.csv",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "address_a,address_b,label,rationale\n\
             0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,same_control,r1\n\
             0xcccccccccccccccccccccccccccccccccccccccc,0xcccccccccccccccccccccccccccccccccccccccc,same_control,skip-self\n\
             0xdddddddddddddddddddddddddddddddddddddddd,0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee,different_control,r2\n",
        )
        .unwrap();
        let pairs = load_gold_pairs(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            pairs[0].address_a,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            pairs[0].address_b,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(pairs[1].label, GoldLabel::DifferentControl);
    }

    #[test]
    fn union_addresses_is_sorted_unique() {
        let gold = vec![
            GoldPair {
                address_a: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                address_b: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                label: GoldLabel::Uncertain,
                rationale: String::new(),
            },
            GoldPair {
                address_a: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                address_b: "0xcccccccccccccccccccccccccccccccccccccccc".to_string(),
                label: GoldLabel::Uncertain,
                rationale: String::new(),
            },
        ];
        let u = union_addresses(&gold);
        assert_eq!(
            u,
            vec![
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                "0xcccccccccccccccccccccccccccccccccccccccc".to_string(),
            ]
        );
    }

    #[test]
    fn parses_csv_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "unmasking_did_gold_test_{}.csv",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "address_a,address_b,label,rationale\n0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,same_control,test\n",
        )
        .unwrap();
        let pairs = load_gold_pairs(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].label, GoldLabel::SameControl);
    }
}
