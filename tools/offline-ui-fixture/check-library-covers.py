#!/usr/bin/env python3
"""Check the book artwork in one production desktop Scribe Library capture."""

import argparse
import hashlib
import json
import re
from pathlib import Path

from PIL import Image, ImageChops, ImageStat


SIZE = (930, 1240)
BOOKS = tuple(f"demo-{i}" for i in range(15))
EXCLUDED = {f"article:demo-article-{i}" for i in range(3)}
TILE = (151, 210)
# Compare the interior below the update badge and above the progress bar.
INTERIOR = (5, 38, 146, 197)
MAX_MAE = 5.0
MAX_DIFFERENT = 0.045
MIN_MARGIN = 2.0


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_fixture(root):
    receipt = load_json(root / "library-covers.json")
    require(receipt.get("format") == "weread-offline-library-covers-v1" and
            receipt.get("profile") == "scribe", "Unexpected fixture identity")
    excluded = receipt.get("excluded_book_ids")
    require(isinstance(excluded, list) and len(excluded) == len(EXCLUDED) and
            set(excluded) == EXCLUDED, "Unexpected non-Library books")
    targets = receipt.get("targets")
    require(isinstance(targets, list) and len(targets) == len(BOOKS), "Expected 15 book targets")
    cache = root.resolve() / "demo" / "covers"
    require(cache.resolve() == cache, "Expected a fixture-owned cover directory")
    references = {}
    for target in targets:
        book = target["book_id"]
        require(book in BOOKS and book not in references, "Duplicate or unknown book target")
        row, column = divmod(BOOKS.index(book), 5)
        require((target["row"], target["column"]) == (row, column), f"Incorrect slot for {book}")
        url = f"https://example.invalid/library-covers/{book}.png"
        raw_id = book.encode("ascii")
        key = "cover:" + hashlib.sha256(len(raw_id).to_bytes(8, "big") + raw_id + url.encode("ascii")).hexdigest()
        require(target["cover_url"] == url and target["cache_key"] == key,
                f"Invalid source identity for {book}")
        sha = target["thumbnail_sha256"]
        filename = hashlib.sha256(key.encode("ascii")).hexdigest() + "-" + sha + ".png"
        path = root / target["thumbnail"]
        require(target["thumbnail"] == f"demo/covers/{filename}" and path.resolve().parent == cache
                and path.is_file(), f"Invalid thumbnail for {book}")
        require(re.fullmatch("[0-9a-f]{64}", sha) and digest(path) == sha,
                f"Thumbnail digest mismatch for {book}")
        with Image.open(path) as source:
            require(source.format == "PNG" and source.mode == "L" and source.size == (400, 560),
                    f"Unexpected cached grayscale PNG for {book}")
            # At scale one the product centers the image at its intrinsic size inside a clipped tile.
            x, y = (source.width - TILE[0]) // 2, (source.height - TILE[1]) // 2
            template = source.crop((x, y, x + TILE[0], y + TILE[1])).crop(INTERIOR)
        require(ImageStat.Stat(template).stddev[0] >= 25, f"Flat artwork reference for {book}")
        references[book] = template
    return references


def load_frame(path, metadata):
    fields = load_json(metadata)
    require(fields.get("route") == "library" and fields.get("backend") == "desktop" and
            fields.get("profile") == "scribe" and fields.get("scale_factor") == 1.0,
            "Expected a scale-1 desktop Scribe Library capture")
    for key in ("design_logical_size", "logical_size", "window_size", "snapshot_size"):
        require(fields.get(key) == list(SIZE), f"Unexpected capture {key}")
    with Image.open(path) as source:
        require(source.format == "PNG" and source.size == SIZE, "Unexpected capture PNG geometry")
        rgba = source.convert("RGBA")
        require(rgba.getchannel("A").getextrema() == (255, 255), "Capture contains transparent pixels")
        return rgba.convert("L")


def origin(book):
    row, col = divmod(BOOKS.index(book), 5)
    # Scribe cover grid: 32px margin, 28px column gap, 22px top inset.
    # The first row starts below the status, search, heading and sort bars.
    return round(32 + col * (150.8 + 28)), 28 + 52 + 52 + 48 + 22 + row * (210 + 36 + 34)


