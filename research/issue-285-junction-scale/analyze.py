"""Validate frozen inputs and all fresh rounds before aggregating percentiles."""
import argparse
import csv
import hashlib
import json
import re
import statistics
from pathlib import Path


def load(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def aggregate(rows):
    return {key: (max(row[key] for row in rows) if key == "max"
                  else statistics.median(row[key] for row in rows))
            for key in ("p50", "p95", "p99", "max")}


def analyze(directory):
    freeze_path = directory / "freeze.json"
    freeze = load(freeze_path)
    freeze_hash = hashlib.sha256(freeze_path.read_bytes()).hexdigest()
    output = {"source_commit": freeze["sourceCommit"], "freeze_sha256": freeze_hash,
              "certification": freeze["environment"]["certification"], "scales": {}}
    for case in freeze["inputs"]:
        count = case["vehicles"]
        rows = []
        for round_number in (1, 2, 3):
            name = f"render-{count}-round-{round_number}"
            process = load(directory / f"{name}.process.json")
            row = load(directory / f"{name}.json")
            assert process["exitCode"] == 0, name
            assert process["freezeSha256"] == freeze_hash, name
            assert process["binarySha256"] == freeze["executables"]["junction_scale_render"]["sha256"], name
            assert row["input"] == case["prepared"]["input"], name
            assert row["initial_state_digest"] == case["prepared"]["initial_state_digest"], name
            assert not row["allocation_instrumented"], name
            actual = row["counts"]
            assert actual["worlds"] == 1 and actual["minimum_active"] == count, name
            assert actual["observed_ticks"] == 36024, name
            assert actual["maximum_backlog_quanta"] == 2 and actual["two_quantum_backlog_frames"] > 0, name
            assert actual["max_backlog_recovery_frames"] == 1, name
            assert actual["pose_rows_per_frame"] == count, name
            expected_presented = count if count == 10000 else count // 10
            assert actual["transform_rows_per_frame"] == expected_presented, name
            assert actual["minimum_renderer_visible_proxies"] == expected_presented, name
            for field in ("waiting_vehicle_ticks", "reservation_vehicle_ticks",
                          "waiting_zones_with_repeated_requests",
                          "vehicles_with_repeated_conflict_requests"):
                assert row["resource_loads"][field] > 0, (name, field)
            rows.append(row)
        processes = [load(directory / f"render-{count}-round-{n}.process.json") for n in (1, 2, 3)]
        assert len({(process["pid"], process["startedUtc"]) for process in processes}) == 3
        for key in ("initial_state_digest", "final_state_digest", "domain_event_digest", "checkpoints"):
            assert rows[0][key] == rows[1][key] == rows[2][key], (count, key)
        allocation = load(directory / f"allocation-{count}.json")
        allocation_process = load(directory / f"allocation-{count}.process.json")
        assert allocation_process["exitCode"] == 0
        assert allocation_process["freezeSha256"] == freeze_hash
        assert allocation_process["binarySha256"] == freeze["executables"]["junction_scale_allocation"]["sha256"]
        assert allocation["allocation_instrumented"] and allocation["input"] == rows[0]["input"]
        assert allocation["allocation"] == {"tick_allocations": 0, "tick_reallocations": 0}
        for key in ("initial_state_digest", "final_state_digest", "domain_event_digest", "checkpoints"):
            assert allocation[key] == rows[0][key], (count, "allocation", key)
        ledger_process = load(directory / f"ledger-{count}.process.json")
        assert ledger_process["exitCode"] == 0 and ledger_process["freezeSha256"] == freeze_hash
        assert ledger_process["binarySha256"] == freeze["executables"]["junction_ledger"]["sha256"]
        ledger_log = (directory / f"ledger-{count}.stderr.log").read_text(encoding="utf-8-sig")
        checkpoints = {int(tick): digest for tick, digest in re.findall(r"checkpoint=(\d+) digest=([0-9a-f]+)", ledger_log)}
        assert checkpoints == {point["observation_tick"]: point["state_digest"] for point in rows[0]["checkpoints"]}
        with (directory / f"ledger-{count}.csv").open(encoding="utf-8-sig", newline="") as file:
            ledger_rows = [{key: int(value) for key, value in row.items()} for row in csv.DictReader(file)]
        assert [row["observation_tick"] for row in ledger_rows] == list(range(1, 4097))
        ledger_high_water = {key: max(row[key] for row in ledger_rows) for key in ledger_rows[0] if key != "observation_tick"}
        owned = ("binding", "committed", "derived", "scratch", "admin")
        ledger_high_water["world_owned_same_tick_total"] = max(sum(row[key] for key in owned) for row in ledger_rows)
        timings = {name: aggregate([row["nanoseconds"][name] for row in rows])
                   for name in rows[0]["nanoseconds"]}
        frame_classes = {
            group: {name: aggregate([row["frame_classes"][group][name] for row in rows])
                    for name in rows[0]["frame_classes"][group]}
            for group in ("zero_step", "one_step", "two_step", "eight_step")}
        tick_budget = 2_000_000 if count == 10000 else 16_000_000
        spatial_p95 = frame_classes["one_step"]["spatial_adapter"]["p95"]
        output["scales"][str(count)] = {
            "nanoseconds": timings, "frame_classes": frame_classes,
            "runtime_p95_budget_ns": tick_budget,
            "runtime_p95_within_budget": timings["tick_and_driver"]["p95"] <= tick_budget,
            "spatial_adapter_one_step_p95_within_4ms": spatial_p95 <= 4_000_000,
            "counts_by_round": [row["counts"] for row in rows],
            "resource_loads_by_round": [row["resource_loads"] for row in rows],
            "final_state_digest": rows[0]["final_state_digest"],
            "domain_event_digest": rows[0]["domain_event_digest"],
            "ledger_high_water_over_4096_tick_replay": ledger_high_water,
            "process_memory_by_round": [{key: process[key] for key in ("peakWorkingSetBytes", "privateBytesSampledPeak", "processCommitPeakBytes")} for process in processes],
            "limits": ["logical component ledger covers H/2H/4H replay; not a full-window process peak",
                       "renderer uses unlit vehicle proxies; no road assets or GPU timestamps",
                       "native pose buffer capacities and ECS/renderer component capacity ledgers unavailable"],
        }
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    result = analyze(args.directory)
    output_path = args.directory / "summary.json"
    with output_path.open("x", encoding="utf-8") as file:
        json.dump(result, file, ensure_ascii=False, indent=2)
    print(output_path)
