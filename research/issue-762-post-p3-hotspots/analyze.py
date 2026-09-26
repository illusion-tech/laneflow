"""Validate the complete matrix before publishing statistics or a file index."""
import argparse
import json
import math
from pathlib import Path
import statistics

from prepare import BASE, STAGES, require, sha
from run import ORDER, write


def rows_from_text(text, mode):
    rows = [json.loads(line.removeprefix("LF762 ")) for line in text.splitlines() if line.startswith("LF762 ")]
    require([r["tick"] for r in rows] == list(range(1, 257)), "tick sequence")
    for row in rows:
        require(type(row["step_ns"]) is int and row["step_ns"] > 0, "step time")
        require(len(row["stages_ns"]) == len(row["calls"]) == 12, "stage shape")
        require(all(type(x) is int and x >= 0 for x in row["stages_ns"] + row["calls"]), "stage values")
        if mode == "stages":
            require(row["calls"] == [1] * 12, "stage calls")
            require(sum(row["stages_ns"][:10]) <= row["step_ns"], "overlapping outer stages")
            require(sum(row["stages_ns"][10:]) <= row["stages_ns"][3], "nested stages")
        else:
            require(row["calls"] == row["stages_ns"] == [0] * 12, "plain instrumentation")
    return rows


def stats(values):
    ordered = sorted(values)
    return {"mean_ms": statistics.mean(ordered) / 1e6,
            "p50_ms": ordered[math.ceil(len(ordered) * .5) - 1] / 1e6,
            "p95_ms": ordered[math.ceil(len(ordered) * .95) - 1] / 1e6,
            "p99_ms": ordered[math.ceil(len(ordered) * .99) - 1] / 1e6,
            "max_ms": ordered[-1] / 1e6}


def analyze(root):
    identity = json.loads((root / "identity.json").read_text())
    require(identity["completed"] and identity["base"] == BASE, "identity")
    require(identity["order"] == ORDER and identity["ticks"] == 256 and identity["workers"] == 4, "protocol")
    expected = {f"{scale}-{n}-{mode}" for scale in ("10k", "100k") for n, mode in enumerate(ORDER, 1)}
    require({p.name.removesuffix(".process.json") for p in root.glob("*.process.json")} == expected, "matrix")
    for mode in ("plain", "stages"):
        require(sha(root / f"{mode}-source.json") == identity["source_indexes"][mode], "source index")
    seen = set()
    executions = set()
    semantics = {}
    runs = []
    for label in sorted(expected):
        scale, number, mode = label.split("-")
        meta = json.loads((root / f"{label}.process.json").read_text())
        require(meta["label"] == label and meta["scale"] == scale and meta["mode"] == mode, "run identity")
        require(meta["uuid"] not in seen, "duplicate UUID")
        seen.add(meta["uuid"])
        require(meta["exit_code"] == 0 and not meta["status_after"] and meta["head_after"] == meta["research_head"] == identity["research_head"], "process state")
        require(meta["binary_after"] == identity["binaries"][mode]["sha256"], "process binary")
        path = root / label
        result = json.loads((path / "result.json").read_text())
        diagnostic = json.loads((path / "diagnostics.json").read_text())
        require(result["scale"] == scale and result["case"] == "MIXED-PEAK", "native case")
        require(diagnostic["invocation"] == meta["command"], "native invocation")
        require(diagnostic["execution_id"] not in executions, "duplicate native execution")
        executions.add(diagnostic["execution_id"])
        require(result["status"] == "probe-complete" and result["completed_ticks"] == 256 and result["error"] is None, "incomplete run")
        require(diagnostic["workers"] == 4 and diagnostic["verified_steps"] == 256, "run settings")
        require(diagnostic["git_commit_at_run"] == identity["research_head"] and diagnostic["git_status_at_run"] == "", "native git identity")
        require(diagnostic["binary"]["sha256"] == identity["binaries"][mode]["sha256"], "native binary")
        for name, digest in result["files"].items():
            require((path / name).stat().st_size == digest["bytes"] and sha(path / name) == digest["sha256"], f"native digest: {name}")
        traffic = {name: sha(path / name) for name in ("ticks.jsonl", "commands.jsonl", "events.jsonl")}
        semantic = {"result": {k: v for k, v in result.items() if k != "files"}, "traffic": traffic}
        require(scale not in semantics or semantics[scale] == semantic, "traffic changed")
        semantics[scale] = semantic
        rows = rows_from_text((root / f"{label}.stderr").read_text(), mode)
        ordered_times = sorted(row["step_ns"] for row in rows)
        for percentile in (50, 95, 99):
            require(ordered_times[math.ceil(256 * percentile / 100) - 1] == diagnostic[f"step_ns_p{percentile}"], "native timing disagreement")
        windows = {}
        for name, start, end in (("all", 0, 256), ("entry", 0, 64), ("screen", 64, 256)):
            part = rows[start:end]
            window = {"step": stats([r["step_ns"] for r in part])}
            if mode == "stages":
                window["stages"] = {stage: stats([r["stages_ns"][index] for r in part]) for index, stage in enumerate(STAGES)}
                window["unattributed"] = stats([r["step_ns"] - sum(r["stages_ns"][:10]) for r in part])
                window["ranking"] = sorted(STAGES[:10], key=lambda s: window["stages"][s]["mean_ms"], reverse=True)
            windows[name] = window
        runs.append({"label": label, "mode": mode, "scale": scale, "windows": windows,
                     "initial_counts": result["initial_counts"], "final_counts": result["final_counts"], "traffic": traffic})
    return {"identity": identity, "runs": runs, "stage_names": STAGES}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("raw", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--verify", action="store_true", help="compare to the published index without overwriting it")
    args = parser.parse_args()
    result = analyze(args.raw)
    result["files"] = [{"path": p.relative_to(args.raw).as_posix(), "bytes": p.stat().st_size, "sha256": sha(p)}
                       for p in sorted(args.raw.rglob("*")) if p.is_file()]
    if args.verify:
        require(result == json.loads(args.output.read_text()), "published evidence differs")
    else:
        require(not args.output.resolve().is_relative_to(args.raw.resolve()), "output must be outside raw package")
        write(args.output, result)
    for run in result["runs"]:
        window = run["windows"]["screen"]
        print(run["label"], window["step"], window.get("ranking", [])[:3])
