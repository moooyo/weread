import io
import json
from contextlib import redirect_stderr
from pathlib import Path
import tarfile
import tempfile
import unittest

from PIL import Image, ImageDraw

from freeze import SCHEDULE
from orientation_contract import CANDIDATE, CONTROL, PILOT
from orientation_gate import GateError, main, pilot_reference, qualify_pictures
from test_orientation import LABEL, encoded, records


def plan_for(pilot):
    plan = dict(purpose="startup-orientation-pilot" if pilot else "startup-orientation-reuse", label=LABEL,
                device="Kindle Scribe 2022", firmware="5.19.5", clock="CLOCK_MONOTONIC", launcher_wait_seconds=2,
                schedule=PILOT if pilot else SCHEDULE, control_features=CONTROL, candidate_features=CANDIDATE)
    if not pilot:
        plan.update(holding="down", raw_rotation=3, expected_panel_alignment_degrees=180)
    return plan


def closeout():
    sequence = [("admit", "none"), ("stage", "none"), ("case", "A-library"), ("case", "B-library"),
                ("restore", "none"), ("collect", "none"), ("close", "none")]
    return dict(passed=True, label=LABEL, released="2026-11-21T02:12:34+00:00",
                operations=[dict(mode=mode, case=case, status=0) for mode, case in sequence])


def public_fields(route):
    image = Image.new("L", (1860, 2480), 255)
    top = 185 if route == "library" else 1350
    ImageDraw.Draw(image).rectangle((92, top, 1320, top + 820), fill=25)
    panel = image.transpose(Image.Transpose.ROTATE_180)
    meta = dict(route=route, profile="scribe", backend="kindle", window_size=image.size, snapshot_size=image.size)
    return {"final.png": encoded(image, "PNG"), "panel.pgm": encoded(panel, "PPM"),
            "thumb.pgm": encoded(panel.resize((48, 64)), "PPM"), "final.metadata.json": json.dumps(meta).encode(),
            "portrait": b"D:3\n", "portrait-launch": b"D:3\n", "portrait-return": b"D:3\n",
            "portrait-source": b"native_idle_calibration\n"}


class PictureGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fields = {route: public_fields(route) for route in ("library", "reader")}

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.archive = self.root / "pictures.tgz"

    def make_archive(self, cases=PILOT, overrides=None, label=LABEL, extra=()):
        leaves = {"out/orientation-batch": (label + "\n").encode()}
        for index, case in enumerate(cases):
            request, markers = records(case, 30 + 8 * index)
            for name, value in (self.fields[case[2:]] | {"request": request.encode(), "markers": markers.encode()}).items():
                leaves[f"out/{case}/{name}"] = value
        leaves.update(overrides or {})
        with tarfile.open(self.archive, "w:gz", compresslevel=1) as archive:
            for name, data in [*leaves.items(), *extra]:
                if data is not None:
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))

    def test_pilot_reference_is_from_the_control_only_after_closed_qualification(self):
        self.make_archive()
        result = pilot_reference(plan_for(True), closeout(), self.archive)
        self.assertEqual(result, dict(purpose="startup-orientation-pilot", label=LABEL, passed=True,
                                      source_case="A-library", route="library", device="Kindle Scribe 2022",
                                      firmware="5.19.5", expected_panel_alignment_degrees=180, raw_rotation=3,
                                      holding="down", released=closeout()["released"], software_scanout_qualified=True))
        upright = encoded(Image.open(io.BytesIO(self.fields["library"]["final.png"])), "PPM")
        self.make_archive(overrides={"out/B-library/panel.pgm": upright})
        with self.assertRaisesRegex(GateError, "B-library: scanout alignment"):
            pilot_reference(plan_for(True), closeout(), self.archive)
        self.make_archive(overrides={f"out/B-library/{name}": self.fields["reader"][name] for name in ("panel.pgm", "final.png")})
        with self.assertRaisesRegex(GateError, "B-library: candidate and control picture bodies differ"):
            pilot_reference(plan_for(True), closeout(), self.archive)

    def test_full_four_picture_gate_is_not_a_pass_for_timing_or_physical_acceptance(self):
        self.make_archive(SCHEDULE[:4])
        result = qualify_pictures(plan_for(False), self.archive)
        self.assertTrue(result["pictures_passed"])
        self.assertEqual(list(result["qualification"]), SCHEDULE[:4])
        self.assertEqual(set(result["capture_pairs"]), {"library", "reader"})
        self.assertTrue(set(result).isdisjoint({"passed", "timing_gate_passed", "released"}))
        for field, value, reason in [("holding", "up", "pose"), ("raw_rotation", 1, "rotation"),
                                     ("expected_panel_alignment_degrees", 0, "alignment")]:
            with self.subTest(field=field), self.assertRaisesRegex(GateError, f"A-library: .*{reason}"):
                qualify_pictures(plan_for(False) | {field: value}, self.archive)

    def test_pilot_operations_require_both_correct_cases_then_clean_close_and_release(self):
        for change in [("released", None), ("released", "unknown"), ("label", "1" * 16),
                       ("passed", False), ("recovery_required", True), ("error", "interrupted"),
                       ("operations", closeout()["operations"][:-1])]:
            with self.subTest(field=change[0]), self.assertRaises(GateError):
                pilot_reference(plan_for(True), closeout() | dict([change]), self.archive)
        for key, value in [("case", "A-library"), ("status", False), ("status", 2)]:
            ops = closeout()
            ops["operations"][3][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(GateError, "case/B-library"):
                pilot_reference(plan_for(True), ops, self.archive)
        with self.assertRaises(GateError):
            pilot_reference(plan_for(True) | {"candidate_features": CONTROL}, closeout(), self.archive)

    def test_pilot_rejects_mixed_pose_unconfirmed_measurements_and_clock_or_path_errors(self):
        for case, field, value in [("B-library", "portrait-launch", b"U:3\n"),
                                   ("A-library", "portrait", b"FIXED:3\n"),
                                   ("B-library", "portrait-source", b"unconfirmed\n"),
                                   ("B-library", "portrait-return", b"D:2\n"),
                                   ("A-library", "portrait", None),
                                   ("B-library", "request", b"REQUEST 30.000000000\n"),
                                   ("B-library", "markers", records("B-library", 38, fallback=True)[1].encode()),
                                   ("A-library", "markers", records("B-library", 30)[1].encode()),
                                   ("A-library", "final.metadata.json", b'{}'),
                                   ("B-library", "thumb.pgm", encoded(Image.new("L", (47, 64)), "PPM"))]:
            with self.subTest(case=case, field=field):
                self.make_archive(overrides={f"out/{case}/{field}": value})
                with self.assertRaisesRegex(GateError, case):
                    pilot_reference(plan_for(True), closeout(), self.archive)

    def test_archive_must_belong_to_this_batch_and_this_stage(self):
        for options in [dict(label="f" * 16), dict(extra=[("out/orientation-batch", (LABEL + "\n").encode())]),
                        dict(extra=[("out/A1/request", b"future sample")]), dict(extra=[("out/../private", b"other")])]:
            with self.subTest(options=options), self.assertRaises(GateError):
                self.make_archive(**options)
                pilot_reference(plan_for(True), closeout(), self.archive)

    def test_cli_writes_no_reference_on_failure_and_preserves_existing_reference(self):
        plan, ops, reference = (self.root / name for name in ("plan.json", "ops.json", "pilot-reference.json"))
        plan.write_text(json.dumps(plan_for(True)))
        ops.write_text(json.dumps(closeout()))
        args = ["pilot", *(str(path) for path in (plan, ops, self.archive, reference))]
        self.make_archive(overrides={"out/B-library/portrait-launch": b"U:3\n"})
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            main(args)
        self.assertFalse(reference.exists())
        self.make_archive()
        self.assertEqual(main(args), 0)
        self.assertTrue(json.loads(reference.read_text())["passed"])
        reference.write_text("existing")
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            main(args)
        self.assertEqual(reference.read_text(), "existing")


if __name__ == "__main__":
    unittest.main()
