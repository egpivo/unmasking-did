# Scroll DAO governance Safes — first reproducible finding

**Date**: 2026-05-01
**Chain**: Scroll mainnet (chain-id 534352)
**`ALCHEMY_BASE_URL`**: `https://scroll-mainnet.g.alchemy.com/v2`
**`SAFE_TX_SERVICE_URL`**: `https://safe-transaction-scroll.safe.global`
**`ALCHEMY_TRANSFER_CATEGORIES`**: `external,erc20` (Scroll rejects the `internal` category)
**Code commit**: see git log on the `data/first-scroll-finding` branch
**Run id**: `run-1777602821371447` (queryable in `clustering_runs` for full audit)

## Goal

A small, narrow, reproducible test of the tool against documented Scroll
DAO governance Safes. **Not a deanonymization claim and not a Sybil
investigation** — these are publicly-attributed multisigs whose owners
are visible through the official Safe Transaction Service. The point is
to verify the pipeline produces an explainable cluster on real on-chain
data, with the evidence trail intact.

## Input set

Six Scroll Safes, all verified live against the Scroll Safe Transaction
Service before ingestion:

| # | Address | Role |
|---|---|---|
| 1 | `0x20fa362323447506d9d0c02483ae97c4e2d6b607` | Scroll DAO Treasury Multisig (3-of-5, current main treasury) |
| 2 | `0xd0d05390d922a2c45a70eaa4601600f236c02acc` | Operations & Accountability Committee Multisig (2-of-3) |
| 3 | `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434` | Delegates Incentives Multisig (2-of-3, nonce 0) |
| 4 | `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20` | Community Allocation Multisig (2-of-3, nonce 0) |
| 5 | `0xe47b51a31ad43acb72a224fab4a17999311e2e48` | Ecosystem Allocation Multisig (2-of-3) |
| 6 | `0xcca54b0916cee2186b47e9709bedcb7041a8f761` | Scroll Admin Multisig (DAO 2.0 contracts owner, 4-of-6) — **negative control** |

## Linking parameters

```
min_evidence = 2     # repeated overlap required: a single shared owner is not enough
fan_out_cap  = 50    # default; nothing in this run exceeded the cap
```

`safe_owner` is treated as **medium** evidence (not strong-alone). The merge
rule per the project taxonomy: a pair must accumulate at least
`min_evidence` shared `(kind, key)` edges, with at least one of them ≥
medium. With `min_evidence = 2`, no Safe pair merges on a single shared
EOA owner.

## Result

```
Cluster A: 5 distinct Scroll DAO Safes forming one shared governance-control cluster
            (cluster_id = 0x20fa362323447506d9d0c02483ae97c4e2d6b607)
            Members: #1, #2, #3, #4, #5

Cluster B: 1 disjoint Admin Multisig as a negative control
            (cluster_id = 0xcca54b0916cee2186b47e9709bedcb7041a8f761)
            Members: #6
```

This matches the prior expectation derived from public documentation
(`scrolldaotreasury.com`, `forum.scroll.io` governance threads). The
Admin Multisig stays disjoint because none of its 6 owner EOAs overlap
with the Treasury / committee owner set.

## Evidence trail for Cluster A

The cluster was justified by **two kinds of medium evidence stacking**:

- **3 shared `safe_owner` keys** — all 5 governance Safes share the same
  three EOAs as signers:

  | Owner EOA | Role |
  |---|---|
  | `0x558581b0345d986ba5bd6f04efd27e2a5b991320` | shared signer across #1–#5 |
  | `0x73506528332becf6121f71ac9aad43646a41994c` | shared signer across #1–#5 |
  | `0xbc72d9f10f6626271092764467983122cf15e3f4` | shared signer across #1–#5 |

  Each of these three EOAs creates a 5-clique edge across the cluster.

