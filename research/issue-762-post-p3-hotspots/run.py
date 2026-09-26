"""Serial, bounded acquisition. Run from a clean reachable research commit."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import tomllib
import uuid

from prepare import BASE, REPO, require, sha, source_index

ORDER = ["plain", "stages", "stages", "plain", "plain", "stages"]


def git(*args):
    return subprocess.check_output(["git", "-C", str(REPO), *args], text=True).strip()


def write(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def input_index(root):
    result = {}
    for scale in ("10k", "100k"):
        plan = root / "plans" / f"{scale}-smoke.toml"
        value = tomllib.loads(plan.read_text())
        require(value["window"] == {"purpose": "probe", "warm_up_ticks": 0, "observation_ticks": 256}, "plan window")
        require(value["scale"] == scale, "plan scale")
        result[str(plan.relative_to(root))] = sha(plan)
        artifacts = root / "inputs" / f"urban-{scale}"
        manifest = artifacts / "manifest.toml"
        require(sha(manifest) == value["manifest_digest"], "manifest identity")
        result[str(manifest.relative_to(root))] = sha(manifest)
        for name, expected in value["files"].items():
            path = artifacts / name
            require(path.stat().st_size == expected["bytes"] and sha(path) == expected["sha256"], f"input {name}")
            result[str(path.relative_to(root))] = expected["sha256"]
    return result


def acquire(output, inputs):
    require(not git("status", "--porcelain"), "dirty research checkout")
    head = git("rev-parse", "HEAD")
    require(subprocess.run(["git", "-C", str(REPO), "merge-base", "--is-ancestor", BASE, head]).returncode == 0, "base ancestry")
    output.mkdir(parents=True, exist_ok=False)
    identity = {"base": BASE, "research_head": head, "research_tree": git("rev-parse", "HEAD^{tree}"),
                "order": ORDER, "workers": 4, "ticks": 256, "input_root": str(inputs),
                "inputs": input_index(inputs), "source_indexes": {}, "binaries": {},
                "rustc": subprocess.check_output(["rustc", "+1.98.0", "-Vv"], text=True),
                "processor": os.environ.get("PROCESSOR_IDENTIFIER"), "logical_cpus": os.cpu_count(),
                "power": subprocess.check_output(["powercfg", "/getactivescheme"]).decode(errors="replace"),
                "started_utc": datetime.now(timezone.utc).isoformat()}
    for mode in ("plain", "stages"):
        source = REPO / "target" / f"hotspot-{mode}"
        recorded = json.loads((REPO / "target" / f"{mode}-source.json").read_text(encoding="utf-8-sig"))
        require(recorded["base"] == BASE and recorded["mode"] == mode, "export identity")
        require(source_index(source) == recorded["source_files"], "export drift")
        write(output / f"{mode}-source.json", recorded)
        identity["source_indexes"][mode] = sha(output / f"{mode}-source.json")
        binary = REPO / "target" / "hotspot-binaries" / f"{mode}.exe"
        identity["binaries"][mode] = {"path": str(binary), "sha256": sha(binary)}
    write(output / "identity.json", identity)
    for scale in ("10k", "100k"):
        for number, mode in enumerate(ORDER, 1):
            label = f"{scale}-{number}-{mode}"
            destination = output / label
            binary = Path(identity["binaries"][mode]["path"])
            require(git("rev-parse", "HEAD") == head and not git("status", "--porcelain"), "research changed")
            require(sha(binary) == identity["binaries"][mode]["sha256"], "binary changed")
            command = [str(binary), "run", str(inputs / "inputs" / f"urban-{scale}"),
                       str(inputs / "plans" / f"{scale}-smoke.toml"), str(destination), "--workers", "4"]
            metadata = {"uuid": str(uuid.uuid4()), "label": label, "scale": scale, "mode": mode,
                        "research_head": head, "started_utc": datetime.now(timezone.utc).isoformat(), "command": command}
            write(output / f"{label}.process.json", metadata)
            with (output / f"{label}.stdout").open("wb") as stdout, (output / f"{label}.stderr").open("wb") as stderr:
                process = subprocess.run(command, cwd=REPO, stdout=stdout, stderr=stderr)
            metadata.update(exit_code=process.returncode, ended_utc=datetime.now(timezone.utc).isoformat(),
                            head_after=git("rev-parse", "HEAD"), status_after=git("status", "--porcelain"),
                            binary_after=sha(binary))
            write(output / f"{label}.process.json", metadata)
            require(process.returncode == 0 and metadata["head_after"] == head and not metadata["status_after"], "run failed or source drift")
            require(metadata["binary_after"] == identity["binaries"][mode]["sha256"], "binary drift")
            diagnostics = json.loads((destination / "diagnostics.json").read_text())
            print(label, "p50_ms", diagnostics["step_ns_p50"] / 1e6, "p95_ms", diagnostics["step_ns_p95"] / 1e6, flush=True)
    require(input_index(inputs) == identity["inputs"], "input drift")
    for mode in ("plain", "stages"):
        recorded = json.loads((output / f"{mode}-source.json").read_text())
        require(source_index(REPO / "target" / f"hotspot-{mode}") == recorded["source_files"], "source drift")
    identity.update(completed=True, ended_utc=datetime.now(timezone.utc).isoformat())
    write(output / "identity.json", identity)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("inputs", type=Path)
    args = parser.parse_args()
    acquire(args.output.resolve(), args.inputs.resolve())
