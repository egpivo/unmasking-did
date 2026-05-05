#!/usr/bin/env python3
"""Phase 1b: deterministic stratified governance/control samples (no linking).

Governance: merge voter + proposer CSVs (same first_seen rules as Phase 1), then
stratify across the frozen block window (no earliest-N cap).

Control: replay ARB Transfer logs (RPC) unless --control-candidates-csv points at
a persisted pool of address,first_seen_block rows; exclude full governance set;
same service fanout exclusion as Phase 1; then stratify.

Does not run linking or one-hop enrichment.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import phase1_arbitrum_gov_seeds as p1


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_canonical(obj: Any) -> str:
    body = json.dumps(obj, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def merge_governance_from_csv(
    voter_path: Path, proposer_path: Path
) -> list[tuple[str, int, str]]:
    """Return sorted (address, first_seen_block, seed_type) matching Phase 1 merge."""
    gov_first: dict[str, int] = {}
    gov_type: dict[str, str] = {}
    with voter_path.open(newline="", encoding="utf-8") as f:
        r = csv.DictReader(f)
        for row in r:
            a = row["address"].strip().lower()
            b = int(row["first_seen_block"])
            gov_first[a] = b
            gov_type[a] = "voter"
    with proposer_path.open(newline="", encoding="utf-8") as f:
        r = csv.DictReader(f)
        for row in r:
            a = row["address"].strip().lower()
            b = int(row["first_seen_block"])
            if a in gov_first:
                gov_first[a] = min(gov_first[a], b)
                if gov_type[a] != "voter":
                    gov_type[a] = "proposer"
            else:
                gov_first[a] = b
                gov_type[a] = "proposer"
    return sorted((a, gov_first[a], gov_type[a]) for a in gov_first)


def stratum_index(block: int, start_block: int, end_block: int, n_strata: int) -> int:
    span = end_block - start_block + 1
    if span <= 0:
        raise ValueError("invalid window")
    b = max(start_block, min(end_block, block))
    rel = b - start_block
    return min(n_strata - 1, (rel * n_strata) // span)


def stratified_select(
    rows: list[tuple[str, int, str]],
    start_block: int,
    end_block: int,
    n_strata: int,
    n_total: int,
) -> tuple[list[tuple[str, int, str]], dict[str, Any]]:
    """Equal quota per stratum, remainder filled from global reserve (deterministic)."""
    if n_total % n_strata != 0:
        raise ValueError("n_total must divide n_strata for equal strata quotas")
    quota = n_total // n_strata
    buckets: dict[int, list[tuple[str, int, str]]] = defaultdict(list)
    for addr, blk, st in rows:
        buckets[stratum_index(blk, start_block, end_block, n_strata)].append(
            (addr, blk, st)
        )
    for k in buckets:
        buckets[k].sort(key=lambda x: (x[1], x[0]))

    selected: list[tuple[str, int, str]] = []
    per_stratum_picked: dict[int, int] = {}
    reserve: list[tuple[str, int, str]] = []
    for i in range(n_strata):
        bucket = buckets.get(i, [])
        take = bucket[:quota]
        per_stratum_picked[i] = len(take)
        selected.extend(take)
        reserve.extend(bucket[quota:])

    reserve.sort(key=lambda x: (x[1], x[0]))
    j = 0
    while len(selected) < n_total and j < len(reserve):
        selected.append(reserve[j])
        j += 1

    meta = {
        "n_strata": n_strata,
        "quota_per_stratum": quota,
        "per_stratum_selected_first_pass": per_stratum_picked,
        "reserve_fill_count": j,
    }
    return selected, meta


def count_by_stratum(
    rows: list[tuple[str, int, str]], start_block: int, end_block: int, n_strata: int
) -> dict[int, int]:
    c: dict[int, int] = defaultdict(int)
    for addr, blk, st in rows:
        c[stratum_index(blk, start_block, end_block, n_strata)] += 1
    return {str(i): c.get(i, 0) for i in range(n_strata)}


def build_control_pool_rpc(
    rpc_url: str,
    start_block: int,
    end_block: int,
    gov_set: set[str],
    service_fanout_threshold: int,
    control_chunk_size: int,
    stats: p1.QueryStats,
) -> list[tuple[str, int]]:
    control_first: dict[str, int] = {}
    participant_counts: Counter[str] = Counter()

    def absorb_transfer_batch(batch: list[dict[str, Any]]) -> None:
        for lg in batch:
            b = int(lg["blockNumber"], 16)
            topics = lg.get("topics") or []
            if len(topics) < 3:
                continue
            frm = p1.addr_from_topic(topics[1])
            to = p1.addr_from_topic(topics[2])
            for p in (frm, to):
                participant_counts[p] += 1
                control_first[p] = min(control_first.get(p, b), b)

    ranges = p1.chunk_ranges(start_block, end_block, control_chunk_size)
    p1.process_logs_chunked(
        rpc_url,
        ranges,
        [p1.ARB_TOKEN],
        [p1.TOPIC_TRANSFER],
        stats,
        absorb_transfer_batch,
    )

    extreme_service = {
        a for a, c in participant_counts.items() if c >= service_fanout_threshold
    }
    known_service: set[str] = set()

    control_filtered: list[tuple[str, int]] = []
    for a, b in control_first.items():
        if (
            a != p1.ZERO_ADDR
            and a not in gov_set
            and a not in known_service
            and a not in extreme_service
        ):
            control_filtered.append((a, b))
    control_filtered.sort(key=lambda x: (x[1], x[0]))
    return control_filtered


def control_pool_sha256(rows: list[tuple[str, int]]) -> str:
    h = hashlib.sha256()
    for addr, blk in rows:
        h.update(f"{addr},{blk}\n".encode("utf-8"))
    return h.hexdigest()


def main() -> None:
    ap = argparse.ArgumentParser(description="Phase 1b stratified Arbitrum gov/control seeds.")
    ap.add_argument("--start-block", type=int, default=428203933)
    ap.add_argument("--end-block", type=int, default=459307198)
    ap.add_argument("--voter-csv", type=Path, default=Path("data/seeds/arbitrum_gov_90d_voter.csv"))
    ap.add_argument("--proposer-csv", type=Path, default=Path("data/seeds/arbitrum_gov_90d_proposer.csv"))
    ap.add_argument(
        "--control-candidates-csv",
        type=Path,
        default=None,
        help="Optional two-column CSV address,first_seen_block (no header or header skipped). "
        "If set, skips RPC for control pool.",
    )
    ap.add_argument("--rpc-url", default="https://arb1.arbitrum.io/rpc")
    ap.add_argument("--control-chunk-size", type=int, default=20000)
    ap.add_argument("--service-fanout-threshold", type=int, default=1000)
    ap.add_argument("--n-strata", type=int, default=10)
    ap.add_argument("--sample-size", type=int, default=500)
    ap.add_argument("--out-gov", type=Path, default=Path("data/seeds/arbitrum_gov_90d_governance_stratified500.csv"))
    ap.add_argument("--out-control", type=Path, default=Path("data/seeds/arbitrum_gov_90d_control_stratified500.csv"))
    ap.add_argument("--out-json", type=Path, default=Path("out/phase1b_arbitrum_gov_seed_quality.json"))
    args = ap.parse_args()

    if args.sample_size % args.n_strata != 0:
        raise SystemExit("--sample-size must be divisible by --n-strata")

    gov_all = merge_governance_from_csv(args.voter_csv, args.proposer_csv)
    gov_set = {a for a, _b, _t in gov_all}

    gov_pool_counts = count_by_stratum(
        list(gov_all), args.start_block, args.end_block, args.n_strata
    )
    gov_sample, gov_meta = stratified_select(
        list(gov_all), args.start_block, args.end_block, args.n_strata, args.sample_size
    )
    gov_sample_set = {a for a, _b, _t in gov_sample}

    qstats = p1.QueryStats()
    control_pool: list[tuple[str, int]]
    control_source: str

    if args.control_candidates_csv is not None:
        control_source = f"csv:{args.control_candidates_csv}"
        control_pool = []
        with args.control_candidates_csv.open(newline="", encoding="utf-8") as f:
            r = csv.reader(f)
            first = True
            for row in r:
                if not row:
                    continue
                if first and row[0].lower() == "address":
                    first = False
                    continue
                first = False
                a = row[0].strip().lower()
                b = int(row[1])
                if a in gov_set or a == p1.ZERO_ADDR:
                    continue
                control_pool.append((a, b))
        control_pool.sort(key=lambda x: (x[1], x[0]))
    else:
        control_source = "rpc_transfer_replay"
        control_pool = build_control_pool_rpc(
            args.rpc_url,
            args.start_block,
            args.end_block,
            gov_set,
            args.service_fanout_threshold,
            args.control_chunk_size,
            qstats,
        )

    pool_sha = control_pool_sha256(control_pool)
    control_as_gov = [a for a, _ in control_pool if a in gov_set]
    if control_as_gov:
        raise SystemExit(f"control pool still contains governance addresses: {control_as_gov[:5]}...")

    control_typed = [(a, b, "control") for a, b in control_pool]
    control_sample, ctl_meta = stratified_select(
        control_typed, args.start_block, args.end_block, args.n_strata, args.sample_size
    )
    control_sample_tuples = [(a, b) for a, b, _t in control_sample]

    overlap = gov_sample_set & {a for a, _ in control_sample_tuples}
    if overlap:
        raise SystemExit(f"governance/control sample overlap: {list(overlap)[:10]}")

    voter_sha = sha256_file(args.voter_csv)
    proposer_sha = sha256_file(args.proposer_csv)

    hash_input = {
        "phase": "1b",
        "frozen_window": {"start_block": args.start_block, "end_block": args.end_block},
        "stratification": {
            "n_strata": args.n_strata,
            "sample_size_each_cohort": args.sample_size,
            "ordering_within_stratum": ["first_seen_block", "address_lexicographic"],
            "stratum_rule": "equal_block_width_inclusive_window",
        },
        "governance_csv_inputs": {
            "voter_csv": str(args.voter_csv),
            "voter_sha256": voter_sha,
            "proposer_csv": str(args.proposer_csv),
            "proposer_sha256": proposer_sha,
            "governance_unique_count": len(gov_all),
        },
        "control_pool": {
            "source": control_source,
            "service_fanout_threshold": args.service_fanout_threshold,
            "control_chunk_size": args.control_chunk_size,
            "pool_row_count": len(control_pool),
            "pool_sha256": pool_sha,
        },
    }
    input_snapshot_hash = sha256_canonical(hash_input)

    def block_minmax(rows: list[tuple[str, int, str]]) -> tuple[int, int]:
        bs = [b for _a, b, _t in rows]
        return min(bs), max(bs)

    def block_minmax2(rows: list[tuple[str, int]]) -> tuple[int, int]:
        bs = [b for _a, b in rows]
        return min(bs), max(bs)

    gov_min, gov_max = block_minmax(gov_sample)
    ctl_min, ctl_max = block_minmax2(control_sample_tuples)

    report = {
        "input_snapshot_hash": input_snapshot_hash,
        "hash_input": hash_input,
        "query_stats": {
            "total_calls": qstats.total_calls,
            "failed_chunks": qstats.failed_chunks,
            "retried_chunks": qstats.retried_chunks,
            "retries_attempted": qstats.retries_attempted,
            "split_chunks": qstats.split_chunks,
        },
        "governance": {
            "source_unique_count": len(gov_all),
            "sample_size": len(gov_sample),
            "first_seen_block_min": gov_min,
            "first_seen_block_max": gov_max,
            "pool_per_stratum_counts": gov_pool_counts,
            "sample_per_stratum_counts_by_first_seen_block": count_by_stratum(
                gov_sample, args.start_block, args.end_block, args.n_strata
            ),
            "stratified_meta": gov_meta,
            "excludes_earliest_only_cap": True,
        },
        "control": {
            "pool_size_after_exclusions": len(control_pool),
            "sample_size": len(control_sample_tuples),
            "first_seen_block_min": ctl_min,
            "first_seen_block_max": ctl_max,
            "sample_per_stratum_counts_by_first_seen_block": count_by_stratum(
                control_sample, args.start_block, args.end_block, args.n_strata
            ),
            "stratified_meta": ctl_meta,
            "control_pool_excludes_all_governance_identities": len(control_as_gov) == 0,
            "governance_exclusion_set_size": len(gov_set),
            "overlap_governance_sample_and_control_sample": sorted(overlap),
        },
        "out_files": {
            "governance": str(args.out_gov),
            "control": str(args.out_control),
        },
    }

    args.out_json.parent.mkdir(parents=True, exist_ok=True)
    args.out_gov.parent.mkdir(parents=True, exist_ok=True)
    args.out_control.parent.mkdir(parents=True, exist_ok=True)

    p1.write_seed_csv(args.out_gov, gov_sample)
    p1.write_control_csv(args.out_control, control_sample_tuples)

    with args.out_json.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
        f.write("\n")

    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
