"""Validate all three paired runs before emitting #680 compact measurements."""

import argparse
import csv
import json
import re
import statistics
from pathlib import Path

SIZES = {"unique": [256, 512, 1024, 2048, 4096, 8192], "corridors": [128, 256, 512, 1024, 2048]}
SAMPLE = re.compile(r"PREFLIGHT,(unique|corridors),(\d+),(\d+),(\d+),(\d+),(\d+),(\d+)$")


def analyze(directory: Path):
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8-sig"))
    expected_order = ["round-1-baseline", "round-1-candidate", "round-2-candidate", "round-2-baseline", "round-3-baseline", "round-3-candidate"]
    if manifest.get("order") != expected_order or not manifest.get("completedUtc"):
        raise ValueError("incomplete or unexpected paired run order")
    samples = {}
    for name in expected_order:
        round_number = int(name.split("-")[1])
        variant = name.split("-")[2]
        log = (directory / f"{name}.log").read_text(encoding="utf-8-sig")
        if "test result: ok. 1 passed; 0 failed;" not in log:
            raise ValueError(f"failed or incomplete test: {name}")
        seen = set()
        for line in log.splitlines():
            match = SAMPLE.search(line)
            if not match:
                continue
            kind, size, inner_round, nanoseconds, comparisons, scratch, source_bytes = match.groups()
            size, inner_round = int(size), int(inner_round)
            key = kind, size, inner_round
            if size not in SIZES[kind] or inner_round not in (1, 2, 3) or key in seen:
                raise ValueError(f"invalid or duplicate sample: {name}, {key}")
            seen.add(key)
            values = tuple(map(int, (nanoseconds, comparisons, scratch, source_bytes)))
            if values[0] <= 0:
                raise ValueError(f"nonpositive duration: {name}, {key}")
            samples[round_number, variant, kind, size, inner_round] = values
        if len(seen) != 33:
            raise ValueError(f"missing samples in {name}: {len(seen)}")

    rows = []
    for kind, sizes in SIZES.items():
        for size in sizes:
            stable = {}
            for variant in ("baseline", "candidate"):
                metrics = {samples[r, variant, kind, size, inner][1:] for r in (1, 2, 3) for inner in (1, 2, 3)}
                if len(metrics) != 1:
                    raise ValueError(f"unstable structural metrics: {variant}, {kind}, {size}")
                stable[variant] = metrics.pop()
            if stable["baseline"][2] != stable["candidate"][2]:
                raise ValueError(f"source size mismatch: {kind}, {size}")
            if kind == "unique" and stable["baseline"][0] != size * (size - 1) // 2:
                raise ValueError(f"unexpected pairwise comparison count: {size}")
            if kind == "unique" and not 0 < stable["candidate"][0] < size * size:
                raise ValueError(f"missing candidate comparison count: {size}")
            for round_number in (1, 2, 3):
                times = {variant: statistics.median(samples[round_number, variant, kind, size, inner][0] for inner in (1, 2, 3)) / 1_000_000 for variant in ("baseline", "candidate")}
                rows.append({
                    "kind": kind, "size": size, "round": round_number,
                    "baseline_comparisons": stable["baseline"][0] if kind == "unique" else "",
                    "candidate_comparisons": stable["candidate"][0] if kind == "unique" else "",
                    "baseline_scratch_bytes": stable["baseline"][1],
                    "candidate_scratch_bytes": stable["candidate"][1],
                    "source_bytes": stable["candidate"][2],
                    "baseline_median_ms": f"{times['baseline']:.6f}",
                    "candidate_median_ms": f"{times['candidate']:.6f}",
                    "change_percent": f"{(times['candidate'] / times['baseline'] - 1) * 100:.3f}",
                })
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    rows = analyze(args.directory)
    with args.output.open("w", newline="", encoding="utf-8") as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    for kind, sizes in SIZES.items():
        for size in sizes:
            group = [row for row in rows if row["kind"] == kind and row["size"] == size]
            baseline = statistics.median(float(row["baseline_median_ms"]) for row in group)
            candidate = statistics.median(float(row["candidate_median_ms"]) for row in group)
            print(f"{kind:9} {size:5} {baseline:12.6f} -> {candidate:10.6f} ms ({(candidate / baseline - 1) * 100:+.3f}%), scratch={group[0]['candidate_scratch_bytes']}")
