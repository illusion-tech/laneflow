"""Summarize the fixed #583 records; no benchmark or profiling dependencies."""

import json
from pathlib import Path
from statistics import median


ROOT = Path(__file__).resolve().parent


def records(filename, prefix):
    result = []
    for line in (ROOT / "evidence" / filename).read_text(encoding="utf-8-sig").splitlines():
        if prefix + " " not in line:
            continue
        fields = line.split(prefix + " ", 1)[1].split()
        row = dict(field.split("=", 1) for field in fields)
        result.append({key: int(value) if value.isdecimal() else value for key, value in row.items()})
    return result


def summarize():
    ticks = records("wall-clock.txt", "profile-tick")
    memories = records("stages.txt", "profile-memory")
    stages = records("stages.txt", "profile-stage")
    allocations = records("allocation.txt", "profile-allocation")
    cutovers = records("cutover.txt", "cutover")
    assert [len(rows) for rows in [ticks, memories, stages, allocations, cutovers]] == [18, 18, 198, 6, 27]
    scenes = list(dict.fromkeys(row["scene"] for row in ticks))
    assert len(scenes) == 6
    summary = {"tick": [], "cutover": []}
    owners = ["binding", "committed", "derived", "workspace", "administrative"]
    memory_fields = ["source_world_owned", "shared_root", *owners, "journal_bytes"]
    for scene in scenes:
        wall = [row for row in ticks if row["scene"] == scene]
        memory = [row for row in memories if row["scene"] == scene]
        allocation, = [row for row in allocations if row["scene"] == scene]
        assert len(wall) == len(memory) == 3
        assert {row["round"] for row in wall} == {row["round"] for row in memory} == {0, 1, 2}
        assert len({row["digest"] for row in [*wall, *memory, allocation]}) == 1
        assert len({row["journal_bytes"] for row in [*wall, *memory, allocation]}) == 1
        assert len({tuple(row[key] for key in memory_fields) for row in memory}) == 1
        assert sum(memory[0][key] for key in owners) == memory[0]["source_world_owned"]
        by_stage = {}
        residual = []
        for row in wall:
            assert row["individual"] == row["active"] == row["intent"]
            assert row["presented"] == row["aggregate"] == 0
            batch = [stage for stage in stages if stage["scene"] == scene and stage["round"] == row["round"]]
            assert len(batch) == len({stage["stage"] for stage in batch}) == 11
            assert all(stage["calls"] == row["steps"] == allocation["steps"] for stage in batch)
            whole, = [stage["ns"] for stage in batch if stage["stage"] == "whole_step"]
            child_sum = sum(stage["ns"] for stage in batch if stage["stage"] != "whole_step")
            assert 0 <= child_sum <= whole
            residual.append(100 * (whole - child_sum) / whole)
            for stage in batch:
                samples = by_stage.setdefault(stage["stage"], {"mean_ms_per_tick": [], "whole_percent": []})
                samples["mean_ms_per_tick"].append(stage["ns"] / stage["calls"] / 1_000_000)
                samples["whole_percent"].append(100 * stage["ns"] / whole)
        summary["tick"].append({
            "scene": scene,
            "steps_per_round": wall[0]["steps"],
            "delta_ms": wall[0]["delta_ms"],
            "counts": {key: wall[0][key] for key in ["individual", "active", "intent", "presented", "aggregate"]},
            "median_round_ms": {key: median(row[key + "_ns"] for row in wall) / 1_000_000 for key in ["p50", "p95", "p99"]},
            "max_ms": max(row["max_ns"] for row in wall) / 1_000_000,
            "stage_median": {name: {key: median(values) for key, values in samples.items()} for name, samples in by_stage.items()},
            "residual_percent_median": median(residual),
            "retained_bytes": {key: memory[0][key] for key in memory_fields},
            "allocation": {key: value for key, value in allocation.items() if key not in ["scene", "digest"]},
            "digest": wall[0]["digest"],
        })
    keys = ["count", "active", "edges", "routes", "mode"]
    groups = dict.fromkeys(tuple(row[key] for key in keys) for row in cutovers)
    for group in groups:
        rows = [row for row in cutovers if tuple(row[key] for key in keys) == group]
        assert len(rows) == 3 and {row["sample"] for row in rows} == {0, 1, 2}
        summary["cutover"].append({
            **dict(zip(keys, group)),
            "median_ms": {key: median(row[key + "_us"] for row in rows) / 1_000 for key in ["prepare", "pump", "commit", "digest", "paused"]},
            "journal_bytes": rows[0]["journal_bytes"],
        })
    return summary


if __name__ == "__main__":
    print(json.dumps(summarize(), indent=2, ensure_ascii=False))
