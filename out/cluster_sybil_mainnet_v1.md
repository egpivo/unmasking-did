# Cluster Sybil-style coordination diagnostics
## Dataset summary
- **Database**: `data/unmask_eval_mainnet_v1.db`
- **tables_present**: `['_sqlx_migrations', 'addresses', 'clustering_runs', 'did_documents', 'ens_records', 'entity_clusters', 'evidence', 'safe_owners', 'sqlite_sequence', 'suspected_service_keys', 'transfers']`
- **clustering_run_id**: `run-1777806659290129`
- **num_clusters**: `4`
- **num_clustered_addresses**: `20`
- **row_count_evidence**: `77759`
- **row_count_transfers**: `206001`
- **row_count_entity_clusters**: `20`
- **row_count_clustering_runs**: `1`
- **row_count_safe_owners**: `28`
- **row_count_did_documents**: `0`
- **Heuristic constants**: `{
  "high_top_funder_share_min": 0.65,
  "high_top_sink_share_min": 0.6,
  "possible_consolidation_top_sink_share_min": 0.55,
  "possible_consolidation_max_unique_sinks": 4,
  "medium_top_funder_share_min": 0.35,
  "medium_top_sink_share_min": 0.35,
  "short_burst_blocks": 100
}`

## Top `candidate_high` clusters
_None in this snapshot._

## Top common-funder clusters (by `top_funder_share`)
| cluster_id | n_addr | top_funder | share | unique funders |
|---|---:|---|---:|---:|
| `0x2e59a20f205bb85a89c53f1936454680651e618e` | 1 | `0x6357d3843d715496257e338a878ab0b72040a918` | 1.0 | 1 |
| `0x323a76393544d5ecca80cd6ef2a560c6a395b7e3` | 1 | `0x6357d3843d715496257e338a878ab0b72040a918` | 1.0 | 1 |
| `0x02d61347e5c6ea5604f3f814c5b5498421cebdeb` | 17 | `0x0000000000000000000000000000000000000000` | 0.6470588235294118 | 76357 |

## Top common-sink clusters (by `top_sink_share`)
| cluster_id | n_addr | top_sink | share | unique sinks |
|---|---:|---|---:|---:|

## Caveats
- These are **Sybil-style coordination features**, not proof of an attack.
- An actual Sybil attack requires a **target objective** such as airdrop farming, vote manipulation, or reputation abuse.
- **Verified DID** evidence is reported separately; it does **not** automatically imply Sybil behavior.
- **Shared Safe ownership** describes a control relation; it is **not** inherently malicious.
- Metrics are **heuristic** and may be inflated by benign batch funding, custodial patterns, or sparse transfer caches.
