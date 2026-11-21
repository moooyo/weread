"""Exercise only a synthetic Xvfb screen, using the real ARM programs under QEMU."""

import json
import fcntl
import os
from pathlib import Path
import runpy
import secrets
import subprocess
import tempfile
import unittest

from PIL import Image, ImageDraw, ImageFont


ROOT = Path("/work")
LIBS = ROOT / "runtime/usr/lib/arm-linux-gnueabihf"
MODELS = ROOT / "runtime/usr/share/tesseract-ocr/5/tessdata"


class TileTests(unittest.TestCase):
    def setUp(self):
        if os.environ.get("WEREAD_XVFB_FIXTURE") != "1" or Path.cwd() != ROOT or not os.environ.get("DISPLAY"):
            self.fail("Use only the dedicated synthetic Xvfb screen")
        self.display = runpy.run_path(str(ROOT / "test-native-target.py"))["FixtureDisplay"]()
        self.addCleanup(self.display.close)
        self.cover = Image.open(ROOT / "weread-cover-thumbnail.png").convert("L")
        self.font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 56)
        self.label = secrets.token_hex(8)
        source = (ROOT / "test-mnt/us/.synthetic-media").stat()
        mounted = Path("/mnt/us/.synthetic-media").stat()
        self.assertEqual((source.st_dev, source.st_ino), (mounted.st_dev, mounted.st_ino))
        stage = Path("/mnt/us") / (".weread-render-" + self.label)
        stage.mkdir(mode=0o700)
        self.addCleanup(stage.rmdir)
        owner = stage / "owner"
        owner.write_text(self.label + "\n")
        self.addCleanup(owner.unlink)
        self.intent = stage / "tile-dispatch-intent"
        self.addCleanup(self.intent.unlink, missing_ok=True)
        active = stage / "observer-active"
        active.write_text(self.label + "\n")
        self.addCleanup(active.unlink)
        self.monitor = active.open("r")
        self.addCleanup(self.monitor.close)
        fcntl.flock(self.monitor, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def paint(self, covers, tab="library", text=None):
        image = Image.new("L", (1860, 2480), 255)
        drawing = ImageDraw.Draw(image)
        for column, word in ((330, "Home"), (1270, "Library")):
            drawing.text((column, 2350), word, 0, font=self.font)
            if word.lower() == tab:
                box = drawing.textbbox((column, 2350), word, font=self.font)
                drawing.rectangle((box[0], box[3] + 8, box[2], box[3] + 12), fill=0)
        if text:
            drawing.text((85, 602), text, 0, font=self.font)
        for picture, column, row, width in covers:
            image.paste(picture.resize((width, width * 3 // 2), Image.Resampling.LANCZOS), (column, row))
        self.display.paint(image)

    def invoke(self, mode, accepted=True):
        active = mode not in ("card", "navigation")
        program = ROOT / ("card-input" if active else "native-target")
        with tempfile.TemporaryDirectory() as work:
            process = subprocess.run(["qemu-arm", "-cpu", "cortex-a9", LIBS / "ld-linux-armhf.so.3", "--inhibit-cache",
                                      "--library-path", LIBS, program, mode, MODELS], cwd=work,
                                     env=dict(os.environ, OMP_THREAD_LIMIT="1", WEREAD_TILE_LABEL=self.label), capture_output=True, timeout=65)
            self.assertEqual(list(Path(work).iterdir()), [])
        self.assertNotIn(b"Private placeholder", process.stdout + process.stderr)
        if not accepted:
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(process.stdout, b"")
            return
        self.assertEqual(process.returncode, 0, process.stderr)
        return json.loads(process.stdout)

    def test_scaled_unique_public_tile_has_monotonic_button_release(self):
        self.paint([(self.cover, 779, 443, 316)], text="Private placeholder")
        located = self.invoke("card")
        self.assertEqual(located["title"]["count"], 1)
        self.assertLess(located["home"]["underline_pct"], 21)
        self.assertGreater(located["library"]["underline_pct"], 59)
        self.assertFalse(self.intent.exists())
        result = self.invoke("open-card")
        self.assertEqual(self.intent.read_bytes(), (self.label + "\n").encode())
        self.invoke("open-card", False)
        self.assertEqual(result["target"], "open-card")
        self.assertEqual(result["clock"], "CLOCK_MONOTONIC")
        self.assertTrue(result["down_confirmed"] and result["buttons_clear_after"])
        self.assertLessEqual(result["down_before_ns"], result["down_after_ns"])
        self.assertGreaterEqual(result["up_before_ns"], result["down_after_ns"] + 50_000_000)
        self.assertLessEqual(result["up_before_ns"], result["up_after_ns"])

    def test_double_artwork_even_at_different_scales_does_not_click(self):
        self.paint([(self.cover, 70, 235, 256)])
        self.assertEqual(self.invoke("card")["title"]["count"], 1)
        self.paint([(self.cover, 70, 235, 256), (self.cover, 1020, 1050, 330)])
        self.assertEqual(self.invoke("card")["title"]["count"], 2)
        self.invoke("open-card", False)
        self.assertFalse(self.intent.exists())

    def test_paperback_text_without_the_public_penguin_is_not_a_tile(self):
        impostor = self.cover.copy()
        ImageDraw.Draw(impostor).rectangle((77, 146, 177, 289), fill=250)
        self.paint([(impostor, 85, 671, 350)], text="WeRead for Kindle")
        self.assertEqual(self.invoke("card")["title"]["count"], 0)
        self.invoke("open-card", False)
        self.assertFalse(self.intent.exists())

    def test_home_recent_tile_never_authorizes_library_click(self):
        self.paint([(self.cover, 80, 260, 256)], tab="home")
        self.invoke("open-card", False)
        self.assertFalse(self.intent.exists())

    def test_dead_or_cancelled_observer_cannot_issue_an_intent(self):
        self.paint([(self.cover, 779, 443, 316)])
        fcntl.flock(self.monitor, fcntl.LOCK_UN)
        self.invoke("open-card", False)
        self.assertFalse(self.intent.exists())
        fcntl.flock(self.monitor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        cancel = self.intent.with_name("render-cancel")
        cancel.write_text(self.label + "\n")
        self.addCleanup(cancel.unlink)
        self.invoke("open-card", False)
        self.assertFalse(self.intent.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
