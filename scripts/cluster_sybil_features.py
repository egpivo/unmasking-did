#!/usr/bin/env python3
"""
Cluster-level Sybil-style *coordination* diagnostics from cached SQLite only.

This script does not call RPC, does not train models, and does not assert
confirmed attacks. Output tiers are heuristic labels (candidate_low / medium / high risk).
"""

from __future__ import annotations

import argparse
import csv
import json
import sqlite3
from collections import Counter, defaultdict
from itertools import combinations
from pathlib import Path
from typing import Any

# Forward-compatible with evidence kinds that may appear in DB exports.
VERIFIED_KINDS = frozenset(
    {"did_controller_verified", "verified_did", "signature_verified"}
)
# Non-verified DID-like rows (verified kinds counted separately).
DID_KINDS = frozenset({"did", "did_controller"})
ENS_KINDS = frozenset({"ens", "ens_handle"})


def q1(conn: sqlite3.Connection, sql: str, params: tuple[Any, ...] = ()) -> Any:
    row = conn.execute(sql, params).fetchone()
    return row[0] if row else None


def list_tables(conn: sqlite3.Connection) -> set[str]:
    cur = conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
    return {r[0] for r in cur.fetchall()}


def table_columns(conn: sqlite3.Connection, table: str) -> list[str] | None:
    if table not in list_tables(conn):
        return None
    cur = conn.execute(f"PRAGMA table_info({table})")
    return [r[1] for r in cur.fetchall()]


def pick_latest_cluster_run(conn: sqlite3.Connection) -> str | None:
    if "clustering_runs" not in list_tables(conn) or "entity_clusters" not in list_tables(conn):
        return None
    row = conn.execute(
        "SELECT run_id FROM clustering_runs ORDER BY started_at DESC LIMIT 1"
    ).fetchone()
    return str(row[0]) if row else None


def load_clusters(conn: sqlite3.Connection, run_id: str) -> dict[str, set[str]]:
    rows = conn.execute(
        "SELECT cluster_id, address FROM entity_clusters WHERE cluster_run_id = ?",
        (run_id,),
    ).fetchall()
    out: dict[str, set[str]] = defaultdict(set)
    for cid, addr in rows:
        out[str(cid)].add(str(addr).lower())
    return dict(out)


def tier_order(t: str) -> int:
    return {"candidate_high": 0, "candidate_medium": 1, "low": 2}.get(t, 3)


def temporal_pattern(block_span: int | None, short_burst_blocks: int) -> str:
    if block_span is None:
        return "unknown"
    if block_span == 0:
        return "same_block"
    if block_span <= short_burst_blocks:
        return "short_burst"
    return "long_span"


def extract_payload_tokens(payload: str | None) -> tuple[set[str], set[str]]:
    """Return (contracts, methods) from a single payload_json string, best-effort."""
    contracts: set[str] = set()
    methods: set[str] = set()
    if not payload or not payload.strip():
        return contracts, methods
    try:
        obj = json.loads(payload)
    except json.JSONDecodeError:
        return contracts, methods
    if not isinstance(obj, dict):
        return contracts, methods

    def add_str(key: str, dest: set[str]) -> None:
        v = obj.get(key)
        if isinstance(v, str) and v.strip():
            dest.add(v.strip().lower())
        elif isinstance(v, list):
            for x in v:
                if isinstance(x, str) and x.strip():
                    dest.add(x.strip().lower())

    for k in ("contract", "contract_address", "to", "address"):
        add_str(k, contracts)
    for k in ("method_id", "method", "selector", "sig", "function_selector"):
        add_str(k, methods)
    return contracts, methods


def first_funders_from_transfers(
    conn: sqlite3.Connection,
    addrs: set[str],
    transfer_cols: list[str] | None,
) -> dict[str, str | None]:
    """Per-address first incoming transfer funder (by block), lowercased funder or None."""
    if not transfer_cols or "transfers" not in list_tables(conn):
        return {a: None for a in addrs}
    need = {"to_addr", "from_addr", "block_num"}
    if not need.issubset(set(transfer_cols)):
        return {a: None for a in addrs}
    out: dict[str, str | None] = {a: None for a in addrs}
    if not addrs:
        return out
    alist = sorted(addrs)
    chunk = 400
    for i in range(0, len(alist), chunk):
        part = alist[i : i + chunk]
        ph = ",".join("?" * len(part))
        # ROW_NUMBER so chunked queries do not miss the true global first row per recipient.
        try:
            rows = conn.execute(
                f"""
                SELECT to_l, from_addr FROM (
                  SELECT lower(to_addr) AS to_l, from_addr, block_num, id,
                         ROW_NUMBER() OVER (PARTITION BY lower(to_addr) ORDER BY block_num ASC, id ASC) AS rn
                  FROM transfers
                  WHERE lower(to_addr) IN ({ph})
                    AND block_num IS NOT NULL
                ) x
                WHERE rn = 1
                """,
                part,
            ).fetchall()
        except sqlite3.OperationalError:
            continue
        for to_l, from_a in rows:
            t = str(to_l).lower()
            if t in out and out[t] is None:
                out[t] = str(from_a).lower()
    return out


