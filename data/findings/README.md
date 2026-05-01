# findings/

Each Markdown file in this directory is a **report artifact** generated
by `cargo run -- report` against a specific input set, on a specific
chain, at a specific run. They are committed to the repo so a reader of
the project can see the actual outputs the tool has produced — not just
synthetic snippets in the README.

## Naming convention

```
data/findings/YYYY-MM-DD-<chain>-<short-slug>.md
```

Examples:

- `2026-05-01-scroll-smoke.md`
- `2026-05-15-scroll-dao-treasury-safes.md`
- `2026-06-02-mainnet-airdrop-sybil-cluster.md`

## What goes in a finding

A finding is the **direct output** of `cargo run -- report` plus a
short prose preface. The Markdown report itself already contains:

- the run id (links into `clustering_runs` / `entity_clusters` for audit)
- input parameters (`min_evidence`, `fan_out_cap`)
- the cluster structure with shared evidence keys
- suspected service keys (fan-out-cap hits)
- a reproducibility footer

The preface should add:

- the **input address set** that produced the report
- the **chain** and the `ALCHEMY_BASE_URL` / `SAFE_TX_SERVICE_URL`
  used (for reproducibility on the right network)
- a one-paragraph reading: which clusters were expected, which were
  surprising, which look like false positives or misses
- known caveats — particularly when running on an L2 where ENS or
  the CEX blacklist provide weaker coverage than on Ethereum mainnet

## What does NOT go in here

- raw SQLite databases (gitignored at the parent `data/*` level)
- credentials of any kind
- private personal data — this project is about linking public Web3
  identifiers, not naming real-world humans
