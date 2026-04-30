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
cargo run -- add-ens-record --address 0xVitalik... --twitter @VitalikButerin --github vbuterin
cargo run -- link --min-evidence 1
cargo run -- metrics --threshold 0.5
```

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
| `ens_handle`    | medium   | `ens_records` table (manual entry)    | M2, manual    |
| `safe_owner`    | medium   | `safe_owners` table (EOA owners only) | M2, manual    |
| `did_controller`| strong   | `did:ethr` / `did:pkh` documents      | M3, planned   |

`ens_handle` keys take the form `"<service>:<handle>"` with the handle
lowercased and the leading `@` stripped — so `@joseph` and `Joseph`
resolve to the same key and merge accordingly.

`safe_owner` only emits attestations for **EOA** owners. Owners that are
themselves Safes are recorded for audit (`owner_is_safe = 1`) but
excluded from edge generation: shared Safe-of-safe ownership tells us
nothing about human-level control on its own.

Until the automated resolvers land (PR 2.5 for ENS, PR 3.5 for Safe Tx
Service), populate the caches manually:

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
  Safe shared-EOA-owner evidence shipping; both populated via manual CLI
  for now.
- **M2.5 — automated resolvers**: replace `add-ens-record` with a
  subgraph or contract-call ingestor; replace `add-safe-owner` with a
  Safe Transaction Service ingestor.
- **M3 — DID and metrics**: ingest `did:ethr` / `did:pkh` documents via
  `ssi`, link by proven controller key (strong evidence), surface
  decentralization metrics in a small report (HTML or notebook).

## Project layout

```
src/
  main.rs        clap CLI
  lib.rs         re-exports
  config.rs      env loading (ALCHEMY_API_KEY, DATABASE_URL)
  alchemy/       JSON-RPC wrapper + alchemy_getAssetTransfers
  ens/           ENS records (manual entry — automated resolver in PR 2.5)
  safe/          Safe owner records (manual entry — automated resolver in PR 3.5)
  evidence/      Strength + EvidenceKind types, per-kind extractors
  storage/       SQLite schema + sqlx repo
  linking/       feature extraction + union-find
  metrics/       Nakamoto coefficient, Gini
migrations/      initial schema
tests/           integration tests
```

## License

MIT — see [LICENSE](LICENSE).
