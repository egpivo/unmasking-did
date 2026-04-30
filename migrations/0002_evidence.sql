-- Typed evidence attestations. Append-only.
-- Each row claims `address` exhibits property (kind, key) with `strength`,
-- observed at `observed_block` from `source`. Two addresses sharing the
-- same (kind, key) become a clustering edge at graph-build time.
CREATE TABLE IF NOT EXISTS evidence (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    address        TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    key            TEXT    NOT NULL,
    strength       INTEGER NOT NULL,
    source         TEXT    NOT NULL,
    observed_block INTEGER NOT NULL,
    payload_json   TEXT,
    created_at     TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE (address, kind, key, source)
);
CREATE INDEX IF NOT EXISTS idx_evidence_kind_key ON evidence(kind, key);
CREATE INDEX IF NOT EXISTS idx_evidence_address  ON evidence(address);

-- (kind, key) groups whose fan-out exceeded the cap during a clustering
-- run. Append-only: keeps a behavioral history of which keys behave like
-- services even when not in any hardcoded blacklist.
CREATE TABLE IF NOT EXISTS suspected_service_keys (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    cluster_run_id TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    key            TEXT    NOT NULL,
    fan_out        INTEGER NOT NULL,
    detected_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_ssk_kind_key ON suspected_service_keys(kind, key);
CREATE INDEX IF NOT EXISTS idx_ssk_run      ON suspected_service_keys(cluster_run_id);

-- One row per `cargo run -- link` invocation. Anchors every cluster row
-- and every suspected-service-key row to the parameters that produced it.
CREATE TABLE IF NOT EXISTS clustering_runs (
    run_id      TEXT    PRIMARY KEY,
    params_json TEXT    NOT NULL,
    started_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    notes       TEXT
);

-- Replace M1's empty entity_clusters with the audit-aware shape.
-- cluster_id is now TEXT and equal to the minimum address in the cluster
-- (deterministic, human-readable). The old INTEGER auto-counter was
-- non-deterministic across runs — it violated the project's reproducibility
-- rule. The table was empty in M1 so DROP+CREATE is lossless.
DROP TABLE IF EXISTS entity_clusters;
CREATE TABLE entity_clusters (
    cluster_run_id TEXT NOT NULL,
    cluster_id     TEXT NOT NULL,
    address        TEXT NOT NULL,
    evidence_json  TEXT,
    computed_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (cluster_run_id, cluster_id, address),
    FOREIGN KEY (cluster_run_id) REFERENCES clustering_runs(run_id)
);
CREATE INDEX IF NOT EXISTS idx_entity_clusters_run ON entity_clusters(cluster_run_id);
