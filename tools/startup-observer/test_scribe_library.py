"""Replay public Gray8 frames on a Linux verification host; never open a device."""

import argparse
from pathlib import Path
import subprocess
import tempfile

from PIL import Image, ImageOps


HERE = Path(__file__).resolve().parent
REVIEW = HERE.parents[1] / "docs/review"
SIZE = (1860, 2480)
GALLERY_POSITIVES = ("library", "library-list")
GALLERY_NEGATIVES = ("detail", "marks", "mp", "notes", "reader-menu", "reader", "search",
                     "settings", "stats", "storage", "toc", "type")


def load(path):
    with Image.open(path) as image:
        if image.format != "PNG" or image.size != SIZE:
            raise ValueError(f"Not a native-size Scribe PNG: {path.name}")
        return image.convert("L")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gallery", type=Path, help="directory of the public Scribe firmware-gallery PNGs")
    parser.add_argument("--negative", type=Path, action="append", default=[], help="another public non-Library PNG")
    args = parser.parse_args()
    reference = load(REVIEW / "startup-performance-20261117-3423e73/candidate-library.png")
    independent = load(REVIEW / "native-query-20261117-4bc333e/candidate-library.png")
    ink, paper = Image.new("L", SIZE, 0), Image.new("L", SIZE, 255)
    only_heading, only_tab = paper.copy(), paper.copy()
    for frame, box in ((only_heading, (64, 160, 190, 258)), (only_tab, (180, 2340, 280, 2410))):
        frame.paste(reference.crop(box), box)
    cases = [("reference", reference, True), ("other physical batch", independent, True),
             ("heading only", only_heading, False), ("tab only", only_tab, False),
             ("black", ink, False), ("white", paper, False),
             ("unqualified inverted polarity", ImageOps.invert(reference), False)]
    if args.gallery:
        for expected, names in ((True, GALLERY_POSITIVES), (False, GALLERY_NEGATIVES)):
            cases.extend((name, load(args.gallery / f"{name}.png"), expected) for name in names)
    cases.extend((path.name, load(path), False) for path in args.negative)
    with tempfile.TemporaryDirectory() as work:
        binary = Path(work) / "replay"
        subprocess.run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-DPUBLIC_FRAME_REPLAY",
                        str(HERE / "scribe-library.c"), "-o", str(binary)], check=True)
        for name, image, expected in cases:
            result = subprocess.run([binary], input=image.tobytes(), capture_output=True, check=True)
            if result.stdout != (b"MATCH\n" if expected else b"NO_MATCH\n"):
                raise ValueError(f"Unexpected classification: {name}")
        if subprocess.run([binary], input=b"", capture_output=True).returncode != 2:
            raise ValueError("Short fixture accepted")
    print(f"Passed {len(cases)} classifications and rejected an incomplete fixture.")


if __name__ == "__main__":
    main()
