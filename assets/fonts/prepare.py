"""Extract reproducible Simplified Chinese font subsets from Noto CJK collections."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTCollection, TTFont


UNICODE_RANGES = (
    (0x0000, 0x024F),
    (0x02B0, 0x052F),
    (0x1E00, 0x1EFF),
    (0x2000, 0x27BF),
    (0x2E80, 0x2FDF),
    (0x2FF0, 0x312F),
    (0x31A0, 0x31FF),
    (0x3200, 0x9FFF),
    (0xF900, 0xFAFF),
    (0xFE00, 0xFE1F),
    (0xFE30, 0xFE4F),
    (0xFF00, 0xFFEF),
    (0x1F100, 0x1F2FF),
    (0x20000, 0x2FA1F),
    (0x30000, 0x323AF),
    (0xE0100, 0xE01EF),
)

PACKAGE_VERSION = "1:20240730+repack1-1"
SOURCE_PACKAGES = {
    "fonts-noto-cjk": {
        "version": PACKAGE_VERSION,
        "archive_sha256": "f5dc28a754e17327d99f0a612134d92c8dd6187314ae967cb77f25df60860139",
        "url": "https://deb.debian.org/debian/pool/main/f/fonts-noto-cjk/fonts-noto-cjk_20240730+repack1-1_all.deb",
    },
    "fonts-noto-cjk-extra": {
        "version": PACKAGE_VERSION,
        "archive_sha256": "16081e6982d2c8de4d773cf124c09ea25e6e1e472cd7fc62a58a52a1c241b41a",
        "url": "https://deb.debian.org/debian/pool/main/f/fonts-noto-cjk/fonts-noto-cjk-extra_20240730+repack1-1_all.deb",
    },
}
SOURCE_SHA256 = {
    "NotoSansCJK-Regular.ttc": "b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a",
    "NotoSansCJK-Medium.ttc": "197d5e1e019faca33a4d55931c7d68b8056f3b97cb862049f5cb8de9efdfb8ce",
    "NotoSansCJK-Bold.ttc": "faa5f3656a78b2e2d450d27fe8382c778bc2b6bb5ea29c986664a6a435056ceb",
    "NotoSerifCJK-Light.ttc": "0a8fad491c0c2bf29b5b87e1e338c24162752713dec35f35acb48743a29ce433",
    "NotoSerifCJK-Regular.ttc": "5d9c31a059600193c9d7968a998bde886ccdc77e934006ad243b41794c496a7d",
    "NotoSerifCJK-Medium.ttc": "16fe6d7230f82155748e10e47420dd661a899a200e05a77ee029ab1eb0fb5f29",
    "NotoSerifCJK-Bold.ttc": "1505ee3b9c0890fae6302ee0e9c6fd74d690f4a55a6ace1d9944f3f6352d622d",
}

FACES = (
    ("hei", "NotoSansCJK-Regular.ttc", "Noto Sans CJK SC", "Regular", "NotoSansSC-Regular.otf", "\u601d\u6e90\u9ed1\u4f53", 400, "fonts-noto-cjk"),
    ("hei-medium", "NotoSansCJK-Medium.ttc", "Noto Sans CJK SC", "Medium", "NotoSansSC-Medium.otf", "\u601d\u6e90\u9ed1\u4f53", 500, "fonts-noto-cjk-extra"),
    ("hei-bold", "NotoSansCJK-Bold.ttc", "Noto Sans CJK SC", "Bold", "NotoSansSC-Bold.otf", "\u601d\u6e90\u9ed1\u4f53", 700, "fonts-noto-cjk"),
    ("song-light", "NotoSerifCJK-Light.ttc", "Noto Serif CJK SC", "Light", "NotoSerifSC-Light.otf", "\u601d\u6e90\u5b8b\u4f53", 300, "fonts-noto-cjk-extra"),
    ("song", "NotoSerifCJK-Regular.ttc", "Noto Serif CJK SC", "Regular", "NotoSerifSC-Regular.otf", "\u601d\u6e90\u5b8b\u4f53", 400, "fonts-noto-cjk"),
    ("song-medium", "NotoSerifCJK-Medium.ttc", "Noto Serif CJK SC", "Medium", "NotoSerifSC-Medium.otf", "\u601d\u6e90\u5b8b\u4f53", 500, "fonts-noto-cjk-extra"),
    ("song-bold", "NotoSerifCJK-Bold.ttc", "Noto Serif CJK SC", "Bold", "NotoSerifSC-Bold.otf", "\u601d\u6e90\u5b8b\u4f53", 700, "fonts-noto-cjk"),
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def unicode_ranges(codepoints: set[int]) -> list[str]:
    result: list[str] = []
    first = previous = None
    for codepoint in sorted(codepoints):
        if first is None:
            first = previous = codepoint
        elif codepoint == previous + 1:
            previous = codepoint
        else:
            result.append(f"U+{first:04X}" if first == previous else f"U+{first:04X}-{previous:04X}")
            first = previous = codepoint
    if first is not None:
        result.append(f"U+{first:04X}" if first == previous else f"U+{first:04X}-{previous:04X}")
    return result


def face_family(font: TTFont) -> str:
    return font["name"].getDebugName(16) or font["name"].getDebugName(1)


def face_style(font: TTFont) -> str:
    return font["name"].getDebugName(17) or font["name"].getDebugName(2)


def prepare(
    source_dir: Path,
    extra_source_dir: Path,
    output_dir: Path,
    package_version: str,
    copyright_file: Path,
) -> None:
    if package_version != PACKAGE_VERSION:
        raise ValueError(f"Only the reviewed Debian source version {PACKAGE_VERSION} is supported")
    output_dir.mkdir(parents=True, exist_ok=True)
    requested = {codepoint for first, last in UNICODE_RANGES for codepoint in range(first, last + 1)}
    records: list[dict] = []
    for identifier, source_name, family, style, output_name, label, weight, package in FACES:
        source = (extra_source_dir if package == "fonts-noto-cjk-extra" else source_dir) / source_name
        source_digest = sha256(source)
        if source_digest != SOURCE_SHA256[source_name]:
            raise ValueError(f"The source checksum differs from the reviewed font: {source_name}")
        collection = TTCollection(source, lazy=True)
        matches = [
            (index, font)
            for index, font in enumerate(collection.fonts)
            if face_family(font) == family and face_style(font) == style
        ]
        if len(matches) != 1:
            raise ValueError(f"Expected exactly one {family} {style} face in {source}")
        face_index, font = matches[0]
        if font["OS/2"].usWeightClass != weight:
            raise ValueError(f"The source does not declare the requested weight {weight}: {source_name}")
        source_cmap = set(font.getBestCmap())
        required_cmap = source_cmap & requested
        source_version = font["name"].getDebugName(5)
        copyright_notice = font["name"].getDebugName(0)
        legacy_family = font["name"].getDebugName(1)
        legacy_style = font["name"].getDebugName(2)
        trademark = font["name"].getDebugName(7)
        cff_notice = font["CFF "].cff.topDictIndex[0].Notice
        if "SIL Open Font License" not in (font["name"].getDebugName(13) or ""):
            raise ValueError(f"OFL metadata is missing from {source_name}")
        font.recalcTimestamp = False
        options = subset.Options()
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14, 16, 17, 21, 22]
        options.name_legacy = True
        options.name_languages = ["*"]
        options.layout_features = ["ccmp", "liga", "clig", "kern", "calt"]
        options.glyph_names = False
        options.notdef_glyph = True
        options.notdef_outline = True
        options.recommended_glyphs = True
        options.hinting = False
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(unicodes=required_cmap)
        subsetter.subset(font)
        output = output_dir / output_name
        font.save(output, reorderTables=True)
        reloaded = TTFont(output, recalcTimestamp=False)
        actual_cmap = set(reloaded.getBestCmap())
        if actual_cmap != required_cmap:
            raise ValueError(f"Unicode coverage changed unexpectedly for {output_name}")
        if face_family(reloaded) != family or face_style(reloaded) != style:
            raise ValueError(f"Font family changed unexpectedly for {output_name}")
        if reloaded["OS/2"].usWeightClass != weight:
            raise ValueError(f"Font weight changed unexpectedly for {output_name}")
        compressed = io.BytesIO()
        with gzip.GzipFile(filename="", mode="wb", fileobj=compressed, compresslevel=9, mtime=0) as stream:
            stream.write(output.read_bytes())
        compressed_output = output.with_suffix(".otf.gz")
        compressed_output.write_bytes(compressed.getvalue())
        records.append({
            "id": identifier,
            "label": label,
            "family": family,
            "style": style,
            "legacy_family": legacy_family,
            "legacy_style": legacy_style,
            "weight": weight,
            "file": compressed_output.name,
            "available": True,
            "bytes": compressed_output.stat().st_size,
            "sha256": sha256(compressed_output),
            "decompressed_file": output_name,
            "decompressed_bytes": output.stat().st_size,
            "decompressed_sha256": sha256(output),
            "version": source_version,
            "copyright": copyright_notice,
            "source_trademark": trademark,
            "source_cff_notice": cff_notice,
            "license": "OFL-1.1",
            "source": "https://github.com/notofonts/noto-cjk/tree/Sans2.004/Sans" if "Sans" in family else "https://github.com/notofonts/noto-cjk/tree/Serif2.003/Serif",
            "source_revision": "523d033d6cb47f4a80c58a35753646f5c3608a78" if "Sans" in family else "9b0f1436e455d902de067a2501422e5dc71ad16b",
            "source_file": source_name,
            "source_sha256": source_digest,
            "source_face_index": face_index,
            "source_package": f"Debian {package} {package_version}",
            "unicode_codepoints": len(actual_cmap),
            "glyphs": reloaded["maxp"].numGlyphs,
            "coverage": unicode_ranges(actual_cmap),
        })
        print(f"{compressed_output.name}: {compressed_output.stat().st_size:,} gzip bytes; {output.stat().st_size:,} raw bytes; {len(actual_cmap):,} Unicode codepoints", flush=True)
        reloaded.close()
        collection.close()
    manifest = {
        "format_version": 1,
        "prepared_on": "2026-11-03",
        "tool": f"fontTools {fontTools.__version__}",
        "package_archive_sha256": "f5dc28a754e17327d99f0a612134d92c8dd6187314ae967cb77f25df60860139",
        "source_packages": SOURCE_PACKAGES,
        "hinting": False,
        "layout_features": ["ccmp", "liga", "clig", "kern", "calt"],
        "gzip_mtime": 0,
        "description": "SC-face subsets retaining all source-mapped characters in the declared Unicode ranges, not a fixture-text subset.",
        "requested_coverage": [f"U+{first:04X}-{last:04X}" for first, last in UNICODE_RANGES],
        "fonts": records,
    }
    (output_dir / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    copyright_text = copyright_file.read_text(encoding="utf-8")
    license_stanza = copyright_text.split("\nLicense: SIL-1.1\n")[-1].split("\nLicense: GPL-3+\n")[0]
    license_lines = [line[1:] if line.startswith(" ") else line for line in license_stanza.splitlines()]
    license_text = "\n".join("" if line == "." else line for line in license_lines).strip()
    notices = sorted({record["copyright"] for record in records})
    license_header = "Noto CJK font subsets\n\n" + "\n".join(notices) + "\nCopyright: 2010-2012, Google Corporation (Debian upstream attribution).\n\n"
    (output_dir / "OFL.txt").write_text(license_header + license_text + "\n", encoding="utf-8")
    print(f"Total compressed font bytes: {sum(record['bytes'] for record in records):,}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, default=Path("/usr/share/fonts/opentype/noto"))
    parser.add_argument("--extra-source-dir", type=Path, help="Use an extracted fonts-noto-cjk-extra package without installing it")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--package-version", default=PACKAGE_VERSION)
    parser.add_argument("--copyright-file", type=Path, default=Path("/usr/share/doc/fonts-noto-cjk/copyright"))
    arguments = parser.parse_args()
    prepare(
        arguments.source_dir,
        arguments.extra_source_dir or arguments.source_dir,
        arguments.output_dir,
        arguments.package_version,
        arguments.copyright_file,
    )


if __name__ == "__main__":
    main()
