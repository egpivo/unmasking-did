# Run Spec: Arbitrum Governance Participants (90d)

Status: draft for review  
Execution: **not started** (spec only)

## 1) Objective

Build a bounded, repeatable dataset for **coordination monitoring** over Arbitrum governance participation.

Framing:
- This is **not** Sybil detection.
- This is **not** malicious labeling.
- This is an evidence-based analysis of coordination signals and shared-control patterns in governance-related participation.

## 2) Chain / Window

- Chain: **Arbitrum One**
- Time scope: **last 90 days** from run start timestamp
- Block scope: resolve and freeze exact `[start_block, end_block]` immediately before acquisition

Run metadata to record:
- window start/end timestamps (UTC)
- window start/end blocks
- resolution method used for timestamp -> block mapping

### 2.1) Frozen pre-run window (resolved, not executed)

- `end_ts_utc`: `2026-05-04T13:31:00Z`
- `start_ts_utc`: `2026-02-03T13:31:00Z`
- `end_block`: `459307198`
- `start_block`: `428203933`

Block-resolution method:
- Timestamp-to-block mapping via DefiLlama endpoint:
  - `https://coins.llama.fi/block/arbitrum/<unix_ts>`
- Resolution mode for this run:
  - timestamp-anchored window (not `latest`/`safe`/`finalized` head polling)
  - no additional safety-margin subtraction applied, because end block is fixed by `end_ts_utc`

## 3) Seed Contracts

Governance contract seeds (official Arbitrum DAO deployment addresses):

- Core Governor: `0xf07DeD9dC292157749B6Fd268E37DF6EA38395B9`
- Treasury Governor: `0x789fC99093B09aD01C34DC7251D0C89ce743e5a4`

### 3.1) ABI Verification Record (locked pre-run)

Fetched date (UTC): `2026-05-04`

Primary sources:
- Arbitrum governance deployment manifest:
  - `https://raw.githubusercontent.com/ArbitrumFoundation/governance/main/files/mainnet/deployedContracts.json`
  - GitHub branch snapshot commit (main): `ae6cf85b88e8d5dbf83d95b2551a6808f66f052c`
- Sourcify metadata (implementation ABI):
  - `https://repo.sourcify.dev/contracts/partial_match/42161/0x065620d99E1785Ccf56Fa95462d3012Eb844FDC9/metadata.json`

Proxy/implementation mapping used:
- Core Governor proxy: `0xf07DeD9dC292157749B6Fd268E37DF6EA38395B9`
- Treasury Governor proxy: `0x789fC99093B09aD01C34DC7251D0C89ce743e5a4`
- Shared governor logic implementation: `0x065620d99E1785Ccf56Fa95462d3012Eb844FDC9`

Verified event definitions from implementation ABI:
- `VoteCast(address,uint256,uint8,uint256,string)`
  - topic0: `0xb8e138887d0aa13bab447e82de9d5c1777041ecd21ca36ba824ff1e6c07ddda4`
  - indexed fields: `voter`
- `VoteCastWithParams(address,uint256,uint8,uint256,string,bytes)`
  - topic0: `0xe2babfbac5889a709b63bb7f598b324e08bc5a4fb9ec647fb3cbc9ec07eb8712`
  - indexed fields: `voter`
- `ProposalCreated(uint256,address,address[],uint256[],string[],bytes[],uint256,uint256,string)`
  - topic0: `0x7d84a6263ae0d98d3329bd7b46bb4e8d6f98cd35a7adb45c274c8b7fd5ebd5e0`
  - indexed fields: none

## 4) Event Extraction Plan

1. Fetch verified ABI for each governor contract.
2. Use explicit event signatures (deterministic extraction contract):
   - Primary voter events:
     - `VoteCast(address voter, uint256 proposalId, uint8 support, uint256 weight, string reason)`
     - `VoteCastWithParams(address voter, uint256 proposalId, uint8 support, uint256 weight, string reason, bytes params)` (if present in ABI)
   - Optional proposer event (kept separately labeled):
     - `ProposalCreated(uint256 proposalId, address proposer, address[] targets, uint256[] values, string[] signatures, bytes[] calldatas, uint256 startBlock, uint256 endBlock, string description)`
3. Extract participant addresses from those exact events within the fixed block window.
   - `VoteCast`: extract `voter` from indexed topic
   - `VoteCastWithParams`: extract `voter` from indexed topic
   - `ProposalCreated`: decode `proposer` from event data (not indexed)
4. Normalize addresses:
   - lowercase hex
   - deduplicate
