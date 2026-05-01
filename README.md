# unmasking-did

> Measuring the gap between decentralized identifiers and decentralized entities on Ethereum.

![status](https://img.shields.io/badge/status-early%20research%2C%20not%20production-orange)

## Why

Decentralized identifier counts and decentralized entity counts are not the
same thing. A protocol can advertise tens of thousands of distinct DIDs,
wallets, or credential subjects while the controlling entities behind them
number in the dozens. `unmasking-did` quantifies that gap by linking
fragmented public identity evidence — wallets, DID documents, controllers,
funding flows — into an explainable identity graph, then reporting cluster-
level decentralization metrics.

The goal is **auditable entity linking**, not deanonymization. Every merged
cluster carries the evidence trail that justified the merge.

## Architecture

```
            +--------------------+   +-------------------+   +-----------------+
            |  Alchemy JSON-RPC  |   |   ENS subgraph    |   |  Safe Tx API    |
            +---------+----------+   +---------+---------+   +--------+--------+
                      |                        |                      |
                      v                        v                      v
                          +--------------------------------------+
                          |        local SQLite cache            |
                          |  addresses / transfers / ens / ...   |
                          +-------------------+------------------+
                                              |
                                              v
                                  +-----------------------+
                                  |  feature extraction   |
                                  |  (funding, ENS, Safe) |
                                  +-----------+-----------+
                                              |
                                              v
                                  +-----------------------+
                                  |  union-find clustering|
                                  +-----------+-----------+
                                              |
                                              v
                          +--------------------------------------+
                          |  metrics: Nakamoto, Gini, top-k,     |
                          |  identifiers-per-entity              |
                          +--------------------------------------+
```

## Quickstart

```bash
cp .env.example .env
# put your Alchemy key in .env

cargo run -- ingest --address 0xVitalikButerin...
cargo run -- ingest --address 0xSomeOtherAddr...
cargo run -- link --min-evidence 1
cargo run -- metrics --threshold 0.5
cargo run -- report > finding.md          # blog-ready Markdown
cargo run -- report --format json         # same data, structured
```

`metrics` and `report` both read the most recent persisted clustering
run from `entity_clusters` — they do **not** re-cluster. Re-run `link`
to refresh, then `report` again for new numbers.

`ingest` does three things in one shot, all best-effort:

1. fetches transfers from `alchemy_getAssetTransfers`,
2. resolves the address against an ENS service (default
   `api.ensideas.com`, override via `ENS_RESOLVER_URL`) and stores any
   `name` / twitter / github / telegram into `ens_records`,
3. queries the Safe Transaction Service (default
   `safe-transaction-mainnet.safe.global`, override via
   `SAFE_TX_SERVICE_URL`) — if the address is a Safe, it stores
   `safe_owners` rows and probes each owner with `eth_getCode` to
   distinguish EOA owners from contract (likely Safe-of-safe) owners.

A network failure on step 2 or 3 is logged and skipped — only the
primary transfers ingest is mandatory. ENS records and Safe ownership
can also be entered manually with `add-ens-record` / `add-safe-owner`
for testing or to override what the resolvers returned.

`ingest` writes to a local SQLite at `DATABASE_URL` (default
`sqlite://data/unmask.db`). Re-runs hit the cache; the schema enforces
`UNIQUE (tx_hash, from_addr, to_addr, asset, value)` so repeated ingests
do not duplicate transfers.

## Evidence model

Every link the clustering can make is grounded in a typed attestation
persisted to the `evidence` table:

```
(address, kind, key, strength, source, observed_block, payload)
```

Two addresses get merged when they share at least one `(kind, key)` AND
either (a) one of those edges is `STRONG`, or (b) the per-pair edge count
reaches `--min-evidence` AND at least one edge is `MEDIUM`. Weak edges
never merge on their own — only ranking and tie-breaking. `(kind, key)`
groups exceeding a fan-out cap (50) are flagged as suspected service keys
and dropped from edge generation; their fan-out is recorded in
`suspected_service_keys` for review.

Evidence kinds shipping today:

| Kind            | Strength | Source                                | Status        |
|-----------------|----------|---------------------------------------|---------------|
| `funded_by`     | medium   | `alchemy_getAssetTransfers` cache     | M1, automated |
| `ens_handle`    | medium   | `ens_records` (auto via `ingest`)     | M2.5, automated |
| `safe_owner`    | medium   | `safe_owners` (auto via `ingest`, EOA owners only) | M2.5, automated |
| `did_controller`| strong   | `did:ethr` / `did:pkh` documents      | M3, planned   |

`ens_handle` keys take the form `"<service>:<handle>"` with the handle
lowercased and the leading `@` stripped — so `@joseph` and `Joseph`
resolve to the same key and merge accordingly.

`safe_owner` only emits attestations for **EOA** owners. Owners that are
themselves Safes are recorded for audit (`owner_is_safe = 1`) but
excluded from edge generation: shared Safe-of-safe ownership tells us
nothing about human-level control on its own.

Manual entry remains available for testing and overrides:

```bash
cargo run -- add-ens-record \
  --address 0xa1a1... --name alice.eth --twitter @joseph --github joseph-w

cargo run -- add-safe-owner \
  --safe 0xsafe... --owner 0xeoa... --threshold 2
```

## Roadmap

- **M1 — funding-source linking** *(done)*: shared non-CEX funder evidence,
  pipeline split into `extract → attest → build`, full audit trail in
  `evidence` / `entity_clusters` / `clustering_runs`.
- **M2 — ENS and Safe linking** *(done)*: ENS text-record co-handle and
  Safe shared-EOA-owner evidence, both as medium signals.
- **M2.5 — automated resolvers** *(done)*: `ingest` now also fetches
  ENS records (REST shim, configurable) and Safe ownership (Safe Tx
  Service); each Safe owner is probed with `eth_getCode` so contract
  owners are flagged as non-EOA and excluded from clustering.
- **Report polish** *(done)*: `cargo run -- report` renders the latest
  persisted clustering run as Markdown (or JSON via `--format json`),
  with a summary, top clusters with evidence trail, suspected service
  keys, and a reproducibility footer pointing at `clustering_runs` /
  `entity_clusters` / `evidence`. `metrics` now also reads from the
  persisted run — neither command re-clusters.
- **M3 — DID and metrics**: ingest `did:ethr` / `did:pkh` documents via
  `ssi`, link by proven controller key (strong evidence), surface
  decentralization metrics in a small report (HTML or notebook).

## Project layout

```
src/
  main.rs        clap CLI
  lib.rs         re-exports
  config.rs      env loading (ALCHEMY_API_KEY, DATABASE_URL)
  alchemy/       JSON-RPC wrapper + alchemy_getAssetTransfers + eth_getCode
  ens/           EnsRecord type
  safe/          SafeOwner type
  evidence/      Strength + EvidenceKind types, per-kind extractors
  resolvers/     HTTP wrappers around ENS REST shim + Safe Tx Service
  linking/       petgraph clustering + invariants
  metrics/       Nakamoto coefficient, Gini
  report/        Markdown rendering of a persisted run
  storage/       SQLite schema + sqlx repo
  linking/       feature extraction + union-find
  metrics/       Nakamoto coefficient, Gini
migrations/      initial schema
tests/           integration tests
```

## Non-mainnet usage

`unmasking-did` can run against any Alchemy-supported EVM network by changing the Alchemy base URL and transfer categories.

For Scroll:

```env
ALCHEMY_BASE_URL=https://scroll-mainnet.g.alchemy.com/v2
ALCHEMY_TRANSFER_CATEGORIES=external,erc20
SAFE_TX_SERVICE_URL=https://safe-transaction-scroll.safe.global
```
Notes:

- ENS resolution is still Ethereum-mainnet oriented, so many L2 addresses will not have ENS records.
- Funding evidence on L2s is often bridge-heavy. High fan-out funders should be filtered or down-weighted.
- Safe queries require the chain-specific Safe Transaction Service URL.

## License

MIT — see [LICENSE](LICENSE).
