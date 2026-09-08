"""Summarize raw WPA samples under the fixed-window stack anchor only."""

import argparse
import csv
import hashlib
import json
import re
from collections import Counter
from decimal import Decimal
from pathlib import Path

CASES = ("1k-256", "10k-256", "10k-16")
ANCHOR = "runtime_profile_evidence::cpu_sampling::observed_cpu_window"
STEP = "laneflow_runtime::facade::TrafficWorld::step"
CSV_NAME = "CPU_Usage_(Sampled)_LaneFlow_raw_samples.csv"


def function(frame):
    return frame.split("!", 1)[-1]


def ranked(weights, counts, denominator):
    return [
        {
            "function": name,
            "samples": counts[name],
            "weight_ms": weight / 1_000_000,
            "percent": 100 * weight / denominator,
        }
        for name, weight in weights.most_common()
    ]


def analyze(capture, case, output):
    export = capture / f"{case}-export"
    csv_path = export / CSV_NAME
    run = json.loads((capture / f"{case}.run.json").read_text(encoding="utf-8-sig"))
    assert run["exit_code"] == 0 and "error" not in run
    trace_summary = (export / "trace-summary.txt").read_text(encoding="utf-8-sig")
    for counter in ("Lost Buffers", "Lost Events"):
        match = re.search(rf"Total # {counter}\s*:\s*(\d+)", trace_summary)
        assert match and int(match[1]) == 0, (case, counter)
    stdout = (capture / f"{case}.stdout.txt").read_text(encoding="utf-8-sig")
    window = re.search(r"rounds=(\d+) steps=(\d+).*observed_wall_ns=(\d+) digest=(\w+)", stdout)
    assert window

    self_weights, self_counts = Counter(), Counter()
    inclusive_weights, inclusive_counts = Counter(), Counter()
    stacks, addresses, threads = Counter(), Counter(), Counter()
    process_count = process_ns = anchor_count = anchor_ns = step_count = step_ns = 0
    runtime_unknown = leaf_unknown = no_stack = 0
    with csv_path.open(encoding="utf-8-sig", newline="") as source:
        for row in csv.DictReader(source):
            assert row["Process"].endswith(f"({run['workload_pid']})"), row["Process"]
            assert int(row["Count"]) == 1, "WPA must export raw, ungrouped samples"
            ns = int(Decimal(row["Weight (ms)"]) * 1_000_000)
            process_count += 1
            process_ns += ns
            frames = row["Stack"].split("/")
            names = [function(frame) for frame in frames]
            if not row["Stack"] or row["Stack"] == "[Root]":
                no_stack += 1
            if ANCHOR not in names:
                continue
            anchor_count += 1
            anchor_ns += ns
            if STEP not in names:
                continue
            step_count += 1
            step_ns += ns
            frames = frames[names.index(STEP):]
            names = [function(frame) for frame in frames]
            threads[row["Thread ID"]] += 1
            leaf = row["Function"]
            if leaf in ("?", "", "Unknown"):
                leaf_unknown += 1
                leaf = frames[-1]
            runtime_unknown += any(
                "runtime_profile_evidence" in frame and "PDB not found" in frame
                for frame in frames
            )
            self_weights[leaf] += ns
            self_counts[leaf] += 1
            for name in set(names):
                inclusive_weights[name] += ns
                inclusive_counts[name] += 1
            stacks[";".join(frames)] += ns
            addresses[(row["Address"], leaf)] += ns

    assert step_count > 1_000 and runtime_unknown == 0
    assert len(threads) == 1, threads
    assert sum(self_weights.values()) == step_ns
    result = {
        "case": case,
        "workload_pid": run["workload_pid"],
        "csv_sha256": hashlib.sha256(csv_path.read_bytes()).hexdigest(),
        "rounds": int(window[1]),
        "steps": int(window[2]),
        "observed_wall_ms": int(window[3]) / 1_000_000,
        "digest": window[4],
        "process_samples": process_count,
        "process_sample_weight_ms": process_ns / 1_000_000,
        "process_no_stack_samples": no_stack,
        "anchor_samples": anchor_count,
        "anchor_sample_weight_ms": anchor_ns / 1_000_000,
        "step_samples": step_count,
        "step_sample_weight_ms": step_ns / 1_000_000,
        "step_thread_samples": dict(threads),
        "step_unknown_function_samples": leaf_unknown,
        "step_missing_application_pdb_samples": runtime_unknown,
        "lost_events": 0,
        "lost_buffers": 0,
        "self_functions": ranked(self_weights, self_counts, step_ns),
        "inclusive_functions": ranked(inclusive_weights, inclusive_counts, step_ns),
    }
    (output / f"{case}-stacks.folded").write_text(
        "".join(f"{stack} {weight}\n" for stack, weight in stacks.most_common()), encoding="utf-8"
    )
    with (output / f"{case}-addresses.csv").open("w", encoding="utf-8", newline="") as dest:
        writer = csv.writer(dest)
        writer.writerow(("address", "function", "weight_ns"))
        writer.writerows((address, name, weight) for (address, name), weight in addresses.most_common())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    capture = args.capture.resolve()
    output = capture / "cpu-analysis"
    output.mkdir(exist_ok=False)
    results = [analyze(capture, case, output) for case in CASES]
    (output / "summary.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    for result in results:
        print(f"{result['case']}: samples={result['step_samples']} weight_ms={result['step_sample_weight_ms']:.3f}")
        for row in result["self_functions"][:15]:
            print(f"  {row['percent']:6.2f}% {row['function']}")
    print(f"CPU_ANALYSIS={output}")


if __name__ == "__main__":
    main()
