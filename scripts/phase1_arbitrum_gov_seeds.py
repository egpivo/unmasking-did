#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import json
import subprocess
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any


CORE_GOV = "0xf07ded9dc292157749b6fd268e37df6ea38395b9"
TREASURY_GOV = "0x789fc99093b09ad01c34dc7251d0c89ce743e5a4"
ARB_TOKEN = "0x912ce59144191c1204e64559fe8253a0e49e6548"

TOPIC_VOTE_CAST = "0xb8e138887d0aa13bab447e82de9d5c1777041ecd21ca36ba824ff1e6c07ddda4"
TOPIC_VOTE_CAST_PARAMS = "0xe2babfbac5889a709b63bb7f598b324e08bc5a4fb9ec647fb3cbc9ec07eb8712"
TOPIC_PROPOSAL_CREATED = "0x7d84a6263ae0d98d3329bd7b46bb4e8d6f98cd35a7adb45c274c8b7fd5ebd5e0"
TOPIC_TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
ZERO_ADDR = "0x0000000000000000000000000000000000000000"


@dataclass
class QueryStats:
    total_calls: int = 0
    failed_chunks: int = 0
    retried_chunks: int = 0
    retries_attempted: int = 0
    split_chunks: int = 0


def rpc_call(url: str, method: str, params: list[Any], timeout: int = 40) -> Any:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    cp = subprocess.run(
        [
            "curl",
            "-sS",
            "--max-time",
            str(timeout),
            "-H",
            "content-type: application/json",
            "-d",
            payload,
            url,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if cp.returncode != 0:
        raise RuntimeError(f"curl failed: {cp.stderr.strip()}")
    raw = cp.stdout
    try:
        obj = json.loads(raw)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"non-json rpc response: {raw[:200]}") from e
    if "error" in obj:
        raise RuntimeError(f"rpc error: {obj['error']}")
    return obj["result"]


def hex_block(n: int) -> str:
    return hex(n)


def chunk_ranges(start_block: int, end_block: int, chunk_size: int) -> list[tuple[int, int]]:
    out: list[tuple[int, int]] = []
    cur = start_block
    while cur <= end_block:
        hi = min(cur + chunk_size - 1, end_block)
        out.append((cur, hi))
        cur = hi + 1
    return out


def addr_from_topic(topic: str) -> str:
    return "0x" + topic[-40:].lower()


def decode_proposal_created_proposer(data_hex: str) -> str | None:
    # ABI head (9 args, all non-indexed): arg0=proposalId, arg1=proposer, ...
    # proposer occupies bytes [32:64) in the ABI-encoded blob.
    if not data_hex or not data_hex.startswith("0x"):
        return None
    body = data_hex[2:]
    if len(body) < 64 * 2:
        return None
    proposer_word = body[64:128]
    return "0x" + proposer_word[-40:].lower()


def fetch_chunk_with_split(
    rpc_url: str,
    lo: int,
    hi: int,
    addresses: list[str],
    topic0_list: list[str] | None,
    stats: QueryStats,
    retries: int,
    sleep_s: float,
    min_chunk_size: int,
) -> list[dict[str, Any]]:
    params = {
        "fromBlock": hex_block(lo),
        "toBlock": hex_block(hi),
        "address": addresses if len(addresses) > 1 else addresses[0],
    }
    if topic0_list is not None:
        params["topics"] = [topic0_list]

    last_err: Exception | None = None
    for attempt in range(retries + 1):
        stats.total_calls += 1
        if attempt > 0:
            stats.retries_attempted += 1
            stats.retried_chunks += 1
        try:
            return rpc_call(rpc_url, "eth_getLogs", [params])
        except Exception as e:
            last_err = e
            time.sleep(sleep_s * (attempt + 1))

    # If this range is dense, split recursively until min chunk.
    if hi - lo + 1 > min_chunk_size:
        mid = (lo + hi) // 2
        stats.split_chunks += 1
        left = fetch_chunk_with_split(
            rpc_url, lo, mid, addresses, topic0_list, stats, retries, sleep_s, min_chunk_size
        )
        right = fetch_chunk_with_split(
            rpc_url, mid + 1, hi, addresses, topic0_list, stats, retries, sleep_s, min_chunk_size
        )
        return left + right

    stats.failed_chunks += 1
    if last_err:
        return []
    return []


def get_logs_chunked(
    rpc_url: str,
    ranges: list[tuple[int, int]],
    addresses: list[str],
    topic0_list: list[str] | None,
    stats: QueryStats,
    retries: int = 3,
    sleep_s: float = 0.5,
    min_chunk_size: int = 1000,
) -> list[dict[str, Any]]:
    logs: list[dict[str, Any]] = []
    for lo, hi in ranges:
        logs.extend(
            fetch_chunk_with_split(
                rpc_url,
                lo,
                hi,
                addresses,
                topic0_list,
                stats,
                retries,
                sleep_s,
                min_chunk_size,
            )
        )
    return logs


def process_logs_chunked(
    rpc_url: str,
    ranges: list[tuple[int, int]],
    addresses: list[str],
    topic0_list: list[str] | None,
    stats: QueryStats,
    processor,
    retries: int = 3,
    sleep_s: float = 0.5,
    min_chunk_size: int = 1000,
) -> None:
    for lo, hi in ranges:
        batch = fetch_chunk_with_split(
            rpc_url,
            lo,
            hi,
            addresses,
            topic0_list,
            stats,
            retries,
            sleep_s,
            min_chunk_size,
        )
        processor(batch)


def write_seed_csv(path: Path, rows: list[tuple[str, int, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["address", "first_seen_block", "seed_type"])
        for addr, blk, seed_type in rows:
            w.writerow([addr, blk, seed_type])


def write_control_csv(path: Path, rows: list[tuple[str, int]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["address", "first_seen_block", "seed_type"])
        for addr, blk in rows:
            w.writerow([addr, blk, "control"])


def main() -> None:
    ap = argparse.ArgumentParser(description="Phase 1 Arbitrum gov/control seed extraction.")
    ap.add_argument("--rpc-url", default="https://arb1.arbitrum.io/rpc")
    ap.add_argument("--start-block", type=int, default=428203933)
    ap.add_argument("--end-block", type=int, default=459307198)
    ap.add_argument("--gov-chunk-size", type=int, default=50000)
    ap.add_argument("--control-chunk-size", type=int, default=20000)
    ap.add_argument("--max-gov-seeds", type=int, default=500)
    ap.add_argument("--control-min", type=int, default=80)
    ap.add_argument("--control-max", type=int, default=150)
    ap.add_argument("--service-fanout-threshold", type=int, default=1000)
    ap.add_argument("--out-dir", type=Path, default=Path("data/seeds"))
    args = ap.parse_args()

    gov_ranges = chunk_ranges(args.start_block, args.end_block, args.gov_chunk_size)
    control_ranges = chunk_ranges(args.start_block, args.end_block, args.control_chunk_size)
    qstats = QueryStats()

    # Preferred strategy: combined query for two governors + topic list.
    gov_logs = get_logs_chunked(
        args.rpc_url,
        gov_ranges,
        [CORE_GOV, TREASURY_GOV],
        [TOPIC_VOTE_CAST, TOPIC_VOTE_CAST_PARAMS, TOPIC_PROPOSAL_CREATED],
        qstats,
    )
    query_strategy = "combined"

    voters_first_block: dict[str, int] = {}
    proposers_first_block: dict[str, int] = {}
    voter_seen = 0
    proposer_seen = 0

    for lg in gov_logs:
        b = int(lg["blockNumber"], 16)
        t0 = (lg.get("topics") or [""])[0].lower()
        if t0 in (TOPIC_VOTE_CAST, TOPIC_VOTE_CAST_PARAMS):
            topics = lg.get("topics") or []
            if len(topics) > 1:
                voter = addr_from_topic(topics[1])
                voters_first_block[voter] = min(voters_first_block.get(voter, b), b)
                voter_seen += 1
        elif t0 == TOPIC_PROPOSAL_CREATED:
            proposer = decode_proposal_created_proposer(lg.get("data", ""))
            if proposer:
                proposers_first_block[proposer] = min(proposers_first_block.get(proposer, b), b)
                proposer_seen += 1

    voter_rows = sorted((a, b, "voter") for a, b in voters_first_block.items())
    proposer_rows = sorted((a, b, "proposer") for a, b in proposers_first_block.items())

    gov_first_block: dict[str, int] = {}
    gov_seed_type: dict[str, str] = {}
    for a, b in voters_first_block.items():
        gov_first_block[a] = b
        gov_seed_type[a] = "voter"
    for a, b in proposers_first_block.items():
        if a in gov_first_block:
            gov_first_block[a] = min(gov_first_block[a], b)
            if gov_seed_type[a] != "voter":
                gov_seed_type[a] = "proposer"
        else:
            gov_first_block[a] = b
            gov_seed_type[a] = "proposer"

    gov_all = sorted((a, gov_first_block[a], gov_seed_type[a]) for a in gov_first_block)
    gov_seed_bucket = (
        "<300" if len(gov_all) < 300 else ("300-800" if len(gov_all) <= 800 else ">800")
    )
    cap_applied = False
    gov_all_capped = gov_all
    if len(gov_all) > 800:
        gov_all_capped = sorted(gov_all, key=lambda x: (x[1], x[0]))[: args.max_gov_seeds]
        cap_applied = True

    gov_set = {a for a, _b, _t in gov_all_capped}

    # Control cohort: ARB Transfer participants (stream processed to avoid huge memory).
    control_first: dict[str, int] = {}
    participant_counts: Counter[str] = Counter()

    def absorb_transfer_batch(batch: list[dict[str, Any]]) -> None:
        for lg in batch:
            b = int(lg["blockNumber"], 16)
            topics = lg.get("topics") or []
            if len(topics) < 3:
                continue
            frm = addr_from_topic(topics[1])
            to = addr_from_topic(topics[2])
            for p in (frm, to):
                participant_counts[p] += 1
                control_first[p] = min(control_first.get(p, b), b)

    process_logs_chunked(
        args.rpc_url,
        control_ranges,
        [ARB_TOKEN],
        [TOPIC_TRANSFER],
        qstats,
        absorb_transfer_batch,
    )

    control_candidates_before_excl = len(control_first)
    extreme_service = {
        a for a, c in participant_counts.items() if c >= args.service_fanout_threshold
    }

    # known service addresses where configured: none explicitly configured for Arbitrum in this phase.
    known_service: set[str] = set()

    control_filtered = [
        (a, b)
        for a, b in control_first.items()
        if a != ZERO_ADDR
        and a not in gov_set
        and a not in known_service
        and a not in extreme_service
    ]
    control_filtered.sort(key=lambda x: (x[1], x[0]))
    control_after_exclusion = len(control_filtered)
    control_selected = control_filtered[: args.control_max]

    out_dir = args.out_dir
    write_seed_csv(out_dir / "arbitrum_gov_90d_voter.csv", voter_rows)
    write_seed_csv(out_dir / "arbitrum_gov_90d_proposer.csv", proposer_rows)
    write_seed_csv(out_dir / "arbitrum_gov_90d_governance_all.csv", gov_all_capped)
    write_control_csv(out_dir / "arbitrum_gov_90d_control.csv", control_selected)

    summary = {
        "frozen_window": {"start_block": args.start_block, "end_block": args.end_block},
        "query_strategy": query_strategy,
        "chunk_size_governance": args.gov_chunk_size,
        "chunk_size_control": args.control_chunk_size,
        "chunk_count_governance": len(gov_ranges),
        "chunk_count_control": len(control_ranges),
        "query_stats": {
            "total_calls": qstats.total_calls,
            "failed_chunks": qstats.failed_chunks,
            "retried_chunks": qstats.retried_chunks,
            "retries_attempted": qstats.retries_attempted,
            "split_chunks": qstats.split_chunks,
        },
        "counts": {
            "voter_seeds": len(voter_rows),
            "proposer_seeds": len(proposer_rows),
            "governance_unique_after_dedupe": len(gov_all),
            "control_candidates_before_exclusion": control_candidates_before_excl,
            "control_addresses_after_exclusion": control_after_exclusion,
            "control_selected": len(control_selected),
        },
        "governance_seed_bucket": gov_seed_bucket,
        "deterministic_cap_applied": cap_applied,
        "deterministic_cap_to": args.max_gov_seeds if cap_applied else None,
        "cap_removed_count": (len(gov_all) - len(gov_all_capped)) if cap_applied else 0,
        "service_fanout_threshold": args.service_fanout_threshold,
        "extreme_service_address_count": len(extreme_service),
        "out_files": [
            str(out_dir / "arbitrum_gov_90d_voter.csv"),
            str(out_dir / "arbitrum_gov_90d_proposer.csv"),
            str(out_dir / "arbitrum_gov_90d_governance_all.csv"),
            str(out_dir / "arbitrum_gov_90d_control.csv"),
        ],
    }
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