5. Label seed provenance:
   - `seed_type = voter` for `VoteCast` / `VoteCastWithParams`
   - `seed_type = proposer` for proposal creators (if included)
6. Target seed cap:
   - preferred max: **500 unique addresses**
   - deterministic cap rule (if needed): sort by `(first_seen_block, address)` and take first 500

Notes:
- Keep proposer-derived seeds separate from voter-derived seeds for analysis clarity.
- Do not mix unrelated governance contracts in this first pass.

### 4.1) Log Query Execution Strategy

Preferred strategy:
- one `eth_getLogs` stream per block chunk using:
  - `address`: both governor proxies
  - `topics[0]`: list of verified event topic0 hashes
- chunk by block range for reliability and retryability

Fallback strategy:
- per `(contract x topic0)` query, chunked by block range

Run metadata must record:
- strategy used (`combined` or `per_contract_topic`)
- chunk size
- retry/backoff settings

## 5) Inclusion / Exclusion Rules

Inclusion:
- addresses directly present in governance participation events from the two seed contracts, within the fixed window
- one-hop enrichment only from seed addresses

Exclusion:
- no broad crawling
- no recursive expansion beyond one hop
- no unrelated protocol-wide scans

Evidence filtering:
- exclude known service addresses from linking evidence (e.g., known CEX/service keys as configured)
- fan-out cap logic applies to drop service-like keys

Flag-only (do not auto-exclude):
- bridge addresses
- protocol treasury addresses
- delegate tooling / operations wallets

## 6) Evidence to Collect

For each included seed (and one-hop context), collect:

- `funded_by` evidence
- first incoming funder (from cached transfer history)
- definition note: “First incoming funder” is computed **within the observed block window only**; it is not guaranteed to be the true first funder in the address lifetime.
- funding burst diagnostics (same block / short burst / long span)
- common sink diagnostics
- `safe_owner` evidence when address is a Safe
- ENS-linked evidence where available
- DID controller evidence only if already supported by current pipeline/components

Constraints:
- no new ingestion source in this run spec
- no trace/debug RPC methods

## 7) Stop Conditions / Guardrails

Seed-count rules:
- If `< 300` unique seeds: widen window to **120 days** before adding additional contracts.
- If `> 800` unique seeds: apply deterministic cap rule and keep **max 500** for MVP.

Operational rules:
- estimate query count before acquisition starts
- avoid trace/debug methods
- keep run bounded and reproducible
- do not execute acquisition until this spec is approved

### 7.1) Control Dataset (small, bounded)

Add a small comparison cohort to calibrate coordination signals against non-objective-linked behavior.

Chosen control cohort:
- **ARB token transfer-based ecosystem control cohort**
- purpose: compare governance voter/proposer coordination signals against general ARB transfer participant behavior in the same window.

Control event definition (ARB token `0x912CE59144191C1204E64559FE8253a0e49E6548`):
- event signature: `Transfer(address,address,uint256)`
- full typed event: `Transfer(address indexed from, address indexed to, uint256 value)`
- topic0: `0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef`
- indexed fields: `from`, `to`

Participant extraction rule:
- include both `from` and `to` participants
- apply exclusions before cohort selection:
  - zero address
  - governance voter/proposer seed addresses
  - known service addresses (where configured)
  - extreme fan-out service-like addresses
  - optional contract-address exclusion via `eth_getCode` when EOA-like control cohort is enforced
- normalize lowercase, then deduplicate

Deterministic control sampling:
- sort by `(first_seen_block, address)`
- take `80-150` addresses (default cap target: 150)
- enforce exclusion of governance voter/proposer seed set before final sample

Control constraints:
- target size: 80-150 addresses
- one-hop enrichment only
- same evidence pipeline and filtering rules as primary cohort
- no broad crawling or new data sources

Control exclusions:
- exclude zero address (`0x0000000000000000000000000000000000000000`)
- exclude addresses already in governance voter/proposer seed sets
- exclude known service addresses where configured
- exclude extreme fan-out service-like addresses (same fan-out policy used in primary cohort)
- if enforcing EOA-like control cohort, exclude contract addresses via `eth_getCode`

Purpose:
- compare common-funder / burst / sink / shared-control feature prevalence versus objective-linked seeds.

## 8) Planned Outputs

Required output artifacts:

1. Seed CSV:
   - `data/seeds/arbitrum_gov_90d.csv`
2. SQLite DB snapshot:
   - `data/unmask_arbitrum_gov_90d.db` (or similarly versioned run-specific path)
