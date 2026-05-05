#!/usr/bin/env python3
import argparse
import csv
import itertools
import json
import math
import sqlite3
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


SEMANTIC_CLASS = {
    "funded_by": "event",
    "transfer": "event",
    "interaction": "event",
    "safe_owner": "control",
    "owner": "control",
    "admin": "control",
    "deployer": "control",
    "ens": "identity",
    "ens_handle": "identity",
    "profile": "identity",
    "claim": "identity",
    "did": "identity",
    "did_controller": "identity",
    "did_controller_verified": "verified",
    "verified_did": "verified",
    "signature_verified": "verified",
}

DIRECTED_KINDS = {"funded_by", "transfer", "interaction", "safe_owner", "owner", "admin", "deployer"}
DEFAULT_SHORT_BURST_BLOCKS = 100


def q1(conn: sqlite3.Connection, sql: str) -> Any:
    row = conn.execute(sql).fetchone()
    return row[0] if row else None


def classify_kind(kind: str) -> str:
    return SEMANTIC_CLASS.get(kind, "unknown")


def load_addresses(conn: sqlite3.Connection) -> list[str]:
    rows = conn.execute("SELECT address FROM addresses ORDER BY address").fetchall()
    return [r[0] for r in rows]


def parse_gold(path: Path | None) -> dict[tuple[str, str], str]:
    if path is None or not path.exists():
        return {}
    with path.open("r", newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)
        cols = set(reader.fieldnames or [])

    a_col = None
    b_col = None
    for x, y in [
        ("addr_a", "addr_b"),
        ("address_a", "address_b"),
        ("left", "right"),
        ("a", "b"),
    ]:
        if x in cols and y in cols:
            a_col, b_col = x, y
            break
    if not a_col:
        return {}

    label_col = None
    for c in ["label", "same_control", "gold_label", "is_match", "same_entity", "y"]:
        if c in cols:
            label_col = c
            break
    if not label_col:
        return {}

    out: dict[tuple[str, str], str] = {}
    for r in rows:
        a = (r.get(a_col) or "").strip().lower()
        b = (r.get(b_col) or "").strip().lower()
        if not a or not b:
            continue
        key = tuple(sorted((a, b)))
        out[key] = (r.get(label_col) or "").strip()
    return out


def calc_scores(feat: dict[str, Any]) -> tuple[float, float, float]:
    kinds = feat["semantic_classes_present"]
    event = "event" in kinds
    control = "control" in kinds
    identity = "identity" in kinds
    verified = "verified" in kinds
    hub_penalty = feat["hub_penalty"]

    related = 0.0
    control_score = 0.0
    did = 0.0

    if event:
        related += min(0.55, 0.14 * feat["funded_by_count"]) * hub_penalty
    if control:
        related += min(0.35, 0.12 * feat["safe_owner_count"]) * hub_penalty
        control_score += min(0.9, 0.22 * feat["safe_owner_count"]) * hub_penalty
    if identity:
        related += min(0.3, 0.1 * feat["identity_count"]) * hub_penalty
    if verified:
        related += min(0.25, 0.1 * feat["verified_count"]) * hub_penalty
        did += min(0.95, 0.35 * feat["verified_count"]) * hub_penalty

    # Conservative caps + constraints.
    related = min(1.0, related)
    control_score = min(1.0, control_score)
    did = min(1.0, did)
    if not verified:
        did = 0.0
    return related, control_score, did


def has_any_evidence(feat: dict[str, Any]) -> bool:
    return evidence_total(feat) > 0


def recommend_semantic_floor(feat: dict[str, Any]) -> str:
    """Recommendation follows evidence class, not score thresholds (scores rank only).

    Precedence: verified > identity > safe_owner > event-only.
    When identity outranks control, safe_owner may still be present; see
    `secondary_recommendation_signals` and extended `semantic_caveat`.
    """
    kinds = set(feat.get("semantic_classes_present") or [])
    if not has_any_evidence(feat):
        return "insufficient_evidence"
    if feat.get("verified_count", 0) > 0 or "verified" in kinds:
        return "verified_did_or_controller"
    if feat.get("identity_count", 0) > 0 or "identity" in kinds:
        return "identity_claim_candidate"
    if feat.get("safe_owner_count", 0) > 0 or feat.get("has_safe_owner"):
        return "control_link_candidate"
    if kinds and all(k == "event" for k in kinds):
        return "related_only"
    return "related_only"


