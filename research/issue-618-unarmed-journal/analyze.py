"""Validate all #618 process logs and summarize paired independent-process rounds."""

import argparse
import gzip
import json
import math
from pathlib import Path
import random
import statistics


def percentile(values, percent):
    ordered = sorted(values)
    return ordered[math.ceil(len(ordered) * percent / 100) - 1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes = json.loads((args.input / "processes.json").read_text(encoding="utf-8-sig"))
    assert len(processes) == 96, "Require every process from all 12 paired rounds"
    samples = []
    summaries = []
    identities = set()
    digests = {}
    journal_bytes = {}
    for process in processes:
        key = (process["round"], process["case"], process["variant"])
        assert key not in identities
        identities.add(key)
        assert process["exit_code"] == 0
        text = (args.input / process["stdout"]).read_text(encoding="utf-8-sig")
        assert "test result: ok. 1 passed; 0 failed;" in text
        assert not (args.input / process["stderr"]).read_text(encoding="utf-8-sig").strip()
        windows = [json.loads(line.split("journal-ab ", 1)[1])
                   for line in text.splitlines() if "journal-ab {" in line]
        expected_windows = 130 if process["case"] == 0 else 18
        assert len(windows) == expected_windows
        observed = []
        for index, window in enumerate(windows):
            assert window["case"] == process["case"]
            assert window["window"] == index
            assert window["warmup"] == (index < 2)
            assert len(window["step_ns"]) == 64 and min(window["step_ns"]) > 0
            assert window["digest"] == digests.setdefault(window["case"], window["digest"])
            assert window["journal_bytes"] == journal_bytes.setdefault(window["case"], window["journal_bytes"])
            samples.append({"round": process["round"], "variant": process["variant"], **window})
            if not window["warmup"]:
                observed.extend(window["step_ns"])
        summaries.append({**process, "steps": len(observed),
                          "mean_ns": statistics.mean(observed),
                          "p50_ns": percentile(observed, 50),
                          "p95_ns": percentile(observed, 95),
                          "p99_ns": percentile(observed, 99),
                          "max_ns": max(observed)})
    expected = {(r, c, v) for r in range(12) for c in range(4) for v in ("baseline", "candidate")}
    assert identities == expected
    results = []
    rng = random.Random(618)
    for case in range(4):
        selected = [row for row in summaries if row["case"] == case]
        baseline = sorted((row for row in selected if row["variant"] == "baseline"), key=lambda row: row["round"])
        candidate = sorted((row for row in selected if row["variant"] == "candidate"), key=lambda row: row["round"])
        log_ratios = [math.log(c["mean_ns"] / b["mean_ns"]) for b, c in zip(baseline, candidate)]
        effect = lambda values: 100 * math.expm1(statistics.mean(values))
        bootstrap = [effect(rng.choices(log_ratios, k=12)) for _ in range(10_000)]
        results.append({"case": case, "pairs": 12, "digest": digests[case],
                        "journal_bytes": journal_bytes[case],
                        "baseline_median_process_mean_ns": statistics.median(row["mean_ns"] for row in baseline),
                        "candidate_median_process_mean_ns": statistics.median(row["mean_ns"] for row in candidate),
                        "paired_mean_change_percent": effect(log_ratios),
                        "paired_bootstrap_95_percent": [percentile(bootstrap, 2.5), percentile(bootstrap, 97.5)],
                        "pair_changes_percent": [100 * math.expm1(value) for value in log_ratios],
                        "median_process_p50_change_percent": statistics.median(100 * (c["p50_ns"] / b["p50_ns"] - 1) for b, c in zip(baseline, candidate)),
                        "baseline_median_process_p95_ns": statistics.median(row["p95_ns"] for row in baseline),
                        "candidate_median_process_p95_ns": statistics.median(row["p95_ns"] for row in candidate)})
    payload = {"method": "12 balanced pairs; geometric mean of paired process mean ratios; 10000 paired bootstrap resamples, seed 618; negative means faster", "cases": results, "processes": summaries}
    (args.output / "summary.json").write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    raw = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in samples).encode()
    (args.output / "samples.jsonl.gz").write_bytes(gzip.compress(raw, mtime=0))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