3. Markdown report:
   - `out/arbitrum_gov_90d_report.md`
4. Graph JSON:
   - `out/arbitrum_gov_90d_graph.json`
5. Optional viewer artifact:
   - static D3/HTML-compatible output (if generated)
6. Coordination feature summary:
   - JSON and/or Markdown summary artifact for cluster-level coordination features

All outputs should include run metadata references (window, contract seeds, params hash, generation timestamp).

## 9) Conservative Reporting Language

Use:
- “coordination evidence”
- “governance-control participation graph”
- “shared-control signals”
- “operational coupling”

Avoid:
- “confirmed Sybil”
- “proven attack”
- real-world human identity inference

Reporting stance:
- describe evidence strength and uncertainty
- separate strong/medium/weak signals clearly
- include negative/control examples where suspicious structure may still be benign

## 10) Risks / Caveats

Known interpretation risks:

- delegate tooling can create shared infrastructure patterns
- relayers can create misleading common-funder/common-sink edges
- CEX/bridge funding can inflate apparent coordination
- legitimate team/treasury operations may resemble operational coupling
- DID evidence depth may be low in this dataset, limiting DID-centric conclusions

Mitigations:
- service-key filtering + fan-out caps
- explicit caveat section in report
- avoid intent/malice claims without objective-specific abuse evidence

## Phase 2 CLI (pre-merge validation)

Phase 2 is implemented as `cargo run -- phase2-arbitrum-gov` (see `src/phase2_arbitrum_gov.rs`). Until a full ingest + link completes successfully, **do not** treat outputs as findings or publish article claims.

### 1) No seed expansion (code + CSV contract)

- **Exactly 500 governance + 500 control** rows are required from the stratified seed CSVs; otherwise the command errors and does not proceed.
- **Only those 1000 addresses** are passed to `upsert_address`, transfer fetch, and `link`. Counterparties appear in the `transfers` table as endpoints only; they are **not** added to the seed / clustering address set.

### 2) Snapshot hash preserved

- Phase 2 reads **`input_snapshot_hash`** from `out/phase1b_arbitrum_gov_seed_quality.json` (default `--phase1b-json` path) and copies it into the Markdown report and `out/phase2_arbitrum_gov_summary.json` unchanged.
- Phase 1b value (frozen for this run): `9bbba872b5590936fd6ac9cb046e6e85a7eeb315d8b2351d188d69ea285219bb`.

### 3) Conservative language (Phase 2 artifacts)

- Phase 2 report / summary JSON use **coordination** framing only (no “Sybil detected”, no “attack” labeling). Section 9 of this spec still applies to any narrative derived from outputs.

### 4) Default storage / output paths

| Artifact | Default path |
|----------|----------------|
| SQLite DB | `data/unmask_arbitrum_gov_v1.db` (`--database` or full `sqlite://…` URL) |
| Markdown report | `out/phase2_arbitrum_gov_report.md` |
| Graph JSON (D3) | `out/phase2_arbitrum_gov.graph.json` |
| Run summary JSON | `out/phase2_arbitrum_gov_summary.json` |

Seeds (read-only): `data/seeds/arbitrum_gov_90d_governance_stratified500.csv`, `data/seeds/arbitrum_gov_90d_control_stratified500.csv`.

### 5) Failure mode — Alchemy 403 / partial runs

- **`alchemy_getAssetTransfers` returns HTTP 403** when the Alchemy API key is not valid for **Arbitrum** (wrong app/network, key disabled for that chain, or billing/access restrictions). Fix: create or enable an **Arbitrum One** app in Alchemy and set **`ARBITRUM_ALCHEMY_API_KEY`** (or **`ALCHEMY_API_KEY`** as fallback). Optionally set **`ARBITRUM_ALCHEMY_BASE_URL`** (default `https://arb-mainnet.g.alchemy.com/v2`). At startup, Phase 2 logs which base URL and which env var supplied the key (never the secret).
- **A partial database is not valid** for analysis. On Phase 2 **command failure**, the binary removes the SQLite file and the default Phase 2 report / graph / summary paths so stale artifacts are not mistaken for a completed run. If you **kill** the process manually, delete those paths yourself or rerun with `--overwrite-db`.

## Pre-Run Review Checklist

- [ ] Window timestamps and exact start/end blocks resolved and recorded
- [ ] Event signatures confirmed from current ABI
- [ ] Seed extraction query logic reviewed
- [ ] Deterministic cap rule documented
- [ ] Query-count estimate documented
- [ ] Output paths finalized
- [ ] Conservative language confirmed in report templates

