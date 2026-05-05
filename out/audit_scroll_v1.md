# Evidence Audit: `data/unmask_eval_scroll_v1.db`

## Table Row Counts
- addresses: 9
- transfers: 44
- ens_records: 9
- safe_owners: 23
- evidence: 44
- clustering_runs: 1
- entity_clusters: 9

## Evidence Kinds
- safe_owner: 23
- funded_by: 21

## Semantic Class Counts
- control_evidence: 23
- event_evidence: 21

## Temporal Coverage
- has_block_number: True
- min_block: 8342611
- max_block: 33544627
- has_timestamp: False
- min_timestamp: None
- max_timestamp: None

## Duplication/Inflation Diagnostics
- raw_evidence_rows: 44
- unique_pair_count: 43
- unique_pair_kind_count: 44
- unique_pair_kind_key_count: 44
- unique_source_count: 22
- unique_tx_hash_count: 0
- raw_to_unique_pair_ratio: 1.0232558139534884
- raw_to_unique_pair_kind_ratio: 1.0

## Probabilistic Readiness
- can_model_relatedness: True — Event/control/identity evidence can support relatedness candidate ranking.
- can_model_control_link: True — Control-evidence kinds (e.g. safe_owner/admin/owner) are present.
- can_model_identity_claim: False — No ENS/DID-like identity claim evidence present.
- can_model_same_did_or_shared_controller: False — No cryptographically/registry verified DID/controller evidence; unverified DID claims alone are insufficient.

## Gold Labels
- path: data/eval/gold_pairs.scroll_bounded_v1.csv
- rows: 36
- label_column: label
- appears_pair_labels_only: True
- note: Gold labels appear to be pair labels, not necessarily same-DID labels.
