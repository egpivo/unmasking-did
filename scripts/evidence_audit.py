#!/usr/bin/env python3
import argparse
import csv
import json
import sqlite3
from collections import Counter
from pathlib import Path
from typing import Any


SEMANTIC_RULES = {
    "event_evidence": {"funded_by", "transfer", "interaction"},
    "control_evidence": {"safe_owner", "owner", "admin", "deployer"},
    "identity_claim": {"ens", "ens_handle", "profile", "claim", "did", "did_controller"},
    "verified_identity_controller": {
        "did_controller_verified",
        "verified_did",
        "signature_verified",
    },
}

DIRECTED_KINDS = {"funded_by", "transfer", "interaction", "safe_owner", "owner", "admin", "deployer"}


def q1(conn: sqlite3.Connection, sql: str, params: tuple = ()) -> Any:
    cur = conn.execute(sql, params)
    row = cur.fetchone()
    return row[0] if row else None


def list_tables(conn: sqlite3.Connection) -> set[str]:
    cur = conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
    return {r[0] for r in cur.fetchall()}


def table_count(conn: sqlite3.Connection, table: str) -> int | None:
    if table not in list_tables(conn):
        return None
    return int(q1(conn, f"SELECT COUNT(*) FROM {table}") or 0)


def classify_kind(kind: str) -> str:
    for cls, kinds in SEMANTIC_RULES.items():
        if kind in kinds:
            return cls
    return "unknown_unclassified"


def evidence_breakdown(conn: sqlite3.Connection) -> dict[str, Any]:
    out: dict[str, Any] = {}
    rows = conn.execute(
        "SELECT kind, COUNT(*) AS c FROM evidence GROUP BY kind ORDER BY c DESC"
    ).fetchall()
    kind_counts = {k: int(c) for k, c in rows}
    out["kind_counts"] = kind_counts

    src_rows = conn.execute(
        "SELECT source, COUNT(*) AS c FROM evidence GROUP BY source ORDER BY c DESC LIMIT 200"
    ).fetchall()
    out["source_counts"] = {s: int(c) for s, c in src_rows}

    semantic_counter = Counter()
    for k, c in kind_counts.items():
        semantic_counter[classify_kind(k)] += c
    out["semantic_class_counts"] = dict(semantic_counter)

    out["directionality"] = {
        "directed_kinds": sorted([k for k in kind_counts if k in DIRECTED_KINDS]),
        "undirected_or_ambiguous_kinds": sorted([k for k in kind_counts if k not in DIRECTED_KINDS]),
    }

    raw_rows = int(q1(conn, "SELECT COUNT(*) FROM evidence") or 0)
    uniq_pair = int(q1(conn, "SELECT COUNT(DISTINCT address || '|' || key) FROM evidence") or 0)
    uniq_pair_kind = int(
        q1(conn, "SELECT COUNT(DISTINCT address || '|' || kind || '|' || key) FROM evidence") or 0
    )
    uniq_pair_kind_key = uniq_pair_kind
    uniq_source = int(q1(conn, "SELECT COUNT(DISTINCT source) FROM evidence") or 0)
    tx_hash_in_payload = None
    try:
        tx_hash_in_payload = int(
            q1(
                conn,
                "SELECT COUNT(DISTINCT json_extract(payload_json, '$.tx_hash')) "
                "FROM evidence WHERE payload_json IS NOT NULL "
                "AND json_extract(payload_json, '$.tx_hash') IS NOT NULL "
                "AND json_extract(payload_json, '$.tx_hash') <> ''",
            )
            or 0
        )
    except sqlite3.OperationalError:
        tx_hash_in_payload = None

    out["duplication_diagnostics"] = {
        "raw_evidence_rows": raw_rows,
        "unique_pair_count": uniq_pair,
        "unique_pair_kind_count": uniq_pair_kind,
        "unique_pair_kind_key_count": uniq_pair_kind_key,
        "unique_source_count": uniq_source,
        "unique_tx_hash_count": tx_hash_in_payload,
        "raw_to_unique_pair_ratio": (raw_rows / uniq_pair) if uniq_pair else None,
        "raw_to_unique_pair_kind_ratio": (raw_rows / uniq_pair_kind) if uniq_pair_kind else None,
    }
    return out


