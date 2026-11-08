"""Rasterize the native startup labels from the existing bundled font and translations."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import struct
from pathlib import Path

import PIL
from PIL import Image, ImageDraw, ImageFont


LABELS = (
    ("app-name", "app_name", 54, 0),
    ("starting", "startup_starting", 40, 0),
    ("fonts", "startup_fonts", 30, 68),
    ("display", "startup_display", 30, 68),
    ("library", "startup_library", 30, 68),
)


def prepare(font_path: Path, strings_path: Path, output_dir: Path) -> None:
    strings = json.loads(strings_path.read_text(encoding="utf-8"))
    compressed_font = font_path.read_bytes()
    font_data = gzip.decompress(compressed_font)
    records = []
    output_dir.mkdir(parents=True, exist_ok=True)
    for name, key, size, shade in LABELS:
        label = strings[key]
        if not isinstance(label, str) or not label.strip():
            raise ValueError(f"Startup translation must contain text: {key}")
        font = ImageFont.truetype(io.BytesIO(font_data), size=size)
        bounds = font.getbbox(label)
        padding = 6
        width = bounds[2] - bounds[0] + padding * 2
        height = bounds[3] - bounds[1] + padding * 2
        if not 0 < width <= 2_048 or not 0 < height <= 256:
            raise ValueError(f"Startup artwork is outside its supported dimensions: {key}")
        image = Image.new("L", (width, height), 255)
        ImageDraw.Draw(image).text(
            (padding - bounds[0], padding - bounds[1]), label, font=font, fill=shade
        )
        pixels = image.tobytes()
        asset = struct.pack("<II", width, height) + pixels
        path = output_dir / f"{name}.gray"
        path.write_bytes(asset)
        records.append(
            {
                "file": path.name,
                "translation_key": key,
                "translation_sha256": hashlib.sha256(label.encode("utf-8")).hexdigest(),
                "font_size_px": size,
                "foreground_gray": shade,
                "width": width,
                "height": height,
                "sha256": hashlib.sha256(asset).hexdigest(),
            }
        )
    manifest = {
        "format_version": 1,
        "encoding": "little-endian u32 width, u32 height, then row-major grayscale u8 pixels",
        "source_font": font_path.name,
        "source_font_sha256": hashlib.sha256(compressed_font).hexdigest(),
        "font_license": "OFL-1.1; see assets/fonts/OFL.txt and assets/fonts/NOTICES.txt",
        "tool": f"Pillow {PIL.__version__}",
        "images": records,
    }
    (output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=Path, default=root / "assets/fonts/NotoSansSC-Regular.otf.gz")
    parser.add_argument("--strings", type=Path, default=root / "assets/i18n/zh-CN.json")
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    prepare(args.font, args.strings, args.output)


if __name__ == "__main__":
    main()