def sink_stats_from_transfers(
    conn: sqlite3.Connection,
    addrs: set[str],
    transfer_cols: list[str] | None,
) -> tuple[int, str | None, int, float | None]:
    """Outgoing transfers from cluster members to non-members."""
    if not addrs or not transfer_cols or "transfers" not in list_tables(conn):
        return 0, None, 0, None
    need = {"from_addr", "to_addr"}
    if not need.issubset(set(transfer_cols)):
        return 0, None, 0, None
    alist = sorted(addrs)
    counts: Counter[str] = Counter()
    chunk = 400
    for i in range(0, len(alist), chunk):
        part = alist[i : i + chunk]
        ph = ",".join("?" * len(part))
        rows = conn.execute(
            f"""
            SELECT lower(to_addr) AS t, COUNT(*) AS c
            FROM transfers
            WHERE lower(from_addr) IN ({ph})
              AND lower(to_addr) NOT IN ({ph})
            GROUP BY lower(to_addr)
            """,
            part + part,
        ).fetchall()
        for t, c in rows:
            counts[str(t)] += int(c)
    if not counts:
        return 0, None, 0, None
    top_sink, top_n = counts.most_common(1)[0]
    total = sum(counts.values())
    share = top_n / total if total else None
    return len(counts), top_sink, top_n, share


