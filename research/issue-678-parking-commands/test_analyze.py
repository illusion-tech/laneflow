"""校验分析器对正式场景矩阵、跨轮重复/缺失与等量场景替换的判定。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class ScenarioMatrixTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.release_lines = []
        self.work_lines = []
        cases = []
        for active in (128, 1024):
            for parked in (64, 4096):
                for commands in (1, 16, 64):
                    percentages = (0, 100) if commands == 1 else (0, 50, 100)
                    cases.extend((active, parked, commands, percent, 0) for percent in percentages)
        cases.extend((1024, 4096, 64, 50, order) for order in (1, 2))
        for active, parked, commands, percent, order in cases:
            case = (f"Case {{ active: {active}, parked: {parked}, commands: {commands}, "
                    f"success_percent: {percent}, order: {order} }}")
            successes = commands * percent // 100
            calls = ([(True, 1)] * successes + [(False, 1)] * (commands - successes)) * 16
            self.release_lines.append(
                f"parking-command case={case} samples=16 warmup=2 digest=abc "
                f"batch_ns={[commands] * 16} commands_ns={calls}\n"
            )
            self.work_lines.append(f"parking-work case={case} calls: {commands} digest=abc\n")
        self.model_lines = [
            f"parking-index-model initial={initial} hot={hot} queries={queries} mode={mode} "
            f"retained=8 writes=1 ns={[1] * 32}\n"
            for initial in (128, 1024)
            for hot in ("false", "true")
            for queries in (1, 64)
            for mode in range(3)
        ] + [
            f"parking-active-model initial={initial} parked={parked} mode={mode} "
            f"retained=8 writes=1 ns={[1] * 32}\n"
            for initial in (128, 1024)
            for parked in (64, 4096)
            for mode in range(4)
        ]
        self.write_commands(self.release_lines, self.work_lines)
        for round_number in range(1, 4):
            self.write_models(round_number, self.model_lines)

    def write_commands(self, release, work):
        for round_number in range(1, 4):
            (self.root / f"release-{round_number}.log").write_text(
                "".join(release), encoding="utf-8"
            )
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
        self.assertEqual(len(report["latency"]), 34)
        self.assertEqual(report["measured_calls"], 52608)
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

    def test_unexpected_command_replacing_expected_case_is_rejected(self):
        # 三轮和工作量日志一起替换同一个场景；数量、轮次、摘要、成功率仍一致。
        release = self.release_lines.copy()
        work = self.work_lines.copy()
        release[0] = release[0].replace("active: 128", "active: 129", 1)
        work[0] = work[0].replace("active: 128", "active: 129", 1)
        self.write_commands(release, work)
        result = self.analyze()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("command cases: missing=['128/192/1/0/0']", result.stderr)
        self.assertIn("unexpected=['129/193/1/0/0']", result.stderr)
        self.assertFalse((self.root / "analysis.json").exists())

    def check_replaced_model_is_rejected(self, kind):
        models = self.model_lines.copy()
        index = next(i for i, line in enumerate(models) if line.startswith(f"parking-{kind}-model"))
        models[index] = models[index].replace("initial=128", "initial=129", 1)
        for round_number in range(1, 4):
            self.write_models(round_number, models)
        result = self.analyze()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"model cases: missing=['{kind}:initial=128,", result.stderr)
        self.assertIn(f"unexpected=['{kind}:initial=129,", result.stderr)
        self.assertFalse((self.root / "analysis.json").exists())

    def test_unexpected_index_model_replacing_expected_case_is_rejected(self):
        self.check_replaced_model_is_rejected("index")

    def test_unexpected_active_model_replacing_expected_case_is_rejected(self):
        self.check_replaced_model_is_rejected("active")


if __name__ == "__main__":
    unittest.main()
