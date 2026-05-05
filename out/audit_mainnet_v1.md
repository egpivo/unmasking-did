# Evidence Audit: `data/unmask_eval_mainnet_v1.db`

## Table Row Counts
- addresses: 20
- transfers: 206001
- ens_records: 20
- safe_owners: 28
- evidence: 77759
- clustering_runs: 1
- entity_clusters: 20

## Evidence Kinds
- funded_by: 77743
- safe_owner: 16

## Semantic Class Counts
- event_evidence: 77743
- control_evidence: 16

## Temporal Coverage
- has_block_number: True
- min_block: 9456662
- max_block: 24999155
- has_timestamp: False
- min_timestamp: None
- max_timestamp: None

## Duplication/Inflation Diagnostics
- raw_evidence_rows: 77759
- unique_pair_count: 77757
- unique_pair_kind_count: 77759
- unique_pair_kind_key_count: 77759
- unique_source_count: 73588
- unique_tx_hash_count: 0
- raw_to_unique_pair_ratio: 1.0000257211569377
- raw_to_unique_pair_kind_ratio: 1.0

## Probabilistic Readiness
- can_model_relatedness: True — Event/control/identity evidence can support relatedness candidate ranking.
- can_model_control_link: True — Control-evidence kinds (e.g. safe_owner/admin/owner) are present.
- can_model_identity_claim: False — No ENS/DID-like identity claim evidence present.
- can_model_same_did_or_shared_controller: False — No cryptographically/registry verified DID/controller evidence; unverified DID claims alone are insufficient.

## Gold Labels
- path: data/eval/gold_pairs.mainnet_bounded_v1.csv
- rows: 190
- label_column: label
- appears_pair_labels_only: True
- note: Gold labels appear to be pair labels, not necessarily same-DID labels.
