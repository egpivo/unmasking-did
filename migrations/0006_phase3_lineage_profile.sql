-- Phase 3A/3B: policy-profile metadata + lineage status persistence.

-- 1) Dataset-run policy metadata (compatible with existing rows).
ALTER TABLE dataset_runs ADD COLUMN policy_profile_id TEXT;
ALTER TABLE dataset_runs ADD COLUMN stable_threshold REAL;
ALTER TABLE dataset_runs ADD COLUMN related_threshold REAL;

CREATE INDEX IF NOT EXISTS idx_dataset_runs_chain_profile_created
ON dataset_runs(chain, policy_profile_id, created_at);

-- Backfill conservative defaults for historical rows.
-- TODO: for future multi-chain shared DBs, prefer `legacy_or_unknown_v1`
-- unless run params explicitly prove this conservative profile.
UPDATE dataset_runs
SET policy_profile_id = COALESCE(policy_profile_id, 'arbitrum_gov_conservative_v1'),
    stable_threshold = COALESCE(stable_threshold, 0.5),
    related_threshold = COALESCE(related_threshold, 0.1);

-- 2) Lineage table needs nullable sides for `new`/`disappeared`.
ALTER TABLE cluster_lineage RENAME TO cluster_lineage_old;

CREATE TABLE cluster_lineage (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id_current    TEXT,
    cluster_id_current TEXT,
    run_id_previous   TEXT,
    cluster_id_previous TEXT,
    overlap_count     INTEGER NOT NULL,
    jaccard           REAL NOT NULL,
    transition_label  TEXT NOT NULL,
    computed_at       TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id_current) REFERENCES dataset_runs(run_id),
    FOREIGN KEY (run_id_previous) REFERENCES dataset_runs(run_id)
);

INSERT INTO cluster_lineage (
    run_id_current, cluster_id_current, run_id_previous, cluster_id_previous,
    overlap_count, jaccard, transition_label, computed_at
)
SELECT
    run_id_current, cluster_id_current, run_id_previous, cluster_id_previous,
    overlap_count, jaccard, COALESCE(transition_label, 'related'), computed_at
FROM cluster_lineage_old;

DROP TABLE cluster_lineage_old;

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_current
ON cluster_lineage(run_id_current, cluster_id_current);

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_previous
ON cluster_lineage(run_id_previous, cluster_id_previous);

CREATE INDEX IF NOT EXISTS idx_cluster_lineage_transition
ON cluster_lineage(transition_label);
