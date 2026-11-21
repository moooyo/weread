import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

from PIL import Image, ImageDraw

from analyze import metrics
from analyze_orientation import CANDIDATE, CONTROL, SCHEDULE, orientation_sample, report, require_closed_trial


LABEL = "0123456789abcdef"


def receipts():
    plan = dict(purpose="startup-orientation-reuse", label=LABEL, device="Kindle Scribe 2022", firmware="5.19.5",
                clock="CLOCK_MONOTONIC", schedule=SCHEDULE, control_features=CONTROL, candidate_features=CANDIDATE,
                launcher_wait_seconds=2, holding="down", raw_rotation=3, expected_panel_alignment_degrees=180)
    sequence = [("admit", "none"), ("stage", "none")]
    sequence += [("case", case) for case in SCHEDULE[:4]] + [("collect", "none")]
    sequence += [("case", case) for case in SCHEDULE[4:]]
    sequence += [("restore", "none"), ("collect", "none"), ("close", "none")]
    operations = dict(label=LABEL, passed=True, released="fixture", operations=[
        {"mode": mode, "case": case, "status": 0} for mode, case in sequence])
    return plan, operations


def records(case, base, *, fallback=False):
    outcome = "timeout" if case.startswith("A") else "reused"
    stages = ["main_enter", "splash_orientation_begin", "splash_orientation_" + ("timeout" if fallback else outcome),
              "startup_feedback_ready", "window_created", "fonts_registered", "software_ready", "touch_opened",
              "event_loop_orientation_begin", "event_loop_orientation_" + outcome, "portrait_opened",
              "first_application_frame_send"]
    trace = []
    elapsed = 0
    for stage in stages:
        elapsed += 500_000_000 if stage.endswith("_timeout") else 10_000_000
        if case == "B4" and stage == "first_application_frame_send":
            elapsed += 1_100_000_000
        trace.append(f"WEREAD_STARTUP {base + elapsed // 10**9}.{elapsed % 10**9:09d} {stage}")
    return f"REQUEST {base}.000000000\n", "\n".join(trace) + "\n"


def encoded(image, kind):
    output = io.BytesIO()
    image.save(output, format=kind)
    return output.getvalue()


def batch(path, *, reversed_panel=False, fallback=False, picture_fallback=False, overlapping=False, changed_pose=None):
    with tarfile.open(path, "w:gz", compresslevel=1) as archive:
        def put(name, data):
            item = tarfile.TarInfo("out/" + name)
            item.size = len(data)
            archive.addfile(item, io.BytesIO(data))

        put("orientation-batch", (LABEL + "\n").encode())
        for case in SCHEDULE:
            for leaf in ("portrait", "portrait-launch", "portrait-return"):
                put(f"{case}/{leaf}", b"U:3\n" if case == changed_pose else b"D:3\n")
            put(case + "/portrait-source", b"native_idle_calibration\n")
        thumbs = {}
        for route in ("library", "reader"):
            image = Image.new("L", (1860, 2480), 255)
            top = 180 if route == "library" else 1400
            ImageDraw.Draw(image).rectangle((50, top, 1400, top + 800), fill=25)
            meta = {"route": route, "profile": "scribe", "backend": "kindle", "snapshot_size": image.size,
                    "window_size": image.size}
            for lane in "AB":
                case = lane + "-" + route
                panel = image if reversed_panel and case == "B-library" else image.transpose(Image.Transpose.ROTATE_180)
                thumbs[case] = encoded(panel.resize((48, 64)), "PPM")
                request, trace = records(case, 30 + 10 * SCHEDULE.index(case), fallback=picture_fallback)
                for leaf, data in [("final.metadata.json", json.dumps(meta).encode()), ("final.png", encoded(image, "PNG")),
                                   ("panel.pgm", encoded(panel, "PPM")), ("thumb.pgm", thumbs[case]),
                                   ("request", request.encode()), ("markers", trace.encode())]:
                    put(case + "/" + leaf, data)
        for index, case in enumerate(SCHEDULE[4:]):
            base = 100 if overlapping and case == "B3" else 100 + 10 * index
            request, trace = records(case, base, fallback=fallback and case == "B2")
            for leaf, data in [("request", request.encode()), ("markers", trace.encode()),
                               ("thumb.pgm", thumbs[case[0] + "-library"])]:
                put(case + "/" + leaf, data)


