#!/usr/bin/env python3
"""Measure real offline App image publication on the designated remote worker.

The identical fixture source runs against the baseline and candidate libraries.
Only the candidate enables the tool's optional performance-diagnostics feature.
Dispatch/tick timings exclude snapshots and PNG encoding. Ready latency includes
20 ms model polling; no Kindle driver submission or optical completion is timed.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
from typing import Any
import zipfile

from PIL import Image, ImageChops


FORMAT = "weread-reader-image-performance-v1"
SOURCE_FORMAT = "weread-offline-perf05-images-v1"
PERF05_SHA256 = "0e11719838c8da38388be9f3f389fa59ce671625bb033b1707842bce520cf076"
MAX_SOURCE_BYTES = 8 * 1024 * 1024
CHECKPOINT_CHAPTERS = {
    "first-read": 0,
    "same-page": 0,
    "forward-1": 1,
    "forward-2": 2,
    "forward-3": 3,
    "revisit-2": 2,
    "ordinary-back": 2,
    "rapid-final": 0,
    "before-idle": 0,
    "after-idle": 0,
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def prepare_images(arguments: argparse.Namespace, output: Path) -> dict[str, Any]:
    output.mkdir()
    inputs: list[tuple[str, bytes]] = []
    archive_hash = None
    if arguments.cbz:
        archive_hash = sha256(arguments.cbz)
        require(archive_hash == PERF05_SHA256, "The CBZ is not the reviewed public Perf05 fixture")
        with zipfile.ZipFile(arguments.cbz) as archive:
            entries = sorted(
                (entry for entry in archive.infolist() if not entry.is_dir() and entry.filename.lower().endswith(".png")),
                key=lambda entry: entry.filename,
            )
            require(len(entries) == 8, "Perf05 must contain exactly eight PNG members")
            for entry in entries:
                require(0 < entry.file_size <= MAX_SOURCE_BYTES, "A Perf05 member exceeds the current source budget")
                payload = archive.read(entry)
                require(len(payload) == entry.file_size, "A Perf05 member was truncated")
                inputs.append((entry.filename, payload))
    else:
        paths = sorted(arguments.images_dir.glob("*.png"))
        require(len(paths) == 8, "The PNG directory must contain exactly eight members")
        for path in paths:
            require(path.is_file() and 0 < path.stat().st_size <= MAX_SOURCE_BYTES, "A PNG source exceeds its budget")
            inputs.append((path.name, path.read_bytes()))
    members = []
    for index, (original_name, payload) in enumerate(inputs):
        path = output / f"page-{index:02}.png"
        path.write_bytes(payload)
        with Image.open(path) as image:
            require(image.format == "PNG" and image.size == (2480, 3508), "The input is not the expected high-resolution PNG")
            mode = image.mode
        members.append({
            "index": index, "archive_name": original_name, "path": str(path),
            "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest(),
            "size": [2480, 3508], "mode": mode,
        })
    return {"archive_sha256": archive_hash, "members": members}


def actions() -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []

    def action(at: int, name: str, value: str, label: str) -> None:
        result.append({"at_ms": at, "section": "reader", "action": name, "value": value, "label": label})

    def checkpoint(at: int, label: str) -> None:
        result.append({"at_ms": at, "section": "probe", "action": "checkpoint", "value": label})

    checkpoint(1200, "first-read")
    action(1450, "menu", "false", "same-page-redraw-1")
    action(1650, "menu", "true", "same-page-menu-open")
    action(1850, "menu", "false", "same-page-menu-close")
    checkpoint(2250, "same-page")
    for chapter, at in ((1, 2600), (2, 4100), (3, 5600)):
        action(at, "open-chapter", f"demo-chapter-{chapter}", f"forward-chapter-{chapter}")
        checkpoint(at + 1150, f"forward-{chapter}")
    action(7100, "open-chapter", "demo-chapter-2", "revisit-chapter-2")
    checkpoint(8250, "revisit-2")
    action(8500, "turn", "1", "ordinary-page-forward")
    checkpoint(9100, "ordinary-next")
    action(9350, "turn", "-1", "ordinary-page-back")
    checkpoint(10400, "ordinary-back")
    for index, (chapter, at) in enumerate(((4, 10700), (5, 10740), (6, 10780), (0, 10820))):
        action(at, "open-chapter", f"demo-chapter-{chapter}", f"rapid-{index}")
    checkpoint(12250, "rapid-final")
    action(12500, "menu", "false", "idle-deadline-reset")
    checkpoint(12900, "before-idle")
    checkpoint(19900, "after-idle")
    return result


def summary(samples: list[int]) -> dict[str, Any]:
    return {
        "count": len(samples), "median_us": statistics.median(samples) if samples else None,
        "minimum_us": min(samples) if samples else None, "maximum_us": max(samples) if samples else None,
        "samples_us": samples,
    }


def timing_summary(report: dict[str, Any]) -> dict[str, Any]:
    dispatches = report["dispatches"]
    groups = {
        "same_page": [entry for entry in dispatches if entry["label"].startswith("same-page-")],
        "forward_chapter": [entry for entry in dispatches if entry["label"].startswith("forward-chapter-")],
        "revisit": [entry for entry in dispatches if entry["label"].startswith("revisit-")],
        "ordinary_page": [entry for entry in dispatches if entry["label"].startswith("ordinary-page-")],
        "rapid": [entry for entry in dispatches if entry["label"].startswith("rapid-")],
    }
    return {
        "attach_us": report["attach_us"], "initial_ready_us": report["initial_ready_us"],
        "dispatch": {name: summary([entry["dispatch_us"] for entry in entries]) for name, entries in groups.items()},
        "ready": {name: summary([entry["ready_us"] for entry in entries if entry["ready_us"] is not None]) for name, entries in groups.items()},
        "tick": summary([entry["tick_us"] for entry in report["ticks"]]),
        "superseded_requests": sum(bool(entry["superseded"]) for entry in dispatches),
    }


def pictures(checkpoint: dict[str, Any]) -> list[tuple[str, list[int], str]]:
    return [(row["id"], row["size"], row["sha256"]) for row in checkpoint["pictures"]]


def body_difference(left_path: Path, right_path: Path) -> dict[str, Any]:
    with Image.open(left_path) as source:
        left = source.convert("RGB")
    with Image.open(right_path) as source:
        right = source.convert("RGB")
    require(left.size == right.size, "Compared viewports differ")
    # Exclude only the existing clock/header and footer, not any image pixels.
    roi = (0, 76, left.width, left.height - 64)
    difference = ImageChops.difference(left.crop(roi), right.crop(roi))
    bounds = difference.getbbox()
    return {"roi": list(roi), "difference_bbox": list(bounds) if bounds else None, "identical": bounds is None}


def validate_case(report: dict[str, Any], receipt: dict[str, Any], source: dict[str, Any]) -> dict[str, Any]:
    require(report["format"] == FORMAT and receipt["format"] == SOURCE_FORMAT, "The reports use an unsupported format")
    require(receipt["synthetic"] is True and len(receipt["chapters"]) == 8, "The run is not the declared synthetic fixture")
    require(report["initial_ready_us"] is not None, "The first Reader model never became ready")
    expected = {}
    for chapter, original in zip(receipt["chapters"], source["members"], strict=True):
        require(chapter["source_sha256"] == original["sha256"] == chapter["asset_id"], "A canonical image lost its acquired source identity")
        require(chapter["anchor"] == 0 and chapter["source_size"] == [2480, 3508], "A source image has an unexpected canonical anchor or geometry")
        expected[chapter["index"]] = chapter["image_id"]
    checkpoints = {entry["label"]: entry for entry in report["checkpoints"]}
    for label, chapter in CHECKPOINT_CHAPTERS.items():
        checkpoint = checkpoints[label]
        require(checkpoint["state"]["chapter"] == f"Perf05 frame {chapter}", f"The {label} target chapter is stale")
        require(all(row["loaded"] for row in checkpoint["state"]["images"]), f"The {label} image is not ready")
        require(len(checkpoint["pictures"]) == 1 and checkpoint["pictures"][0]["id"] == expected[chapter], f"The {label} image is missing or belongs to another source")
        row = checkpoint["pictures"][0]
        with Image.open(row["path"]) as image:
            require(image.mode == "RGB" and list(image.size) == row["size"], "Prepared model readback changed format")
            require(hashlib.sha256(image.tobytes()).hexdigest() == row["sha256"], "Prepared RGB readback does not match the recorded model")
            red, green, blue = image.split()
            require(ImageChops.difference(red, green).getbbox() is None and ImageChops.difference(red, blue).getbbox() is None, "A Reader model is not grayscale")
            require(all(count == 0 or value % 17 == 0 for value, count in enumerate(red.histogram())), "The displayed Off/Software/fallback model escaped the sixteen-tone palette")
    require(pictures(checkpoints["first-read"]) == pictures(checkpoints["same-page"]), "A same-page control action changed the prepared picture")
    require(pictures(checkpoints["forward-2"]) == pictures(checkpoints["revisit-2"]), "Revisiting a chapter changed its picture")
    require(pictures(checkpoints["revisit-2"]) == pictures(checkpoints["ordinary-back"]), "A page round trip changed its picture")
    require(checkpoints["ordinary-next"]["state"]["page"] != checkpoints["revisit-2"]["state"]["page"], "The ordinary page turn was not effective")
    require(pictures(checkpoints["first-read"]) == pictures(checkpoints["rapid-final"]), "Rapid requests left an obsolete image visible")
    idle_difference = body_difference(Path(checkpoints["before-idle"]["path"]), Path(checkpoints["after-idle"]["path"]))
    require(idle_difference["identical"], "Optional cover work changed the visible Reader body")
    if report["diagnostics_available"] and report["cover_seed"] == "full":
        sleep = checkpoints["after-idle"]["diagnostics"]["sleep"]
        require(sleep["ready"] and sleep["full_resolution"], "The deferred full sleep cover did not recover after idle")
    return {
        "source_members": 8, "checked_image_checkpoints": len(CHECKPOINT_CHAPTERS),
        "ordinary_turn_effective": True, "rapid_final_source_correct": True,
        "same_page_pixels_identical": True, "revisit_pixels_identical": True,
        "idle_body": idle_difference,
        "sleep_full_ready": checkpoints["after-idle"]["diagnostics"].get("sleep") if report["diagnostics_available"] else None,
    }


def run(arguments: argparse.Namespace) -> None:
    require(sys.platform.startswith("linux"), "Run this verifier only on ssh test-env")
    binary = arguments.binary.resolve(strict=True)
    output = arguments.output_dir.resolve() / f"run-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{os.getpid()}"
    output.mkdir(parents=True)
    source = prepare_images(arguments, output / "source-images")
    write_json(output / "source.json", source)
    command_file = output / "actions.json"
    write_json(command_file, actions())
    environment = {key: value for key, value in os.environ.items() if not key.startswith("WEREAD_")}
    environment.update({"SLINT_BACKEND": "winit-software", "SLINT_SCALE_FACTOR": "1", "LIBGL_ALWAYS_SOFTWARE": "1", "RUST_BACKTRACE": "1"})
    cases = []
    for cover in arguments.covers.split(","):
        require(cover in ("full", "thumbnail", "none"), "Unknown cover seed choice")
        for method in arguments.methods.split(","):
            require(method in ("off", "software", "hardware"), "Unknown image method")
            identifier = f"{arguments.profile}-{method}-{cover}"
            case_root = output / identifier
            case_root.mkdir()
            report_path = case_root / "raw.json"
            command = [
                "xvfb-run", "-a", "--server-args=-screen 0 2200x3000x24", str(binary),
                "--source-dir", str(arguments.source_dir.resolve()), "--device", arguments.profile,
                "--screen", "reader", "--data-dir", str(case_root / "data"),
                "--capture", str(case_root / "final.png"), "--actions", str(command_file),
                "--exit-after-ms", "20700", "--demo", "--offline",
                "--performance-images", str(output / "source-images"), "--performance-report", str(report_path),
                "--performance-label", arguments.label, "--performance-cover", cover, "--image-method", method,
            ]
            with (case_root / "process.log").open("w", encoding="utf-8") as log:
                process = subprocess.Popen(command, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    returncode = process.wait(timeout=arguments.timeout_seconds)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=3)
                    raise
            require(returncode == 0, f"The real App fixture failed for {identifier}; inspect process.log")
            raw = read_json(report_path)
            receipt = read_json(case_root / "data" / "synthetic-perf05-images.json")
            checks = validate_case(raw, receipt, source)
            cases.append({"id": identifier, "report": str(report_path), "checks": checks, "timings": timing_summary(raw)})
    report = {
        "format": FORMAT, "label": arguments.label, "binary": str(binary), "binary_sha256": sha256(binary),
        "source": source, "cases": cases, "passed": len(cases), "failed": 0,
        "boundaries": [
            "Fresh isolated data and a new process; OS file caches are not flushed.",
            "Chapter-open and ordinary page-turn timings are distinct operations.",
            "WeRead has no prepared-next-page bitmap mode. Network chapter prefetch is inactive offline.",
            "Actions and readiness are polled every 20 ms; App.tick runs every 100 ms in both candidates. Dispatch/tick times exclude checkpoint PNG work.",
            "Desktop observations do not measure device SEND acceptance, optical latency, or panel quality.",
        ],
    }
    path = output / "report.json"
    write_json(path, report)
    print(json.dumps({"report": str(path), "passed": report["passed"], "failed": 0}))


def compare(arguments: argparse.Namespace) -> None:
    baseline, candidate = read_json(arguments.baseline), read_json(arguments.candidate)
    require(baseline["format"] == candidate["format"] == FORMAT, "The paired reports have unsupported formats")
    require([row["sha256"] for row in baseline["source"]["members"]] == [row["sha256"] for row in candidate["source"]["members"]], "Paired runs did not use identical acquired PNG bytes")
    previous = {case["id"]: case for case in baseline["cases"]}
    current = {case["id"]: case for case in candidate["cases"]}
    require(previous.keys() == current.keys(), "Paired reports have different case scopes")
    cases = []
    for identifier in previous:
        before, after = read_json(Path(previous[identifier]["report"])), read_json(Path(current[identifier]["report"]))
        old = {entry["label"]: entry for entry in before["checkpoints"]}
        new = {entry["label"]: entry for entry in after["checkpoints"]}
        require(old.keys() == new.keys(), "Paired checkpoint scopes differ")
        pixel_checks = []
        for label in old:
            require(pictures(old[label]) == pictures(new[label]), f"The {identifier}/{label} prepared pixels changed")
            difference = body_difference(Path(old[label]["path"]), Path(new[label]["path"]))
            require(difference["identical"], f"The {identifier}/{label} visible Reader body changed")
            pixel_checks.append({"label": label, "prepared_pixels_identical": True, "body": difference})
        cases.append({"id": identifier, "baseline": previous[identifier]["timings"], "candidate": current[identifier]["timings"], "pixels": pixel_checks})
    report = {"format": FORMAT, "baseline_binary_sha256": baseline["binary_sha256"], "candidate_binary_sha256": candidate["binary_sha256"], "cases": cases, "passed": len(cases), "failed": 0,
        "boundary": "Identical public fixture and model pixels. Remote desktop software timings are not Aidoku results or physical Kindle latency."}
    write_json(arguments.output, report)
    print(json.dumps({"comparison": str(arguments.output), "passed": len(cases), "failed": 0}))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    execute = commands.add_parser("run")
    execute.add_argument("--binary", type=Path, required=True)
    execute.add_argument("--source-dir", type=Path, required=True)
    execute.add_argument("--output-dir", type=Path, required=True)
    source = execute.add_mutually_exclusive_group(required=True)
    source.add_argument("--cbz", type=Path)
    source.add_argument("--images-dir", type=Path)
    execute.add_argument("--label", default="candidate")
    execute.add_argument("--profile", choices=("scribe", "pw6"), default="scribe")
    execute.add_argument("--methods", default="software")
    execute.add_argument("--covers", default="full")
    execute.add_argument("--timeout-seconds", type=int, default=120)
    paired = commands.add_parser("compare")
    paired.add_argument("--baseline", type=Path, required=True)
    paired.add_argument("--candidate", type=Path, required=True)
    paired.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    if arguments.command == "run":
        run(arguments)
    else:
        compare(arguments)


if __name__ == "__main__":
    main()