def compute_cluster_features(
    cluster_id: str,
    addrs: set[str],
    evidence_rows: list[sqlite3.Row],
    conn: sqlite3.Connection,
    transfer_cols: list[str] | None,
    short_burst_blocks: int,
    overlap_available: bool,
) -> dict[str, Any]:
    n_addr = len(addrs)
    # --- identifiers: member addresses + distinct identity keys on those addresses ---
    idents: set[str] = set(addrs)
    for r in evidence_rows:
        k = str(r["kind"])
        if k in DID_KINDS | VERIFIED_KINDS | ENS_KINDS | {"claim", "profile"}:
            key = str(r["key"]).strip()
            if key:
                idents.add(key.lower() if key.startswith("0x") else key)

    num_evidence_rows = len(evidence_rows)

    # --- funding from funded_by evidence ---
    fb_blocks: list[int] = []
    for r in evidence_rows:
        if str(r["kind"]) != "funded_by":
            continue
        ob = r["observed_block"]
        if ob is not None and int(ob) > 0:
            fb_blocks.append(int(ob))

    distinct_per_funder: dict[str, set[str]] = defaultdict(set)
    for r in evidence_rows:
        if str(r["kind"]) != "funded_by":
            continue
        distinct_per_funder[str(r["key"]).lower()].add(str(r["address"]).lower())
    num_unique_funders = len(distinct_per_funder)
    top_funder = None
    top_funder_count = 0
    top_funder_share: float | None = None
    if distinct_per_funder and n_addr > 0:
        top_funder = max(distinct_per_funder.keys(), key=lambda k: len(distinct_per_funder[k]))
        top_funder_count = len(distinct_per_funder[top_funder])
        top_funder_share = top_funder_count / n_addr

    funding_block_min = min(fb_blocks) if fb_blocks else None
    funding_block_max = max(fb_blocks) if fb_blocks else None
    funding_block_span: int | None = None
    if funding_block_min is not None and funding_block_max is not None:
        funding_block_span = funding_block_max - funding_block_min
    funding_burst_label = temporal_pattern(funding_block_span, short_burst_blocks)

    first_map = first_funders_from_transfers(conn, addrs, transfer_cols)
    first_funder_shared_count: int | None = None
    if any(v is not None for v in first_map.values()):
        fc = Counter([v for v in first_map.values() if v is not None])
        first_funder_shared_count = fc.most_common(1)[0][1] if fc else 0

    # --- safe_owner: address = safe, key = owner (per extract.rs) ---
    safe_rows = [r for r in evidence_rows if str(r["kind"]) == "safe_owner"]
    safe_owner_count = len(safe_rows)
    owner_to_safes: dict[str, set[str]] = defaultdict(set)
    for r in safe_rows:
        owner_to_safes[str(r["key"]).lower()].add(str(r["address"]).lower())
    unique_safe_owners = len(owner_to_safes)
    shared_safe_owner_count = sum(1 for _o, ss in owner_to_safes.items() if len(ss) >= 2)

    control_link_density: float | None = None
    if n_addr >= 2:
        max_pairs = n_addr * (n_addr - 1) / 2.0
        edge_pairs: set[tuple[str, str]] = set()
        for _owner, safes in owner_to_safes.items():
            if len(safes) < 2:
                continue
            for a, b in combinations(sorted(s for s in safes if s in addrs), 2):
                edge_pairs.add((a, b))
        control_link_density = len(edge_pairs) / max_pairs if max_pairs else None

    # --- identity ---
    ens_count = sum(1 for r in evidence_rows if str(r["kind"]) in ENS_KINDS)
    did_count = sum(1 for r in evidence_rows if str(r["kind"]) in DID_KINDS)
    verified_did_count = sum(1 for r in evidence_rows if str(r["kind"]) in VERIFIED_KINDS)
    did_verified_present = verified_did_count > 0

    # --- sinks ---
    num_unique_sinks, top_sink, top_sink_count, top_sink_share = sink_stats_from_transfers(
        conn, addrs, transfer_cols
    )
    possible_consolidation = False
    if top_sink_share is not None and n_addr >= 2:
        possible_consolidation = top_sink_share >= 0.55 and num_unique_sinks <= 4

    # --- behavioral overlap (optional) ---
    contract_overlap_summary: str
    method_overlap_summary: str
    if not overlap_available:
        contract_overlap_summary = "unavailable: json_extract / payload scan not supported or skipped"
        method_overlap_summary = contract_overlap_summary
    else:
        addr_contracts: dict[str, set[str]] = {a: set() for a in addrs}
        addr_methods: dict[str, set[str]] = {a: set() for a in addrs}
        for r in evidence_rows:
            p = r["payload_json"] if "payload_json" in r.keys() else None
            cset, mset = extract_payload_tokens(str(p) if p else None)
            a = str(r["address"]).lower()
            if a in addr_contracts:
                addr_contracts[a] |= cset
                addr_methods[a] |= mset
        nonempty_c = sum(1 for s in addr_contracts.values() if s)
        nonempty_m = sum(1 for s in addr_methods.values() if s)
        if nonempty_c < 2:
            contract_overlap_summary = (
                f"sparse_or_absent: addresses_with_contract_tokens={nonempty_c}/{n_addr}"
            )
        else:
            addrs_c = [a for a, s in addr_contracts.items() if s]
            jsums = []
            for x, y in combinations(addrs_c, 2):
                sx, sy = addr_contracts[x], addr_contracts[y]
                u = sx | sy
                j = len(sx & sy) / len(u) if u else 0.0
                jsums.append(j)
            mj = sum(jsums) / len(jsums) if jsums else 0.0
            contract_overlap_summary = (
                f"mean_pairwise_jaccard={mj:.4f} (addrs_with_tokens={len(addrs_c)}/{n_addr})"
            )
        if nonempty_m < 2:
            method_overlap_summary = (
                f"sparse_or_absent: addresses_with_method_tokens={nonempty_m}/{n_addr}"
            )
        else:
            addrs_m = [a for a, s in addr_methods.items() if s]
            jsums = []
            for x, y in combinations(addrs_m, 2):
                sx, sy = addr_methods[x], addr_methods[y]
                u = sx | sy
                j = len(sx & sy) / len(u) if u else 0.0
                jsums.append(j)
            mj = sum(jsums) / len(jsums) if jsums else 0.0
            method_overlap_summary = (
                f"mean_pairwise_jaccard={mj:.4f} (addrs_with_tokens={len(addrs_m)}/{n_addr})"
            )

    # --- tier (conservative, transparent) ---
    reasons: list[str] = []

    cond_high_funding = (
        n_addr > 2
        and (top_funder_share or 0) >= 0.65
        and funding_burst_label in ("same_block", "short_burst")
    )
    cond_high_sink = n_addr > 2 and (top_sink_share or 0) >= 0.6 and possible_consolidation

    cond_medium_funding = (
        n_addr >= 2
        and (top_funder_share or 0) >= 0.35
        and (top_funder_share or 0) < 0.65
        and funding_burst_label != "long_span"
    )
    cond_medium_sink = n_addr >= 2 and (top_sink_share or 0) >= 0.35 and (top_sink_share or 0) < 0.6
    cond_medium_shared_owner = shared_safe_owner_count >= 1 and n_addr >= 2
    cond_medium_single_funder = num_unique_funders == 1 and n_addr >= 2

    if cond_high_funding or cond_high_sink:
        tier = "candidate_high"
        if cond_high_funding:
            reasons.append(
                "candidate_high: cluster_size>2 with concentrated funder "
                f"(top_funder_share>={0.65}) and tight funding timing ({funding_burst_label})"
            )
        if cond_high_sink:
            reasons.append(
                "candidate_high: consolidation-like outbound pattern "
                f"(top_sink_share>={0.6}, few distinct sinks)"
            )
    elif cond_medium_funding or cond_medium_sink or cond_medium_shared_owner or cond_medium_single_funder:
        tier = "candidate_medium"
        if cond_medium_funding:
            reasons.append(
                "candidate_medium: moderate shared funder concentration without full high-tier pattern"
            )
        if cond_medium_sink:
            reasons.append(
                "candidate_medium: non-trivial shared sink mass (below high-tier threshold)"
            )
        if cond_medium_shared_owner:
            reasons.append(
                "candidate_medium: shared Safe owner across>=2 safes (control relation; not malicious alone)"
            )
        if cond_medium_single_funder:
            reasons.append("candidate_medium: single unique funder across>=2 members")
    else:
        tier = "low"
        reasons.append("low: no coordination heuristic fired beyond baseline")

    return {
        "cluster_id": cluster_id,
        "num_identifiers": len(idents),
        "num_addresses": n_addr,
        "num_evidence_rows": num_evidence_rows,
        "num_unique_funders": num_unique_funders,
        "top_funder": top_funder,
        "top_funder_count": top_funder_count,
        "top_funder_share": top_funder_share,
        "first_funder_shared_count": first_funder_shared_count,
        "funding_block_min": funding_block_min,
        "funding_block_max": funding_block_max,
        "funding_block_span": funding_block_span,
        "funding_burst_label": funding_burst_label,
        "safe_owner_count": safe_owner_count,
        "unique_safe_owners": unique_safe_owners,
        "shared_safe_owner_count": shared_safe_owner_count,
        "control_link_density": control_link_density,
        "ens_count": ens_count,
        "did_count": did_count,
        "verified_did_count": verified_did_count,
        "did_verified_present": did_verified_present,
        "num_unique_sinks": num_unique_sinks,
        "top_sink": top_sink,
        "top_sink_count": top_sink_count,
        "top_sink_share": top_sink_share,
        "possible_consolidation": possible_consolidation,
        "contract_overlap_summary": contract_overlap_summary,
        "method_overlap_summary": method_overlap_summary,
        "sybil_risk_tier": tier,
        "sybil_risk_reasons": reasons,
    }


