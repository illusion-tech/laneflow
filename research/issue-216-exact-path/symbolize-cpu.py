"""Resolve sampled instruction addresses with the exact binary's inline PDB data."""

import argparse
import csv
import hashlib
import json
import subprocess
from collections import Counter
from pathlib import Path

CASES = ("1k-256", "10k-256", "10k-16")


def image_range(capture, case, binary_name):
    run = json.loads((capture / f"{case}.run.json").read_text(encoding="utf-8-sig"))
    images = []
    with (capture / f"{case}-export/process-images.txt").open(encoding="utf-8-sig", newline="") as source:
        for row in csv.reader(source, skipinitialspace=True):
            if len(row) < 11 or row[2] != "Image":
                continue
            if row[4].endswith(f"({run['workload_pid']})") and row[10] == binary_name:
                images.append((int(row[5], 16), int(row[6], 16)))
    assert len(set(images)) == 1, (case, images)
    return images[0]


def local_source(filename):
    normalized = filename.replace("\\", "/")
    if "/crates/" in normalized:
        return "crates/" + normalized.split("/crates/", 1)[1]
    if "/rust/library/" in normalized:
        return "rust/library/" + normalized.split("/rust/library/", 1)[1]
    return normalized


def analyze(capture, output, binary, symbolizer, result):
    case = result["case"]
    base, end = image_range(capture, case, binary.name)
    with (capture / f"cpu-analysis/{case}-addresses.csv").open(encoding="utf-8", newline="") as source:
        addresses = list(csv.DictReader(source))
    application = [row for row in addresses if base <= int(row["address"], 16) < end]
    stdin = "".join(f"0x{int(row['address'], 16) - base:x}\n" for row in application)
    process = subprocess.run(
        [str(symbolizer), "--obj", str(binary), "--relative-address", "--inlines", "--output-style=JSON"],
        input=stdin, text=True, capture_output=True, check=True,
    )
    decoded = [json.loads(line) for line in process.stdout.splitlines() if line.strip()]
    assert len(decoded) == len(application)
    line_weights, function_weights = Counter(), Counter()
    records = []
    resolved_weight = inline_weight = 0
    for row, symbols in zip(application, decoded):
        assert int(symbols["Address"], 16) == int(row["address"], 16) - base
        weight = int(row["weight_ns"])
        frames = symbols["Symbol"]
        for frame in frames:
            frame["FileName"] = local_source(frame["FileName"])
        runtime_frames = [frame for frame in frames if frame["FileName"].startswith("crates/laneflow-runtime/")]
        located = runtime_frames[0] if runtime_frames else next(
            (frame for frame in frames if frame["Line"] > 0), None
        )
        if located:
            resolved_weight += weight
            key = (located["FileName"], located["Line"], located["FunctionName"])
            line_weights[key] += weight
            function_weights[located["FunctionName"]] += weight
        if len(frames) > 1:
            inline_weight += weight
        records.append({
            "rva": symbols["Address"], "native_function": row["function"],
            "weight_ns": weight, "inline_frames": frames,
        })
    total_ns = round(result["step_sample_weight_ms"] * 1_000_000)
    (output / f"{case}-addresses.json").write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")
    summary = {
        "case": case,
        "image_base": hex(base), "image_end": hex(end),
        "application_address_count": len(application),
        "source_resolved_percent": 100 * resolved_weight / total_ns,
        "inline_expansion_percent": 100 * inline_weight / total_ns,
        "nearest_runtime_source_lines": [
            {"file": file, "line": line, "function": name, "weight_ms": ns / 1_000_000, "percent": 100 * ns / total_ns}
            for (file, line, name), ns in line_weights.most_common()
        ],
        "nearest_runtime_source_functions": [
            {"function": name, "weight_ms": ns / 1_000_000, "percent": 100 * ns / total_ns}
            for name, ns in function_weights.most_common()
        ],
    }
    print(f"{case}: source resolved={summary['source_resolved_percent']:.2f}%, inline expansion={summary['inline_expansion_percent']:.2f}%")
    for row in summary["nearest_runtime_source_lines"][:16]:
        print(f"  {row['percent']:6.2f}% {row['file']}:{row['line']} {row['function']}")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--symbolizer", type=Path, required=True)
    args = parser.parse_args()
    capture = args.capture.resolve()
    metadata = json.loads((capture / "environment.json").read_text(encoding="utf-8-sig"))
    binary = Path(metadata["binary"])
    assert hashlib.sha256(binary.read_bytes()).hexdigest().upper() == metadata["binary_sha256"]
    assert hashlib.sha256(binary.with_suffix(".pdb").read_bytes()).hexdigest().upper() == metadata["pdb_sha256"]
    output = capture / "cpu-source-analysis"
    output.mkdir(exist_ok=False)
    results = json.loads((capture / "cpu-analysis/summary.json").read_text(encoding="utf-8"))
    assert [result["case"] for result in results] == list(CASES)
    summary = [analyze(capture, output, binary, args.symbolizer, result) for result in results]
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(f"CPU_SOURCE_ANALYSIS={output}")


if __name__ == "__main__":
    main()
