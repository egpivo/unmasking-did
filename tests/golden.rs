//! Golden snapshot of M1 clustering output.
//!
//! Locks the public behavior of `cluster_by_funding` against a fixed
//! fixture so the M1 → evidence-table refactor cannot silently change
//! which addresses end up in which cluster, or which funders justify a
//! merge. Compares CONTENT, not the integer `cluster_id` (which is
//! about to switch from auto-increment to `min(address_in_cluster)`).

use serde::{Deserialize, Serialize};
use unmasking_did::alchemy::Transfer;
use unmasking_did::linking::{cluster_by_funding, ClusterReport};
use unmasking_did::storage::{connect, run_migrations, Repo};

const ALICE: &str = "0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const BOB: &str = "0xb2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
const CAROL: &str = "0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";
const DAVE: &str = "0xd4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4";
const EVE: &str = "0xe5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5";
const FUNDER1: &str = "0xf1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
const FUNDER2: &str = "0xf2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2";
const FUNDER3: &str = "0xf3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3";
const FUNDER4: &str = "0xf4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4";
const BINANCE: &str = "0x28c6c06298d514db089934071355e5743bf21d60";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CanonicalCluster {
    addresses: Vec<String>,
    shared_funders: Vec<String>,
}

fn canonicalize(reports: &[ClusterReport]) -> Vec<CanonicalCluster> {
    let mut v: Vec<CanonicalCluster> = reports
        .iter()
        .map(|r| {
            let mut addresses = r.addresses.clone();
            addresses.sort();
            let mut shared_funders = r.shared_funders.clone();
            shared_funders.sort();
            CanonicalCluster {
                addresses,
                shared_funders,
            }
        })
        .collect();
    v.sort_by(|a, b| a.addresses.cmp(&b.addresses));
    v
}

fn t(from: &str, to: &str, block: i64, tx: &str) -> Transfer {
    Transfer {
        from_addr: from.to_string(),
        to_addr: to.to_string(),
        value: Some("1".to_string()),
        block_num: Some(block),
        tx_hash: Some(tx.to_string()),
        asset: Some("ETH".to_string()),
    }
}

#[tokio::test]
async fn golden_m1_clustering() {
    // Scenario:
    //   alice ⇄ bob   share two non-CEX funders (F1, F2) — should merge
    //   carol ⇄ dave  share one non-CEX funder  (F3)     — should merge
    //   eve           lonely with F4                       — singleton
    //   bob, carol    additionally both funded by Binance — must NOT merge
    //                 (CEX is in the M1 blacklist and contributes 0 evidence)
    let pool = connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let repo = Repo::new(pool);

    let transfers = [
        t(FUNDER1, ALICE, 100, "0x01"),
        t(FUNDER1, BOB, 101, "0x02"),
        t(FUNDER2, ALICE, 102, "0x03"),
        t(FUNDER2, BOB, 103, "0x04"),
        t(FUNDER3, CAROL, 104, "0x05"),
        t(FUNDER3, DAVE, 105, "0x06"),
        t(FUNDER4, EVE, 106, "0x07"),
        t(BINANCE, BOB, 107, "0x08"),
        t(BINANCE, CAROL, 108, "0x09"),
    ];
    for tr in &transfers {
        repo.insert_transfer(tr).await.unwrap();
    }
    for a in [ALICE, BOB, CAROL, DAVE, EVE] {
        repo.upsert_address(a, None).await.unwrap();
    }

    let reports = cluster_by_funding(
        &repo,
        &[
            ALICE.into(),
            BOB.into(),
            CAROL.into(),
            DAVE.into(),
            EVE.into(),
        ],
        1,
    )
    .await
    .unwrap();

    let actual = canonicalize(&reports);
    let expected = load_fixture();

    if actual != expected {
        let actual_json = serde_json::to_string_pretty(&actual).unwrap();
        panic!(
            "golden_m1_clustering mismatch.\n\
             expected (tests/fixtures/golden_m1.json):\n{}\n\n\
             actual:\n{}\n",
            serde_json::to_string_pretty(&expected).unwrap(),
            actual_json
        );
    }
}

fn load_fixture() -> Vec<CanonicalCluster> {
    let path = format!(
        "{}/tests/fixtures/golden_m1.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse fixture {path}: {e}"))
}