def load_evidence_for_addresses(
    conn: sqlite3.Connection,
    all_addrs: set[str],
    evidence_cols: list[str] | None,
) -> tuple[dict[str, list[sqlite3.Row]], list[sqlite3.Row]]:
    """Rows keyed by `evidence.address`, plus `safe_owner` rows whose owner (`key`) is in the set."""
    if not evidence_cols or "evidence" not in list_tables(conn) or "address" not in evidence_cols:
        return {}, []
    out: dict[str, list[sqlite3.Row]] = defaultdict(list)
    safe_owner_by_owner: list[sqlite3.Row] = []
    if not all_addrs:
        return {}, []
    base_cols = ["address", "kind", "key", "source", "observed_block"]
    if "payload_json" in evidence_cols:
        base_cols.append("payload_json")
    base_cols = [c for c in base_cols if c in evidence_cols]
    alist = sorted(all_addrs)
    chunk = 500
    for i in range(0, len(alist), chunk):
        part = alist[i : i + chunk]
        ph = ",".join("?" * len(part))
        q = f"SELECT {', '.join(base_cols)} FROM evidence WHERE lower(address) IN ({ph})"
        for row in conn.execute(q, part):
            out[str(row["address"]).lower()].append(row)
    if "kind" in evidence_cols:
        for i in range(0, len(alist), chunk):
            part = alist[i : i + chunk]
            ph = ",".join("?" * len(part))
            try:
                for row in conn.execute(
                    f"SELECT {', '.join(base_cols)} FROM evidence WHERE kind = 'safe_owner' "
                    f"AND lower(key) IN ({ph})",
                    part,
                ):
                    safe_owner_by_owner.append(row)
            except sqlite3.OperationalError:
                break
    return dict(out), safe_owner_by_owner


