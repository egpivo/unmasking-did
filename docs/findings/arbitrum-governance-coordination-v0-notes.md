# Arbitrum Governance Coordination v0 Notes

Companion notes for `arbitrum-governance-coordination-v0.md`.

## Artifact Provenance

- Conservative summary: `out/phase2_relink_conservative_summary.json`
- Markdown report (latest run): `out/phase2_arbitrum_gov_report.md`
- Graph JSON (latest run): `out/phase2_arbitrum_gov.graph.json`
- SQLite store: `data/unmask_arbitrum_gov_v1.db`

Latest conservative run metadata:

- `run_id`: `run-1777967119432576`
- `params_json`: `{"address_count":1000,"fan_out_cap":50,"funded_by_policy":{"enabled":true,"min_shared_keys":2,"min_short_burst_hits":2,"service_fan_out_cap":50,"short_burst_block_delta":5000},"min_evidence":1}`

## Key Quantitative Snapshot

- `cluster_count`: 976
- `top_cluster_size`: 5
- `multi_address_clusters`: 13
- top-5 clusters are all non-mixed governance/control (`mixed_gov_control=false`)

## Scope Guardrails (for publication)

- Do not call this Sybil detection.
- Do not infer real-world identity.
- Present as coordination evidence under conservative policy.
- Include caveats about funded_by fragility and sparse DID depth.

## Suggested Figure Pair (for article)

1. “Before policy hardening” shape summary (giant mixed cluster)
2. “After conservative policy” shape summary (small, non-mixed top clusters)

Keep the figure caption explicit that policy choice materially changes cluster topology.