class OrientationTests(unittest.TestCase):
    def run_batch(self, **options):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fixture.tgz"
            batch(path, **options)
            return report(*receipts(), path)

    def test_six_pairs_keep_slow_case_and_do_not_claim_touch(self):
        result = self.run_batch()
        self.assertTrue(result["timing_gate_passed"])
        self.assertEqual(list(result["samples"]), SCHEDULE[4:])
        self.assertEqual(result["orientation_outcomes"]["B"]["event_loop"], {"reused": 6})
        self.assertEqual(result["request_to_send_ms"]["improved_pairs"], 5)
        self.assertLess(result["request_to_send_ms"]["paired_savings"][3], 0)
        self.assertEqual(result["samples"]["A0"]["request_ns"], 100_000_000_000)
        self.assertIn("touch response", result["not_measured"])

    def test_fallback_is_retained_and_not_credited_as_reuse(self):
        result = self.run_batch(fallback=True)
        self.assertFalse(result["timing_gate_passed"])
        self.assertEqual(result["orientation_outcomes"]["B"]["splash"], {"reused": 5, "timeout": 1})

    def test_rejects_normalizing_an_unexpected_half_turn(self):
        with self.assertRaisesRegex(ValueError, "B-library: observed panel rotation"):
            self.run_batch(reversed_panel=True)

    def test_timing_and_non_timing_launches_require_observed_pose(self):
        for case in ("A-library", "B0", "A4"):
            with self.subTest(case=case), self.assertRaisesRegex(ValueError, f"{case}: observed portrait"):
                self.run_batch(changed_pose=case)

    def test_candidate_pictures_must_exercise_reuse_not_just_fallback(self):
        with self.assertRaisesRegex(ValueError, "B-library: qualification pictures"):
            self.run_batch(picture_fallback=True)

    def test_rejects_overlapping_or_reordered_raw_stamps(self):
        with self.assertRaisesRegex(ValueError, "B3: device times overlap"):
            self.run_batch(overlapping=True)

    def test_legacy_parser_and_new_outcome_contract_are_separate(self):
        request, trace = records("B1", 100)
        with self.assertRaisesRegex(ValueError, "Missing orientation outcome"):
            metrics(request, trace)
        for extra in ("splash_orientation_novel", "splash_orientation_failed", "splash_orientation_reused"):
            with self.assertRaises(ValueError):
                orientation_sample(request, trace + f"WEREAD_STARTUP 101.100000000 {extra}\n")
        with self.assertRaises(ValueError):
            orientation_sample(request, trace.replace("touch_opened", "not_touch_opened"))

    def test_rejects_font_lane_unclosed_batch_and_failed_operation(self):
        plan, operations = receipts()
        for wrong in (dict(control_features=["kindle"]), dict(clock="CLOCK_BOOTTIME"), dict(launcher_wait_seconds=1),
                      dict(purpose="font"), dict(holding="unknown"), dict(expected_panel_alignment_degrees=90)):
            with self.assertRaises(ValueError):
                require_closed_trial(plan | wrong, operations)
        for wrong in (dict(passed=False), dict(released=None), dict(label="old"), dict(recovery_required=True),
                      dict(operations=operations["operations"][:-1])):
            with self.assertRaises(ValueError):
                require_closed_trial(plan, operations | wrong)
        operations["operations"][8]["status"] = 1
        with self.assertRaisesRegex(ValueError, "failed"):
            require_closed_trial(plan, operations)


if __name__ == "__main__":
    unittest.main()
