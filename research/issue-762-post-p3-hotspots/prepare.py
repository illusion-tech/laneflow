"""Export the pinned main tree and add research-only stage observation."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile

BASE = "37c5e1af3c72b0f60cf2dd889bbf6713b2a77054"
HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
STAGES = ["Preflight", "Occupancy", "WaitingPrepare", "ConflictPrepare", "MotionLoop",
          "WaitingFinalize", "Signals", "ConflictFinalize", "WaitingOutputs", "Commit",
          "Frontier", "P4"]


def require(value, message):
    if not value:
        raise RuntimeError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_index(root):
    return {p.relative_to(root).as_posix(): sha(p) for p in sorted(root.rglob("*"))
            if p.is_file() and "target" not in p.relative_to(root).parts}


def edit(root, relative, old, new):
    path = root / relative
    text = path.read_text(encoding="utf-8")
    require(text.count(old) == 1, f"anchor count: {relative}: {old[:80]}")
    path.write_text(text.replace(old, new), encoding="utf-8", newline="\n")


def prepare(destination, mode):
    destination.mkdir(parents=True, exist_ok=False)
    archive = subprocess.check_output(["git", "-C", str(REPO), "archive", BASE])
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(destination, filter="data")
    host = "tools/laneflow-urban-harness/src/host.rs"
    reset = "                laneflow_runtime::research_reset();\n" if mode == "stages" else ""
    arrays = "laneflow_runtime::research_take()" if mode == "stages" else "([0_u128; 12], [0_u64; 12])"
    edit(destination, host, "                let started = Instant::now();\n                let result = world.step(input);\n                let elapsed = nanos(started.elapsed());",
         reset + "                let started = Instant::now();\n                let result = world.step(input);\n                let elapsed = nanos(started.elapsed());\n"
         + f"                let (stages, calls) = {arrays};\n"
         + '                eprintln!("LF762 {{\\\"tick\\\":{},\\\"step_ns\\\":{},\\\"stages_ns\\\":{:?},\\\"calls\\\":{:?}}}", world.tick_index(), elapsed, stages, calls);')
    if mode == "stages":
        kernel = "crates/laneflow-runtime/src/kernel/"
        edit(destination, kernel + "mod.rs", "#[cfg(test)]\npub(crate) mod performance_profile;", "pub(crate) mod performance_profile;")
        tick = destination / (kernel + "tick.rs")
        text = tick.read_text(encoding="utf-8")
        # Only activate existing batch clocks, never test failpoints or test algorithms.
        pattern = r"        #\[cfg\(test\)\]\n(?=        (?:let (?:_commit|preflight|occupancy|waiting|conflict|motion|signal|output)_timer|drop\((?:preflight|occupancy|waiting|conflict|motion|signal|output)_timer\)))"
        text, count = re.subn(pattern, "", text)
        require(count == 19, f"stage clock anchors: {count}")
        tick.write_text(text, encoding="utf-8", newline="\n")
        profiler = (HERE / "profile.rs").read_text(encoding="utf-8")
        (destination / (kernel + "performance_profile.rs")).write_text(profiler, encoding="utf-8", newline="\n")
        lib = destination / "crates/laneflow-runtime/src/lib.rs"
        with lib.open("a", encoding="utf-8", newline="\n") as stream:
            stream.write('\n/// 研究专用；在公共 step 计时外重置协调器时钟。\n#[doc(hidden)]\npub fn research_reset() { kernel::performance_profile::reset(); }\n/// 研究专用；读取协调器批次墙钟，不累加 worker CPU。\n#[doc(hidden)]\npub fn research_take() -> ([u128; 12], [u64; 12]) { kernel::performance_profile::take() }\n')
        edit(destination, kernel + "conflict_tick.rs", "        self.rebuild_conflict_frontier()?;",
             "        let frontier_timer = super::performance_profile::begin(super::performance_profile::Stage::Frontier);\n        self.rebuild_conflict_frontier()?;\n        drop(frontier_timer);")
        edit(destination, kernel + "conflict_tick.rs", "        self.acquire_conflict_candidates(tick)\n    }",
             "        let _p4_timer = super::performance_profile::begin(super::performance_profile::Stage::P4);\n        self.acquire_conflict_candidates(tick)\n    }")
    return source_index(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("destination", type=Path)
    parser.add_argument("mode", choices=["plain", "stages"])
    args = parser.parse_args()
    index = prepare(args.destination.resolve(), args.mode)
    print(json.dumps({"base": BASE, "mode": args.mode, "source_files": index}, indent=2))
