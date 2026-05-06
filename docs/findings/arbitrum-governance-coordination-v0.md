# Arbitrum Governance Coordination v0 (Conservative)

Status: validated v0 finding package  
Scope: coordination-monitoring output only (not Sybil detection)

## Dataset Design

- Chain: Arbitrum One
- Seed design: fixed `500 governance + 500 control` (stratified Phase 1b seeds)
- Frozen block window: `428203933 -> 459307198`
- Seed expansion: none (exactly 1000 seed addresses ingested)
- Enrichment bound: one-hop only, bounded pagination/caps
- Raw artifacts:
  - `data/unmask_arbitrum_gov_v1.db`
  - `out/arbitrum_gov_relink_conservative_summary.json`
  - `out/arbitrum_gov_report.md`
  - `out/arbitrum_gov.graph.json`

## Why Raw funded_by Over-linked

The initial funded_by-dominant linking regime produced a giant mixed component (top size 425), driven by infrastructure-like funding hubs rather than interpretable coordination structure:

- service/router/high-fanout funders connected many addresses
- zero/system address semantics (e.g., mint-like transfer patterns) appeared as funded_by keys
- resulting graph connectivity was overly sensitive to shared infra rather than robust multi-signal evidence

## Conservative Policy Used

Run parameters (from latest conservative relink):

- `--conservative-funded-by`
- `--funded-by-service-fan-out-cap 50`
- `--funded-by-min-shared-keys 2`
- `--funded-by-min-short-burst-hits 2`
- `--funded-by-short-burst-block-delta 5000`
- `--min-evidence 1`

Policy intent:

- a single shared funder is insufficient evidence to merge
- funded_by-only merges require repeated shared funders plus short-burst timing support
- service-like funded_by keys are suppressed (including zero/system and known router/bridge classes)
- `safe_owner` and `did_controller` behavior remains unchanged

This is safer because it reduces hub-induced false joins while retaining a bounded path for repeated, temporally coupled funded_by patterns.

## Validated Conservative Result

From `out/arbitrum_gov_relink_conservative_summary.json`:

- `cluster_count = 976`
- `top_cluster_size = 5`
- `multi_address_clusters = 13`

Top 5 clusters (all `mixed_gov_control = false`):

1. `0x131a2a94de9e350200c81b79002f826657b727ec` — size 5 (`gov 0 / control 5`)
2. `0x74bf7a1829acbb487d205c0a247c2b5717b0d0fe` — size 4 (`gov 4 / control 0`)
3. `0xc4bc0ea537cc70948fc5965f70a3ff66d8af4ab2` — size 4 (`gov 0 / control 4`)
4. `0x01f761805c90bc8abfaf504075afdb87e1e5dc08` — size 3 (`gov 3 / control 0`)
5. `0x05790ed5045266591e8b1f254e236ed2c83f29e0` — size 3 (`gov 3 / control 0`)

## Interpretation Boundary

- These are coordination-evidence clusters, not Sybil/attack conclusions.
- No real-world identity inference is supported by this output.
- Clusters should be treated as evidence-supported candidate structures for monitoring and audit.

## Caveats

- funded_by remains a fragile evidence family even under conservative gating
- DID depth is effectively absent in this dataset (limited DID-controller contribution)
- no objective-specific abuse proof is established
- graph JSON is intentionally bounded/truncated for interactive viewer performance

## What v0 Demonstrates

- evidence-based linking is highly policy-sensitive
- conservative funded_by gating prevents giant service-driven clusters
- the monitor can produce reproducible, auditable, bounded v0 outputs

---

## Phase 3 Options (Proposal Only)

### Option 1: Consecutive-window monitoring

Implementation scope:
- run the same Arbitrum governance/control process on the next adjacent window
- add run-over-run cluster lineage/stability tracking (cluster persistence/splits/merges)

Expected data need:
- one additional seeded run per cadence interval (weekly/monthly)
- same seed recipe + conservative policy profile for comparability

Article value:
- strongest for “monitoring” narrative (trend and stability evidence)
- demonstrates operational repeatability and policy robustness over time

Risk:
- lineage heuristics can be noisy if membership churn is high
- requires careful run metadata discipline

Recommended next step if chosen:
- freeze a “v1 monitor profile” and execute one adjacent-window rerun with lineage tables

### Option 2: Cross-chain robustness (Base or Optimism)

Implementation scope:
- replicate the exact conservative method on one additional chain
- compare cluster shape and policy behavior across chains

Expected data need:
- one full seeded run on second chain + equivalent governance/control seed design

Article value:
- strong external-validity signal
- improves credibility of policy claims beyond Arbitrum

Risk:
- governance primitives and infra topology differ by chain
- might require chain-specific policy tuning

Recommended next step if chosen:
- run a small pilot on one chain with unchanged policy first, then evaluate drift

### Option 3: DID-focused M3.5

Implementation scope:
- automated `did:ethr` resolver + cryptographic verification path
- elevate strong DID/controller evidence coverage

Expected data need:
- DID-rich address cohorts or targeted DID-inclusive seeds

Article value:
- aligns tightly with the unmasking-did thesis (entity-vs-identifier gap)

Risk:
- sparse DID usage may yield low immediate signal in governance cohorts
- higher implementation complexity before visible monitor gains

Recommended next step if chosen:
- prototype resolver on a DID-dense subset before full monitor integration

## Recommendation

Recommend **Option 1 (Consecutive-window monitoring)** as Phase 3.

Reason: the v0 result shows policy stabilization is now good enough to prioritize cadence value (repeatability + stability over time), which matches the long-term weekly/monthly monitoring goal and provides the clearest near-term article narrative.
