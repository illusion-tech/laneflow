import copy
import json
import unittest

from analyze import rows_from_text, stats


class StageEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.rows = [{"tick": n, "step_ns": 1000, "stages_ns": [90] * 10 + [20, 30], "calls": [1] * 12}
                     for n in range(1, 257)]

    def parse(self, rows, mode="stages"):
        return rows_from_text("\n".join("LF762 " + json.dumps(r) for r in rows), mode)

    def test_valid_partition_and_nested_children(self):
        self.assertEqual(len(self.parse(self.rows)), 256)

    def test_reject_missing_duplicate_or_reordered_ticks(self):
        for rows in (self.rows[:-1], self.rows + [self.rows[-1]], list(reversed(self.rows))):
            with self.assertRaises(RuntimeError):
                self.parse(rows)

    def test_reject_worker_cpu_sum_or_nested_double_count(self):
        for index, value in ((0, 1001), (10, 91)):
            rows = copy.deepcopy(self.rows)
            rows[100]["stages_ns"][index] = value
            with self.assertRaises(RuntimeError):
                self.parse(rows)

    def test_reject_missing_clock_and_plain_mislabelling(self):
        rows = copy.deepcopy(self.rows)
        rows[200]["calls"][4] = 0
        with self.assertRaises(RuntimeError):
            self.parse(rows)
        with self.assertRaises(RuntimeError):
            self.parse(self.rows, "plain")

    def test_reject_negative_fractional_and_bad_shape(self):
        for value in (-1, 0.5, True):
            rows = copy.deepcopy(self.rows)
            rows[0]["stages_ns"][0] = value
            with self.assertRaises(RuntimeError):
                self.parse(rows)
        rows[0]["stages_ns"] = []
        with self.assertRaises(RuntimeError):
            self.parse(rows)

    def test_nearest_rank_tail_is_not_average(self):
        values = [1_000_000] * 95 + [10_000_000] * 5
        result = stats(values)
        self.assertEqual(result["p95_ms"], 1)
        self.assertEqual(result["p99_ms"], 10)
        self.assertEqual(result["mean_ms"], 1.45)


if __name__ == "__main__":
    unittest.main()
