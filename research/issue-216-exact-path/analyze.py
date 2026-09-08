"""Validate and summarize the finite #216 records, without running benchmarks."""

import json
from pathlib import Path
from statistics import median
import sys


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "evidence"


def records(filename, prefix):
    rows = []
    for line in (ROOT / filename).read_text(encoding="utf-8-sig").splitlines():
        marker = prefix + " "
        if marker not in line:
            continue
        fields = dict(field.split("=", 1) for field in line.split(marker, 1)[1].split())
        rows.append({key: int(value) if value.isdecimal() else value for key, value in fields.items()})
    return rows


def summarize():
    pairs = records("pairs.txt", "exact-pair")
    work = records("attribution.txt", "exact-work")
    stages = records("attribution.txt", "exact-stage")
    clocks = records("attribution.txt", "exact-clock")
    equivalence = records("equivalence.txt", "exact-equivalence")
    production = records("wall-clock.txt", "profile-tick")
    broad = records("stages.txt", "profile-stage")
    memories = records("stages.txt", "profile-memory")
    assert [len(rows) for rows in [pairs, work, stages, clocks, equivalence, production, broad, memories]] == [48, 24, 192, 1, 4, 18, 198, 18]
    scenes = list(dict.fromkeys(row["scene"] for row in pairs))
    assert len(scenes) == 4
    summary = {"clock_read_mean_ns": clocks[0]["ns"] / clocks[0]["calls"], "scenes": []}
    owners = ["binding", "committed", "derived", "workspace", "administrative"]
    allocation = ["allocations", "reallocations", "allocated_bytes", "deallocated_bytes", "reallocated_bytes"]
    timing = ["p50_ns", "p95_ns", "p99_ns", "sum_ns"]
    for row in production:
        batch = [stage for stage in broad if (stage["scene"], stage["round"]) == (row["scene"], row["round"])]
        ledger, = [item for item in memories if (item["scene"], item["round"]) == (row["scene"], row["round"])]
        assert len(batch) == len({stage["stage"] for stage in batch}) == 11
        assert all(stage["calls"] == row["steps"] for stage in batch)
        whole, = [stage["ns"] for stage in batch if stage["stage"] == "whole_step"]
        assert sum(stage["ns"] for stage in batch if stage["stage"] != "whole_step") <= whole
        assert ledger["digest"] == row["digest"]
        assert sum(ledger[key] for key in owners) == ledger["source_world_owned"]
    for scene in scenes:
        paired = [row for row in pairs if row["scene"] == scene]
        actual_work = [row for row in work if row["scene"] == scene]
        exact, = [row for row in equivalence if row["scene"] == scene]
        wall = [row for row in production if row["scene"] == scene]
        memory = [row for row in memories if row["scene"] == scene]
        assert len({row["digest"] for row in [*paired, *actual_work, exact, *wall, *memory]}) == 1
        assert len({(row["records"], row["inspections"], row["occurrence_walks"]) for row in [*paired, *actual_work]}) == 1
        assert all(row["steps"] == 64 for row in [*paired, *actual_work, exact])
        assert all(sum(row[key] for key in owners) == row["source_world_owned"] for row in paired)
        assert all(row[key] == 0 for row in paired for key in allocation)
        assert paired[0]["occurrence_walks"] > 0 if scene.startswith("multi-edge") else paired[0]["occurrence_walks"] == 0
        variants = {}
        for variant in ["false", "true"]:
            rows = [row for row in paired if row["candidate"] == variant]
            assert len(rows) == 6 and {row["round"] for row in rows} == set(range(6))
            assert len({row["source_world_owned"] for row in rows}) == 1
            batch = [row for row in stages if row["scene"] == scene and row["candidate"] == variant]
            names = list(dict.fromkeys(row["stage"] for row in batch))
            assert len(names) == 8
            by_stage = {}
            for name in names:
                measured = [row for row in batch if row["stage"] == name]
                assert len(measured) == 3 and {row["round"] for row in measured} == set(range(3))
                is_batch = name.startswith("occupancy_")
                assert all(row["calls"] == (64 if is_batch else 64 * paired[0]["active"]) for row in measured)
                assert all(0 < row["samples"] <= row["calls"] for row in measured)
                if is_batch:
                    assert all(row["samples"] == row["calls"] for row in measured)
                else:
                    assert all(row["samples"] == (row["calls"] + row["round"] * 19 - 1) // 67 - (row["round"] * 19 - 1) // 67 for row in measured)
                by_stage[name] = {"sampled_mean_ns": median(row["ns"] / row["samples"] for row in measured), "calls_per_tick": measured[0]["calls"] / 64}
            variants[variant] = {
                "median_round_ms": {key[:-3]: median(row[key] for row in rows) / 1_000_000 for key in timing},
                "max_ms": max(row["max_ns"] for row in rows) / 1_000_000,
                "retained_bytes": {key: rows[0][key] for key in ["source_world_owned", "shared_root", *owners, "pending_bytes"]},
                "allocation": {key: rows[0][key] for key in allocation},
                "stages": by_stage,
            }
        changes = {key: [] for key in timing}
        for round_ in range(6):
            left, = [row for row in paired if row["round"] == round_ and row["candidate"] == "false"]
            right, = [row for row in paired if row["round"] == round_ and row["candidate"] == "true"]
            assert (left["position"], right["position"]) == ((0, 1) if round_ % 2 == 0 else (1, 0))
            assert right["source_world_owned"] - left["source_world_owned"] == right["pending_bytes"]
            assert left["pending_bytes"] == 0
            for key in timing:
                changes[key].append(100 * (right[key] / left[key] - 1))
        summary["scenes"].append({
            "scene": scene,
            "digest": exact["digest"],
            "work_per_window": {key: paired[0][key] for key in ["active", "steps", "records", "inspections", "occurrence_walks"]},
            "unmodified_production_median_ms": {key[:-3]: median(row[key] for row in wall) / 1_000_000 for key in timing} if wall else None,
            "baseline_batch_stage_percent": {
                name: median(100 * next(row["ns"] for row in broad if row["scene"] == scene and row["round"] == round_ and row["stage"] == name) / next(row["ns"] for row in broad if row["scene"] == scene and row["round"] == round_ and row["stage"] == "whole_step") for round_ in range(3))
                for name in ["occupancy", "motion_loop"]
            } if wall else None,
            "variants": variants,
            "paired_change_percent": {key[:-3]: {"median": median(values), "min": min(values), "max": max(values), "candidate_faster_rounds": sum(value < 0 for value in values)} for key, values in changes.items()},
        })
    return summary


if __name__ == "__main__":
    print(json.dumps(summarize(), indent=2, ensure_ascii=False))
