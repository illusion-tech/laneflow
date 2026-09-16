"""校验分析器对完整三轮及跨轮重复/缺失日志的判定。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class ModelRoundsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        release = []
        work = []
        for active in range(1, 35):
            case = (f"Case {{ active: {active}, parked: 1, commands: 1, "
                    "success_percent: 100, order: 0 }")
            release.append(
                f"parking-command case={case} samples=16 warmup=2 digest=abc "
                f"batch_ns={[1] * 16} commands_ns={[(True, 1)] * 16}\n"
            )
            work.append(f"parking-work case={case} calls: 1 digest=abc\n")
        self.model_lines = [
            f"parking-index-model size={i} retained=8 writes=1 ns={[1] * 32}\n"
            for i in range(40)
        ]
        for round_number in range(1, 4):
            (self.root / f"release-{round_number}.log").write_text(
                "".join(release), encoding="utf-8"
            )
            self.write_models(round_number, self.model_lines)
        (self.root / "work.log").write_text("".join(work), encoding="utf-8")

    def write_models(self, round_number, lines):
        (self.root / f"models-{round_number}.log").write_text(
            "".join(lines), encoding="utf-8"
        )

    def analyze(self):
        return subprocess.run(
            [sys.executable, str(Path(__file__).with_name("analyze.py")), str(self.root)],
            capture_output=True, text=True, check=False,
        )

    def test_complete_rounds_are_accepted(self):
        result = self.analyze()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads((self.root / "analysis.json").read_text(encoding="utf-8"))
        self.assertEqual(len(report["models"]), 40)
        self.assertEqual(len(report["model_rounds"]), 120)

    def test_duplicate_round_cannot_replace_missing_round(self):
        # 总数仍为120，首个场景仍有三条且存储值相同，但来源轮次是1、1、3。
        self.write_models(1, self.model_lines + self.model_lines[:1])
        self.write_models(2, self.model_lines[1:])
        result = self.analyze()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("model rounds:", result.stderr)
        self.assertFalse((self.root / "analysis.json").exists())

    def test_balanced_duplicate_cases_in_one_round_are_rejected(self):
        # 每轮仍有40条，总数不变；一个场景有四条，另一个只有两条。
        self.write_models(2, self.model_lines[1:] + self.model_lines[1:2])
        result = self.analyze()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("model rounds:", result.stderr)
        self.assertFalse((self.root / "analysis.json").exists())


if __name__ == "__main__":
    unittest.main()
