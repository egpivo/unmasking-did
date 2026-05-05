# Monthly Monitoring Schema Proposal (SQLite-first, Postgres-ready)

This is a **design-only** proposal for extending the current data model into a repeatable monthly coordination-monitoring pipeline.

Scope:
- Keep SQLite as primary backend.
- Preserve run-scoped reproducibility.
- Avoid overclaiming attack detection.
- Keep SQL portable where practical so a future Postgres migration is straightforward.

## Design Principles

- Keep storage access behind `Repo` methods.
- Use migrations for all schema changes.
- Treat each run as immutable: append, do not overwrite prior run outputs.
- Record reproducibility metadata per run: parameters, block range, input hash, code commit.
- Favor portable SQL patterns:
  - Use standard `INSERT ... ON CONFLICT DO UPDATE` where possible.
  - Avoid engine-specific features unless behind repository abstractions.

## Existing Tables to Keep

Current tables already cover core ingestion and evidence:
- `addresses`
- `transfers`
- `ens_records`
- `safe_owners`
- `did_documents`
- `evidence`
- `clustering_runs`
- `entity_clusters`
- `suspected_service_keys`

## Proposed Additions

### 1) Dataset Runs (top-level run metadata)

```sql
CREATE TABLE IF NOT EXISTS dataset_runs (
    run_id              TEXT PRIMARY KEY,
    chain               TEXT NOT NULL,
    window_start_block  INTEGER NOT NULL,
    window_end_block    INTEGER NOT NULL,
    window_start_ts     TEXT,
    window_end_ts       TEXT,
    cadence             TEXT NOT NULL, -- e.g. "monthly", "weekly"
    seed_spec_json      TEXT NOT NULL, -- objective-linked seed definition
    params_json         TEXT NOT NULL, -- pipeline params snapshot
    input_snapshot_hash TEXT NOT NULL, -- deterministic hash of seed+params+window
    code_commit         TEXT NOT NULL, -- git commit used for run
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    notes               TEXT
);
```

Purpose:
- One canonical row per monthly/weekly run.
- Anchors all monitoring outputs and enables reproducibility.

### 2) Run Inputs (auditable seed provenance)

```sql
CREATE TABLE IF NOT EXISTS run_inputs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id       TEXT NOT NULL,
    input_type   TEXT NOT NULL, -- contract, address, label_set, query_spec
    input_ref    TEXT NOT NULL, -- canonical id/address/slug
    source       TEXT NOT NULL, -- manual, github-list, protocol-doc, etc.
    metadata_json TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);
```

Purpose:
- Explicit provenance for seed set construction.
- Makes article claims auditable.

### 3) Run Metrics (dataset-level KPIs)

```sql
CREATE TABLE IF NOT EXISTS run_metrics (
    run_id                     TEXT PRIMARY KEY,
    num_seed_inputs            INTEGER NOT NULL,
    num_seed_addresses         INTEGER NOT NULL,
    num_addresses_total        INTEGER NOT NULL,
    num_transfers              INTEGER NOT NULL,
    num_evidence_rows          INTEGER NOT NULL,
    num_clusters               INTEGER NOT NULL,
    num_multi_address_clusters INTEGER NOT NULL,
    top_cluster_size           INTEGER NOT NULL,
    metadata_json              TEXT, -- additional aggregate diagnostics
    computed_at                TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);
```

Purpose:
- Standard run summary for month-over-month trend lines.

### 4) Cluster Metrics (per-cluster features per run)

```sql
CREATE TABLE IF NOT EXISTS cluster_metrics (
    run_id                    TEXT NOT NULL,
    cluster_id                TEXT NOT NULL,
    num_addresses             INTEGER NOT NULL,
    num_identifiers           INTEGER NOT NULL,
    num_evidence_rows         INTEGER NOT NULL,
    num_unique_funders        INTEGER,
    top_funder                TEXT,
    top_funder_share          REAL,
    first_funder_shared_count INTEGER,
    funding_block_min         INTEGER,
    funding_block_max         INTEGER,
    funding_block_span        INTEGER,
    funding_burst_label       TEXT,
    shared_safe_owner_count   INTEGER,
    control_link_density      REAL,
    num_unique_sinks          INTEGER,
    top_sink                  TEXT,
    top_sink_share            REAL,
    possible_consolidation    INTEGER, -- 0/1
    sybil_risk_tier           TEXT,    -- low / candidate_medium / candidate_high
    reasons_json              TEXT,
    computed_at               TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (run_id, cluster_id),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);
```

Purpose:
- Stable, queryable diagnostics table for reports and dashboards.
- Keeps conservative tiering explicit and auditable.

### 5) Cluster Lineage (month-over-month evolution)

```sql
CREATE TABLE IF NOT EXISTS cluster_lineage (
    run_id_current       TEXT NOT NULL,
    cluster_id_current   TEXT NOT NULL,
    run_id_previous      TEXT NOT NULL,
    cluster_id_previous  TEXT NOT NULL,
    overlap_count        INTEGER NOT NULL,
    jaccard              REAL NOT NULL,
    transition_label     TEXT NOT NULL, -- stable/grown/shrunk/split/merged/new/retired
    computed_at          TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (run_id_current, cluster_id_current, run_id_previous, cluster_id_previous),
    FOREIGN KEY (run_id_current) REFERENCES dataset_runs(run_id),
    FOREIGN KEY (run_id_previous) REFERENCES dataset_runs(run_id)
);
```

