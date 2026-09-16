"""回归校验矩阵闭合与不完整/异语义输入的拒绝。"""
import subprocess
import sys
import tempfile
from pathlib import Path

script = Path(__file__).with_name("analyze.py")
cases = []
for active in [128, 1024]:
    for parked in [64, 4096]:
        for commands in [1, 16, 64]:
            for success in [0, 50, 100]:
                if commands > 1 or success != 50:
                    cases.append((active, parked, commands, success, 0, "normal"))
cases.extend((1024, 4096, 64, 50, order, "normal") for order in [1, 2])
cases.extend((active, 64, 64, success, 0, shape)
             for shape in ["capacity", "high_water"] for active in [128, 1024] for success in [0, 100])


def line(case):
    active, parked, commands, success, order, shape = case
    flags = [success == 100 or (success == 50 and i % 2 == 0) for i in range(commands)]
    if order:
        flags.sort(key=lambda ok: ok != (order == 1))
    calls = str([(ok, 100) for _ in range(16) for ok in flags]).lower()
    return (f"active-order case=Case {{ active: {active}, parked: {parked}, commands: {commands}, "
            f"success_percent: {success}, order: {order} }} shape={shape} samples=16 warmup=2 "
            f"digest=abcd batch_ns={[100 * commands] * 16} commands_ns={calls}\n")


good = "".join(map(line, cases)) + "test result: ok. 1 passed\n"
with tempfile.TemporaryDirectory(prefix="active-order-analyzer-") as directory:
    root = Path(directory)
    for kind in ["A", "B", "Acontrol"]:
        for round_ in range(1, 4):
            (root / f"{kind}-{round_}.log").write_text(good, encoding="utf-8")
    target = root / "B-2.log"
    variants = {
        "complete": (good, True),
        "missing": (good.replace(line(cases[0]), ""), False),
        "duplicate": (good + line(cases[0]), False),
        "replaced": (good.replace("active: 128", "active: 129", 1), False),
        "different_digest": (good.replace("digest=abcd", "digest=bcde", 1), False),
        "wrong_samples": (good.replace("samples=16", "samples=15", 1), False),
        "wrong_outcome": (good.replace("(false, 100)", "(true, 100)", 1), False),
        "wrong_batch_sum": (good.replace("batch_ns=[100,", "batch_ns=[101,", 1), False),
    }
    for name, (content, valid) in variants.items():
        target.write_text(content, encoding="utf-8")
        result = subprocess.run([sys.executable, str(script), str(root)], capture_output=True)
        assert (result.returncode == 0) == valid, (name, result.stderr.decode())
        print(f"{name}: expected {'accept' if valid else 'reject'}")
