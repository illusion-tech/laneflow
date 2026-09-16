"""Validate all 42 cases, every batch/call, A/B digests and three A/A controls."""
import ast
import csv
import json
import math
import re
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
pattern = re.compile(
    r"active-order case=Case \{ active: (\d+), parked: (\d+), commands: (\d+), "
    r"success_percent: (\d+), order: (\d+) \} shape=(\w+) samples=(\d+) "
    r"warmup=(\d+) digest=([0-9a-f]+) batch_ns=(\[.*?\]) commands_ns=(\[.*\])"
)
expected = set()
for active in [128, 1024]:
    for parked in [64, 4096]:
        for commands in [1, 16, 64]:
            for success in [0, 50, 100]:
                if commands == 1 and success == 50:
                    continue
                expected.add((active, parked, commands, success, 0, "normal"))
for order in [1, 2]:
    expected.add((1024, 4096, 64, 50, order, "normal"))
for shape in ["capacity", "high_water"]:
    for active in [128, 1024]:
        for success in [0, 100]:
            expected.add((active, 64, 64, success, 0, shape))
assert len(expected) == 42


def read(path):
    result = {}
    text = path.read_text(encoding="utf-8-sig")
    assert "test result: ok. 1 passed" in text, path
    for match in pattern.finditer(text):
        key = (*map(int, match.group(1, 2, 3, 4, 5)), match[6])
        assert key not in result, (path, key)
        assert (int(match[7]), int(match[8])) == (16, 2)
        batches = ast.literal_eval(match[10])
        calls = ast.literal_eval(match[11].replace("true", "True").replace("false", "False"))
        assert len(batches) == 16 and len(calls) == key[2] * 16
        indices = list(range(key[2]))
        succeeds = lambda i: key[3] == 100 or (key[3] == 50 and i % 2 == 0)
        if key[4]:
            indices.sort(key=lambda i: succeeds(i) != (key[4] == 1))
        groups = {"batch": batches, "cold_success": [], "warm_success": [], "rejected": []}
        for batch in range(16):
            subset = calls[batch * key[2]:(batch + 1) * key[2]]
            assert sum(ns for _, ns in subset) == batches[batch]
            assert [ok for ok, _ in subset] == [succeeds(i) for i in indices]
            cold = True
            for ok, ns in subset:
                groups["cold_success" if ok and cold else "warm_success" if ok else "rejected"].append(ns)
                if ok:
                    cold = False
        result[key] = {"digest": match[9], "groups": groups}
    assert result.keys() == expected, (path, expected - result.keys())
    return result


def stats(values):
    values = sorted(values)
    return {"n": len(values), "p50": statistics.median(values),
            "p95": values[math.ceil(len(values) * .95) - 1],
            "p99": values[math.ceil(len(values) * .99) - 1], "max": values[-1]}


runs = {(kind, r): read(root / f"{kind}-{r}.log") for kind in ["A", "B", "Acontrol"] for r in range(1, 4)}
rows = []
for key in sorted(expected):
    assert len({run[key]["digest"] for run in runs.values()}) == 1, key
    for group in ["batch", "cold_success", "warm_success", "rejected"]:
        if not runs["A", 1][key]["groups"][group]:
            continue
        row = dict(zip(["active", "parked", "commands", "success_percent", "order", "shape"], key))
        row["group"] = group
        row["digest"] = runs["A", 1][key]["digest"]
        for kind in ["A", "B", "Acontrol"]:
            values = [ns for r in range(1, 4) for ns in runs[kind, r][key]["groups"][group]]
            row.update({f"{kind}_{k}": v for k, v in stats(values).items()})
        row["p50_change_percent"] = (row["B_p50"] / row["A_p50"] - 1) * 100
        row["aa_p50_change_percent"] = (row["Acontrol_p50"] / row["A_p50"] - 1) * 100
        for r in range(1, 4):
            a = statistics.median(runs["A", r][key]["groups"][group])
            b = statistics.median(runs["B", r][key]["groups"][group])
            control = statistics.median(runs["Acontrol", r][key]["groups"][group])
            row[f"round{r}_change_percent"] = (b / a - 1) * 100
            row[f"round{r}_aa_percent"] = (control / a - 1) * 100
        rows.append(row)
with (root / "summary.csv").open("w", newline="", encoding="utf-8") as output:
    writer = csv.DictWriter(output, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)
(root / "summary.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
print(f"Validated {len(runs)} runs x {len(expected)} cases; all semantic digests match")
for row in rows:
    if row["group"] == "batch":
        print("{shape:10} A={active:4} P={parked:4} C={commands:2} S={success_percent:3} O={order} "
              "A/B={A_p50:9.0f}/{B_p50:9.0f}ns change={p50_change_percent:+6.1f}% AA={aa_p50_change_percent:+5.1f}%".format(**row))
