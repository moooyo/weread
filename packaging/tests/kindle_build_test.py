#!/usr/bin/env python3
"""Check the resolved features of the everyday Kindle build without compiling it."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
TARGET = "armv7-unknown-linux-musleabihf"


class KindleBuildTests(unittest.TestCase):
    def test_release_uses_exact_pose_without_experimental_features(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Path(temporary)
            build_script = fixture / "scripts/build-kindle.sh"
            build_script.parent.mkdir()
            shutil.copyfile(ROOT / "scripts/build-kindle.sh", build_script)
            headers = fixture / "X11"
            headers.mkdir()
            for header in ("Xlib.h", "Xutil.h"):
                (headers / header).touch()
            cargo = fixture / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$BUILD_ARGS"\nexit 42\n')
            cargo.chmod(0o755)
            (fixture / "zig").symlink_to(cargo)
            arguments = fixture / "arguments"
            result = subprocess.run(["sh", build_script], cwd=fixture, env=os.environ | {
                "PATH": f"{fixture}:{os.environ['PATH']}", "BUILD_ARGS": str(arguments),
                "X11_HEADER_ROOT": str(fixture), "CARGO_TARGET_DIR": str(fixture / "target"),
            })
            self.assertEqual(result.returncode, 42)
            subcommand, *flags = arguments.read_text().splitlines()

        self.assertEqual(subcommand, "zigbuild")
        for required in ("--release", "--locked", "--no-default-features"):
            self.assertIn(required, flags)
        target_flag = flags.index("--target")
        self.assertEqual(flags[target_flag + 1], TARGET)
        flags[target_flag] = "--filter-platform"
        flags.remove("--release")
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--format-version=1", *flags], cwd=ROOT,
        ))
        nodes = {node["id"]: node["features"] for node in metadata["resolve"]["nodes"]}
        for name, expected in (
            ("weread", {"kindle", "bundled-font-preflight", "startup-orientation-reuse"}),
            ("slint-backend-kindle", {"startup-orientation-reuse"}),
        ):
            with self.subTest(package=name):
                package_id, = (entry["id"] for entry in metadata["packages"] if entry["name"] == name)
                self.assertCountEqual(nodes[package_id], expected)


if __name__ == "__main__":
    unittest.main()
