"""Verify fixed-schedule first-SEND samples against independent Library and Reader captures."""

import argparse
import io
import json
from pathlib import Path
import re
import statistics
import tarfile

from PIL import Image, ImageChops, ImageStat
from freeze import SCHEDULE


TRACE = re.compile(r"WEREAD_STARTUP ([0-9]+)\.([0-9]{9}) ([a-z0-9_]+)")


def metrics(request, trace):
    match = re.fullmatch(r"REQUEST ([0-9]+)\.([0-9]{9})\s*", request)
    if not match:
        raise ValueError("Missing or repeated device request timestamp")
    start = int(match[1]) * 10**9 + int(match[2])
    times = {}
    previous = start
    for line in trace.splitlines():
        match = TRACE.fullmatch(line)
        if not match:
            raise ValueError("Malformed native timing record")
        stamp = int(match[1]) * 10**9 + int(match[2])
        if stamp < previous or match[3] in times:
            raise ValueError("Repeated stage, mixed clock or out-of-order time")
        times[match[3]] = previous = stamp
    markers = ["main_enter", "window_created", "fonts_registered", "software_ready", "first_application_frame_send"]
    if any(key not in times for key in markers) or [times[key] for key in markers] != sorted(times[key] for key in markers):
        raise ValueError("Incomplete or misordered primary stages")
    result = {"request_to_send_ms": (times[markers[-1]] - start) / 1e6,
              "font_ms": (times[markers[2]] - times[markers[1]]) / 1e6,
              "request_to_main_ms": (times[markers[0]] - start) / 1e6,
              "t_from_request_ns": {key: value - start for key, value in times.items()}}
    for site in ("splash", "event_loop"):
        prefix = site + "_orientation_"
        found = [status for status in ("success", "failed", "unavailable", "timeout") if prefix + status in times]
        if len(found) != 1 or prefix + "begin" not in times or times[prefix + found[0]] < times[prefix + "begin"]:
            raise ValueError("Missing orientation outcome")
        result[site] = {"outcome": found[0], "elapsed_ms": (times[prefix + found[0]] - times[prefix + "begin"]) / 1e6}
    return result


def mae(left, right):
    return ImageStat.Stat(ImageChops.difference(left, right)).mean[0]


def cropped(image):
    margin = 72 if image.height == 2480 else 2
    return image.crop((0, margin, image.width, image.height - margin))


def picture(reader, name):
    meta = json.loads(reader(name, "final.metadata.json"))
    if (meta["route"] != name.split("-")[1] or meta["backend"] != "kindle" or meta["profile"] != "scribe"
            or meta["window_size"] != [1860, 2480] or meta["snapshot_size"] != [1860, 2480]):
        raise ValueError(f"Wrong route or physical capture geometry for {name}")
    raw = Image.open(io.BytesIO(reader(name, "final.png")))
    if raw.size != (1860, 2480) or raw.convert("RGBA").getchannel("A").getextrema() != (255, 255):
        raise ValueError("Invalid software image")
    software = cropped(raw.convert("L"))
    scanout = Image.open(io.BytesIO(reader(name, "panel.pgm")))
    if scanout.size != raw.size or scanout.mode != "L" or ImageStat.Stat(software).stddev[0] < 8:
        raise ValueError("Invalid or uninformative picture")
    options = [(mae(software, cropped(value)), turn, cropped(value))
               for turn, value in [(0, scanout), (180, scanout.transpose(Image.Transpose.ROTATE_180))]]
    score, turn, aligned = min(options, key=lambda item: item[0])
    other = max(item[0] for item in options)
    if score > 8 or other - score < 3:
        raise ValueError(f"{name}: ambiguous physical/software agreement ({score:.3f}, {other:.3f})")
    return software, aligned, {"aligned_degrees": turn, "driver_software_body_mae": score, "opposite_body_mae": other}


def analyze(path, qualifications=False):
    with tarfile.open(path, "r:gz") as archive:
        def read(case, name):
            entry = archive.getmember(f"out/{case}/{name}")
            if not entry.isfile() or not 0 < entry.size <= 12 * 1024**2:
                raise ValueError("Unexpected output member")
            return archive.extractfile(entry).read()

        def thumb(case):
            image = Image.open(io.BytesIO(read(case, "thumb.pgm")))
            if image.mode != "L" or image.size != (48, 64):
                raise ValueError("Invalid frame thumbnail")
            return cropped(image)

        result = {"capture_pairs": {}, "qualification": {}, "samples": {}}
        references = {}
        for route in ("library", "reader"):
            first = picture(read, "A-" + route)
            second = picture(read, "B-" + route)
            for lane, item in zip("AB", (first, second)):
                case = lane + "-" + route
                result["qualification"][case] = item[2]
                references[case] = thumb(case)
            software = mae(first[0], second[0])
            physical = mae(first[1], second[1])
            result["capture_pairs"][route] = {"software_body_mae": software, "panel_body_mae": physical}
            if software > 0.1 or physical > 0.5:
                raise ValueError(f"{route} A/B body differs: {software:.3f}, {physical:.3f}")
        if qualifications:
            return result
        for case in SCHEDULE[4:]:
            lane = case[0]
            row = metrics(read(case, "request").decode(), read(case, "markers").decode())
            actual = thumb(case)
            right = mae(actual, references[lane + "-library"])
            wrong = mae(actual, references[lane + "-reader"])
            if right > 5 or wrong - right < 3:
                raise ValueError(f"{case}: Library content not corroborated ({right:.3f}, {wrong:.3f})")
            row.update(library_thumb_mae=right, reader_thumb_mae=wrong)
            result["samples"][case] = row
        for key in ("request_to_send_ms", "font_ms"):
            baseline, candidate = ([result["samples"][f"{lane}{index}"][key] for index in range(1, 7)] for lane in "AB")
            savings = [left - right for left, right in zip(baseline, candidate)]
            result[key] = {"baseline_median": statistics.median(baseline), "candidate_median": statistics.median(candidate),
                           "paired_savings": savings, "median_paired_saving": statistics.median(savings),
                           "improved_pairs": sum(value > 0 for value in savings)}
        return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("result", type=Path)
    parser.add_argument("--qualifications", action="store_true")
    args = parser.parse_args()
    result = analyze(args.archive, args.qualifications)
    args.result.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "samples"}))
