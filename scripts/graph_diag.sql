-- Diagnostic summaries over SQLite evidence / clusters (mainnet-scale).
-- Does not render graphs; safe to run on large `evidence` tables.
--
-- Usage (path is the file part of DATABASE_URL, without sqlite://):
--   sqlite3 data/unmask_eval_mainnet_v1.db < scripts/graph_diag.sql
--
-- Use the same DB you ran `link` against before `export-graph`.

.mode column
.headers on

SELECT 'evidence_row_count' AS metric, COUNT(*) AS value FROM evidence;

SELECT '--- evidence kind counts ---' AS section;
SELECT kind, COUNT(*) AS n FROM evidence GROUP BY kind ORDER BY n DESC;

SELECT '--- top 25 (kind,key) by attestation rows ---' AS section;
SELECT kind, key, COUNT(*) AS attestations
FROM evidence
GROUP BY kind, key
ORDER BY attestations DESC
LIMIT 25;

SELECT '--- top 20 addresses by attestation count ---' AS section;
SELECT address, COUNT(*) AS attestations
FROM evidence
GROUP BY address
ORDER BY attestations DESC
LIMIT 20;

SELECT '--- source duplication ---' AS section;
SELECT source, COUNT(*) AS n FROM evidence GROUP BY source ORDER BY n DESC;

SELECT '--- strength code counts (1=weak,2=medium,3=strong) ---' AS section;
SELECT strength, COUNT(*) AS n FROM evidence GROUP BY strength ORDER BY strength DESC;

SELECT '--- distribution: how many distinct keys touch N addresses ---' AS section;
SELECT addr_fanout AS distinct_addresses_per_key, COUNT(*) AS how_many_keys
FROM (
  SELECT kind, key, COUNT(DISTINCT address) AS addr_fanout
  FROM evidence
  GROUP BY kind, key
)
GROUP BY addr_fanout
ORDER BY addr_fanout DESC;

SELECT '--- latest run: cluster size distribution ---' AS section;
SELECT cluster_id, COUNT(*) AS cluster_size
FROM entity_clusters
WHERE cluster_run_id = (
  SELECT run_id FROM clustering_runs ORDER BY started_at DESC LIMIT 1
)
GROUP BY cluster_id
ORDER BY cluster_size DESC;