def entity_confidence_and_caveat(feat: dict[str, Any], rec: str) -> tuple[str, str]:
    hp = float(feat.get("hub_penalty") or 1.0)
    hub_note = ""
    if hp < 0.75:
        hub_note = f" Hub-like pattern (hub_penalty={hp:.3f}) lowers confidence; does not change evidence class."

    if rec == "verified_did_or_controller":
        return "strong", "Verified DID/controller signal (if present in export)." + hub_note
    if rec == "identity_claim_candidate":
        conf = "medium" if hp >= 0.5 else "medium_low"
        msg = "Identity claim without cryptographic verification in this baseline."
        if feat.get("safe_owner_count", 0) > 0 or feat.get("has_safe_owner"):
            msg += (
                " Also present: safe_owner (control relation). "
                "Primary label is identity_claim_candidate per precedence "
                "(verified > identity > safe_owner > event); "
                "control is not same-entity proof."
            )
        return conf, msg + hub_note
    if rec == "control_link_candidate":
        conf = "medium" if hp >= 0.5 else "medium_low"
        return (
            conf,
            "safe_owner is a control relation, not proof of same human/entity." + hub_note,
        )
    if rec == "related_only":
        return "weak", "Event/interaction evidence only; not proof of common ownership or control." + hub_note
    return "none", "No usable evidence rows for this pair." + hub_note


def temporal_pattern(block_span: int | None, short_burst_blocks: int) -> str:
    if block_span is None:
        return "unknown"
    if block_span == 0:
        return "same_block"
    if block_span <= short_burst_blocks:
        return "short_burst"
    return "long_span"


