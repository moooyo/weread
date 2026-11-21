"""Exercise the complete observer with an owned Linux child and public software frames."""

import os
import struct
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

from pose_splash_reference import reference


HERE = Path(__file__).resolve().parent
ASSETS = Path(os.environ.get("POSE_SPLASH_TEST_ASSETS", HERE.parents[1] / "assets/startup"))
VISIT = "B-reader"
LABEL = "1122334455667788"
NONCE = "2" * 32
STRIDE, HEIGHT = 1872, 2480


def reference_page(kind, inverse):
    paper = 0 if inverse else 255
    result = bytearray([paper]) * (STRIDE * HEIGHT)
    if kind == "blank":
        return result
    for name, left, top in [("app-name", 694, 1088), ("starting", 824, 1201)]:
        if kind == "no_title" and name == "app-name":
            continue
        data = (ASSETS / (name + ".gray")).read_bytes()
        width, height = struct.unpack_from("<II", data)
        for row in range(height):
            line = data[8 + row * width:8 + (row + 1) * width]
            if inverse:
                line = bytes(255 - gray for gray in line)
            for angle in ((0, 180) if kind == "both" else (180,) if kind == "turned" else (0,)):
                offset = (top + row) * STRIDE + left if not angle else (HEIGHT - 1 - top - row) * STRIDE + 1860 - left - width
                result[offset:offset + width] = line if not angle else line[::-1]
    return result


def marker(stage, ns):
    return f"WEREAD_STARTUP {ns // 10**9}.{ns % 10**9:09d} {stage}\n"


class SplashObserver(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        cls.base = Path(cls.temp.name)
        cls.prefix = str(cls.base / ".weread-orientation-")
        cls.lock = cls.base / "lock"
        cls.stage = Path(cls.prefix + LABEL)
        cls.bin = cls.stage / "B/app/weread"
        cls.log = cls.stage / "B/data/logs/application-bootstrap.log"
        for parent in (cls.lock, cls.bin.parent, cls.log.parent, cls.stage / "out" / VISIT / "entered"):
            parent.mkdir(parents=True, exist_ok=True)
        (cls.stage / "owner").write_text(LABEL + "\n")
        (cls.stage / "out" / VISIT / "entered/arm").write_text(VISIT + "\n")
        (cls.base / "pose_splash_points.h").write_text("".join("{%d,%d,%d,%d},\n" % point for point in reference(ASSETS)))
        subprocess.run(["cc", "-x", "c", "-", "-o", cls.bin], input=b"#include <unistd.h>\nint main(void){for(;;)pause();}", check=True)
        cls.observer = cls.base / "observer"
        cls.prefix_command = []
        compiler = ["cc"]
        if os.environ.get("POSE_SPLASH_TEST_ARM") == "1":
            compiler = ["zig", "cc", "-target", "arm-linux-musleabihf", "-mcpu=cortex_a8", "-static", "-s"]
            cls.prefix_command = ["qemu-arm", "-cpu", "cortex-a8"]
        subprocess.run([*compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2", "-I", cls.base,
                        "-DPOSE_SPLASH_FIXTURE", f'-DPOSE_STAGE_PREFIX="{cls.prefix}"', f'-DPOSE_LOCK="{cls.lock}"',
                        HERE / "pose_splash.c", "-o", cls.observer], check=True)

    def check(self, kind, expected, *, inverse=False, fault=""):
        page = self.base / "page"
        page.write_bytes(reference_page(kind, inverse))
        self.addCleanup(page.unlink, missing_ok=True)
        env = dict(os.environ, POSE_SPLASH_TEST_PAGE=str(page))
        if inverse:
            env["POSE_SPLASH_TEST_INVERSE"] = "1"
        args = [self.bin, "--kindle", "--device", "scribe", "--data-dir", self.stage / "B/data", "--demo", "--offline",
                "--screen", "reader", "--exit-after-ms", "35000", "--capture", self.stage / "out" / VISIT / "final.png"]
        with self.log.open("w") as log:
            with subprocess.Popen(args, stderr=log if fault != "foreign_log" else subprocess.DEVNULL) as gui:
                try:
                    birth = Path(f"/proc/{gui.pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
                    for leaf, value in {"nonce": NONCE, "generation": f"1:{NONCE}", "application-pid": str(gui.pid),
                                        "application-birth": birth, "application-owned": f"1:{NONCE}:{gui.pid}:{birth}"}.items():
                        (self.lock / leaf).write_text(value + "\n")
                    if fault == "wrong_owner":
                        (self.lock / "application-owned").write_text("1:stale\n")
                    start = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
                    log.write(marker("startup_feedback_ready", start))
                    if fault == "duplicate":
                        log.write(marker("startup_feedback_ready", start + 1))
                    log.flush()
                    with subprocess.Popen([*self.prefix_command, self.observer, self.stage, VISIT], env=env,
                                          text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as observer:
                        time.sleep(0.30)
                        finish = start + 1 if fault == "late" else time.clock_gettime_ns(time.CLOCK_MONOTONIC)
                        log.write(marker("startup_feedback_finished", finish)); log.flush()
                        output, error = observer.communicate(timeout=5)
                    self.assertEqual(observer.returncode, expected, (error, output))
                    if expected in (0, 20):
                        fields = output.split()
                        self.assertEqual((len(fields), fields[0], fields[3]), (6, "SPLASH", "3"))
                        begin, end, _, zero, turned = map(int, fields[1:])
                        self.assertTrue(start < begin <= end < finish)
                        actual, other = (zero, turned) if expected == 0 else (turned, zero)
                        self.assertEqual(actual, 0); self.assertGreaterEqual(other, 64)
                        self.assertEqual(error, "" if expected == 0 else "splash_refused rotated\n")
                    else:
                        self.assertEqual(output, "")
                        reason = {"duplicate": "markers", "wrong_owner": "owner_missing", "late": "window_missed",
                                  "foreign_log": "owner_missing"}.get(fault, "ambiguous")
                        self.assertEqual(error, "splash_refused " + reason + "\n")
                finally:
                    gui.terminate()

    def test_upright(self): self.check("normal", 0)
    def test_native_inverted_grayscale(self): self.check("normal", 0, inverse=True)
    def test_rotated_is_a_measured_failure(self): self.check("turned", 20)
    def test_blank(self): self.check("blank", 1)
    def test_partial(self): self.check("no_title", 1)
    def test_ambiguous_half_turn(self): self.check("both", 1)
    def test_sample_after_finished(self): self.check("normal", 1, fault="late")
    def test_duplicate_marker(self): self.check("normal", 1, fault="duplicate")
    def test_foreign_owner(self): self.check("normal", 1, fault="wrong_owner")
    def test_foreign_log_inode(self): self.check("normal", 1, fault="foreign_log")


if __name__ == "__main__":
    unittest.main()
