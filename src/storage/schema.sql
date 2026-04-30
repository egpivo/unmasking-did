-- Canonical schema. Mirrors migrations/0001_init.sql.
-- Kept here for reference and for embedded bootstrap when running tests
-- against an in-memory SQLite without a migrations directory.

CREATE TABLE IF NOT EXISTS addresses (
    address          TEXT PRIMARY KEY,
    first_seen_block INTEGER,
    label            TEXT
);

CREATE TABLE IF NOT EXISTS transfers (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    from_addr TEXT NOT NULL,
    to_addr   TEXT NOT NULL,
    value     TEXT,
    block_num INTEGER,
    tx_hash   TEXT,
    asset     TEXT,
    UNIQUE (tx_hash, from_addr, to_addr, asset, value)
);

CREATE INDEX IF NOT EXISTS idx_transfers_to   ON transfers(to_addr);
CREATE INDEX IF NOT EXISTS idx_transfers_from ON transfers(from_addr);

CREATE TABLE IF NOT EXISTS ens_records (
    address  TEXT PRIMARY KEY,
    name     TEXT,
    twitter  TEXT,
    github   TEXT,
    telegram TEXT
);

CREATE TABLE IF NOT EXISTS entity_clusters (
    cluster_id    INTEGER NOT NULL,
    address       TEXT    NOT NULL,
    evidence_json TEXT,
    PRIMARY KEY (cluster_id, address)
);