def load_pair_features(conn: sqlite3.Connection, short_burst_blocks: int) -> dict[tuple[str, str], dict[str, Any]]:
    rows = conn.execute(
        "SELECT address, kind, key, source, observed_block FROM evidence ORDER BY kind, key"
    ).fetchall()

    grouped: dict[tuple[str, str], list[sqlite3.Row]] = defaultdict(list)
    for r in rows:
        grouped[(r["kind"], r["key"])].append(r)

    pair: dict[tuple[str, str], dict[str, Any]] = {}

    for (kind, key), items in grouped.items():
        addr_counts = Counter([r["address"] for r in items])
        addr_sources: dict[str, set[str]] = defaultdict(set)
        addr_blocks: dict[str, list[int]] = defaultdict(list)
        for r in items:
            addr_sources[r["address"]].add(r["source"])
            if r["observed_block"] and int(r["observed_block"]) > 0:
                addr_blocks[r["address"]].append(int(r["observed_block"]))

        addrs = sorted(addr_counts.keys())
        if len(addrs) < 2:
            continue
        fanout = len(addrs)
        hub_penalty = 1.0 / max(1.0, math.log2(fanout + 1.0))
        sem = classify_kind(kind)

        for a, b in itertools.combinations(addrs, 2):
            k = (a, b)
            if k not in pair:
                pair[k] = {
                    "address_a": a,
                    "address_b": b,
                    "has_funded_by": False,
                    "funded_by_count": 0,
                    "has_safe_owner": False,
                    "safe_owner_count": 0,
                    "identity_count": 0,
                    "verified_count": 0,
                    "evidence_kind_counts": Counter(),
                    "source_set": set(),
                    "min_block": None,
                    "max_block": None,
                    "directed_kinds": set(),
                    "undirected_or_ambiguous_kinds": set(),
                    "semantic_classes_present": set(),
                    "hub_penalty_accum": 0.0,
                    "hub_penalty_n": 0,
                }
            p = pair[k]

            support = min(addr_counts[a], addr_counts[b])
            p["evidence_kind_counts"][kind] += support
            p["source_set"].update(addr_sources[a])
            p["source_set"].update(addr_sources[b])

            vals = addr_blocks[a] + addr_blocks[b]
            if vals:
                lo, hi = min(vals), max(vals)
                p["min_block"] = lo if p["min_block"] is None else min(p["min_block"], lo)
                p["max_block"] = hi if p["max_block"] is None else max(p["max_block"], hi)

            if kind in DIRECTED_KINDS:
                p["directed_kinds"].add(kind)
            else:
                p["undirected_or_ambiguous_kinds"].add(kind)
            p["semantic_classes_present"].add(sem)
            p["hub_penalty_accum"] += hub_penalty
            p["hub_penalty_n"] += 1

            if kind == "funded_by":
                p["has_funded_by"] = True
                p["funded_by_count"] += support
            if kind == "safe_owner":
                p["has_safe_owner"] = True
                p["safe_owner_count"] += support
            if sem == "identity":
                p["identity_count"] += support
            if sem == "verified":
                p["verified_count"] += support

    for p in pair.values():
        p["source_count"] = len(p["source_set"])
        p["block_span"] = (
            (p["max_block"] - p["min_block"])
            if p["min_block"] is not None and p["max_block"] is not None
            else None
        )
        p["temporal_pattern"] = temporal_pattern(p["block_span"], short_burst_blocks)
        p["directionality_summary"] = {
            "directed": sorted(p["directed_kinds"]),
            "undirected_or_ambiguous": sorted(p["undirected_or_ambiguous_kinds"]),
        }
        p["semantic_classes_present"] = sorted(p["semantic_classes_present"])
        p["evidence_kind_counts"] = dict(p["evidence_kind_counts"])
        p["hub_penalty"] = (
            p["hub_penalty_accum"] / p["hub_penalty_n"] if p["hub_penalty_n"] else 1.0
        )
        del p["source_set"]
        del p["directed_kinds"]
        del p["undirected_or_ambiguous_kinds"]
        del p["hub_penalty_accum"]
        del p["hub_penalty_n"]

        rs, cs, ds = calc_scores(p)
        p["related_score"] = round(rs, 4)
        p["control_score"] = round(cs, 4)
        p["did_score"] = round(ds, 4)
        p["recommendation"] = recommend_semantic_floor(p)
        rec = p["recommendation"]
        ec, cave = entity_confidence_and_caveat(p, rec)
        p["entity_confidence"] = ec
        p["semantic_caveat"] = cave
        secondary: list[str] = []
        if rec == "identity_claim_candidate" and (p.get("safe_owner_count", 0) > 0 or p.get("has_safe_owner")):
            secondary.append("control_link_candidate")
        p["secondary_recommendation_signals"] = secondary
        p["signals_present"] = sorted(set(p.get("semantic_classes_present") or []))

    return pair


def add_zero_pairs(pairs: dict[tuple[str, str], dict[str, Any]], addresses: list[str], short_burst_blocks: int) -> None:
    for a, b in itertools.combinations(sorted([x.lower() for x in addresses]), 2):
        if (a, b) in pairs:
            continue
        pairs[(a, b)] = {
            "address_a": a,
            "address_b": b,
            "has_funded_by": False,
            "funded_by_count": 0,
            "has_safe_owner": False,
            "safe_owner_count": 0,
            "identity_count": 0,
            "verified_count": 0,
            "evidence_kind_counts": {},
            "source_count": 0,
            "min_block": None,
            "max_block": None,
            "block_span": None,
            "temporal_pattern": temporal_pattern(None, short_burst_blocks),
            "directionality_summary": {"directed": [], "undirected_or_ambiguous": []},
            "semantic_classes_present": [],
            "hub_penalty": 1.0,
            "related_score": 0.0,
            "control_score": 0.0,
            "did_score": 0.0,
            "recommendation": "insufficient_evidence",
            "entity_confidence": "none",
            "semantic_caveat": "No usable evidence rows for this pair.",
            "secondary_recommendation_signals": [],
            "signals_present": [],
        }