- **1 shared `funded_by` key** — the Treasury Safe (#1) funded both the
  Operations Committee (#2) and the Ecosystem Allocation Safe (#5) with
  USDT and USDC transfers (8 transfers in total over blocks 31,098,210
  through 33,431,926). #2 and #5 therefore share `funded_by =
  0x20fa362323447506d9d0c02483ae97c4e2d6b607` as a fourth pairwise edge.
  Notable: a clustering subject (the Treasury) is also acting as an
  evidence value here. That's expected and legitimate per the model —
  the symmetric `(kind, key)` merge does not distinguish the role.

`#3` and `#4` (Delegates Incentives, Community Allocation) had `nonce 0`
on Scroll — never executed any transactions, so they contribute zero
`funded_by` evidence. They merge into the cluster purely on the three
shared `safe_owner` edges. This is a useful signal: a Safe that has
*never been used* on chain but shares signers with peers can still be
correctly attributed to the same governance-control cluster.

## Reading

What this finding **does** show:

- The pipeline ingests / extracts / clusters / reports correctly against
  real Scroll mainnet data.
- Cross-kind evidence stacking works: `safe_owner` and `funded_by`
  contribute independent signal to the same cluster.
- The `min_evidence = 2` rule is doing real work — a single shared owner
  would have been insufficient under this threshold; the model required
  the *pattern* of repeated overlap to merge.
- A deliberately-disjoint Safe (`#6`) stays unmerged, confirming the
  model is not over-eager.

What this finding **does not** show:

- Any deanonymization. All input addresses are publicly-documented
  governance multisigs.
- Any Sybil investigation or wallet-to-real-person mapping.
- Anything about `did_controller` or `ens_handle` evidence — none was
  observed for these addresses (none of the six have ENS records on
  Ethereum mainnet via api.ensideas.com).

## Caveats and L2 reality checks

- **ENS evidence was empty.** ENS reverse-resolution queries Ethereum
  mainnet; Scroll-active governance Safes typically don't have a mainnet
  ENS primary name set on these contract addresses. This is expected,
  not a model failure.
- **The CEX blacklist is mainnet-specific.** None of the standard
  Binance / Coinbase / Kraken hot-wallet addresses appear on Scroll;
  the dominant funder pattern on L2 is the Scroll bridge
  (`0x6774Bcbd5cecef1336b5300fb5186a12ddd8b367` and similar). For this
  particular finding, none of the Safes had bridge inflows in their
  cached transfer history (Safes #3, #4, #6 had zero transfers; the
  rest had a small number of stablecoin movements between each other),
  so neither the blacklist nor the fan-out cap was triggered. On a
  larger Scroll-active address set, the fan-out cap would do the heavy
  lifting against bridges.
- **Renderer gap (logged for follow-up).** The Markdown report's "Top
  Clusters" section currently filters out singleton clusters, so the
  rendered output below shows only Cluster A. Cluster B (the Admin
  Multisig negative control) is recorded in `entity_clusters` and the
  raw `link` JSON output, but does not appear in the Markdown body.
  Worth fixing in the report renderer so future findings always show
  declared negative controls.

## Sources

- Scroll DAO Treasury wallets API — https://scrolldaotreasury.com/api/wallets
- Scroll DAO Treasury Allocation Dashboard announcement — https://forum.scroll.io/t/scroll-daos-treasury-allocation-dashboard/1450
- RFC: Scroll DAO Multisig Management Policy — https://forum.scroll.io/t/rfc-scroll-dao-multisig-management-policy/1396
- Governance Update: Security Council Transition — https://forum.scroll.io/t/governance-update-security-council-transition-contributor-roles-operational-adjustments/1470
- Scroll DAO 2.0 Governance Framework Update proposal — https://gov.scroll.io/proposals/93184587259984953492038155824853212755439337616974783443519474771894862587881

---

## Generated report (verbatim from `cargo run -- report`)

# unmasking-did Report

**Run**: `run-1777602821371447` (started 2026-05-01 02:33:41)
**Parameters**: `{"address_count":6,"fan_out_cap":50,"min_evidence":2}`

## Summary

- Addresses analyzed: **6**
- Inferred entities: **2**
- Identifiers per entity: **3.00**
- Nakamoto coefficient (>50% of population): **1**
- Gini coefficient: **0.333**

## Top Clusters

### Cluster 1 — `0x20fa…b607` (5 addresses)

Connected via:
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607`
- `0x558581b0345d986ba5bd6f04efd27e2a5b991320`
- `0x73506528332becf6121f71ac9aad43646a41994c`
- `0xbc72d9f10f6626271092764467983122cf15e3f4`

Members:
- `0x20fa362323447506d9d0c02483ae97c4e2d6b607`
- `0x756ed67a0e73dd1ec4facbc307ca79c28d930b20`
- `0x7964e7bf48948c9e1d89f419cad8ef7d8d8f0434`
- `0xd0d05390d922a2c45a70eaa4601600f236c02acc`
- `0xe47b51a31ad43acb72a224fab4a17999311e2e48`

## Reproducibility

Cluster identities are deterministic: `cluster_id = min(address)`. Re-running the same `link` invocation against the same `evidence` rows that produced run `run-1777602821371447` will yield byte-identical clusters. Run metadata, parameters, evidence trail, and cluster membership are all preserved in SQLite tables `clustering_runs`, `evidence`, `entity_clusters`, and `suspected_service_keys`.