def temporal_coverage(conn: sqlite3.Connection) -> dict[str, Any]:
    has_block = int(q1(conn, "SELECT COUNT(*) FROM evidence WHERE observed_block > 0") or 0) > 0
    min_block = q1(conn, "SELECT MIN(observed_block) FROM evidence WHERE observed_block > 0")
    max_block = q1(conn, "SELECT MAX(observed_block) FROM evidence WHERE observed_block > 0")

    has_ts = False
    min_ts = None
    max_ts = None
    try:
        ts_count = int(
            q1(
                conn,
                "SELECT COUNT(*) FROM evidence WHERE payload_json IS NOT NULL AND ("
                "json_extract(payload_json, '$.timestamp') IS NOT NULL OR "
                "json_extract(payload_json, '$.observed_at') IS NOT NULL)",
            )
            or 0
        )
        has_ts = ts_count > 0
        if has_ts:
            min_ts = q1(
                conn,
                "SELECT MIN(COALESCE(json_extract(payload_json, '$.timestamp'), "
                "json_extract(payload_json, '$.observed_at'))) "
                "FROM evidence WHERE payload_json IS NOT NULL",
            )
            max_ts = q1(
                conn,
                "SELECT MAX(COALESCE(json_extract(payload_json, '$.timestamp'), "
                "json_extract(payload_json, '$.observed_at'))) "
                "FROM evidence WHERE payload_json IS NOT NULL",
            )
    except sqlite3.OperationalError:
        pass

    return {
        "has_block_number": has_block,
        "min_block": min_block,
        "max_block": max_block,
        "has_timestamp": has_ts,
        "min_timestamp": min_ts,
        "max_timestamp": max_ts,
    }


def readiness(evd: dict[str, Any]) -> dict[str, Any]:
    kinds = set(evd["kind_counts"].keys())
    has_event = any(k in SEMANTIC_RULES["event_evidence"] for k in kinds)
    has_control = any(k in SEMANTIC_RULES["control_evidence"] for k in kinds)
    has_identity = any(k in SEMANTIC_RULES["identity_claim"] for k in kinds)
    has_verified = any(k in SEMANTIC_RULES["verified_identity_controller"] for k in kinds)

    return {
        "can_model_relatedness": {
            "value": has_event or has_control or has_identity or has_verified,
            "reason": "Event/control/identity evidence can support relatedness candidate ranking."
            if (has_event or has_control or has_identity or has_verified)
            else "No meaningful evidence kinds present.",
        },
        "can_model_control_link": {
            "value": has_control,
            "reason": "Control-evidence kinds (e.g. safe_owner/admin/owner) are present."
            if has_control
            else "No explicit control-evidence kinds present.",
        },
        "can_model_identity_claim": {
            "value": has_identity or has_verified,
            "reason": "Identity/DID claim evidence is present."
            if (has_identity or has_verified)
            else "No ENS/DID-like identity claim evidence present.",
        },
        "can_model_same_did_or_shared_controller": {
            "value": has_verified,
            "reason": "Verified DID/controller evidence is present."
            if has_verified
            else "No cryptographically/registry verified DID/controller evidence; unverified DID claims alone are insufficient.",
        },
        "summary": (
            "funded_by-only data can support relatedness candidate ranking, but cannot support DID identity verification."
            if (has_event and not (has_control or has_identity or has_verified))
            else None
        ),
    }


def table_rows(conn: sqlite3.Connection) -> dict[str, int | None]:
    names = [
        "addresses",
        "transfers",
        "ens_records",
        "safe_owners",
        "evidence",
        "clustering_runs",
        "entity_clusters",
    ]
    return {n: table_count(conn, n) for n in names}


