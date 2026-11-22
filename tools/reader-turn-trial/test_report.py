import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

import report


def samples(delta=0):
    rows = []
    for sequence in range(1, 25):
        base = sequence * 1_000_000_000
        full = sequence % 6 == 0
        row = dict(sequence=sequence, warmup=sequence <= 3, chapter=0,
                   source_page=sequence, target_page=sequence + 1,
                   source_anchor=(sequence - 1) * 100, target_anchor=sequence * 100,
                   source_line_start=(sequence - 1) * 100, target_line_start=sequence * 100,
                   marker=sequence + 20, full=full, rect=[0, 0, 1860, 2480], waveform=2 if full else 257)
        for name, offset in zip(report.TIMES, (0, 50, 70, 72, 150, 155, 156)):
            row[name] = base + offset * 1_000_000 + (delta if name not in ("press_ns", "release_ns") else 0)
        row.update(wait_begin_ns=row["sent_ns"] + 10, wait_end_ns=row["sent_ns"] + 1_000_000,
                   wait_marker=row["marker"], wait_status="acknowledged")
        rows.append(row)
    return rows


class TurnReportTests(unittest.TestCase):
    def test_real_file_shape_and_cleanup_gate(self):
        with tempfile.TemporaryDirectory() as temporary:
            label = "d" * 16
            visit = Path(temporary) / f"visit-{label}"
            visit.mkdir()
            rows = samples()
            (visit / "turn-result.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
            (visit / "turn-complete").write_bytes(b"24\n")
            receipt = dict(label=label, lane="B", mode="native", chapters=1863, passed=True,
                           released=True, native_restored=True, connection_closed=True)
            (visit / "receipt.json").write_text(json.dumps(receipt))
            self.assertEqual(report.load_visit("B", label, visit), ("B", label, rows))
            receipt["released"] = False
            (visit / "receipt.json").write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, "closeout"):
                report.load_visit("B", label, visit)

    def test_excludes_warmups_and_pairs_only_matching_waits(self):
        baseline = samples(2_000_000)
        candidate = samples()
        candidate[4].update(wait_marker=None, wait_begin_ns=None, wait_end_ns=None, wait_status="unavailable")
        report.check_rows(baseline)
        report.check_rows(candidate)
        result = report.compare([("A", "a" * 16, baseline), ("B", "b" * 16, candidate)])
        groups = result["pairs"][0]["same_target_A_minus_B"]
        self.assertEqual(groups["full"]["sequences"], [6, 12, 18, 24])
        self.assertEqual(groups["partial"]["release_to_send_return"]["n"], 17)
        self.assertEqual(groups["partial"]["release_to_matching_wait_return"]["n"], 16)
        self.assertEqual(groups["partial"]["release_to_send_return"]["median_ms"], 2.0)

    def test_rejects_mixed_frames_timestamps_mode_and_waits(self):
        rows = samples()
        for change in [{"marker": rows[5]["marker"]}, {"wait_marker": 99}, {"warmup": 0},
                       {"wait_status": "not-witnessed"}, {"full": True}, {"waveform": 2},
                       {"model_ns": 50}, {"rect": [0, 0, 1861, 2480]},
                       {"source_page": 2}, {"source_anchor": 20}]:
            with self.subTest(change=change), self.assertRaises(ValueError):
                mutated = deepcopy(rows)
                mutated[6].update(change)
                report.check_rows(mutated)
        shifted = deepcopy(rows)
        shifted[3]["target_line_start"] += 1
        shifted[4]["source_line_start"] += 1
        report.check_rows(shifted)
        with self.assertRaisesRegex(ValueError, "same public page"):
            report.compare([("A", "a" * 16, rows), ("B", "b" * 16, shifted)])

    def test_only_one_or_two_balanced_pairs(self):
        rows = samples()
        visits = [(lane, str(index) * 16, rows) for index, lane in enumerate("ABBA")]
        self.assertEqual(len(report.compare(visits)["pairs"]), 2)
        for invalid in [visits[:1], [visits[0], visits[0]], visits[:2] * 2,
                        [visits[0], visits[1], visits[3], visits[2]]]:
            with self.subTest(order=[visit[0] for visit in invalid]), self.assertRaises(ValueError):
                report.compare(invalid)


if __name__ == "__main__":
    unittest.main()
