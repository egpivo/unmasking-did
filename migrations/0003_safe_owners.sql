-- Safe multisig ownership cache.
--
-- Each row claims that `safe_address` is a Gnosis Safe whose set of
-- owners includes `owner_address`. `owner_is_safe = 1` flags that the
-- owner is itself a Safe (as opposed to an EOA): per the project's
-- evidence taxonomy, only EOA owners qualify as medium evidence for
-- shared control. Safe-of-safe ownership is recorded for completeness
-- but excluded from edge generation.
--
-- `threshold` and `observed_block` are stored for reference / audit;
-- neither participates in clustering for M2.
CREATE TABLE IF NOT EXISTS safe_owners (
    safe_address   TEXT    NOT NULL,
    owner_address  TEXT    NOT NULL,
    owner_is_safe  INTEGER NOT NULL DEFAULT 0,
    threshold      INTEGER,
    observed_block INTEGER,
    source         TEXT    NOT NULL DEFAULT 'manual',
    created_at     TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (safe_address, owner_address)
);
CREATE INDEX IF NOT EXISTS idx_safe_owners_owner ON safe_owners(owner_address);