def parse_gold(gold_path: Path | None) -> dict[str, Any] | None:
    if gold_path is None:
        return None
    if not gold_path.exists():
        return {"path": str(gold_path), "error": "file_not_found"}

    with gold_path.open("r", newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)
        headers = [h.strip() for h in (reader.fieldnames or [])]

    if not rows:
        return {"path": str(gold_path), "rows": 0}

    label_col = None
    for c in ["label", "is_match", "same_entity", "gold_label", "y"]:
        if c in headers:
            label_col = c
            break

    label_counts = Counter()
    if label_col:
        for r in rows:
            label_counts[(r.get(label_col) or "").strip()] += 1

    has_did_cols = any("did" in h.lower() for h in headers)
    pairish_cols = any(h in headers for h in ["addr_a", "addr_b", "left", "right", "a", "b", "address_a", "address_b"])

    return {
        "path": str(gold_path),
        "rows": len(rows),
        "columns": headers,
        "label_column": label_col,
        "label_counts": dict(label_counts),
        "appears_pair_labels_only": bool(pairish_cols and not has_did_cols),
        "note": "Gold labels appear to be pair labels, not necessarily same-DID labels."
        if (pairish_cols and not has_did_cols)
        else None,
    }


def to_markdown(audit: dict[str, Any]) -> str:
    tr = audit["table_row_counts"]
    ev = audit["evidence"]
    tc = audit["temporal_coverage"]
    rd = audit["probabilistic_readiness"]
    gold = audit.get("gold")
    lines = [
        f"# Evidence Audit: `{audit['db_path']}`",
        "",
        "## Table Row Counts",
        *(f"- {k}: {v}" for k, v in tr.items()),
        "",
        "## Evidence Kinds",
        *(f"- {k}: {v}" for k, v in ev["kind_counts"].items()),
        "",
        "## Semantic Class Counts",
        *(f"- {k}: {v}" for k, v in ev["semantic_class_counts"].items()),
        "",
        "## Temporal Coverage",
        f"- has_block_number: {tc['has_block_number']}",
        f"- min_block: {tc['min_block']}",
        f"- max_block: {tc['max_block']}",
        f"- has_timestamp: {tc['has_timestamp']}",
        f"- min_timestamp: {tc['min_timestamp']}",
        f"- max_timestamp: {tc['max_timestamp']}",
        "",
        "## Duplication/Inflation Diagnostics",
        *(f"- {k}: {v}" for k, v in ev["duplication_diagnostics"].items()),
        "",
        "## Probabilistic Readiness",
        *(f"- {k}: {v['value']} — {v['reason']}" for k, v in rd.items() if isinstance(v, dict)),
    ]
    if rd.get("summary"):
        lines.extend(["", f"- summary: {rd['summary']}"])
    if gold:
        lines.extend(
            [
                "",
                "## Gold Labels",
                f"- path: {gold.get('path')}",
                f"- rows: {gold.get('rows')}",
                f"- label_column: {gold.get('label_column')}",
                f"- appears_pair_labels_only: {gold.get('appears_pair_labels_only')}",
                f"- note: {gold.get('note')}",
            ]
        )
    lines.append("")
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser(description="Evidence inventory / probabilistic readiness audit.")
    ap.add_argument("--db", required=True, help="Path to SQLite DB")
    ap.add_argument("--out", required=True, help="Output JSON path")
    ap.add_argument("--gold", default=None, help="Optional gold CSV path")
    ap.add_argument("--out-md", default=None, help="Optional markdown output path")
    args = ap.parse_args()

    db_path = Path(args.db)
    out_path = Path(args.out)
    out_md = Path(args.out_md) if args.out_md else out_path.with_suffix(".md")
    gold_path = Path(args.gold) if args.gold else None

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row

    ev = evidence_breakdown(conn)
    audit = {
        "db_path": str(db_path),
        "table_row_counts": table_rows(conn),
        "evidence": ev,
        "temporal_coverage": temporal_coverage(conn),
        "probabilistic_readiness": readiness(ev),
        "gold": parse_gold(gold_path),
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(audit, indent=2))
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_md.write_text(to_markdown(audit))
    print(f"wrote {out_path}")
    print(f"wrote {out_md}")


if __name__ == "__main__":
    main()