def gold_diagnostics(rows: list[dict[str, Any]], gold: dict[tuple[str, str], str]) -> dict[str, Any] | None:
    if not gold:
        return None
    by_label_rec = defaultdict(int)
    by_rec = Counter()
    by_label = Counter()
    for r in rows:
        key = tuple(sorted((r["address_a"], r["address_b"])))
        if key not in gold:
            continue
        lab = gold[key]
        rec = r["recommendation"]
        by_label_rec[(lab, rec)] += 1
        by_rec[rec] += 1
        by_label[lab] += 1
    return {
        "gold_pair_count_matched": sum(by_label.values()),
        "counts_by_gold_label": dict(by_label),
        "counts_by_recommendation_on_gold": dict(by_rec),
        "confusion_style": [
            {"gold_label": gl, "recommendation": rc, "count": c}
            for (gl, rc), c in sorted(by_label_rec.items(), key=lambda x: (-x[1], x[0][0], x[0][1]))
        ],
        "caveat": "Small/curated gold sets; diagnostics are directional only, not statistically definitive.",
    }


def evidence_total(row: dict[str, Any]) -> int:
    return sum(int(v) for v in row.get("evidence_kind_counts", {}).values())


def bucket_block_span(block_span: int | None, short_burst_blocks: int) -> str:
    if block_span is None:
        return "unknown"
    if block_span == 0:
        return "0_same_block"
    if block_span <= short_burst_blocks:
        return f"1_short_burst_1_to_{short_burst_blocks}"
    return f"2_long_span_gt_{short_burst_blocks}"


