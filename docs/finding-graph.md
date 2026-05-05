# Finding graph (D3)

A bounded JSON view of a clustering run, plus a single-page D3 v7
viewer for inspecting it interactively. Complements the existing
Markdown / DOT / JSON outputs from `report`:

- **Markdown** (`report --format md`) — narrative, blog-ready.
- **DOT** (`report --format dot`) — Graphviz; convert to SVG with
  the standard `dot` CLI (see the
  [README's "Optional: render the DOT to SVG"](../README.md#optional-render-the-dot-to-svg)
  section for the one-liner).
- **JSON** (`report --format json`) — same data the Markdown report
  shows, structured for piping.
- **Finding graph** (`export-graph`, this page) — D3-friendly
  `{ nodes, links, limits }` shape; renders interactively with the
  static `viewer/viewer.html` page.

## What it shows

Two node kinds:

- **identifier** — a clustering subject (a wallet / Safe address).
  One node per cluster member.
- **evidence** — one node per unique `(kind, key)` value, e.g. one
  EOA owner that multiple Safes share, or one shared funder. Two
  identifiers connected to the same evidence node make the share
  visible at a glance.

Links go identifier → evidence, one per attestation that survived
the bounding filters. Link type is the evidence kind
(`safe_owner`, `funded_by`, `ens_handle`, `did_controller`); link
strength is the attestation strength (`weak` / `medium` / `strong`).

## Bounded by construction

- **`depth = 1`** — only direct evidence on cluster members. No
  recursive expansion (a funder's other funders are never added).
- **`max_identifier_nodes`** (default 50) — caps subjects with
  whole-cluster semantics: a cluster is either fully included or
  skipped. If a single cluster on its own would push past the cap,
  that cluster is skipped and iteration continues — smaller
  clusters can still fit. Skipped cluster IDs are listed in
  `limits.applied_filters`. Partial slices of a cluster are never
  emitted, so a viewer can never read a half-rendered cluster as
  the full thing.
- **`max_evidence_nodes`** (default 200) — caps `(kind, key)`
  evidence nodes.
- **`fan_out_cap`** (default 50) — `(kind, key)` groups whose
  **run-level** member count exceeds the cap are dropped (services
  / bridges / CEX hot wallets that fan out to many recipients),
  matching the linker's clustering behavior in `linking::features`.
  Important: fan-out is computed across the FULL run before any
  identifier truncation, so a service-like key shared by N+1
  addresses cannot sneak through by being represented by only N
  identifiers in the truncated subset.
- **CEX blacklist** — already applied at extraction time in
  `extract_funded_by`, so blacklisted hot wallets never reach the
  evidence table to begin with.

When a cap was hit, `limits.truncated_identifiers` /
`limits.truncated_evidence` is `true` in the JSON, and the
`applied_filters` list documents what was active.

## Scroll vs Ethereum mainnet (what to open in the viewer)

**Scroll** — small, bounded seed sets (e.g. DAO governance Safes): a
**full qualitative graph** is appropriate. Default `export-graph`
limits are usually plenty; the committed example graph under
`data/findings/` remains the default **demo / inspection** target.

**Ethereum mainnet** — large DAO / treasury ingests: the SQLite
`evidence` table can contain **tens or hundreds of thousands of
attestation rows**. Do **not** treat that row count as something to
render in D3 or Graphviz by default.

- **Qualitative (Scroll-style) inspection** stays on Scroll (or other
  small L2 runs).
- **Mainnet inspection** should be **capped summaries + diagnostics**:
  - **Bounded graph JSON** — this exporter already collapses many
    rows into **one node per unique `(kind, key)`** and caps the
    number of those evidence nodes with `--max-evidence-nodes`
    (default **200**), plus identifier caps and fan-out filtering.
    You are **not** exporting 77k evidence *nodes* when defaults
    hold; you are exporting at most 200 evidence *keys* (plus
    identifiers), subject to `limits` in the JSON.
  - **Tighter caps for the viewer** — lower `--max-identifier-nodes`,
    `--max-evidence-nodes`, and (in pairwise mode)
    `--max-pairwise-links` if the force layout is sluggish.
  - **Pairwise diagnostic view** — `--graph-mode pairwise` emits
    identifier–identifier scored edges (good for “top linkage
    pressure” at a glance); keep `--max-pairwise-links` modest for
    interactive use.
  - **SQL summaries over the DB** — without any new dashboard:
    [`scripts/graph_diag.sql`](../scripts/graph_diag.sql) prints
    evidence kind counts, top shared `(kind, key)` keys, per-address
    row counts, source duplication, strength histogram, key fan-out
    distribution, and latest-run cluster sizes. Run with
    `sqlite3 <path-to-your.db> < scripts/graph_diag.sql`.
  - **`report --format dot`** — DOT is built from merge-passing edges
    for the **full** run; on mainnet-scale data it can still become
    heavy. Prefer **`export-graph` with conservative caps** or SQL
    diagnostics first; treat full DOT as opt-in.

**TODO (backlog, not implemented here):** optional **capped DOT** export
that mirrors `export-graph` truncation rules for Graphviz users who
want `.dot` without hand-filtering — until then, use JSON export +
viewer or SQL summaries.

## Workflow

The export reads from `entity_clusters` / `evidence`; it does not
re-cluster. So a clustering run has to be persisted first, against
the **same `DATABASE_URL`** you're going to export from. Common
gotcha: running `link` against one DB and `export-graph` against
another (e.g. the project default `sqlite://data/unmask.db`) — the
second command then errors with `no clustering runs found`. If you
hit that, double-check `echo $DATABASE_URL` and the `DATABASE_URL=`
line in `.env`.

1. Persist a clustering run.

   **(a) Quick offline sanity check** — no Alchemy / Safe Tx
   Service / ENS calls needed; uses synthetic evidence injected via
   the `add-*` CLIs. Good for verifying the pipeline locally:

   ```bash
   # Two Safes sharing one EOA owner -> 1 multi-member cluster.
   cargo run -- add-safe-owner \
     --safe  0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1 \
     --owner 0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0
   cargo run -- add-safe-owner \
     --safe  0xb2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2 \
     --owner 0xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0
   cargo run -- link --min-evidence 1
   ```

   **(b) Real-network ingest** — for an actual finding. Requires
   `ALCHEMY_API_KEY`, `SAFE_TX_SERVICE_URL`, etc. configured in
   `.env`. See the project README quickstart and the existing
   `data/findings/` artifacts for concrete address sets:

   ```bash
   cargo run -- ingest --address 0x20fa362323447506d9d0c02483ae97c4e2d6b607
   # ... repeat for the rest of your seed set ...
   cargo run -- link --min-evidence 2
   ```

2. Export the graph:

   ```bash
   mkdir -p out
   cargo run -- export-graph --out out/graph.json
   # or with non-default bounds:
   cargo run -- export-graph --out out/graph.json \
     --max-identifier-nodes 30 --max-evidence-nodes 100 --fan-out-cap 30
   ```

   **Ethereum mainnet (interactive viewer):** keep the JSON small
   enough for D3 force layout — for example:

   ```bash
   cargo run -- export-graph --out out/mainnet-graph.json \
     --max-identifier-nodes 40 --max-evidence-nodes 120 --fan-out-cap 50 \
     --graph-mode pairwise --max-pairwise-links 400
   ```

   Pairwise mode ignores `max_evidence_nodes` for the edge list (it
   scores pairs directly); `--max-pairwise-links` is the relevant cap.

3. View it.

   **Read live SQLite (recommended):** from the repo root, after
   `link`:

   ```bash
   cargo run -- serve --port 8080
   # open http://127.0.0.1:8080/viewer/graph-explorer.html
   ```

   The server exposes **`GET /api/graph`** — same graph builder as
   `export-graph`, backed by **`DATABASE_URL`**. Optional query
   parameters: `mode` (`evidence` \| `pairwise`),
   `max_identifier_nodes`, `max_evidence_nodes`, `fan_out_cap`,
   `max_pairwise_links`, `linkage_params` (file path, pairwise only).
   Static files are also served under `/viewer`, `/out`, and
   `/data/findings/`. **`make serve-app`** wraps the same command.

   **Auto-load from disk (via HTTP):** `viewer/graph-explorer.html` —
   same force layout; when served from the repo root (`make serve`),
   it **fetches** `/api/graph` (live DB when using `cargo run -- serve`),
   then `out/graph.json`, then the committed Scroll example
   `data/findings/2026-05-01-scroll-dao-safes.graph.json`, unless you set
   `?graph=relative/path.json`. **Drag nodes** to rearrange; pan on the
   background; scroll to zoom. **Clear graph** re-runs auto-load; **Open
   other file** uses the file picker as a fallback.

   The original viewer is `viewer/viewer.html` — a single static D3 v7 page,
   no build step. Two ways to load:

   **(a) File picker (works from `file://`).** Open
   `viewer/viewer.html` directly in a browser, click the file
   picker at the top, choose your `out/graph.json`. Always works,
   no server needed.

   **(b) Same-directory auto-fetch (works over HTTP).** Copy the
   viewer next to the JSON and serve them together — the viewer
   tries `fetch("graph.json")` from its own URL on load:

   ```bash
   cp viewer/viewer.html out/
   cd out && python3 -m http.server 8000
   # open http://localhost:8000/viewer.html
   ```

   The browser blocks `fetch()` for `file://` URIs, which is why
   path (a) needs the file picker fallback.

## Scope this is NOT

- **Not a frontend.** No backend calls, no auth, no interactivity
  beyond drag/zoom. The viewer is one ~200-line HTML file with a
  CDN reference to D3.
- **Not real-time.** The graph is a snapshot of the persisted run;
  re-running `link` and `export-graph` regenerates the file.
- **Not historical.** Like the Markdown / DOT outputs, this
  reflects the *current* `evidence` snapshot. Persisting per-run
  edges remains backlog (see `src/report/edges.rs` notes).

## First committed graph

[`data/findings/2026-05-01-scroll-dao-safes.graph.json`](../data/findings/2026-05-01-scroll-dao-safes.graph.json)
is the bounded graph for the 6 Scroll DAO governance Safes from
the [first finding](../data/findings/2026-05-01-scroll-dao-safes.md).
6 identifier nodes + 17 evidence nodes (11 EOA owners across the
Treasury+committee ladder, 6 distinct funders), 30 links, no
truncation.

The JSON's `run_id` differs from the run_id in the paired finding's
Markdown — the graph was exported from a later equivalent re-ingest
(same inputs, same Safe Tx Service URL, same `--min-evidence 2`,
byte-identical cluster shape). See the "Graph artifact" section in
the finding's Markdown for the full reasoning.
