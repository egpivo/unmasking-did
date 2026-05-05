# Blog Include: Arbitrum Conservative Run Spec (v0)

Use this block as the canonical run-spec include for the blog post.

## Canonical run

- run_id: `run-1777967119432576`
- chain: `Arbitrum One`
- policy_profile_id: `arbitrum_gov_conservative_v1`
- monitoring window (frozen): `428203933 -> 459307198`
- input_snapshot_hash: `9bbba872b5590936fd6ac9cb046e6e85a7eeb315d8b2351d188d69ea285219bb`

## Cohort design

- governance seeds: `500` (stratified sample from governance vote events)
- control seeds: `500` (ARB transfer participants in the same window)
- total seeds: `1000`
- scope: predefined seeds only, one-hop enrichment only, no recursive expansion

## Conservative policy settings

```json
{
  "min_evidence": 1,
  "fan_out_cap": 50,
  "funded_by_policy": {
    "enabled": true,
    "min_shared_keys": 2,
    "min_short_burst_hits": 2,
    "service_fan_out_cap": 50,
    "short_burst_block_delta": 5000
  }
}
```

## Global run metrics (canonical)

- addresses clustered: `1000`
- inferred clusters: `976`
- top_cluster_size: `5`
- num_multi_address_clusters: `13`
- identifiers_per_cluster: `1.02`
- nakamoto_coefficient_50pct: `477`
- gini_coefficient: `0.024`
- transfer_rows_inserted: `14101`
- alchemy_calls: `1988`
- is_contract_calls: `15`
- db_size_bytes: `10657792` (~10.16 MB)
- pagination_bias_risk: `false`

## Governance vs Control comparison scope (for blog table)

The comparison table in the blog uses mixed denominators by metric type:

- `multi-address cluster rate`: **address-level** within each cohort (percentage of cohort addresses belonging to clusters with `size > 1`).
- `median cluster size`: **address-level** median of the cluster size for addresses in each cohort.
- `short_burst cluster count`, `top_funder_share > 0.8 count`, `common sink cluster count`, `candidate_medium`, `candidate_high`: **cluster-level**, computed on **pure multi-address clusters** (`size > 1`) that contain addresses from only one cohort (governance-only or control-only).

### Heuristic definitions used in the blog

- `short_burst cluster`: funded_by evidence block span `<= 5000`.
- `top_funder_share > 0.8`: max single funder coverage within cluster `> 0.8`.
- `common sink cluster`: max single sink coverage within cluster `> 0.8`.
- `candidate_medium`: pure multi-address cluster where (`short_burst == true` OR `top_funder_share > 0.8`).
- `candidate_high`: pure multi-address cluster where (`size >= 3` AND `short_burst == true` AND `top_funder_share > 0.8`).

## Interpretation boundary

This run supports coordination-evidence analysis only. It does not support:

- Sybil attack adjudication by itself
- real-world identity attribution
- intent or maliciousness claims