Purpose:
- Stores deterministic run-to-run cluster matching.
- Enables “cluster evolution” framing without attack labeling.

### 6) Graph Exports (artifact registry)

```sql
CREATE TABLE IF NOT EXISTS graph_exports (
    run_id             TEXT PRIMARY KEY,
    graph_json_path    TEXT NOT NULL,
    graph_json_hash    TEXT NOT NULL,
    report_md_path     TEXT NOT NULL,
    report_md_hash     TEXT NOT NULL,
    viewer_path        TEXT,
    generated_at       TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);
```

Purpose:
- Connects DB run rows to exported artifacts for reproducible sharing.

## Index Plan

Indexes should prioritize common read paths:

```sql
-- dataset runs
CREATE INDEX IF NOT EXISTS idx_dataset_runs_chain_created
ON dataset_runs(chain, created_at);

CREATE INDEX IF NOT EXISTS idx_dataset_runs_window
ON dataset_runs(chain, window_start_block, window_end_block);

CREATE INDEX IF NOT EXISTS idx_dataset_runs_input_hash
ON dataset_runs(input_snapshot_hash);

-- run inputs
CREATE INDEX IF NOT EXISTS idx_run_inputs_run
ON run_inputs(run_id);

CREATE INDEX IF NOT EXISTS idx_run_inputs_type_ref
ON run_inputs(input_type, input_ref);

-- transfers (add to existing indexes)
CREATE INDEX IF NOT EXISTS idx_transfers_to_block
ON transfers(to_addr, block_num);

CREATE INDEX IF NOT EXISTS idx_transfers_from_block
ON transfers(from_addr, block_num);

CREATE INDEX IF NOT EXISTS idx_transfers_tx_hash
ON transfers(tx_hash);

-- evidence (add to existing indexes)
CREATE INDEX IF NOT EXISTS idx_evidence_observed_block
ON evidence(observed_block);

CREATE INDEX IF NOT EXISTS idx_evidence_kind_observed_block
ON evidence(kind, observed_block);

CREATE INDEX IF NOT EXISTS idx_evidence_key
ON evidence(key);

-- clusters
CREATE INDEX IF NOT EXISTS idx_entity_clusters_run_cluster
ON entity_clusters(cluster_run_id, cluster_id);

CREATE INDEX IF NOT EXISTS idx_entity_clusters_run_address
ON entity_clusters(cluster_run_id, address);

-- cluster metrics
CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_tier
ON cluster_metrics(run_id, sybil_risk_tier);

CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_top_funder_share
ON cluster_metrics(run_id, top_funder_share);

CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_top_sink_share
ON cluster_metrics(run_id, top_sink_share);

-- lineage
CREATE INDEX IF NOT EXISTS idx_cluster_lineage_current
ON cluster_lineage(run_id_current, cluster_id_current);

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_previous
ON cluster_lineage(run_id_previous, cluster_id_previous);
```

## Month-over-Month Comparison Model

Recommended deterministic approach:

1. Compute address-set overlap between run `t` and run `t-1` clusters.
2. Use Jaccard score `|A∩B| / |A∪B|` to identify best predecessor links.
3. Persist all candidate links over threshold (e.g. `>= 0.25`) in `cluster_lineage`.
4. Derive transition labels:
   - `stable`: high overlap, minor size change
   - `grown` / `shrunk`
   - `split` / `merged`
   - `new` / `retired`
5. Compare feature drift from `cluster_metrics`:
   - funder concentration delta
   - sink concentration delta
   - burst label changes
   - control-density changes

## Repository-Layer Boundary (no direct table coupling in callers)

Continue placing all storage operations behind `Repo` methods. Suggested additions:

- `start_dataset_run(...)`
- `insert_run_inputs(...)`
- `upsert_run_metrics(...)`
- `upsert_cluster_metrics(...)`
- `insert_cluster_lineage(...)`
- `upsert_graph_export(...)`
- `latest_dataset_run_for_chain(...)`
- `cluster_metrics_for_run(...)`
- `compare_runs(prev_run_id, curr_run_id, ...)`

This keeps SQL/backend concerns isolated and makes eventual Postgres migration low-risk.

## Conservative Reporting Contract

Run outputs should keep language explicitly non-accusatory:

- “coordination monitoring”
- “identity-linking evidence graph”
- “shared-control signals”
- “operational coupling”
- “cluster evolution over time”

Do **not** label clusters as confirmed Sybil attacks without objective-specific evidence (e.g., reward abuse, vote manipulation, reputation abuse).

## Minimal Evidence Threshold for Credible Monthly Monitoring

For a publishable monitoring update (without overclaiming attack detection):

- At least 2 sequential runs with fixed process and bounded window.
- At least 300 addresses and 3,000+ evidence rows in-scope.
- At least 10 multi-address clusters.
- At least 5 clusters with meaningful shared-funder or burst/coordinated-funding signal.
- At least 3 clusters with sink concentration / consolidation-style pattern.
- At least 1 strong negative-control example (looks suspicious but plausibly benign).
- Reproducibility fields present for every run (`input_snapshot_hash`, params, block range, code commit).

This supports a credible claim that the system detects and tracks **coordination evidence**, not confirmed malicious behavior.
