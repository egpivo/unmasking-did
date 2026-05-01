-- DID document cache.
--
-- Each row records one observed DID document. `subject_address` is the
-- address embedded in the DID (e.g. `did:ethr:0xabc...` -> `0xabc...`);
-- `controller` is the address authorised to update the DID document.
-- When `controller != subject_address`, the relationship is non-trivial
-- and emitted as STRONG `did_controller` evidence by the extractor —
-- the cluster-merge invariant lets a single such edge bypass
-- `min_evidence` because cryptographic-level shared control is the
-- highest-strength signal in the project's evidence taxonomy.
--
-- Self-controlled DIDs (controller == subject_address, the default for
-- a freshly minted did:ethr or any did:pkh) are recorded for audit but
-- produce NO clustering edge: every address trivially controls its own
-- did:pkh, so emitting that as evidence would be tautological.
CREATE TABLE IF NOT EXISTS did_documents (
    did             TEXT    PRIMARY KEY,
    subject_address TEXT    NOT NULL,
    controller      TEXT    NOT NULL,
    method          TEXT    NOT NULL,
    document_json   TEXT,
    observed_block  INTEGER,
    source          TEXT    NOT NULL DEFAULT 'manual',
    created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_did_documents_subject    ON did_documents(subject_address);
CREATE INDEX IF NOT EXISTS idx_did_documents_controller ON did_documents(controller);
