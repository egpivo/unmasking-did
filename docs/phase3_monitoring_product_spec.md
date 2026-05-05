# Phase 3 Monitoring Product Spec

Status: proposal (no implementation in this document)  
Scope: reproducible coordination monitoring product layer over validated Phase 2 conservative pipeline.

## Policy Profile

Introduce a first-class **policy profile** identifier (example: `arbitrum_gov_conservative_v1`).

A policy profile must explicitly bundle:

- chain
- cadence
- seed recipe/specification
- conservative funded_by parameters
- funded_by service fan-out cap
- merge-rule configuration
- frozen-window rule (how the block window is defined/frozen)

Core rule:

- runs are comparable as a continuous trend **only within the same policy profile**

## 1) Cadence

Supported cadences:

- `weekly`
- `monthly`

Recommended default for rollout:

- start with `monthly` for operational stability
- add `weekly` once run reliability and lineage quality are stable for 2+ consecutive cycles

Each run must be immutable and comparable to prior runs under the same policy profile.

## 2) Run Identity

Every monitoring run must carry a strict identity triple:

- `run_id`: unique run identifier (append-only)
- `window`: frozen `[start_block, end_block]` (+ optional UTC timestamps)
- `input_snapshot_hash`: deterministic hash of seed specification + policy parameters + window

Required identity metadata (minimum):

- chain (`arbitrum`)
- cadence (`weekly` or `monthly`)
- policy profile id (e.g. `arbitrum_gov_conservative_v1`)
- conservative funded_by policy parameters
- code version / commit

## 3) Outputs

Each run must produce and persist:

- summary JSON (machine-readable run metrics)
- Markdown report (human-readable interpretation with caveats)
- graph JSON (bounded interactive visualization payload)
- `cluster_metrics` table rows (per-cluster structured metrics)

Suggested filesystem convention:

- `out/monitor/<run_id>/summary.json`
- `out/monitor/<run_id>/report.md`
- `out/monitor/<run_id>/graph.json`

## 4) Comparison / Lineage

Run-to-run comparison is required for monitoring value.

For each cluster in run `N`, compute lineage status versus run `N-1`:

- `stable`: Jaccard overlap `>= stable_threshold` (default `0.5`)
- `related`: `related_threshold <= Jaccard < stable_threshold` (default `0.1 <= Jaccard < 0.5`)
- `new`: no predecessor above `related_threshold`
- `disappeared`: prior cluster has no successor above `related_threshold`

Thresholds must be configurable and recorded per run in metadata/params so lineage decisions are reproducible.

Lineage should be stored explicitly (not inferred ad hoc from reports), with overlap stats and decision reason.

## 4.1 No-Policy-Drift Rule

If any of the following changes, create a **new policy profile**:

- seed recipe/specification
- evidence policy (e.g. funded_by gating/suppression)
- merge rules

Do not compare runs across policy profiles as one continuous trend line.

Cross-profile comparisons are allowed only as explicit side-by-side experiments with clear caveats.

## 5) Alerting (optional, later phase)

Optional alerts after baseline stabilization:

- significant cluster size increase (absolute or relative threshold)
- appearance of a new multi-address coordination cluster above a configured size threshold

Alerts are product signals for analyst review, not conclusions.

## 6) Storage Model

Reuse the existing monitoring schema proposal:

- `dataset_runs` for top-level run identity and reproducibility
- `cluster_lineage` for run-to-run cluster transitions
- `run_metrics` + `cluster_metrics` for dashboard/report inputs

`dataset_runs` should include:

- `policy_profile_id`
- lineage thresholds used for the run (`stable_threshold`, `related_threshold`)

Do not overwrite prior runs; append-only history.

## 7) Constraints / Interpretation Policy

Hard constraints for all outputs:

- coordination framing only
- no claim of attack attribution
- no Sybil-detection claim
- no real-world identity inference

All run outputs must include caveat text stating:

- governance participation does not imply malicious behavior
- shared infrastructure evidence does not imply same human operator

## Delivery Checklist (per run)

- frozen window recorded
- identity triple recorded (`run_id`, `window`, `input_snapshot_hash`)
- policy profile id recorded
- lineage thresholds recorded
- summary/report/graph emitted
- `cluster_metrics` persisted
- lineage computed against prior run
- interpretation constraints present in report
