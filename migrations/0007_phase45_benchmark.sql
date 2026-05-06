-- Phase 4.5A: controlled synthetic coordination benchmark foundation.
-- Schema is append-only and SQLite-first.

CREATE TABLE IF NOT EXISTS benchmark_runs (
    benchmark_run_id    TEXT PRIMARY KEY,
    scenario_suite_id   TEXT NOT NULL,
    scenario_id         TEXT NOT NULL,
    seed                INTEGER NOT NULL,
    generator_version   TEXT NOT NULL,
    policy_profile_id   TEXT NOT NULL,
    policy_variant      TEXT NOT NULL,
    input_snapshot_hash TEXT NOT NULL,
    code_commit         TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS benchmark_ground_truth_entities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    benchmark_run_id TEXT NOT NULL,
    entity_id        TEXT NOT NULL,
    wallet_id        TEXT NOT NULL,
    cohort           TEXT NOT NULL,
    role_tag         TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs(benchmark_run_id),
    UNIQUE (benchmark_run_id, wallet_id)
);

CREATE TABLE IF NOT EXISTS benchmark_synthetic_evidence (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    benchmark_run_id  TEXT NOT NULL,
    evidence_id       TEXT NOT NULL,
    subject_wallet_id TEXT NOT NULL,
    counterparty_id   TEXT NOT NULL,
    evidence_kind     TEXT NOT NULL,
    strength_hint     TEXT NOT NULL,
    event_time_bucket TEXT,
    sequence_index    INTEGER,
    metadata_json     TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs(benchmark_run_id),
    UNIQUE (benchmark_run_id, evidence_id)
);

CREATE TABLE IF NOT EXISTS benchmark_policy_results (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    benchmark_run_id      TEXT NOT NULL,
    policy_variant        TEXT NOT NULL,
    pred_cluster_id       TEXT NOT NULL,
    wallet_id             TEXT NOT NULL,
    link_explanation_json TEXT,
    created_at            TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs(benchmark_run_id),
    UNIQUE (benchmark_run_id, policy_variant, wallet_id)
);

CREATE TABLE IF NOT EXISTS benchmark_eval_metrics (
    benchmark_run_id               TEXT NOT NULL,
    policy_variant                 TEXT NOT NULL,
    precision                      REAL NOT NULL,
    recall                         REAL NOT NULL,
    f1                             REAL NOT NULL,
    over_merge_rate                REAL NOT NULL,
    under_merge_rate               REAL NOT NULL,
    giant_component_inflation      REAL NOT NULL,
    cluster_purity                 REAL NOT NULL,
    cluster_fragmentation          REAL NOT NULL,
    calibration_json_by_evidence_kind TEXT,
    computed_at                    TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs(benchmark_run_id),
    PRIMARY KEY (benchmark_run_id, policy_variant)
);

CREATE TABLE IF NOT EXISTS benchmark_eval_details (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    benchmark_run_id      TEXT NOT NULL,
    policy_variant        TEXT NOT NULL,
    truth_entity_id       TEXT NOT NULL,
    matched_pred_cluster_id TEXT,
    split_count           INTEGER NOT NULL,
    merge_intrusion_count INTEGER NOT NULL,
    dominant_error_kind   TEXT,
    detail_json           TEXT,
    created_at            TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs(benchmark_run_id)
);

CREATE INDEX IF NOT EXISTS idx_benchmark_runs_scenario_seed
ON benchmark_runs(scenario_id, seed, created_at);

CREATE INDEX IF NOT EXISTS idx_benchmark_runs_profile_variant_created
ON benchmark_runs(policy_profile_id, policy_variant, created_at);

CREATE INDEX IF NOT EXISTS idx_benchmark_truth_run_entity
ON benchmark_ground_truth_entities(benchmark_run_id, entity_id);

CREATE INDEX IF NOT EXISTS idx_benchmark_truth_run_cohort
ON benchmark_ground_truth_entities(benchmark_run_id, cohort);

CREATE INDEX IF NOT EXISTS idx_benchmark_synth_run_kind
ON benchmark_synthetic_evidence(benchmark_run_id, evidence_kind);

CREATE INDEX IF NOT EXISTS idx_benchmark_policy_run_variant_cluster
ON benchmark_policy_results(benchmark_run_id, policy_variant, pred_cluster_id);

CREATE INDEX IF NOT EXISTS idx_benchmark_eval_details_run_variant_entity
ON benchmark_eval_details(benchmark_run_id, policy_variant, truth_entity_id);
