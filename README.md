# unmasking-did

> Measuring the gap between decentralized identifiers and decentralized entities.

[![CI](https://github.com/egpivo/unmasking-did/actions/workflows/ci.yml/badge.svg)](https://github.com/egpivo/unmasking-did/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/egpivo/unmasking-did/graph/badge.svg?token=xwbzGXaFZF)](https://codecov.io/gh/egpivo/unmasking-did)

`unmasking-did` is an auditable, evidence-based coordination analysis pipeline.
It is **not** Sybil detection, deanonymization, or real-world identity attribution.

## Architecture (high-level)

```mermaid
flowchart LR
  A["Seed cohorts<br/>fixed window"]
  B["Bounded ingest<br/>Alchemy / Safe / ENS / DID"]
  C["SQLite cache<br/>typed evidence"]
  D["Conservative linker<br/>service suppression"]
  E["Monitoring artifacts<br/>summary / graph / report"]
  F["Static viewer"]

  A --> B --> C --> D --> E --> F
```

The pipeline keeps raw evidence separate from merge policy. Funding links are
treated as coordination signals, not identity proof.

## Quickstart (canonical run)

```bash
cp .env.example .env
# set ARBITRUM_ALCHEMY_API_KEY (or fallback ALCHEMY_API_KEY)

cargo run --release -- arbitrum-gov --overwrite-db
make serve-viewer
# open http://localhost:8000/viewer/index.html
```

## Canonical outputs

- `out/arbitrum_gov_summary.json`
- `out/arbitrum_gov.graph.json`
- `out/arbitrum_gov_report.md`

The static viewer reads these artifacts directly. It does not query RPC, does
not re-run clustering, and does not render the historical baseline graph; the
baseline section is metrics-only.

## Interpretation boundary

Evidence families such as `funded_by`, `safe_owner`, `ens_handle`, and
`did_controller` are not real-world identity claims. The conservative policy
suppresses service-like infrastructure and avoids treating funding alone as
identity.

Monitoring runs are grouped by `policy_profile_id`; lineage is compared only
within the same profile.

## Key docs

- Run spec: [docs/run-spec-arbitrum-gov-90d.md](docs/run-spec-arbitrum-gov-90d.md)
- Findings: [docs/findings/arbitrum-governance-coordination-v0.md](docs/findings/arbitrum-governance-coordination-v0.md)
- Monitoring spec: [docs/phase3_monitoring_product_spec.md](docs/phase3_monitoring_product_spec.md)

## License

MIT — see [LICENSE](LICENSE).
