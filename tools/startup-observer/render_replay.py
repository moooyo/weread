"""Exercise the daily selector offline against public Scribe frames, never a private Library."""

import argparse
import gzip
import io
from pathlib import Path
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFont, ImageOps


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frames", type=Path)
    parser.add_argument("serif_bold_gz", type=Path)
    args = parser.parse_args()
    code = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as temporary:
        scratch = Path(temporary)
        replay, unit = scratch / "replay", scratch / "unit"
        cc = ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"]
        subprocess.run([*cc, "-DPUBLIC_FRAME_REPLAY", str(code / "render-observer.c"), "-o", replay], check=True)
        subprocess.run([*cc, str(code / "render_observer_test.c"), "-o", unit], check=True)
        subprocess.run([unit], cwd=scratch, check=True)
        samples = []

        def verify(name, image, expected):
            assert image.size == (1860, 2480)
            observed = subprocess.run([replay], input=image.tobytes(), capture_output=True, check=True).stdout.decode().strip()
            if observed != expected:
                raise ValueError(f"{name}: expected {expected}, got {observed}")
            samples.append(name)

        def load(name):
            with Image.open(args.frames / name) as frame:
                return frame.convert("L")

        for name in ("physical-a.png", "physical-b.png", "library.png", "pilot-A.pgm", "pilot-A.png", "pilot-B.pgm", "pilot-B.png"):
            verify(name, load(name), "grid")
        verify("public-list", load("library-list.png"), "list")
        for name in ("detail.png", "settings.png"):
            verify(name, load(name), "none")
        reference = load("physical-a.png")
        for name, image in (("unsupported-half-turn", reference.transpose(Image.Transpose.ROTATE_180)),
                            ("unqualified-inversion", ImageOps.invert(reference))):
            verify(name, image, "none")
        for name, boxes in (("no-navigation", [(0, 2295, 1860, 2480)]), ("no-heading", [(40, 175, 240, 253)]),
                            ("no-shelf-content", [(0, 360, 1860, 2232)]),
                            ("cover-without-caption", [(0, 827, 1860, 945)]), ("no-sort-controls", [(45, 349, 650, 360)])):
            image = reference.copy()
            for box in boxes:
                image.paste(255, box)
            verify(name, image, "none")
        empty = reference.copy()
        empty.paste(255, (0, 360, 1860, 2232))
        label = "\u4e66\u67b6\u8fd8\u662f\u7a7a\u7684"
        face = ImageFont.truetype(io.BytesIO(gzip.decompress(args.serif_bold_gz.read_bytes())), 48)
        draw = ImageDraw.Draw(empty)
        draw.text((786, 1207 - face.getbbox(label)[1]), label, font=face, fill=0)
        verify("empty-title-alone", empty, "none")
        draw.rectangle((645, 1300, 911, 1395), fill=0)
        draw.rectangle((944, 1300, 1214, 1395), outline=0, width=4)
        verify("source-model-empty-label-and-buttons", empty, "empty")
        empty.paste(255, (785, 1200, 1075, 1270))
        verify("empty-buttons-alone", empty, "none")
        assert subprocess.run([replay], input=b"short", capture_output=True).returncode == 2
        print(f"Passed {len(samples)} image cases and rejected truncated input; the empty positive is a source model, not a physical frame.")


if __name__ == "__main__":
    main()
