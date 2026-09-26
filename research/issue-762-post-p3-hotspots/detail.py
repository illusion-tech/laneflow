"""Three short 100k probes to partition the measured ConflictPrepare hotspot."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import statistics
import subprocess

from prepare import BASE, REPO, STAGES, edit, prepare, require, sha, source_index
from run import git, input_index, write
from analyze import stats

NAMES = STAGES + ["P3Discover", "P3Dispatch", "P3Consume", "P3Fused", "P3Tail", "Workload"]


def export(destination):
    prepare(destination, "stages")
    kernel = "crates/laneflow-runtime/src/kernel/"
    for relative in (kernel + "performance_profile.rs", "crates/laneflow-runtime/src/lib.rs"):
        p = destination / relative
        value = p.read_text().replace("; 12]", "; 18]")
        if relative.endswith("performance_profile.rs"):
            value = value.replace("    P4,", "    P4,\n    P3Discover,\n    P3Dispatch,\n    P3Consume,\n    P3Fused,\n    P3Tail,")
            value += '\npub(crate) fn workload(count: usize) { CALLS.with(|cell| { let mut values = cell.get(); values[17] = count as u64; cell.set(values); }); }\n'
        p.write_text(value, encoding="utf-8", newline="\n")
    file = kernel + "conflict_tick.rs"
    edit(destination, file, "    ) -> Result<bool, StepError> {\n        let view = ConflictTaskView {", "    ) -> Result<bool, StepError> {\n        let discover_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Discover);\n        let view = ConflictTaskView {")
    edit(destination, file, "        let workload = inputs.len();", "        let workload = inputs.len();\n        super::performance_profile::workload(workload);\n        drop(discover_timer);")
    edit(destination, file, "        let dispatch_stats =\n            execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);", "        let dispatch_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Dispatch);\n        let dispatch_stats =\n            execution.try_for_each_chunk(view.read, slots, &first_error, chunk_size, compute);\n        drop(dispatch_timer);")
    edit(destination, file, "        for index in 0..self.workspace.conflict_inputs.len() {", "        let _consume_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Consume);\n        for index in 0..self.workspace.conflict_inputs.len() {")
    edit(destination, file, "        if !dispatched {", "        if !dispatched {\n            let _fused_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Fused);")
    edit(destination, file, "        reserve(\n            &mut self.workspace.conflict_staged_decisions,\n            self.workspace.conflict_candidates.len(),", "        let _tail_timer = super::performance_profile::begin(super::performance_profile::Stage::P3Tail);\n        reserve(\n            &mut self.workspace.conflict_staged_decisions,\n            self.workspace.conflict_candidates.len(),")
    return {"base": BASE, "mode": "detail", "source_files": source_index(destination)}


def acquire(output, inputs, parent):
    require(not git("status", "--porcelain"), "dirty source")
    head = git("rev-parse", "HEAD")
    output.mkdir(parents=True, exist_ok=False)
    source = REPO / "target/hotspot-detail"
    index = json.loads((REPO / "target/detail-source.json").read_text(encoding="utf-8-sig"))
    require(source_index(source) == index["source_files"], "detail source")
    write(output / "source.json", index)
    binary = REPO / "target/hotspot-binaries/detail.exe"
    binary_hash = sha(binary)
    parent_result = json.loads(parent.read_text())
    identity = {"base": BASE, "research_head": head, "parent_results_sha256": sha(parent),
                "source_index_sha256": sha(output / "source.json"), "binary_sha256": binary_hash,
                "inputs": input_index(inputs), "ticks": 256, "workers": 4, "stage_names": NAMES}
    require(identity["inputs"] == parent_result["identity"]["inputs"], "changed input")
    write(output / "identity.json", identity)
    for n in range(1, 4):
        label = f"100k-{n}-detail"
        command = [str(binary), "run", str(inputs / "inputs/urban-100k"), str(inputs / "plans/100k-smoke.toml"), str(output / label), "--workers", "4"]
        meta = {"label": label, "command": command, "started_utc": datetime.now(timezone.utc).isoformat()}
        with (output / f"{label}.stdout").open("wb") as stdout, (output / f"{label}.stderr").open("wb") as stderr:
            run = subprocess.run(command, cwd=REPO, stdout=stdout, stderr=stderr)
        meta.update(exit_code=run.returncode, ended_utc=datetime.now(timezone.utc).isoformat(), head_after=git("rev-parse", "HEAD"), status_after=git("status", "--porcelain"), binary_after=sha(binary))
        write(output / f"{label}.process.json", meta)
        require(run.returncode == 0 and meta["head_after"] == head and not meta["status_after"] and meta["binary_after"] == binary_hash, "detail process")
        print(label, "complete", flush=True)
    require(source_index(source) == index["source_files"] and input_index(inputs) == identity["inputs"], "detail final drift")
    identity["completed"] = True
    write(output / "identity.json", identity)


def analyze(root, parent):
    reference = json.loads(parent.read_text())
    identity = json.loads((root / "identity.json").read_text())
    require(identity["completed"] and sha(parent) == identity["parent_results_sha256"], "detail parent")
    require(sha(root / "source.json") == identity["source_index_sha256"], "detail source binding")
    require(identity["base"] == BASE and identity["ticks"] == 256 and identity["workers"] == 4 and identity["stage_names"] == NAMES, "detail protocol")
    require({p.name for p in root.glob("*.process.json")} == {f"100k-{n}-detail.process.json" for n in range(1, 4)}, "detail matrix")
    semantic = None
    executions = set()
    expected_traffic = next(r["traffic"] for r in reference["runs"] if r["scale"] == "100k")
    runs = []
    for n in range(1, 4):
        label = f"100k-{n}-detail"
        path = root / label
        metadata = json.loads((root / f"{label}.process.json").read_text())
        require(metadata["exit_code"] == 0 and metadata["head_after"] == identity["research_head"] and not metadata["status_after"] and metadata["binary_after"] == identity["binary_sha256"], "detail process binding")
        result = json.loads((path / "result.json").read_text())
        diag = json.loads((path / "diagnostics.json").read_text())
        require(result["scale"] == "100k" and result["case"] == "MIXED-PEAK" and diag["invocation"] == metadata["command"], "detail workload binding")
        require(diag["execution_id"] not in executions, "duplicate detail execution")
        executions.add(diag["execution_id"])
        require(diag["git_commit_at_run"] == identity["research_head"] and diag["git_status_at_run"] == "" and diag["binary"]["sha256"] == identity["binary_sha256"], "detail native identity")
        require(result["status"] == "probe-complete" and result["completed_ticks"] == 256 and diag["workers"] == 4 and diag["verified_steps"] == 256, "detail complete")
        for name, value in result["files"].items():
            require(sha(path / name) == value["sha256"] and (path / name).stat().st_size == value["bytes"], "detail native digest")
        require({name: sha(path / name) for name in expected_traffic} == expected_traffic, "detail traffic")
        current = {k: v for k, v in result.items() if k != "files"}
        require(semantic is None or semantic == current, "detail semantic")
        semantic = current
        rows = [json.loads(line[6:]) for line in (root / f"{label}.stderr").read_text().splitlines() if line.startswith("LF762 ")]
        require([r["tick"] for r in rows] == list(range(1, 257)), "detail ticks")
        validate_rows(rows)
        ordered = sorted(r["step_ns"] for r in rows)
        for percentile in (50, 95, 99):
            require(ordered[(256 * percentile + 99) // 100 - 1] == diag[f"step_ns_p{percentile}"], "detail native timing")
        part = rows[64:]
        runs.append({"label": label, "step": stats([r["step_ns"] for r in part]),
                     "stages": {s: stats([r["stages_ns"][i] for r in part]) for i, s in enumerate(NAMES[:-1])},
                     "p3_unattributed": stats([r["stages_ns"][3] - sum(r["stages_ns"][10:17]) for r in part]),
                     "workload_min": min(r["calls"][17] for r in part), "workload_max": max(r["calls"][17] for r in part),
                     "workload_mean": statistics.mean(r["calls"][17] for r in part),
                     "dispatch_ticks": sum(r["calls"][13] for r in part), "fused_ticks": sum(r["calls"][15] for r in part)})
    return {"identity": identity, "runs": runs, "files": [{"path": p.relative_to(root).as_posix(), "bytes": p.stat().st_size, "sha256": sha(p)} for p in sorted(root.rglob("*")) if p.is_file()]}


def validate_rows(rows):
    require([r["tick"] for r in rows] == list(range(1, 257)), "detail ticks")
    for r in rows:
        t, c = r["stages_ns"], r["calls"]
        require(len(t) == len(c) == 18 and all(type(x) is int and x >= 0 for x in t + c), "detail fields")
        require(c[:13] == [1] * 13 and c[16] == 1 and c[13] == c[14], "detail clocks")
        require(c[13] == int(c[17] >= 1024) and c[15] == int(0 < c[17] < 1024), "detail path")
        require(all(c[i] != 0 or t[i] == 0 for i in range(17)) and t[17] == 0, "inactive clock")
        require(sum(t[:10]) <= r["step_ns"] and sum(t[10:17]) <= t[3], "detail partition")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["prepare", "run", "analyze", "verify"])
    parser.add_argument("paths", nargs="+", type=Path)
    a = parser.parse_args()
    if a.action == "prepare":
        print(json.dumps(export(a.paths[0]), indent=2))
    elif a.action == "run":
        acquire(*(p.resolve() for p in a.paths))
    else:
        result = analyze(a.paths[0], a.paths[1])
        if a.action == "verify":
            require(result == json.loads(a.paths[2].read_text()), "published detail evidence differs")
        else:
            require(not a.paths[2].resolve().is_relative_to(a.paths[0].resolve()), "detail output must be outside raw package")
            write(a.paths[2], result)
