-- Monthly/weekly coordination monitoring metadata and artifacts.
-- SQLite-first schema; SQL kept portable for eventual Postgres migration.

CREATE TABLE IF NOT EXISTS dataset_runs (
    run_id              TEXT PRIMARY KEY,
    chain               TEXT NOT NULL,
    run_type            TEXT NOT NULL DEFAULT 'monthly',
    parent_run_id       TEXT,
    window_start_block  INTEGER NOT NULL,
    window_end_block    INTEGER NOT NULL,
    window_start_ts     TEXT,
    window_end_ts       TEXT,
    cadence             TEXT NOT NULL,
    seed_spec_json      TEXT NOT NULL,
    params_json         TEXT NOT NULL,
    input_snapshot_hash TEXT NOT NULL,
    code_commit         TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    notes               TEXT,
    FOREIGN KEY (parent_run_id) REFERENCES dataset_runs(run_id)
);

CREATE TABLE IF NOT EXISTS run_inputs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id        TEXT NOT NULL,
    input_type    TEXT NOT NULL,
    input_ref     TEXT NOT NULL,
    source        TEXT NOT NULL,
    metadata_json TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);

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
    metadata_json              TEXT,
    computed_at                TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);

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
    possible_consolidation    INTEGER,
    coordination_tier         TEXT NOT NULL,
    coordination_reasons_json TEXT,
    computed_at               TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (run_id, cluster_id),
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);

CREATE TABLE IF NOT EXISTS cluster_lineage (
    run_id_current      TEXT NOT NULL,
    cluster_id_current  TEXT NOT NULL,
    run_id_previous     TEXT NOT NULL,
    cluster_id_previous TEXT NOT NULL,
    overlap_count       INTEGER NOT NULL,
    jaccard             REAL NOT NULL,
    transition_label    TEXT,
    computed_at         TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (run_id_current, cluster_id_current, run_id_previous, cluster_id_previous),
    FOREIGN KEY (run_id_current) REFERENCES dataset_runs(run_id),
    FOREIGN KEY (run_id_previous) REFERENCES dataset_runs(run_id)
);

CREATE TABLE IF NOT EXISTS graph_exports (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id        TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    path          TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    generated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    metadata_json TEXT,
    FOREIGN KEY (run_id) REFERENCES dataset_runs(run_id)
);

CREATE INDEX IF NOT EXISTS idx_dataset_runs_chain_created
ON dataset_runs(chain, created_at);

CREATE INDEX IF NOT EXISTS idx_dataset_runs_window
ON dataset_runs(chain, window_start_block, window_end_block);

CREATE INDEX IF NOT EXISTS idx_dataset_runs_input_hash
ON dataset_runs(input_snapshot_hash);

CREATE INDEX IF NOT EXISTS idx_run_inputs_run
ON run_inputs(run_id);

CREATE INDEX IF NOT EXISTS idx_run_inputs_type_ref
ON run_inputs(input_type, input_ref);

CREATE INDEX IF NOT EXISTS idx_transfers_to_block
ON transfers(to_addr, block_num);

CREATE INDEX IF NOT EXISTS idx_transfers_from_block
ON transfers(from_addr, block_num);

CREATE INDEX IF NOT EXISTS idx_transfers_tx_hash
ON transfers(tx_hash);

CREATE INDEX IF NOT EXISTS idx_evidence_observed_block
ON evidence(observed_block);

CREATE INDEX IF NOT EXISTS idx_evidence_kind_observed_block
ON evidence(kind, observed_block);

CREATE INDEX IF NOT EXISTS idx_evidence_key
ON evidence(key);

CREATE INDEX IF NOT EXISTS idx_entity_clusters_run_cluster
ON entity_clusters(cluster_run_id, cluster_id);

CREATE INDEX IF NOT EXISTS idx_entity_clusters_run_address
ON entity_clusters(cluster_run_id, address);

CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_coordination_tier
ON cluster_metrics(run_id, coordination_tier);

CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_top_funder_share
ON cluster_metrics(run_id, top_funder_share);

CREATE INDEX IF NOT EXISTS idx_cluster_metrics_run_top_sink_share
ON cluster_metrics(run_id, top_sink_share);

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_current
ON cluster_lineage(run_id_current, cluster_id_current);

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_previous
ON cluster_lineage(run_id_previous, cluster_id_previous);

CREATE INDEX IF NOT EXISTS idx_graph_exports_run_type
ON graph_exports(run_id, artifact_type);
