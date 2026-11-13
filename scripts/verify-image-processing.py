#!/usr/bin/env python3
"""Exercise real App image preparation with public, isolated Gray8 ramp assets.

Run only on test-env. The standalone fixture runner seeds through the public
BookAssetStore packet API and exports the actual ReaderModel picture buffers.
Desktop hardware requests must fall back to software; no device eligibility,
Y4 submission, panel completion, reflectance levels, or physical sleep is claimed.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import sqlite3
import sys
from typing import Any, Callable

from PIL import Image, ImageChops, ImageDraw, ImageStat


_spec = importlib.util.spec_from_file_location(
    "weread_image_ui_verifier", Path(__file__).with_name("verify-ui.py")
)
assert _spec is not None and _spec.loader is not None
ui = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = ui
_spec.loader.exec_module(ui)
require = ui.require


def grayscale(path: Path) -> Image.Image:
    with Image.open(path) as source:
        return source.convert("L")


def picture(capture: ui.Capture) -> tuple[dict[str, Any], Image.Image]:
    rows = json.loads(capture.path.with_suffix(".images.json").read_text(encoding="utf-8"))
    require(len(rows) == 1, "The initial reader page must show one complete synthetic image")
    row = rows[0]
    require(row["caption"] == "Synthetic Gray8 ramp", "The committed ramp caption is missing")
    path = Path(row["path"])
    require(path.is_file() and path.parent == capture.path.parent, "The picture readback escaped its capture directory")
    image = grayscale(path)
    require(list(image.size) == row["size"], "The actual model buffer dimensions changed")
    return row, image


def picture_bounds(capture: ui.Capture, row: dict[str, Any], image: Image.Image) -> tuple[int, int, int, int]:
    width, height = ui.PROFILES[capture.metadata["profile"]]
    reader = capture.state["reader"]
    scribe = capture.metadata["profile"] == "scribe"
    text_width = width - (300 if scribe and reader["notes_panel"] else 0)
    margin = reader["margin"]
    top = (204 if scribe else 164) if reader["show_heading"] else (76 if scribe else 54)
    frame_width = min(text_width - 2 * margin, row["width"])
    frame_height = max(1, row["height"] - 24)
    scale = min(frame_width / image.width, frame_height / image.height)
    left = margin + (text_width - 2 * margin - frame_width) / 2 + (frame_width - image.width * scale) / 2
    upper = top + row["y"] + (frame_height - image.height * scale) / 2
    bounds = (math.floor(left), math.floor(upper), math.ceil(left + image.width * scale), math.ceil(upper + image.height * scale))
    require(bounds[0] >= margin and bounds[2] <= text_width - margin + 1, "The ramp escaped the reading width")
    require(bounds[1] >= top and bounds[3] <= height - 48, "The full ramp was clipped by the reader viewport")
    return bounds


def seed_integrity(capture: ui.Capture) -> dict[str, Any]:
    root = capture.data_directory
    receipt = json.loads((root / "synthetic-image-ramp.json").read_text(encoding="utf-8"))
    require(receipt["format"] == "weread-offline-gray8-ramp-v1" and receipt["synthetic"], "The seed is not the declared public fixture")
    source = root / receipt["source_file"]
    require(hashlib.sha256(source.read_bytes()).hexdigest() == receipt["source_sha256"], "The original packet SHA does not match its source bytes")
    original = grayscale(source)
    require(original.size == (1024, 256) and set(original.tobytes()) == set(range(256)), "The synthetic source does not retain all 256 luminance inputs")
    require(len(receipt["chapters"]) == 2, "The native-default and replacement demo chapters were not both seeded")
    with sqlite3.connect(capture.database) as connection:
        require(connection.execute("SELECT count(*) FROM download_tasks").fetchone()[0] == 0, "The offline fixture queued a source task")
        for chapter in receipt["chapters"]:
            body, uid = connection.execute(
                "SELECT body,source_id FROM chapters WHERE book_id='demo-0' AND chapter_index=?", (chapter["index"],)
            ).fetchone()
            require(uid == chapter["chapter_uid"], "The fixture changed or confused a canonical chapter UID")
            require(hashlib.sha256(body.encode()).hexdigest() == chapter["text_sha256"], "The ramp is not bound to the exact canonical body")
            require(0 < chapter["anchor"] < len(body), "The image has no valid scalar anchor")
            require(chapter["asset_id"] == receipt["source_sha256"], "The manifest does not identify the acquired original bytes")
            require(chapter["cache_key"] == "book-image:v2:" + receipt["source_sha256"], "The manifest does not use the current canonical image key")
            canonical = grayscale(Path(chapter["canonical_path"]))
            require(canonical.size == original.size and canonical.tobytes() == original.tobytes(), "Canonical preparation quantized or changed the opaque Gray8 fixture")
    require(not capture.state["image_processing"]["hardware_y4_available"], "A desktop fixture unexpectedly claims hardware eligibility")
    return {"source_sha256": receipt["source_sha256"], "canonical_input_levels": 256, "bound_chapters": 2, "source_operations": 0}


def raster_pixels(capture: ui.Capture, method: str) -> dict[str, Any]:
    require(ui.setting(capture.database, "preferences")["image_dithering"] == method, "The complete current preference did not persist")
    row, image = picture(capture)
    tones = set(image.tobytes())
    require(tones == set(range(0, 256, 17)), f"The published {method} model is not a complete sixteen-tone presentation")
    bounds = picture_bounds(capture, row, image)
    screen = grayscale(capture.path)
    middle = (bounds[1] + bounds[3]) // 2
    means = []
    for fraction, low, high in ((0.25, 45, 85), (0.50, 110, 145), (0.75, 175, 210)):
        x = round(bounds[0] + (bounds[2] - bounds[0]) * fraction)
        mean = ImageStat.Stat(screen.crop((x - 3, middle - 8, x + 4, middle + 8))).mean[0]
        require(low <= mean <= high, f"The actual PNG lost or cropped ramp position {fraction}: {mean}")
        means.append(mean)
    interior = image.crop((5, 5, image.width - 5, image.height - 5))
    pixels = interior.load()
    varying = sum(len({pixels[x, y] for y in range(4)}) > 1 for x in range(interior.width))
    if method == "off":
        require(varying == 0, "The default Off image unexpectedly contains a vertical Bayer pattern")
    else:
        require(varying > interior.width // 8, "The software/fallback image has no meaningful four-row Bayer pattern")
    return {"requested_method": method, "desktop_hardware_available": False, "presented_tones": len(tones), "ramp_bounds": list(bounds), "screen_quarter_means": means, "bayer_columns": varying}


def matching_fallback(software: ui.Capture, hardware: ui.Capture) -> dict[str, Any]:
    _, first = picture(software)
    _, second = picture(hardware)
    require(first.size == second.size and ImageChops.difference(first, second).getbbox() is None, "Unsupported Hardware pixels differ from the actual Software picture")
    require(ui.setting(hardware.database, "preferences")["image_dithering"] == "hardware", "Fallback overwrote the requested hardware preference")
    return {"fallback_matches_software": True, "hardware_preference_retained": True}


def unchanged_controls(first: ui.Capture, second: ui.Capture) -> dict[str, Any]:
    row, raster = picture(first)
    bounds = picture_bounds(first, row, raster)
    left, right = grayscale(first.path), grayscale(second.path)
    require(left.size == right.size, "Compared viewports differ")
    # Exclude only the picture and the existing clock/footer bands.
    for image in (left, right):
        draw = ImageDraw.Draw(image)
        draw.rectangle((bounds[0] - 2, bounds[1] - 2, bounds[2] + 2, bounds[3] + 2), fill=255)
    body = (0, 76, left.width, left.height - 64)
    require(ImageChops.difference(left.crop(body), right.crop(body)).getbbox() is None, "Changing image processing changed surrounding text, caption or controls")
    for key in ("page", "page_count", "font_size", "font_choice", "lines"):
        require(first.state["reader"][key] == second.state["reader"][key], f"Image processing moved reader {key}")
    return {"unchanged_nonimage_pixels": True, "body_crop": list(body), "reader_anchor_geometry_unchanged": True}


class ImageProcessingVerifier(ui.Verifier):
    def observe(self, name: str, profile: str, fixture: str, **arguments: Any) -> ui.Capture:
        return self.run_case(name, profile=profile, screen="reader", runner_arguments=("--source-dir", str(self.repository), "--fixture", fixture), **arguments)

    def profile_cases(self, profile: str) -> None:
        prefix = f"image-processing/{profile}"
        default = self.observe(f"{prefix}/default", profile, "image-processing-default")
        self.assertion(f"{prefix}/current-gray8-seed-integrity", lambda: seed_integrity(default), default)
        self.assertion(f"{prefix}/default-off-palette", lambda: raster_pixels(default, "off"), default)
        software = self.observe(f"{prefix}/software", profile, "image-processing-software")
        self.assertion(f"{prefix}/software-bayer", lambda: raster_pixels(software, "software"), software)
        self.assertion(f"{prefix}/surrounding-text-unchanged", lambda: unchanged_controls(default, software), default, software)
        hardware = self.observe(f"{prefix}/hardware", profile, "image-processing-hardware", data_key=f"{prefix}/hardware-data")
        self.assertion(f"{prefix}/unsupported-hardware-software-pixels", lambda: matching_fallback(software, hardware), software, hardware)
        cold = self.observe(f"{prefix}/hardware-cold", profile, "image-processing-persisted", data_key=f"{prefix}/hardware-data")
        self.assertion(f"{prefix}/cold-retains-preference-and-pixels", lambda: matching_fallback(software, cold), software, cold)
        self.assertion(f"{prefix}/cold-retains-text-and-anchor", lambda: unchanged_controls(hardware, cold), hardware, cold)

    def tasks(self) -> list[tuple[str, Callable[[], None]]]:
        return [(f"image-processing/{profile}", lambda p=profile: self.profile_cases(p)) for profile in ("pw6", "scribe")]

    def run(self) -> int:
        status = super().run()
        path = self.output / "report.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        report["suite"] = "Real App Gray8 preparation and offline software fallback"
        report["seed_export"] = "The same standalone runner supports --seed-image-ramp-only --data-dir FRESH --device scribe --image-method hardware without opening a display."
        report["coverage_limits"] = [
            "Public synthetic packets use the actual BookAssetStore commit and actual App Reader image preparation.",
            "Desktop Hardware requests are verified only as unsupported software fallbacks; no fake capability is installed.",
            "Prepared model PNGs and rendered screen regions establish image handling, palette, pattern, text isolation and cold persistence.",
            "Actual backend raw-scene handshakes remain separate controlled core tests; this desktop runner does not submit native updates.",
            "No account, user entitlement, physical Kindle, native Y4 acceptance/completion, panel grayscale count or sleep latency is verified.",
        ]
        path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        return status


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Separately built weread-offline-ui-fixture executable")
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts/image-processing-verification"))
    parser.add_argument("--data-dir", type=Path)
    parser.add_argument("--workers", type=int, choices=(1, 2), default=1)
    arguments = parser.parse_args()
    if os.name != "posix":
        parser.error("Run this fixture only on the designated remote Linux worker through ssh test-env")
    arguments.baseline_dir = None
    arguments.max_diff_ratio = 0.0
    try:
        return ImageProcessingVerifier(arguments).run()
    except Exception as error:
        print(f"Image-processing fixture failed: {ui.redact(str(error))}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
