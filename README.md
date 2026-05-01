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
cargo run -- report --format dot > graph.dot   # Graphviz visualization
```

`metrics` and `report` both read the most recent persisted clustering
run from `entity_clusters` — they do **not** re-cluster. Re-run `link`
to refresh, then `report` again for new numbers.

### Optional: render the DOT to SVG

`report --format dot` emits a Graphviz [DOT](https://graphviz.org/doc/info/lang.html)
file. The `unmasking-did` binary doesn't depend on Graphviz; convert
to SVG (or any other Graphviz target) with the standard `dot` CLI:

```bash
cargo run -- report --format dot > graph.dot
dot -Tsvg graph.dot -o graph.svg
# or PNG:
dot -Tpng graph.dot -o graph.png
```

Each cluster becomes a rounded box labelled by its dominant evidence
kind ("controller-level cluster" when STRONG `did_controller` is
present, "shared-owner cluster" when only `safe_owner` is present,
otherwise generic "evidence-supported cluster"). Edges are aggregated
per pair-per-kind; the label carries the kind, strength, and either
the shared key (single-edge case) or a count + kind-specific noun
("3 shared owners").

Note: DOT output reflects the **current** state of the `evidence`
table. If you've touched the cache since `link` ran, the rendered
graph may diverge from the persisted cluster shape. Re-running
`link` before `report --format dot` keeps them in sync. Persisting
per-pair edges per run for full historical reproducibility is on
the M3.5+ backlog.

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

## Entity linking model

> **This project uses evidence-based entity linking rather than
> black-box clustering. Each link must be backed by typed evidence such
> as shared DID controller, shared Safe owner, or filtered funding
> source.**
>
> **It does not identify real-world people. It links identifiers at
> the controller / governance / infrastructure evidence layer.**

The clustering pipeline maps onto the standard entity-resolution
vocabulary as follows:

| Concept            | What it is here                                                                |
|--------------------|--------------------------------------------------------------------------------|
| Mention / identifier | a wallet address, a DID, or a Safe address                                   |
| Evidence           | a typed attestation: `did_controller`, `safe_owner`, `funded_by`, `ens_handle` |
| Blocking key       | the pair `(kind, key)` — only mentions sharing one ever pair-up                |
| Candidate edge     | two identifiers share the same evidence key                                    |
| Merge policy       | merge if `max_strength == STRONG` **or** (`count >= min_evidence` and `max_strength >= MEDIUM`); weak-only evidence never merges |
| Entity cluster     | a connected component over the merge-passing edges                             |

Every attestation is persisted append-only-per-kind in the `evidence`
table:

```
(address, kind, key, strength, source, observed_block, payload)
```

Re-running `link` for an input address set re-extracts each kind from
its source cache and replaces only that kind's rows for those addresses
— other kinds and other addresses are untouched.

`(kind, key)` groups whose fan-out exceeds a cap (default 50) are
flagged as **suspected service keys** and dropped from edge generation;
their fan-out is recorded in `suspected_service_keys` for review. The
cap is a behavioral defense — it catches CEX hot wallets, bridges,
batch distributors, and faucets even when the hardcoded blacklist
hasn't been updated.

### Evidence kinds

| Evidence kind     | Strength | What it means                                                          | Merge role                             |
|-------------------|----------|------------------------------------------------------------------------|----------------------------------------|
| `did_controller`  | Strong   | Two identifiers resolve to the same non-self DID controller            | Can merge alone                        |
| `safe_owner`      | Medium   | Two Safes share EOA owners                                             | Requires repeated evidence / threshold |
| `funded_by`       | Medium (current); weighted variant planned | Two addresses share a non-service funder | Requires filtering and threshold     |
| `ens_handle`      | Medium   | Two addresses self-declare the same off-chain handle                   | Requires caution / threshold           |

A few kind-specific notes:

- **`did_controller`** is the only kind whose strength is `STRONG`.
  The strong-alone bypass in the merge rule means a single shared
  non-self controller is sufficient to merge two identifiers,
  regardless of `--min-evidence`. Self-controlled DIDs (where
  controller equals subject — true of any `did:pkh` and a freshly
  minted `did:ethr`) emit **no** attestation; that would be a
  self-referential edge with no clustering signal. The strong rating
  presumes cryptographic verification of the controller relationship —
  manual `add-did-document` entries are useful for testing but should
  not be cited as a real cryptographic finding until the M3.5
  automated resolver lands.

- **`safe_owner`** only emits attestations for **EOA** owners. Owners
  that are themselves Safes are recorded for audit
  (`owner_is_safe = 1`) but excluded from edge generation: shared
  Safe-of-safe ownership tells us nothing about human-level control
  on its own. The medium rating means a single shared owner is not
  enough — repeated overlap (`--min-evidence ≥ 2`) is required to
  merge.

- **`funded_by`** is the most fragile kind because real funder
  populations include service addresses (CEXes, bridges, faucets,
  batch distributors) that fan out to thousands of unrelated
  recipients. The current implementation:
    - drops funders that match a small hardcoded CEX blacklist
      (Ethereum mainnet only, see `cex_blacklist()` in
      `src/linking/features.rs`),
    - drops `(kind, key)` groups whose fan-out exceeds the cap,
    - emits all surviving funder edges as `Strength::Medium`.
  A future weighted variant may down-weight by inverse fan-out and
  promote the strongest funder relationships toward `Medium` while
  demoting weaker ones toward `Weak`. Until that lands, treat
  funded-by-only clusters with caution.

- **`ens_handle`** keys take the form `"<service>:<handle>"` with the
  handle lowercased and the leading `@` stripped — so `@joseph` and
  `Joseph` resolve to the same key and merge accordingly. ENS handles
  are self-declared, so the medium rating reflects the social-graph
  nature of the signal: useful in conjunction with other evidence,
  weak on its own against a determined Sybil.

Manual entry remains available for testing and overrides:

```bash
cargo run -- add-ens-record \
  --address 0xa1a1... --name alice.eth --twitter @joseph --github joseph-w

cargo run -- add-safe-owner \
  --safe 0xsafe... --owner 0xeoa... --threshold 2

cargo run -- add-did-document \
  --address 0xa1a1... --controller 0xc0c0... --method ethr
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
- **M3 — DID controller evidence** *(done)*: `did_documents` schema,
  `extract_did_controller` extractor emitting STRONG attestations,
  CLI `add-did-document` for manual entry. The strong-alone bypass in
  the merge invariant now has its first real-pipeline data.
- **M3.5 — automated `did:ethr` resolver** *(planned)*: replace the
  manual `add-did-document` path with a contract call to the
  `EthereumDIDRegistry` (configurable per chain), the same shape as
  M2.5's automated ENS / Safe resolvers.

## Project layout

```
src/
  main.rs        clap CLI
  lib.rs         re-exports
  config.rs      env loading (ALCHEMY_API_KEY, DATABASE_URL)
  alchemy/       JSON-RPC wrapper + alchemy_getAssetTransfers + eth_getCode
  ens/           EnsRecord type
  safe/          SafeOwner type
  did/           DidDocument type
  evidence/      Strength + EvidenceKind types, per-kind extractors
  resolvers/     HTTP wrappers around ENS REST shim + Safe Tx Service
  linking/       petgraph clustering + invariants
  metrics/       Nakamoto coefficient, Gini
  report/        Markdown + Graphviz DOT rendering of a persisted run
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
