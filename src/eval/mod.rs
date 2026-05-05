//! Evaluation harness: gold pair labels × evidence ablations × metrics.
//!
//! **Task (documented contract):** given a fixed set of Web3 identifiers and
//! hand-labeled address pairs (`same_control` / `different_control` /
//! `uncertain`), measure whether the current evidence channels recover known
//! control structure — **not** “same human,” same **control cluster**.
//!
//! CSV format (`address_a`, `address_b`, `label`, `rationale`) is parsed by
//! [`gold::load_gold_pairs`]. Run:
//!
//! ```text
//! cargo run -- eval --gold data/eval/my_pairs.csv
//! ```
//!
//! Curated gold sets (pick one that matches what you ingested):
//!
//! - **`data/eval/gold_pairs.scroll_dao.csv`** — six Scroll DAO Safes only,
//!   C(6,2) pairs (provenance: `data/findings/2026-05-01-scroll-dao-safes.md`).
//! - **`data/eval/gold_pairs.scroll_bounded_v1.csv`** — same six Safes plus
//!   three documented shared-signer EOAs (full C(9,2) grid). Seeds:
//!   `data/eval/seeds_scroll_v1.txt`.
//! - **`data/eval/gold_pairs.mainnet_bounded_v1.csv`** — twenty Ethereum
//!   mainnet governance/protocol contracts across five curated clusters; use
//!   only with a DB ingested at **mainnet** RPC + mainnet Safe Tx Service.
//!   Seeds: `data/eval/seeds_mainnet_v1.txt`. Provenance: `data/eval/SOURCES.txt`.
//!
//! Scroll (534352) and Ethereum mainnet (1) must not be mixed in one SQLite
//! run unless your ingest config matches the chain for every address.

pub mod ablation;
pub mod gold;
pub mod run;

pub use ablation::AblationMode;
pub use gold::{load_gold_pairs, GoldLabel, GoldPair};
pub use run::{run_ablation, run_eval_suite, AblationReport, EvalSuiteReport};
