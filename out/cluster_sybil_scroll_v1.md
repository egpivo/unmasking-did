# Cluster Sybil-style coordination diagnostics
## Dataset summary
- **Database**: `data/unmask_eval_scroll_v1.db`
- **tables_present**: `['_sqlx_migrations', 'addresses', 'clustering_runs', 'did_documents', 'ens_records', 'entity_clusters', 'evidence', 'safe_owners', 'sqlite_sequence', 'suspected_service_keys', 'transfers']`
- **clustering_run_id**: `run-1777727999694630`
- **num_clusters**: `5`
- **num_clustered_addresses**: `9`
- **row_count_evidence**: `44`
- **row_count_transfers**: `44`
- **row_count_entity_clusters**: `9`
- **row_count_clustering_runs**: `1`
- **row_count_safe_owners**: `23`
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
| cluster_id | n_addr | top_funder_share | burst | top_sink_share | tier_reasons (abridged) |
|---|---:|---:|---|---:|---|
| `0x20fa362323447506d9d0c02483ae97c4e2d6b607` | 5 | 0.4 | long_span | 1.0 | candidate_high: consolidation-like outbound pattern (top_sink_share>=0.6, few distinct sinks) |

## Top common-funder clusters (by `top_funder_share`)
| cluster_id | n_addr | top_funder | share | unique funders |
|---|---:|---|---:|---:|
| `0x558581b0345d986ba5bd6f04efd27e2a5b991320` | 1 | `0x8e8c30a592f339715b29e1a43cdabb403eb4b385` | 1.0 | 3 |
| `0x73506528332becf6121f71ac9aad43646a41994c` | 1 | `0xa3a4c786cc72a4d364f4958efc3bccb13afda312` | 1.0 | 10 |
| `0xbc72d9f10f6626271092764467983122cf15e3f4` | 1 | `0x25cc196fd6f6145c5edbae3fdafad762498d167a` | 1.0 | 1 |
| `0x20fa362323447506d9d0c02483ae97c4e2d6b607` | 5 | `0x20fa362323447506d9d0c02483ae97c4e2d6b607` | 0.4 | 6 |

## Top common-sink clusters (by `top_sink_share`)
| cluster_id | n_addr | top_sink | share | unique sinks |
|---|---:|---|---:|---:|
| `0x20fa362323447506d9d0c02483ae97c4e2d6b607` | 5 | `0x73506528332becf6121f71ac9aad43646a41994c` | 1.0 | 1 |

## Caveats
- These are **Sybil-style coordination features**, not proof of an attack.
- An actual Sybil attack requires a **target objective** such as airdrop farming, vote manipulation, or reputation abuse.
- **Verified DID** evidence is reported separately; it does **not** automatically imply Sybil behavior.
- **Shared Safe ownership** describes a control relation; it is **not** inherently malicious.
- Metrics are **heuristic** and may be inflated by benign batch funding, custodial patterns, or sparse transfer caches.