def bounds(book):
    x, y = origin(book)
    return (x, y, x + TILE[0], y + TILE[1])


def errors(reference, actual):
    hist = ImageChops.difference(reference, actual).histogram()
    count = sum(hist)
    return sum(i * n for i, n in enumerate(hist)) / count, sum(hist[25:]) / count


def assess(frame, references):
    results = []
    for book in BOOKS:
        x, y = origin(book)
        left, top, right, bottom = INTERIOR
        # Only allow a one-pixel raster rounding difference inside this book's slot.
        samples = [frame.crop((x + left + dx, y + top + dy, x + right + dx, y + bottom + dy))
                   for dx in (-1, 0, 1) for dy in (-1, 0, 1)]
        scores = {other: min(errors(reference, sample) for sample in samples)
                  for other, reference in references.items()}
        mae, different = scores[book]
        runner = min((other for other in BOOKS if other != book), key=lambda other: scores[other])
        margin = scores[runner][0] - mae
        results.append({"book_id": book, "slot_bounds_xyxy": bounds(book), "mae": round(mae, 4),
                        "fraction_error_gt_24": round(different, 5), "closest_other_id": runner,
                        "mae_margin": round(margin, 4),
                        "matched": mae <= MAX_MAE and different <= MAX_DIFFERENT and margin >= MIN_MARGIN})
    return results


def negative_controls(frame, fallback, references):
    missing, first, second = "demo-7", "demo-3", "demo-11"
    one_missing = frame.copy()
    one_missing.paste(fallback.crop(bounds(missing)), origin(missing))
    swapped = frame.copy()
    swapped.paste(frame.crop(bounds(second)), origin(first))
    swapped.paste(frame.crop(bounds(first)), origin(second))
    cases = (("real_fallback_capture", fallback, list(BOOKS)),
             ("one_fallback_tile", one_missing, [missing]), ("two_swapped_tiles", swapped, [first, second]),
             ("uniform_gray_frame", Image.new("L", SIZE, 128), list(BOOKS)))
    results = {}
    for name, image, expected in cases:
        rejected = [row["book_id"] for row in assess(image, references) if not row["matched"]]
        results[name] = {"expected_rejected": expected, "observed_rejected": rejected,
                         "passed": rejected == expected}
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True, help="public seed directory")
    parser.add_argument("--frame", type=Path, required=True, help="unaltered product capture PNG")
    parser.add_argument("--metadata", type=Path, required=True, help="associated product capture metadata")
    parser.add_argument("--profile", choices=["scribe"], default="scribe")
    parser.add_argument("--report", type=Path, required=True, help="new output JSON file")
    parser.add_argument("--fallback-frame", type=Path, help="optional fresh demo capture with a .metadata.json sidecar")
    args = parser.parse_args()
    report = {"format": "weread-offline-library-cover-check-v1", "profile": args.profile,
              "scope": "Desktop rendered pixels only; no native timing, device or network observation.",
              "reference_transform": "Central 151x210 crop at 1:1 of the 400x560 cached PNG",
              "interior_xyxy": INTERIOR, "maximum_mae": MAX_MAE, "maximum_fraction_error_gt_24": MAX_DIFFERENT,
              "minimum_mae_margin_over_each_other_book": MIN_MARGIN, "passed": False}
    try:
        references = load_fixture(args.fixture)
        frame = load_frame(args.frame, args.metadata)
        report.update({"fixture_receipt_sha256": digest(args.fixture / "library-covers.json"),
                       "frame_sha256": digest(args.frame), "metadata_sha256": digest(args.metadata),
                       "targets": assess(frame, references)})
        if args.fallback_frame:
            sidecar = args.fallback_frame.with_suffix(".metadata.json")
            fallback = load_frame(args.fallback_frame, sidecar)
            report.update({"fallback_frame_sha256": digest(args.fallback_frame), "fallback_metadata_sha256": digest(sidecar),
                           "negative_controls": negative_controls(frame, fallback, references)})
        report["passed"] = all(target["matched"] for target in report["targets"]) and all(
            case["passed"] for case in report.get("negative_controls", {}).values())
    except (ValueError, KeyError, TypeError, OSError) as exc:
        report["error"] = str(exc)
    with args.report.open("x", encoding="utf-8") as output:
        output.write(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