def dataset_summary(
    conn: sqlite3.Connection,
    run_id: str | None,
    clusters: dict[str, set[str]],
) -> dict[str, Any]:
    tables = sorted(list_tables(conn))
    summ: dict[str, Any] = {
        "tables_present": tables,
        "clustering_run_id": run_id,
        "num_clusters": len(clusters),
        "num_clustered_addresses": sum(len(s) for s in clusters.values()),
    }
    for t in ("evidence", "transfers", "entity_clusters", "clustering_runs", "safe_owners", "did_documents"):
        if t in tables:
            summ[f"row_count_{t}"] = int(q1(conn, f"SELECT COUNT(*) FROM {t}") or 0)
    return summ


def write_csv(path: Path, rows: list[dict[str, Any]]) -> None:
    if not rows:
        path.write_text("", encoding="utf-8")
        return
    flat: list[dict[str, Any]] = []
    for r in rows:
        d = dict(r)
        d["sybil_risk_reasons"] = json.dumps(d.get("sybil_risk_reasons") or [])
        flat.append(d)
    keys = sorted({k for row in flat for k in row.keys()})
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        for row in flat:
            w.writerow({k: row.get(k) for k in keys})


def write_markdown(
    path: Path,
    db_path: Path,
    meta: dict[str, Any],
    rows: list[dict[str, Any]],
) -> None:
    high = [r for r in rows if r.get("sybil_risk_tier") == "candidate_high"]
    high.sort(key=lambda r: (-(r.get("num_addresses") or 0), str(r.get("cluster_id"))))

    by_funder = sorted(
        [r for r in rows if (r.get("top_funder_share") or 0) > 0],
        key=lambda r: (-(r.get("top_funder_share") or 0), -(r.get("num_addresses") or 0)),
    )[:20]

    by_sink = sorted(
        [r for r in rows if (r.get("top_sink_share") or 0) > 0],
        key=lambda r: (-(r.get("top_sink_share") or 0), -(r.get("num_addresses") or 0)),
    )[:20]

    lines: list[str] = []
    lines.append("# Cluster Sybil-style coordination diagnostics\n")
    lines.append("## Dataset summary\n")
    lines.append(f"- **Database**: `{db_path}`\n")
    for k, v in meta.get("dataset", {}).items():
        lines.append(f"- **{k}**: `{v}`\n")
    lines.append(f"- **Heuristic constants**: `{json.dumps(meta.get('tier_constants'), indent=2)}`\n")

    lines.append("\n## Top `candidate_high` clusters\n")
    if not high:
        lines.append("_None in this snapshot._\n")
    else:
        lines.append("| cluster_id | n_addr | top_funder_share | burst | top_sink_share | tier_reasons (abridged) |\n")
        lines.append("|---|---:|---:|---|---:|---|\n")
        for r in high[:25]:
            rs = "; ".join((r.get("sybil_risk_reasons") or [])[:2])
            lines.append(
                f"| `{r['cluster_id']}` | {r['num_addresses']} | {r.get('top_funder_share')} | "
                f"{r.get('funding_burst_label')} | {r.get('top_sink_share')} | {rs[:160]} |\n"
            )

    lines.append("\n## Top common-funder clusters (by `top_funder_share`)\n")
    lines.append("| cluster_id | n_addr | top_funder | share | unique funders |\n")
    lines.append("|---|---:|---|---:|---:|\n")
    for r in by_funder[:20]:
        lines.append(
            f"| `{r['cluster_id']}` | {r['num_addresses']} | `{r.get('top_funder')}` | "
            f"{r.get('top_funder_share')} | {r.get('num_unique_funders')} |\n"
        )

    lines.append("\n## Top common-sink clusters (by `top_sink_share`)\n")
    lines.append("| cluster_id | n_addr | top_sink | share | unique sinks |\n")
    lines.append("|---|---:|---|---:|---:|\n")
    for r in by_sink[:20]:
        lines.append(
            f"| `{r['cluster_id']}` | {r['num_addresses']} | `{r.get('top_sink')}` | "
            f"{r.get('top_sink_share')} | {r.get('num_unique_sinks')} |\n"
        )

    lines.append("\n## Caveats\n")
    lines.append(
        "- These are **Sybil-style coordination features**, not proof of an attack.\n"
    )
    lines.append(
        "- An actual Sybil attack requires a **target objective** such as airdrop farming, "
        "vote manipulation, or reputation abuse.\n"
    )
    lines.append(
        "- **Verified DID** evidence is reported separately; it does **not** automatically imply Sybil behavior.\n"
    )
    lines.append(
        "- **Shared Safe ownership** describes a control relation; it is **not** inherently malicious.\n"
    )
    lines.append(
        "- Metrics are **heuristic** and may be inflated by benign batch funding, custodial patterns, or sparse transfer caches.\n"
    )

    path.write_text("".join(lines), encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description="Cluster-level Sybil-style coordination features (SQLite-only).")
    ap.add_argument("--db", required=True, type=Path, help="Path to sqlite db")
    ap.add_argument("--out", required=True, type=Path, help="Output .json or .csv")
    ap.add_argument("--out-md", required=True, type=Path, help="Markdown summary path")
    ap.add_argument(
        "--short-burst-blocks",
        type=int,
        default=100,
        help="Max block span still labeled short_burst (matches pair_baseline default).",
    )
    ap.add_argument(
        "--run-id",
        type=str,
        default=None,
        help="clustering_runs.run_id to use (default: latest by started_at).",
    )
    args = ap.parse_args()

    db_path: Path = args.db
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row

    run_id = args.run_id or pick_latest_cluster_run(conn)
    evidence_cols = table_columns(conn, "evidence")
    transfer_cols = table_columns(conn, "transfers")

    overlap_available = bool(evidence_cols and "payload_json" in evidence_cols)

    tier_constants = {
        "high_top_funder_share_min": 0.65,
        "high_top_sink_share_min": 0.6,
        "possible_consolidation_top_sink_share_min": 0.55,
        "possible_consolidation_max_unique_sinks": 4,
        "medium_top_funder_share_min": 0.35,
        "medium_top_sink_share_min": 0.35,
        "short_burst_blocks": args.short_burst_blocks,
    }

    clusters: dict[str, set[str]] = {}
    if run_id:
        clusters = load_clusters(conn, run_id)

    all_addrs: set[str] = set()
    for s in clusters.values():
        all_addrs |= s

    by_addr, so_by_owner = load_evidence_for_addresses(conn, all_addrs, evidence_cols)

    rows: list[dict[str, Any]] = []
    for cid, addrs in sorted(clusters.items(), key=lambda x: x[0]):
        ev = []
        for a in addrs:
            ev.extend(by_addr.get(a, []))
        ev.extend(r for r in so_by_owner if str(r["key"]).lower() in addrs)
        # Dedupe (safe_owner can appear both via safe address and owner key paths)
        seen: set[tuple[str, str, str, str]] = set()
        uniq_ev: list[sqlite3.Row] = []
        for r in ev:
            src = ""
            if "source" in r.keys() and r["source"] is not None:
                src = str(r["source"])
            keyt = (str(r["address"]).lower(), str(r["kind"]), str(r["key"]).lower(), src)
            if keyt in seen:
                continue
            seen.add(keyt)
            uniq_ev.append(r)
        ev = uniq_ev
        feat = compute_cluster_features(
            cid,
            addrs,
            ev,
            conn,
            transfer_cols,
            args.short_burst_blocks,
            overlap_available,
        )
        rows.append(feat)

    # Stable sort: tier then size desc
    rows.sort(key=lambda r: (tier_order(str(r.get("sybil_risk_tier"))), -int(r.get("num_addresses") or 0), str(r.get("cluster_id"))))

    meta = {
        "db_path": str(db_path.resolve()),
        "clustering_run_id": run_id,
        "dataset": dataset_summary(conn, run_id, clusters),
        "tier_constants": tier_constants,
        "notes": [
            "No RPC; no ML; linker semantics unchanged.",
            "Tiers are candidate labels only (low / candidate_medium / candidate_high).",
        ],
    }
    conn.close()

    out_path: Path = args.out
    out_path.parent.mkdir(parents=True, exist_ok=True)
    args.out_md.parent.mkdir(parents=True, exist_ok=True)

    if out_path.suffix.lower() == ".csv":
        write_csv(out_path, rows)
    else:
        payload = {"meta": meta, "clusters": rows}
        out_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")

    write_markdown(args.out_md, db_path, meta, rows)


if __name__ == "__main__":
    main()
