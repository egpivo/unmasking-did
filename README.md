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
```

`ingest` writes to a local SQLite at `DATABASE_URL` (default
`sqlite://data/unmask.db`). Re-runs hit the cache; the schema enforces
`UNIQUE (tx_hash, from_addr, to_addr, asset, value)` so repeated ingests
do not duplicate transfers.

## How linking works (M1)

For each ingested address, all incoming transfers are scanned to derive a
set of *funders*. Funders that appear in a small hardcoded blacklist of
well-known CEX hot wallets (Binance, Coinbase, Kraken) are dropped, since
funding from a CEX is uninformative — the same hot wallet pays out to
millions of unrelated users.

Two addresses are merged in a union-find when they share at least
`--min-evidence` non-CEX funders. Each cluster in the output carries the
list of funders that justified the merge.

This is **medium evidence** in the project taxonomy: it can support a
link only in conjunction with other signals. The MVP applies it alone for
demonstration; later milestones add ENS / Safe / DID-controller signals
and weight evidence accordingly.

## Roadmap

- **M1 — funding-source linking** *(in progress)*: ingest transfers via
  `alchemy_getAssetTransfers`, build clusters from shared non-CEX funders,
  emit Nakamoto and Gini.
- **M2 — ENS and Safe linking**: add ENS text-record co-handle evidence
  and Safe shared-owner evidence (EOA owners only). Promote both as
  medium evidence; require ≥2 distinct evidence types to merge.
- **M3 — DID and metrics**: ingest `did:ethr` / `did:pkh` documents via
  `ssi`, link by proven controller key, surface decentralization metrics
  in a small report (HTML or notebook).

## Project layout

```
src/
  main.rs        clap CLI
  lib.rs         re-exports
  config.rs      env loading (ALCHEMY_API_KEY, DATABASE_URL)
  alchemy/       JSON-RPC wrapper + alchemy_getAssetTransfers
  ens/           ENS subgraph queries (stub)
  storage/       SQLite schema + sqlx repo
  linking/       feature extraction + union-find
  metrics/       Nakamoto coefficient, Gini
migrations/      initial schema
tests/           integration tests
```

## License

MIT — see [LICENSE](LICENSE).
