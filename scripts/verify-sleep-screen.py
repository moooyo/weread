#!/usr/bin/env python3
"""Verify sleep-cover layouts with the isolated offline desktop fixture runner.

Run only on the designated remote Linux worker through ssh test-env. These
software PNG checks do not measure power-button latency, E Ink refresh or
ghosting, native screen-saver suppression, or physical Kindle sleep and wake.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import statistics
import sys
from typing import Any, Callable

from PIL import Image


_spec = importlib.util.spec_from_file_location(
    "weread_sleep_ui_verifier", Path(__file__).with_name("verify-ui.py")
)
assert _spec is not None and _spec.loader is not None
ui = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = ui
_spec.loader.exec_module(ui)
require = ui.require

COVER_SIZE = (300, 450)
BOOK_TITLE = "Offline Cover Fixture"
BOOK_AUTHOR = "Fixture Author"
NEUTRAL_TITLE = "WeRead for Kindle"


def grayscale(path: Path) -> Image.Image:
    with Image.open(path) as source:
        return source.convert("L")


def ink_count(image: Image.Image, box: tuple[int, int, int, int]) -> int:
    return sum(image.crop(box).histogram()[:181])


def observed_bounds(image: Image.Image) -> tuple[int, int, int, int]:
    bounds = image.point(lambda value: 255 if value < 250 else 0).getbbox()
    require(bounds is not None, "The sleep screen is blank")
    return bounds


def check_cover(capture: ui.Capture) -> dict[str, Any]:
    state = capture.state["sleep_screen"]
    require(state["active"] and state["has_cover"], "The bitmap sleep overlay is inactive")
    require(state["cover_size"] == list(COVER_SIZE), "The synthetic source dimensions changed")
    image = grayscale(capture.path)
    width, height = image.size
    scale = min(width / COVER_SIZE[0], height / COVER_SIZE[1])
    cover_width, cover_height = (value * scale for value in COVER_SIZE)
    left, top = (width - cover_width) / 2, (height - cover_height) / 2
    expected = (left, top, left + cover_width, top + cover_height)
    bounds = observed_bounds(image)
    require(
        all(abs(actual - target) <= 2 for actual, target in zip(bounds, expected)),
        f"The complete cover does not fit proportionally: {bounds} versus {expected}",
    )
    require(bounds[0] > 4 and bounds[2] < width - 4, "The portrait cover lost its side margins")
    require(
        abs((bounds[2] - bounds[0]) / (bounds[3] - bounds[1]) - 2 / 3) < 0.006,
        "The cover was stretched or cropped",
    )

    # Distinct corner markers prove that all four source corners survived fitting.
    markers = ((36, 36, 51), (264, 36, 102), (36, 414, 153), (264, 414, 204), (150, 225, 119))
    samples = []
    for source_x, source_y, expected_tone in markers:
        x, y = round(left + source_x * scale), round(top + source_y * scale)
        values = list(image.crop((x - 2, y - 2, x + 3, y + 3)).getdata())
        actual = statistics.median(values)
        require(abs(actual - expected_tone) <= 2, f"Cover marker at {source_x},{source_y} was clipped or changed: {actual}")
        samples.append({"source": [source_x, source_y], "tone": actual})
    for source_x, source_y in ((150, 3), (150, 447), (3, 225), (297, 225)):
        x, y = round(left + source_x * scale), round(top + source_y * scale)
        require(image.getpixel((x, y)) <= 2, "A complete cover edge is missing")
    require(image.crop((0, 0, int(left) - 2, height)).getextrema() == (255, 255), "Underlying UI leaked into the left cover margin")
    require(image.crop((int(left + cover_width) + 2, 0, width, height)).getextrema() == (255, 255), "Underlying UI leaked into the right cover margin")
    return {"cover_bounds": list(bounds), "source_size": list(COVER_SIZE), "corner_samples": samples, "complete_edges": 4, "image_fit": "contain"}


def check_fallback(capture: ui.Capture, *, neutral: bool) -> dict[str, Any]:
    state = capture.state["sleep_screen"]
    require(state["active"] and not state["has_cover"], "The text fallback is inactive or retains a bitmap")
    require(state["cover_size"] == [0, 0], "The text fallback retained an unrelated bitmap")
    require(state["title"] == (NEUTRAL_TITLE if neutral else BOOK_TITLE), "The fallback title is wrong")
    require(state["author"] == ("" if neutral else BOOK_AUTHOR), "The fallback author is wrong")
    image = grayscale(capture.path)
    width, height = image.size
    bounds = observed_bounds(image)
    expected = (width * 0.12, height * 0.13, width * 0.88, height * 0.87)
    require(
        all(abs(actual - target) <= 2 for actual, target in zip(bounds, expected)),
        f"The fallback frame escaped its reserved region: {bounds}",
    )
    left, top, right, bottom = bounds
    frame_height = bottom - top
    title_box = (left + 24, int(top + frame_height * 0.18), right - 24, int(top + frame_height * 0.60))
    author_box = (left + 24, int(top + frame_height * 0.69), right - 24, int(top + frame_height * 0.84))
    title_ink, author_ink = ink_count(image, title_box), ink_count(image, author_box)
    require(title_ink >= 100, "The fallback title has no visible text pixels")
    if neutral:
        require(author_ink == 0, "The neutral fallback displayed an old book author")
    else:
        require(author_ink >= 50, "The fallback author has no visible text pixels")
    require(image.crop((0, 0, width, max(1, top - 4))).getextrema() == (255, 255), "The fallback exposed the underlying header")
    require(image.crop((0, bottom + 4, width, height)).getextrema() == (255, 255), "The fallback exposed the underlying footer")
    return {"frame_bounds": list(bounds), "title_ink_pixels": title_ink, "author_ink_pixels": author_ink, "neutral": neutral}


def checkpoint(capture: ui.Capture, name: str) -> ui.Capture:
    path = capture.path.with_name(f"{capture.path.stem}.{name}.png")
    require(path.is_file(), f"The same-process {name} checkpoint is missing")
    metadata = json.loads(path.with_suffix(".metadata.json").read_text(encoding="utf-8"))
    state = json.loads(path.with_suffix(".state.json").read_text(encoding="utf-8"))
    return ui.Capture(path, capture.database, metadata, capture.data_directory, state)


def check_restored(capture: ui.Capture) -> dict[str, Any]:
    before, sleeping = checkpoint(capture, "before"), checkpoint(capture, "sleep")
    require(not before.state["sleep_screen"]["active"], "The baseline checkpoint was covered")
    require(not capture.state["sleep_screen"]["active"], "The final reader retained the sleep overlay")
    cover = check_cover(sleeping)
    for current in (before, sleeping, capture):
        require(current.metadata["route"] == "reader" and current.state["shell"]["route"] == "reader", "The presentation changed the live route")
    require(bool(before.state["reader"]["lines"]), "The baseline contains no reading text")
    for key in ("page", "page_count", "font_size", "font_choice", "font_family", "lines", "selection_text"):
        require(before.state["reader"][key] == capture.state["reader"][key], f"Hiding the sleep overlay changed reader {key}")
    width, height = ui.PROFILES[capture.metadata["profile"]]
    margin = 32 if capture.metadata["profile"] == "scribe" else 20
    body = (margin, 100, width - margin, height - 100)
    difference = ui.pixel_difference(before.path, capture.path, crop=body, threshold=0)
    require(difference == 0, f"Restored reading pixels changed by {difference:.4%}")
    return {"reader_body": list(body), "body_difference_ratio": difference, "same_process_checkpoints": [str(before.path), str(sleeping.path)], "sleep_cover": cover}


class SleepScreenVerifier(ui.Verifier):
    def observe(self, name: str, profile: str, fixture: str, **arguments: Any) -> ui.Capture:
        return self.run_case(
            name,
            profile=profile,
            screen="reader",
            runner_arguments=("--source-dir", str(self.repository), "--fixture", fixture),
            **arguments,
        )

    def profile_cases(self, profile: str) -> None:
        prefix = f"sleep-screen/{profile}"
        cover = self.observe(f"{prefix}/cover", profile, "sleep-cover")
        self.assertion(f"{prefix}/complete-cover-and-proportions", lambda: check_cover(cover), cover)
        title = self.observe(f"{prefix}/title", profile, "sleep-title")
        self.assertion(f"{prefix}/missing-cover-text", lambda: check_fallback(title, neutral=False), title)
        neutral = self.observe(f"{prefix}/neutral", profile, "sleep-neutral")
        self.assertion(f"{prefix}/neutral-without-old-book", lambda: check_fallback(neutral, neutral=True), neutral)
        restored = self.observe(
            f"{prefix}/restored",
            profile,
            "sleep-cover",
            actions=(
                ui.action(100, "fixture", "hide"),
                ui.action(250, "fixture", "capture", "before"),
                ui.action(400, "fixture", "show"),
                ui.action(600, "fixture", "capture", "sleep"),
                ui.action(800, "fixture", "hide"),
            ),
        )
        self.assertion(f"{prefix}/same-reader-after-hiding-cover", lambda: check_restored(restored), restored)

    def tasks(self) -> list[tuple[str, Callable[[], None]]]:
        return [(f"sleep-screen/{profile}", lambda p=profile: self.profile_cases(p)) for profile in self.arguments.profiles]

    def run(self) -> int:
        status = super().run()
        report_path = self.output / "report.json"
        report = json.loads(report_path.read_text(encoding="utf-8"))
        report["suite"] = "Sleep-screen public offline presentation fixtures"
        report["binary_role"] = "Disposable offline UI fixture runner; not a product test entry point"
        report["count_definition"] = "Counts include eight captures and eight assertions when both default profiles run; same-process checkpoints are supplementary PNGs."
        report["coverage_limits"] = [
            "Only public synthetic cover pixels and isolated offline demo data were used.",
            "Software UI captures verify contain layout, visible fallbacks, and reader pixels after hiding the overlay.",
            "No power event, backend presentation acknowledgement, native screen-saver suppression, or process suspension was exercised.",
            "No physical Kindle model, firmware, power-button latency, E Ink refresh, ghosting, battery use, or real sleep and wake was verified.",
        ]
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        return status


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Separately built weread-offline-ui-fixture executable")
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts/sleep-screen-verification"))
    parser.add_argument("--data-dir", type=Path)
    parser.add_argument("--profile", choices=("pw6", "scribe"), action="append", dest="profiles")
    parser.add_argument("--workers", type=int, choices=(1, 2), default=1)
    arguments = parser.parse_args()
    if os.name != "posix":
        parser.error("Run this harness only on the designated remote Linux worker through ssh test-env")
    arguments.profiles = arguments.profiles or ["pw6", "scribe"]
    arguments.baseline_dir = None
    arguments.max_diff_ratio = 0.0
    try:
        return SleepScreenVerifier(arguments).run()
    except Exception as error:
        print(f"Sleep-screen verification setup failed: {ui.redact(str(error))}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
