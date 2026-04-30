use anyhow::Result;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::storage::Repo;

use super::union_find::UnionFind;

/// Service / hot-wallet addresses that should not be treated as evidence
/// of shared control. Funding from these is uninformative because they
/// fan out to thousands or millions of unrelated users.
///
/// All addresses MUST be lowercase to match storage normalization.
const CEX_BLACKLIST: &[&str] = &[
    // Binance hot wallets
    "0x28c6c06298d514db089934071355e5743bf21d60",
    "0x21a31ee1afc51d94c2efccaa2092ad1028285549",
    "0xdfd5293d8e347dfe59e90efd55b2956a1343963d",
    "0x56eddb7aa87536c09ccc2793473599fd21a8b17f",
    "0x9696f59e4d72e237be84ffd425dcad154bf96976",
    // Coinbase
    "0x71660c4005ba85c37ccec55d0c4493e66fe775d3",
    "0x503828976d22510aad0201ac7ec88293211d23da",
    "0xa090e606e30bd747d4e6245a1517ebe430f0057e",
    "0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740",
    "0x3cd751e6b0078be393132286c442345e5dc49699",
    // Kraken
    "0x267be1c1d684f78cb4f6a176c4911b741e4ffdc0",
    "0x2910543af39aba0cd09dbb2d50200b3e800a63d2",
    "0xe853c56864a2ebe4576a807d26fdc4a0ada51919",
    "0x53d284357ec70ce289d6d64134dfac8e511c8a3d",
];

pub fn cex_blacklist() -> HashSet<String> {
    CEX_BLACKLIST.iter().map(|s| s.to_string()).collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterReport {
    pub cluster_id: usize,
    pub addresses: Vec<String>,
    pub shared_funders: Vec<String>,
}

/// Build a union-find of `addresses` where two addresses are merged when
/// they share at least `min_evidence` distinct non-CEX funders. Returns
/// the resulting clusters along with the funders that justified the merge.
pub async fn cluster_by_funding(
    repo: &Repo,
    addresses: &[String],
    min_evidence: usize,
) -> Result<Vec<ClusterReport>> {
    let blacklist = cex_blacklist();

    let mut funders_by_addr: HashMap<String, HashSet<String>> = HashMap::new();
    for addr in addresses {
        let normalized = addr.to_lowercase();
        let funders = repo
            .incoming_funders(&normalized)
            .await?
            .into_iter()
            .map(|(f, _)| f.to_lowercase())
            .filter(|f| !blacklist.contains(f))
            .collect::<HashSet<_>>();
        funders_by_addr.insert(normalized, funders);
    }

    let mut uf: UnionFind<String> = UnionFind::new();
    for addr in addresses {
        uf.add(addr.to_lowercase());
    }

    let normalized: Vec<String> = addresses.iter().map(|a| a.to_lowercase()).collect();

    let mut shared_by_pair: HashMap<(String, String), Vec<String>> = HashMap::new();
    for (i, a) in normalized.iter().enumerate() {
        for b in normalized.iter().skip(i + 1) {
            let fa = &funders_by_addr[a];
            let fb = &funders_by_addr[b];
            let shared: Vec<String> = fa.intersection(fb).cloned().collect();
            if shared.len() >= min_evidence.max(1) {
                uf.union(a, b);
                let key = if a < b {
                    (a.clone(), b.clone())
                } else {
                    (b.clone(), a.clone())
                };
                shared_by_pair.insert(key, shared);
            }
        }
    }

    let components = uf.components();
    let mut reports: Vec<ClusterReport> = components
        .into_iter()
        .enumerate()
        .map(|(cluster_id, mut addresses)| {
            addresses.sort();
            let shared_funders =
                collect_shared_funders(&addresses, &funders_by_addr, &shared_by_pair);
            ClusterReport {
                cluster_id,
                addresses,
                shared_funders,
            }
        })
        .collect();

    reports.sort_by(|x, y| y.addresses.len().cmp(&x.addresses.len()));
    for (i, r) in reports.iter_mut().enumerate() {
        r.cluster_id = i;
    }
    Ok(reports)
}

fn collect_shared_funders(
    cluster: &[String],
    funders_by_addr: &HashMap<String, HashSet<String>>,
    shared_by_pair: &HashMap<(String, String), Vec<String>>,
) -> Vec<String> {
    if cluster.len() < 2 {
        return Vec::new();
    }
    let mut all: HashSet<String> = HashSet::new();
    for (i, a) in cluster.iter().enumerate() {
        for b in cluster.iter().skip(i + 1) {
            let key = if a < b {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            };
            if let Some(funders) = shared_by_pair.get(&key) {
                for f in funders {
                    all.insert(f.clone());
                }
            }
        }
    }
    let _ = funders_by_addr;
    let mut v: Vec<String> = all.into_iter().collect();
    v.sort();
    v
}
