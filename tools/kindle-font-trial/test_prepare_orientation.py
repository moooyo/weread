import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from freeze import HERE, ROOT
from prepare_orientation import (CANDIDATE, CONTROL, DEVICE, FACES, FIRMWARE, HELPERS, PICTURES, PILOT,
                                 arm_inputs, check_request_helper, freeze, public_seed, seal, verify_prepared)


def archive_files(path, entries):
    with tarfile.open(path, "w:gz") as archive:
        for name, body in entries:
            member = tarfile.TarInfo(name)
            member.size = len(body)
            archive.addfile(member, io.BytesIO(body))


def save(path, record):
    path.write_text(json.dumps(record) + "\n")


class PrepareTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool_directory = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.tool_directory.cleanup)
        cls.tools = Path(cls.tool_directory.name)
        emulator = cls.tools / "qemu-arm"
        emulator.write_text('#!/bin/sh\n[ "$1:$2" = "-cpu:cortex-a8" ] || exit 99\nshift 2\nexec "$@"\n')
        emulator.chmod(0o700)
        for name, flags in (("orientation", ["-DWEREAD_ORIENTATION_TRIAL=1"]), ("font", [])):
            subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", *flags, str(HERE / "measure.c"),
                            "-o", str(cls.tools / name)], check=True)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name).resolve()
        self.source = self.work / "src"
        for name in ("packaging/kindle/document-launcher.sh", "packaging/kindle/launch.sh",
                     "docs/review/catalog-entry-adapter-20261118/library-cover-bindings.json",
                     "docs/review/library-startup-latency-20261119/rebuild-arm.py"):
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        for lane, features, size in [("baseline", CONTROL, 128), ("candidate", CANDIDATE, 135)]:
            path = self.work / "arm-pair" / lane
            path.mkdir(parents=True)
            elf = bytearray(size)
            elf[:7] = b"\x7fELF\1\1\1"
            struct.pack_into("<HH", elf, 16, 2, 40)
            struct.pack_into("<I", elf, 28, 52)
            struct.pack_into("<I", elf, 36, 0x5000400)
            struct.pack_into("<HH", elf, 42, 32, 1)
            struct.pack_into("<I", elf, 52, 1)
            (path / "weread").write_bytes(elf)
            save(path / "result.json", {"source": "/work/src", "lane": lane, "abi": "armv7-unknown-linux-musleabihf",
                "both_recompiled": True, "application_features": sorted(features), "generated_receipts": FACES,
                "backend_features": [] if lane == "baseline" else [CANDIDATE[-1]],
                "elf": {"bytes": size, "elf_flags": "0x5000400", "program_types": [1]}})
        self.helpers = self.work / "helpers"
        self.helpers.mkdir()
        shutil.copy2(self.tools / "orientation", self.helpers / "measure")
        for helper in HELPERS:
            (self.helpers / helper).write_text(helper)
        bindings = json.loads((self.source / "docs/review/catalog-entry-adapter-20261118/library-cover-bindings.json").read_bytes())
        self.entries = [("library-covers.json", json.dumps(bindings).encode()), ("demo/library.sqlite", b"SQLite format 3\0"),
                        ("demo/covers/cache-index.sqlite3", b"cache"), ("demo/images/cache-index.sqlite3", b"cache"),
                        ("demo/images/" + "0" * 64 + "-" + "1" * 64 + ".png", b"public image")]
        self.entries.extend((item["thumbnail"], b"public thumbnail") for item in bindings["targets"])
        self.seed = self.work / "seed.tar.gz"
        archive_files(self.seed, self.entries)
        self.pilot = self.work / "pilot"
        self.formal = self.work / "formal"
        self.enterContext(patch.dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"]))

    def prepare(self, path, pilot=False):
        return freeze(self.work, path, self.seed, self.helpers, pilot=pilot)

    def reference(self):
        pilot = self.prepare(self.pilot, pilot=True)
        attempt = self.pilot / "batch-fixture"
        attempt.mkdir()
        released = "2026-11-21T02:03:04.010000+00:00"
        modes = [("admit", "none"), ("stage", "none"), *(("case", case) for case in PICTURES),
                 ("restore", "none"), ("collect", "none"), ("close", "none")]
        save(attempt / "operations.json", {"label": pilot["label"], "passed": True, "released": released,
             "operations": [{"mode": mode, "case": case, "status": 0} for mode, case in modes]})
        proof = attempt / "pilot-reference.json"
        save(proof, {"label": pilot["label"], "purpose": PILOT, "source_case": "A-library", "holding": "down",
             "raw_rotation": 1, "expected_panel_alignment_degrees": 180, "route": "library", "device": DEVICE,
             "firmware": FIRMWARE, "passed": True, "software_scanout_qualified": True, "released": released})
        return proof

    def test_pilot_plan_separate_from_formal_seal_and_safe_document_namespace(self):
        reference = self.reference()
        plan = verify_prepared(self.pilot, self.work)
        self.assertEqual(plan["schedule"], PICTURES)
        for field in ("holding", "raw_rotation", "expected_panel_alignment_degrees"):
            self.assertNotIn(field, plan)
        with tarfile.open(self.pilot / "inputs.tgz") as archive:
            self.assertEqual(len(archive.getnames()), 11)
            self.assertEqual(archive.extractfile("A/launch.sh").read(), archive.extractfile("B/launch.sh").read())
            document = archive.extractfile("document").read().decode()
        self.assertIn("/mnt/us/.weread-orientation-" + plan["label"], document)
        self.assertNotIn("weread-indexed-", document)
        self.prepare(self.formal)
        self.assertFalse((self.formal / "plan.json").exists())
        seal(self.work, self.formal, reference)
        frozen = verify_prepared(self.formal, self.work)
        self.assertEqual((frozen["holding"], frozen["raw_rotation"], frozen["expected_panel_alignment_degrees"]), ("down", 1, 180))
        self.assertNotEqual(frozen["label"], plan["label"])
        self.assertEqual(frozen["pilot_label"], plan["label"])
        with self.assertRaises(FileExistsError):
            seal(self.work, self.formal, reference)

    def test_both_request_targets_and_invalid_label_without_a_native_service(self):
        label = "0123456789abcdef"
        check_request_helper(self.tools / "orientation", label, self.work)
        with self.assertRaisesRegex(ValueError, "orientation document"):
            check_request_helper(self.tools / "font", label, self.work)

    def test_bad_common_helper_or_partial_freeze_cannot_publish_or_reuse_a_plan(self):
        measure = self.helpers / "measure"
        measure.unlink()
        measure.symlink_to(self.tools / "orientation")
        with self.assertRaisesRegex(ValueError, "regular project-local"):
            self.prepare(self.pilot, pilot=True)
        self.assertFalse(self.pilot.exists())
        measure.unlink()
        shutil.copy2(self.tools / "font", measure)
        with self.assertRaisesRegex(ValueError, "orientation document"):
            self.prepare(self.pilot, pilot=True)
        self.assertFalse((self.pilot / "plan.json").exists())
        shutil.copy2(self.tools / "orientation", measure)
        with self.assertRaises(FileExistsError):
            self.prepare(self.pilot, pilot=True)

    def test_retained_arms_require_same_source_features_receipts_and_elf(self):
        path = self.work / "arm-pair/candidate/result.json"
        original = json.loads(path.read_bytes())
        for change in ({"source": "/other/src"}, {"lane": "baseline"}, {"backend_features": []},
                       {"application_features": CONTROL}, {"application_features": CANDIDATE + [CANDIDATE[-1]]},
                       {"generated_receipts": [FACES[0]] * 7}, {"both_recompiled": False},
                       {"elf": dict(original["elf"], bytes=127)}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                save(path, original | change)
                arm_inputs(self.work)

    def test_rejects_nonpublic_seed_members_and_missing_public_assets(self):
        for entries in (self.entries + [("demo/session.json", b"not allowed")], self.entries + [self.entries[0]], self.entries[:-1]):
            archive_files(self.seed, entries)
            with self.assertRaises(ValueError):
                public_seed(self.source, self.seed)

    def test_archive_script_and_feature_drift_fail_before_an_admission(self):
        self.prepare(self.pilot, pilot=True)
        program = self.pilot / "device.sh"
        before = program.read_bytes()
        program.write_bytes(before + b"\n")
        with self.assertRaisesRegex(ValueError, "inputs changed"):
            verify_prepared(self.pilot, self.work)
        program.write_bytes(before)
        with (self.pilot / "inputs.tgz").open("ab") as archive:
            archive.write(b"extra")
        with self.assertRaisesRegex(ValueError, "archive length"):
            verify_prepared(self.pilot, self.work)

    def test_reference_must_be_closed_separate_qualified_control(self):
        reference = self.reference()
        self.prepare(self.formal)
        original = json.loads(reference.read_bytes())
        for change in ({"source_case": "B-library"}, {"raw_rotation": True}, {"holding": "unknown"},
                       {"expected_panel_alignment_degrees": 90}, {"software_scanout_qualified": False},
                       {"released": "another-batch"}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                save(reference, original | change)
                seal(self.work, self.formal, reference)
            self.assertFalse((self.formal / "plan.json").exists())
        save(reference, original)
        ops = reference.parent / "operations.json"
        save(ops, json.loads(ops.read_bytes()) | {"recovery_required": True})
        with self.assertRaises(ValueError):
            seal(self.work, self.formal, reference)


if __name__ == "__main__":
    unittest.main()
