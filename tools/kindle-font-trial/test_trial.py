import gzip
import io
import json
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import unittest

from PIL import Image, ImageDraw

from analyze import analyze, metrics
from freeze import HERE, ROOT, SCHEDULE, device_body, launcher
from orientation_contract import CANDIDATE
from run import close_transaction, is_installed_retest, transport


class AnalysisTests(unittest.TestCase):
    def test_collection_error_does_not_skip_verified_close(self):
        for error in (OSError("No room for transfer"), KeyboardInterrupt()):
            calls = []

            def device(mode, **_kwargs):
                calls.append(mode)
                if mode == "collect":
                    raise error
                return True

            self.assertEqual(close_transaction(device, None), (True, False))
            self.assertEqual(calls, ["restore", "collect", "close"])

    def test_restore_alone_never_confirms_close(self):
        for failed, expected in (("close", ["restore", "collect", "close"]),
                                 ("restore", ["restore", "restore", "collect"])):
            calls = []

            def device(mode, **_kwargs):
                calls.append(mode)
                return mode != failed

            self.assertEqual(close_transaction(device, None), (False, True))
            self.assertEqual(calls, expected)

    def test_interrupted_close_does_not_grant_release(self):
        def device(mode, **_kwargs):
            if mode == "close":
                raise KeyboardInterrupt
            return True

        with self.assertRaises(KeyboardInterrupt):
            close_transaction(device, None)

    def test_pixel_and_per_case_route_gates(self):
        def encoded(image, kind):
            output = io.BytesIO()
            image.save(output, format=kind)
            return output.getvalue()

        with tempfile.TemporaryDirectory() as temporary:
            for mistaken in (False, True):
                path = Path(temporary) / f"case-{mistaken}.tgz"
                frames = {}
                with tarfile.open(path, "w:gz", compresslevel=1) as archive:
                    def put(case, leaf, body):
                        member = tarfile.TarInfo(f"out/{case}/{leaf}")
                        member.size = len(body)
                        archive.addfile(member, io.BytesIO(body))

                    for route in ("library", "reader"):
                        image = Image.new("L", (1860, 2480), 255)
                        ImageDraw.Draw(image).rectangle((50, 180 if route == "library" else 1400,
                                                        1400, 1100 if route == "library" else 2200), fill=25)
                        frames[route] = encoded(image.resize((48, 64)), "PPM")
                        for lane in "AB":
                            case = lane + "-" + route
                            meta = {"route": route, "profile": "scribe", "backend": "kindle",
                                    "snapshot_size": [1860, 2480], "window_size": [1860, 2480]}
                            put(case, "final.metadata.json", json.dumps(meta).encode())
                            put(case, "final.png", encoded(image, "PNG"))
                            put(case, "panel.pgm", encoded(image.transpose(Image.Transpose.ROTATE_180), "PPM"))
                            put(case, "thumb.pgm", frames[route])
                    for case in SCHEDULE[4:]:
                        stages = "main_enter splash_orientation_begin splash_orientation_timeout window_created fonts_registered software_ready event_loop_orientation_begin event_loop_orientation_success first_application_frame_send".split()
                        tick = 2 if case[0] == "A" else 1
                        put(case, "request", b"REQUEST 1.000000000\n")
                        put(case, "markers", "".join(f"WEREAD_STARTUP {tick}.{index:09d} {name}\n" for index, name in enumerate(stages, 1)).encode())
                        put(case, "thumb.pgm", frames["reader" if mistaken and case == "B4" else "library"])
                if mistaken:
                    with self.assertRaisesRegex(ValueError, "B4: Library content"):
                        analyze(path)
                else:
                    result = analyze(path)
                    self.assertEqual(len(result["samples"]), 14)
                    self.assertEqual(result["request_to_send_ms"]["improved_pairs"], 6)
                    self.assertEqual(result["qualification"]["A-library"]["aligned_degrees"], 180)

    def test_short_transport_preserves_payload(self):
        script = 'set -eu\n' + ('# filler\n' * 1400) + 'printf "%s\\n" "$1"; cat\n'
        command, header = transport(script, "stage", "0123456789abcdef", "none")
        self.assertLess(len(command), 240)
        payload = b"\x00\xff\npayload"
        result = subprocess.run(["sh", "-c", command], input=header + payload, capture_output=True)
        self.assertEqual((result.returncode, result.stdout), (0, b"stage\n" + payload))
        short = subprocess.run(["sh", "-c", command], input=header[:-20], capture_output=True)
        self.assertEqual((short.returncode, short.stdout), (64, b""))

    def test_schedule(self):
        self.assertEqual(len(set(SCHEDULE)), 18)
        self.assertEqual(SCHEDULE[4:], "A0 B0 A1 B1 B2 A2 A3 B3 B4 A4 A5 B5 B6 A6".split())

    def test_only_the_two_font_plan_types_can_select_a_device_schedule(self):
        self.assertFalse(is_installed_retest({}))
        self.assertTrue(is_installed_retest({"purpose": "installed-retest"}))
        for purpose in (None, "", "startup-orientation-reuse", "other"):
            with self.assertRaisesRegex(ValueError, "Unsupported plan purpose"):
                is_installed_retest({"purpose": purpose})

    def test_monotonic_endpoint(self):
        stages = "main_enter splash_orientation_begin splash_orientation_success window_created fonts_registered software_ready event_loop_orientation_begin event_loop_orientation_timeout first_application_frame_send".split()
        lines = [f"WEREAD_STARTUP 123.{index:09d} {stage}" for index, stage in enumerate(stages, 1)]
        result = metrics("REQUEST 122.000000000\n", "\n".join(lines))
        self.assertAlmostEqual(result["request_to_send_ms"], 1000.000009)
        self.assertEqual(result["event_loop"]["outcome"], "timeout")
        for invalid in [lines[:-1], lines + lines[-1:], list(reversed(lines))]:
            with self.assertRaises(ValueError):
                metrics("REQUEST 122.000000000\n", "\n".join(invalid))

    def test_common_launcher(self):
        original = (ROOT / "packaging/kindle/launch.sh").read_text()
        before, after = original.splitlines(), launcher(original).splitlines()
        self.assertEqual(len(before), len(after))
        changes = [right for left, right in zip(before, after) if left != right]
        self.assertEqual(len(changes), 1)
        self.assertIn('--demo --offline --screen "$WEREAD_TRIAL_ROUTE"', changes[0])
        self.assertIn("sleep 2", after)

    def test_normal_kindle_package_selects_font_preflight_and_orientation_reuse(self):
        source = (ROOT / "scripts/build-kindle.sh").read_text()
        command, = [shlex.split(line) for line in source.splitlines() if line.startswith("cargo zigbuild ")]
        self.assertIn("--no-default-features", command)
        self.assertCountEqual(command[command.index("--features") + 1].split(","), CANDIDATE)
        self.assertEqual(command[command.index("--target") + 1], "$TARGET")

    def test_document_consumes_one_arm(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            label = "0011223344556677"
            body = (HERE / "document.sh.in").read_text().replace("/mnt/us", str(root)).replace("@LABEL@", label)
            doc = root / "trial.sh"
            doc.write_text(body)
            stage = root / (".weread-indexed-" + label)
            out = stage / "out/B-reader"
            out.mkdir(parents=True)
            (stage / "owner").write_text(label)
            (stage / "armed").write_text("B-reader\n")
            home = stage / "B"
            home.mkdir()
            (home / "launch.sh").write_text(
                'sleep 0.1; printf "%s\\n" "$1" "$WEREAD_TRIAL_ROUTE" "$WEREAD_STARTUP_TIMING" "$WEREAD_TRIAL_CAPTURE"\n')
            self.assertEqual(subprocess.run(["sh", doc]).returncode, 0)
            self.assertFalse((stage / "armed").exists())
            self.assertEqual((out / "entry.log").read_text().splitlines(), ["--library", "reader", "1", str(out / "final.png")])
            self.assertNotEqual(subprocess.run(["sh", doc]).returncode, 0)


class DeviceFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.volume = self.root / "volume"
        self.docs = self.volume / "documents"
        self.docs.mkdir(parents=True)
        self.label = "0011223344556677"
        self.stage = self.volume / (".weread-indexed-" + self.label)
        self.doc = self.docs / ("weread-font-trial-" + self.label + ".sh")
        self.daily = self.docs / "weread.sh"
        self.daily.write_bytes(b"untouched")
        script = device_body().replace("/mnt/us", str(self.volume))
        boundary = 'case "$mode" in\n'
        stubs = '''
native() { :; }
state() { printf 'fixture\\n'; }
peer_proof() { :; }
count_doc() { if [ -f "$doc" ]; then echo 1; else echo 0; fi; }
wait_catalog() { [ "$(count_doc)" = "$1" ]; }
scan() { :; }
new_stage() { absent "$stage" && absent "$doc" && sidecars_absent || fail occupied; }
'''
        self.script = self.root / "device"
        self.script.write_text(script.replace(boundary, stubs + boundary))
        self.inputs = io.BytesIO()
        with tarfile.open(fileobj=self.inputs, mode="w:gz") as archive:
            names = ["document", "measure", "seed.tar.gz"]
            names.extend(f"{lane}/{file}" for lane in "AB" for file in ("launch.sh", "app/weread", "app/weread-frame-guard", "app/weread-native-query"))
            for name in names:
                body = name.split("/")[-1].encode()
                item = tarfile.TarInfo(name)
                item.size = len(body)
                archive.addfile(item, io.BytesIO(body))

    def call(self, mode, payload=b""):
        result = subprocess.run(["sh", self.script, mode, self.label], input=payload, capture_output=True)
        self.assertEqual(self.daily.read_bytes(), b"untouched")
        return result.returncode, result.stdout.strip(), result.stderr.strip()

    def test_scoped_cleanup(self):
        self.assertEqual(self.call("stage", self.inputs.getvalue())[:2], (0, b"indexed"))
        self.assertTrue(self.doc.is_file())
        self.assertEqual(self.call("restore")[:2], (0, b"restored"))
        self.assertEqual(self.call("close")[:2], (0, b"closed"))
        self.assertEqual(list(self.docs.iterdir()), [self.daily])
        self.assertFalse(self.stage.exists())

    def test_changed_document_is_not_removed(self):
        self.assertEqual(self.call("stage", self.inputs.getvalue())[0], 0)
        self.doc.write_bytes(b"replacement")
        self.assertNotEqual(self.call("restore")[0], 0)
        self.assertEqual(self.doc.read_bytes(), b"replacement")
        self.assertNotEqual(self.call("close")[0], 0)

    def test_partial_upload_does_not_publish(self):
        self.assertNotEqual(self.call("stage", gzip.compress(b"not tar"))[0], 0)
        self.assertFalse(self.doc.exists())
        self.assertEqual(self.call("restore")[0], 0)
        self.assertEqual(self.call("close")[0], 0)
        self.assertFalse(self.stage.exists())

    def test_unknown_stage_or_document(self):
        self.doc.write_bytes(b"existing")
        self.assertNotEqual(self.call("stage", self.inputs.getvalue())[0], 0)
        self.assertEqual(self.doc.read_bytes(), b"existing")
        self.doc.unlink()
        self.stage.mkdir()
        (self.stage / "owner").write_text("different\n")
        self.assertNotEqual(self.call("close")[0], 0)
        self.assertTrue(self.stage.is_dir())


if __name__ == "__main__":
    unittest.main()
