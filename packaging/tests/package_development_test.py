#!/usr/bin/env python3
"""Check the explicit Kindle development archive contract with synthetic files."""

from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile


PACKAGE_SCRIPT = Path(__file__).resolve().parents[2] / "scripts/package-development.py"
SPEC = importlib.util.spec_from_file_location("weread_package_development", PACKAGE_SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise ImportError("The development archive module could not be loaded")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)

APPLICATION = "weread/app/weread"
LAUNCHER = "documents/weread.sh"

BUILD_INPUTS = {
    LAUNCHER: b"#!/bin/sh\n# Synthetic archive fixture; never executed.\n",
    APPLICATION: b"\x7fELF synthetic static application\x00\x02",
    "weread/app/README.md": b"# Fixture application\n",
    "weread/app/THIRD_PARTY_NOTICES.md": b"# Fixture dependency notices\n",
    "weread/app/config/device-profile.example": b"scribe\n",
    "weread/app/config/device-allowlist.example": b"# No accepted fixture devices.\n",
    "weread/app/manifest.draft.json": b'{"manifest_version": 2}\n',
}
NOTICE_INPUTS = {
    "weread/app/licenses/backend/LICENSE-MIT": b"Synthetic dependency notice.\n",
    "weread/app/licenses/fonts/OFL.txt": b"Synthetic font notice.\n",
    "weread/app/licenses/zip/LICENSE": b"Synthetic ZIP dependency notice.\n",
    "weread/app/licenses/zip/UPSTREAM_REVISION": b"Synthetic pinned ZIP source.\n",
    "weread/app/licenses/zip/PATCHES.md": b"# Synthetic metadata-budget patch\n",
}


class DevelopmentArchiveTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="weread-package-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.staged = self.root / "staged"
        self.output = self.root / "output/weread-development.zip"
        for name, payload in (BUILD_INPUTS | NOTICE_INPUTS).items():
            self.write_staged(name, payload)

    def write_staged(self, name: str, payload: bytes) -> None:
        path = self.staged / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)

    def test_missing_application_refuses_before_creating_archive(self) -> None:
        (self.staged / APPLICATION).unlink()

        with self.assertRaisesRegex(ValueError, "weread/app/weread"):
            PACKAGE.archive(self.staged, self.output)

        self.assertFalse(self.output.exists())
        self.assertFalse(self.output.with_suffix(".sha256").exists())

    def test_application_payload_permissions_and_checksums_survive_archiving(self) -> None:
        checksums = PACKAGE.archive(self.staged, self.output)

        with zipfile.ZipFile(self.output) as archive:
            self.assertEqual(archive.read(APPLICATION), BUILD_INPUTS[APPLICATION])
            self.assertEqual(archive.read(LAUNCHER), BUILD_INPUTS[LAUNCHER])
            for name in (APPLICATION, LAUNCHER):
                with self.subTest(executable=name):
                    information = archive.getinfo(name)
                    archived_mode = information.external_attr >> 16
                    self.assertEqual(information.create_system, 3)
                    self.assertTrue(stat.S_ISREG(archived_mode))
                    self.assertEqual(stat.S_IMODE(archived_mode), 0o755)

            application_digest = hashlib.sha256(archive.read(APPLICATION)).hexdigest()

        self.assertEqual(checksums[APPLICATION], application_digest)
        self.assertEqual(
            checksums[APPLICATION], hashlib.sha256(BUILD_INPUTS[APPLICATION]).hexdigest()
        )
        self.assertEqual(
            checksums[self.output.name], hashlib.sha256(self.output.read_bytes()).hexdigest()
        )
        sidecar = {}
        for line in self.output.with_suffix(".sha256").read_text(encoding="utf-8").splitlines():
            digest, separator, name = line.partition("  ")
            self.assertEqual(separator, "  ")
            sidecar[name] = digest
        self.assertEqual(set(sidecar), {APPLICATION, self.output.name})
        self.assertEqual(sidecar[APPLICATION], checksums[APPLICATION])
        self.assertEqual(sidecar[self.output.name], checksums[self.output.name])

    def test_runtime_private_and_unlisted_files_never_enter_archive(self) -> None:
        excluded = (
            "weread/data/account.json",
            "weread/data/demo/library.sqlite",
            "weread/app/runtime/private-record.json",
            "weread/app/operator-notes.md",
            "weread/app/unused-platform-helper",
            "weread/app/liblipc.so",
            "documents/extra-diagnostic.txt",
            "research/firmware/private.bin",
            "private/session.bin",
        )
        for name in excluded:
            self.write_staged(name, b"Synthetic excluded fixture; no real user data.\n")

        PACKAGE.archive(self.staged, self.output)

        with zipfile.ZipFile(self.output) as archive:
            names = set(archive.namelist())
        self.assertEqual(names, set(BUILD_INPUTS) | set(NOTICE_INPUTS))
        for name in excluded:
            with self.subTest(excluded=name):
                self.assertNotIn(name, names)


if __name__ == "__main__":
    unittest.main()
