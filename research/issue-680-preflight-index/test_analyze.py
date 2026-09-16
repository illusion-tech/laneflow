import json
import tempfile
import unittest
from pathlib import Path

from analyze import SIZES, analyze


class AnalyzeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        order = ["round-1-baseline", "round-1-candidate", "round-2-candidate", "round-2-baseline", "round-3-baseline", "round-3-candidate"]
        (self.root / "manifest.json").write_text(json.dumps({"order": order, "completedUtc": "2026-09-17T00:00:00Z"}), encoding="utf-8")
        for name in order:
            candidate = name.endswith("candidate")
            lines = []
            for kind, sizes in SIZES.items():
                for size in sizes:
                    comparisons = (size * size.bit_length() if candidate else size * (size - 1) // 2) if kind == "unique" else 0
                    scratch = 16 * size if candidate else 0
                    for inner in (1, 2, 3):
                        lines.append(f"PREFLIGHT,{kind},{size},{inner},{10_000 if candidate else 20_000},{comparisons},{scratch},{size * 64}")
            lines.append("test result: ok. 1 passed; 0 failed;")
            (self.root / f"{name}.log").write_text("\n".join(lines), encoding="utf-8")

    def mutate(self, transform):
        path = self.root / "round-1-candidate.log"
        path.write_text(transform(path.read_text(encoding="utf-8")), encoding="utf-8")

    def test_complete_pairs_emit_all_rounds(self):
        rows = analyze(self.root)
        self.assertEqual(len(rows), 33)
        self.assertTrue(all(row["change_percent"] == "-50.000" for row in rows))

    def test_missing_and_duplicate_samples_are_rejected(self):
        original = (self.root / "round-1-candidate.log").read_text(encoding="utf-8")
        for changed in ("\n".join(original.splitlines()[1:]), original + "\n" + original.splitlines()[0]):
            with self.subTest(changed=changed[:60]):
                self.mutate(lambda _: changed)
                with self.assertRaises(ValueError):
                    analyze(self.root)

    def test_structural_mismatch_is_rejected(self):
        self.mutate(lambda text: text.replace("10000,2304,4096,16384", "10000,2304,4096,16385", 1))
        with self.assertRaisesRegex(ValueError, "unstable structural metrics"):
            analyze(self.root)

    def test_failed_test_does_not_become_timing_evidence(self):
        self.mutate(lambda text: text.replace("test result: ok. 1 passed; 0 failed;", "test result: FAILED. 0 passed; 1 failed;"))
        with self.assertRaisesRegex(ValueError, "failed or incomplete"):
            analyze(self.root)


if __name__ == "__main__":
    unittest.main()