def summarize_hub_penalty(rows: list[dict[str, Any]]) -> dict[str, Any]:
    vals = sorted(float(r.get("hub_penalty", 1.0)) for r in rows if r.get("hub_penalty") is not None)
    if not vals:
        return {"count": 0}
    return {
        "count": len(vals),
        "min": round(vals[0], 4),
        "p25": round(vals[len(vals) // 4], 4),
        "median": round(vals[len(vals) // 2], 4),
        "p75": round(vals[(len(vals) * 3) // 4], 4),
        "max": round(vals[-1], 4),
        "pairs_penalty_lt_0_5": sum(1 for v in vals if v < 0.5),
        "pairs_penalty_lt_0_75": sum(1 for v in vals if v < 0.75),
    }


def build_summary_diagnostics(rows: list[dict[str, Any]], short_burst_blocks: int) -> dict[str, Any]:
    rec = Counter(r["recommendation"] for r in rows)
    kind_coverage = Counter()
    for r in rows:
        for kind in r.get("evidence_kind_counts", {}):
            kind_coverage[kind] += 1

    no_evidence = [r for r in rows if evidence_total(r) == 0]
    funded_only = [
        r
        for r in rows
        if r.get("has_funded_by")
        and not r.get("has_safe_owner")
        and r.get("identity_count", 0) == 0
        and r.get("verified_count", 0) == 0
    ]
    safe_owner_present = [r for r in rows if r.get("has_safe_owner")]
    did_present = [r for r in rows if r.get("identity_count", 0) > 0 or r.get("verified_count", 0) > 0]
    verified_present = [r for r in rows if r.get("verified_count", 0) > 0]
    insufficient = [r for r in rows if r["recommendation"] == "insufficient_evidence"]
    temporal = Counter(r.get("temporal_pattern", "unknown") for r in rows)
    span_buckets = Counter(bucket_block_span(r.get("block_span"), short_burst_blocks) for r in rows)
    safe_owner_control = [r for r in safe_owner_present if r["recommendation"] == "control_link_candidate"]
    identity_with_control_signal = [
        r
        for r in rows
        if r["recommendation"] == "identity_claim_candidate"
        and (r.get("safe_owner_count", 0) > 0 or r.get("has_safe_owner"))
    ]
    funded_by_only_rows = [
        r
        for r in funded_only
        if r.get("recommendation") == "related_only"
    ]

    return {
        "recommendation_distribution": dict(rec),
        "evidence_kind_pair_coverage": dict(kind_coverage),
        "pairs_with_no_evidence": len(no_evidence),
        "pairs_with_funded_by": sum(1 for r in rows if r.get("has_funded_by")),
        "pairs_with_funded_by_only": len(funded_only),
        "pairs_with_safe_owner_present": len(safe_owner_present),
        "pairs_with_did_or_identity_present": len(did_present),
        "pairs_with_verified_did_present": len(verified_present),
        "insufficient_evidence_with_zero_evidence": sum(1 for r in insufficient if evidence_total(r) == 0),
        "insufficient_evidence_with_nonzero_evidence": sum(1 for r in insufficient if evidence_total(r) > 0),
        "safe_owner_present_as_control_link_candidate": len(safe_owner_control),
        "safe_owner_present_not_control_link_candidate": len(safe_owner_present) - len(safe_owner_control),
        "pairs_identity_recommendation_with_secondary_control_signal": len(identity_with_control_signal),
        "funded_by_only_pairs_as_related_only": len(funded_by_only_rows),
        "max_did_score": max((float(r.get("did_score", 0)) for r in rows), default=0.0),
        "temporal_patterns": dict(temporal),
        "block_span_buckets": dict(span_buckets),
        "hub_penalty_summary": summarize_hub_penalty(rows),
        "recommendation_policy": "semantic_floor_verified_identity_control_event",
        "short_burst_blocks": short_burst_blocks,
        "caveat": "Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.",
    }


def write_output(path: Path, rows: list[dict[str, Any]], meta: dict[str, Any]) -> None:
    if path.suffix.lower() == ".csv":
        cols = [
            "address_a",
            "address_b",
            "related_score",
            "control_score",
            "did_score",
            "recommendation",
            "funded_by_count",
            "safe_owner_count",
            "identity_count",
            "verified_count",
            "source_count",
            "min_block",
            "max_block",
            "block_span",
            "temporal_pattern",
            "hub_penalty",
            "semantic_classes_present",
            "evidence_kind_counts",
            "directionality_summary",
            "entity_confidence",
            "semantic_caveat",
            "secondary_recommendation_signals",
            "signals_present",
        ]
        with path.open("w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=cols)
            w.writeheader()
            for r in rows:
                row = dict(r)
                row["semantic_classes_present"] = json.dumps(row["semantic_classes_present"], ensure_ascii=False)
                row["evidence_kind_counts"] = json.dumps(row["evidence_kind_counts"], ensure_ascii=False)
                row["directionality_summary"] = json.dumps(row["directionality_summary"], ensure_ascii=False)
                row["secondary_recommendation_signals"] = json.dumps(
                    row.get("secondary_recommendation_signals", []), ensure_ascii=False
                )
                row["signals_present"] = json.dumps(row.get("signals_present", []), ensure_ascii=False)
                w.writerow({k: row.get(k) for k in cols})
    else:
        path.write_text(json.dumps({"meta": meta, "pairs": rows}, indent=2))


def pair_line(r: dict[str, Any]) -> str:
    span = r["block_span"] if r["block_span"] is not None else "unknown"
    return (
        f"- `{r['address_a']}` ↔ `{r['address_b']}` | rec={r['recommendation']} | conf={r.get('entity_confidence', '?')} | "
        f"related={r['related_score']:.3f} control={r['control_score']:.3f} did={r['did_score']:.3f} | "
        f"funded_by={r['funded_by_count']} safe_owner={r['safe_owner_count']} | "
        f"span={span} pattern={r.get('temporal_pattern', 'unknown')} hub={r['hub_penalty']:.3f} | "
        f"kinds={r['evidence_kind_counts']}"
    )


def add_count_section(lines: list[str], title: str, counts: dict[str, Any]) -> None:
    lines.extend(["", f"## {title}"])
    if not counts:
        lines.append("- none")
        return
    for k, v in sorted(counts.items(), key=lambda x: str(x[0])):
        lines.append(f"- {k}: {v}")


def add_pair_section(lines: list[str], title: str, rows: list[dict[str, Any]], limit: int = 5) -> None:
    lines.extend(["", f"## {title}"])
    if not rows:
        lines.append("- none")
        return
    for r in rows[:limit]:
        lines.append(pair_line(r))
        cave = r.get("semantic_caveat")
        if cave:
            lines.append(f"  - caveat: {cave}")


def write_md(path: Path, meta: dict[str, Any], rows: list[dict[str, Any]], gold_diag: dict[str, Any] | None) -> None:
    ranked = sorted(rows, key=lambda r: (max(r["related_score"], r["control_score"], r["did_score"]), r["control_score"], r["related_score"]), reverse=True)
    top = ranked[:20]
    summary = meta["summary_diagnostics"]
    by_related = sorted(
        [r for r in rows if r["recommendation"] == "related_only"],
        key=lambda r: (r["related_score"], r["control_score"], evidence_total(r)),
        reverse=True,
    )
    by_control = sorted(
        [r for r in rows if r["recommendation"] == "control_link_candidate"],
        key=lambda r: (r["control_score"], r["related_score"], evidence_total(r)),
        reverse=True,
    )
    insufficient_zero = sorted(
        [r for r in rows if r["recommendation"] == "insufficient_evidence" and evidence_total(r) == 0],
        key=lambda r: (r["address_a"], r["address_b"]),
    )
    insufficient_nonzero = sorted(
        [r for r in rows if r["recommendation"] == "insufficient_evidence" and evidence_total(r) > 0],
        key=lambda r: (evidence_total(r), r["related_score"], r["control_score"]),
        reverse=True,
    )
    hub_penalized = sorted(
        [r for r in rows if r.get("hub_penalty") is not None and r["hub_penalty"] < 1.0],
        key=lambda r: (r["hub_penalty"], -evidence_total(r)),
    )
    safe_owner_top = sorted(
        [r for r in rows if r.get("has_safe_owner")],
        key=lambda r: (r["safe_owner_count"], r["control_score"], r["related_score"]),
        reverse=True,
    )
    fp_like = []
    if gold_diag:
        gold_map = meta.get("gold_map", {})
        for r in top:
            key = f"{r['address_a']}|{r['address_b']}"
            lab = gold_map.get(key)
            if lab and lab in {"different_control", "different", "no_link"} and r["recommendation"] in {"control_link_candidate", "verified_did_or_controller", "identity_claim_candidate"}:
                fp_like.append((r, lab))

    lines = [
        f"# Pair Baseline Score: `{meta['db_path']}`",
        "",
        "- This is a deterministic, conservative baseline for feasibility/ranking.",
        "- It is **not DID verification**, and it does not alter linker semantics.",
        "",
        "## Recommendation Policy (Semantic Floor)",
        "- **Precedence (single primary label):** verified > identity > safe_owner > event-only",
        "- `verified` evidence → `verified_did_or_controller`",
        "- identity claim (`did`/`ens`/…) → `identity_claim_candidate` (outranks `safe_owner` if both)",
        "- `safe_owner_count > 0` → `control_link_candidate` when no higher-precedence class applies",
        "- event-only (`funded_by`/`transfer`/…) → `related_only` (never promoted to control by count alone)",
        "- If identity wins but `safe_owner` is also present: `secondary_recommendation_signals` includes `control_link_candidate`; `semantic_caveat` states both",
        "- `hub_penalty` affects `entity_confidence` and caveat text only, not semantic class",
        "",
        "## Top Candidate Pairs",
    ]
    for r in top[:10]:
        lines.append(pair_line(r))
    lines.extend(
        [
            "",
            "## Summary Diagnostics",
            f"- recommendation distribution: {summary['recommendation_distribution']}",
            f"- evidence kind pair coverage: {summary['evidence_kind_pair_coverage']}",
            f"- pairs with no evidence: {summary['pairs_with_no_evidence']}",
            f"- pairs with funded_by: {summary['pairs_with_funded_by']}",
            f"- pairs with funded_by only: {summary['pairs_with_funded_by_only']}",
            f"- pairs with safe_owner present: {summary['pairs_with_safe_owner_present']}",
            f"- pairs with DID/identity evidence present: {summary['pairs_with_did_or_identity_present']}",
            f"- pairs with verified DID evidence present: {summary['pairs_with_verified_did_present']}",
            f"- insufficient_evidence with zero evidence: {summary['insufficient_evidence_with_zero_evidence']}",
            f"- insufficient_evidence with nonzero evidence: {summary['insufficient_evidence_with_nonzero_evidence']}",
            f"- safe_owner present → control_link_candidate: {summary['safe_owner_present_as_control_link_candidate']}",
            f"- safe_owner present but not control_link_candidate: {summary['safe_owner_present_not_control_link_candidate']}",
            f"- identity primary with secondary control signal (identity+safe_owner): {summary['pairs_identity_recommendation_with_secondary_control_signal']}",
            f"- funded_by-only pairs as related_only: {summary['funded_by_only_pairs_as_related_only']}",
            f"- max did_score (dataset): {summary['max_did_score']}",
            f"- hub_penalty summary: {summary['hub_penalty_summary']}",
            f"- recommendation_policy: {summary['recommendation_policy']}",
            f"- caveat: {summary['caveat']}",
            "",
            "## Temporal Coverage",
            f"- temporal patterns: {summary['temporal_patterns']}",
            f"- block_span buckets: {summary['block_span_buckets']}",
            "",
            "## Coverage Caveat",
            "- Event-only evidence (`funded_by`/`transfer`) supports relatedness ranking only.",
            "- No DID evidence means `did_score` remains 0 by design.",
            "- Gold pairs are bounded evaluation pairs, not cryptographic DID ground truth.",
            "",
        ]
    )
    add_pair_section(lines, "Top Related Only Pairs", by_related)
    add_pair_section(lines, "Top Control Link Candidate Pairs", by_control)
    add_pair_section(lines, "Insufficient Evidence With Zero Evidence", insufficient_zero)
    add_pair_section(lines, "Insufficient Evidence With Nonzero Evidence", insufficient_nonzero)
    add_pair_section(lines, "Top Hub-Penalized Pairs", hub_penalized)
    add_pair_section(lines, "Top Safe Owner Present Pairs", safe_owner_top)
    if gold_diag:
        lines.extend(
            [
                "## Gold Diagnostics",
                f"- matched gold pairs: {gold_diag['gold_pair_count_matched']}",
                f"- counts by gold label: {gold_diag['counts_by_gold_label']}",
                f"- counts by recommendation on gold: {gold_diag['counts_by_recommendation_on_gold']}",
                f"- caveat: {gold_diag['caveat']}",
                "",
            ]
        )
    if fp_like:
        lines.append("## False-Positive Looking Cases (Gold-Guided)")
        for r, lab in fp_like[:10]:
            lines.append(
                f"- `{r['address_a']}` ↔ `{r['address_b']}` labeled `{lab}` but recommended `{r['recommendation']}` ({r['evidence_kind_counts']})"
            )
        lines.append("")
    path.write_text("\n".join(lines) + "\n")


def main() -> None:
    ap = argparse.ArgumentParser(description="Conservative deterministic pair-level baseline scorer.")
    ap.add_argument("--db", required=True, help="Path to sqlite db")
    ap.add_argument("--gold", default=None, help="Optional gold csv")
    ap.add_argument("--out", required=True, help="Output JSON or CSV")
    ap.add_argument("--out-md", default=None, help="Optional markdown summary path")
    ap.add_argument(
        "--short-burst-blocks",
        type=int,
        default=DEFAULT_SHORT_BURST_BLOCKS,
        help="Maximum block_span labeled short_burst; 0 remains same_block",
    )
    args = ap.parse_args()

    db_path = Path(args.db)
    out_path = Path(args.out)
    out_md = Path(args.out_md) if args.out_md else out_path.with_suffix(".md")

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    pairs = load_pair_features(conn, args.short_burst_blocks)
    addrs = load_addresses(conn)
    add_zero_pairs(pairs, addrs, args.short_burst_blocks)
    rows = [pairs[k] for k in sorted(pairs.keys())]

    gold_map = parse_gold(Path(args.gold)) if args.gold else {}
    gd = gold_diagnostics(rows, gold_map)
    summary = build_summary_diagnostics(rows, args.short_burst_blocks)
    meta = {
        "db_path": str(db_path),
        "pair_count": len(rows),
        "address_count": len(addrs),
        "gold_path": args.gold,
        "gold_map": {"|".join(k): v for k, v in gold_map.items()},
        "gold_diagnostics": gd,
        "summary_diagnostics": summary,
    }
    write_output(out_path, rows, meta)
    write_md(out_md, meta, rows, gd)
    print(f"wrote {out_path}")
    print(f"wrote {out_md}")


if __name__ == "__main__":
    main()
