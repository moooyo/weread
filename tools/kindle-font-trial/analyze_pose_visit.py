"""Read a released candidate Reader visit; never connect to the device or infer a physical turn."""

import argparse
from datetime import datetime
import io
import json
from pathlib import Path
import re
import tarfile

from PIL import Image, ImageChops

from analyze import ImpossibleOrientationTrace, cropped, mae, picture
from analyze_orientation import orientation_sample
from run_pose_visit import schedule_for_visit


CASE = "B-reader"
LABEL_FILE = "out/orientation-batch"
LIMITS = {"request": 96, "markers": 4096, "portrait": 4, "portrait-source": 32,
          "portrait-launch": 4, "portrait-return": 4, "thumb.pgm": 4096,
          "panel.pgm": 1860 * 2480 + 128, "resumed.pgm": 1860 * 2480 + 128,
          "final.metadata.json": 4096, "final.png": 12 * 1024**2, "wake": 128, "splash": 128, "splash-status": 32}
SEQUENCE = [("admit", "none"), ("stage", "none"), ("case", CASE), ("restore", "none"),
            ("collect", "none"), ("close", "none"), ("absent", "none")]


def archive_members(path, label):
    inputs = {}
    limits = {f"out/{CASE}/{name}": size for name, size in LIMITS.items()} | {LABEL_FILE: 17}
    with tarfile.open(path, "r|gz") as archive:
        for member in archive:
            if (not member.isfile() or member.name not in limits or member.name in inputs
                    or not 0 < member.size <= limits[member.name]):
                raise ValueError("Invalid, duplicate or unbounded visit member")
            inputs[member.name] = archive.extractfile(member).read()
    if inputs.pop(LABEL_FILE, b"") != (label + "\n").encode("ascii"):
        raise ValueError("Visit archive label mismatch")
    return {key.split("/")[-1]: raw for key, raw in inputs.items()}


def timestamp(value):
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.utcoffset() is None:
        raise ValueError("Expected an offset in lease timestamps")
    return parsed


def pose(raw):
    if not re.fullmatch(rb"[UD]:[0-3]\n", raw):
        raise ValueError("Invalid native portrait")
    return raw.decode("ascii").strip()


def splash_evidence(raw, helper_status, sample, startup_pose):
    if raw == b"not_observed\n" and helper_status == b"not_observed\n":
        return {"status": "not_observed"}
    match = re.fullmatch(rb"SPLASH (\d{1,20}) (\d{1,20}) ([13]) (\d{1,3}) (\d{1,3})\n", raw)
    if not match or not sample or not startup_pose:
        return {"status": "insufficient"}
    begin, end, rotation, zero_error, turned_error = map(int, match.groups())
    marks = {name: sample["request_ns"] + relative for name, relative in sample["t_from_request_ns"].items()}
    if (rotation != int(startup_pose[-1]) or not 0 <= zero_error <= 255 or not 0 <= turned_error <= 255
            or not marks["startup_feedback_ready"] < begin <= end < marks.get("startup_feedback_finished", 0)
            <= marks["first_application_frame_send"]):
        return {"status": "insufficient"}
    status = ("matches_expected_0_degrees" if helper_status == b"0\n" and zero_error <= 12 and turned_error - zero_error >= 64 else
              "opposite_180_degrees" if helper_status == b"20\n" and turned_error <= 12 and zero_error - turned_error >= 64 else "insufficient")
    return {"status": status, "zero_error": zero_error, "half_turn_error": turned_error,
            "read_after_splash_ready_ms": [(value - marks["startup_feedback_ready"]) / 1e6 for value in (begin, end)]}


def wake_evidence(raw):
    match = re.fullmatch(rb"observed screenSaver stopped same_process wake_ready active (cover|native) (unobserved|[UD]:[0-3])\n", raw)
    if match:
        return {"status": "observed_screensaver_wake", "same_process": True, "same_cycle_ready": True,
                "display_owner": match[1].decode(), "wake_calibration": match[2].decode()}
    match = re.fullmatch(rb"(not_run) (alarm_unavailable|wake_command_unavailable|guardian_unavailable|power_state_unavailable)\n"
                         rb"|(not_qualified) (sleep_handshake_unobserved|wake_handshake_unobserved|power_cycle_unconfirmed)\n", raw)
    if match:
        status, reason = (item.decode() for item in match.groups() if item is not None)
        return {"status": status, "reason": reason}
    return {"status": "insufficient"}


def reader_frame(inputs, panel):
    def read(_case, leaf):
        return inputs[panel if leaf == "panel.pgm" else leaf]

    try:
        if not read(CASE, "final.png").startswith(b"\x89PNG\r\n\x1a\n") or not read(CASE, "panel.pgm").startswith(b"P5\n"):
            raise ValueError("Unexpected Reader image format")
        _, _, details = picture(read, CASE)
    except (OSError, ValueError, KeyError, TypeError, Image.DecompressionBombError):
        return {"status": "insufficient"}
    return {"status": "matches_expected_0_degrees" if details["aligned_degrees"] == 0 else "opposite_180_degrees", **details}


def panel_change(before, after):
    with Image.open(io.BytesIO(before)) as first, Image.open(io.BytesIO(after)) as second:
        if first.mode != "L" or second.mode != "L" or first.size != (1860, 2480) or second.size != first.size:
            raise ValueError("Unexpected Reader panel geometry")
        difference = ImageChops.difference(first, second)
        return {"whole_different_pixels": sum(difference.histogram()[1:]),
                "body_different_pixels": sum(cropped(difference).histogram()[1:]), "body_mae": mae(cropped(first), cropped(second))}


def analyze_visit(plan, receipt, path):
    schedule_for_visit(plan)
    if (receipt.get("label") != plan["label"] or receipt.get("purpose") != plan["purpose"]
            or receipt.get("scope") != "transaction_only" or receipt.get("owner_token_checked_on_release") is not True
            or receipt.get("recovery_required") or timestamp(receipt["released"]) < timestamp(receipt["acquired"])):
        raise ValueError("Not a matching released Reader visit")
    report = {"label": plan["label"], "device": plan["device"], "firmware": plan["firmware"], "released": receipt["released"],
              "physical_turn_and_touch_acceptance": "not_established",
              "collection": "not_qualified", "splash": {"status": "not_assessed"}, "wake": {"status": "not_assessed"},
              "reader_before": {"status": "not_assessed"}, "reader_resumed": {"status": "not_assessed"},
              "not_measured": ["physical turn during startup or sleep", "physical touch mapping", "deep system suspend",
                               "optical ink", "ordinary Library card startup time"]}
    operations = receipt.get("operations")
    if (receipt.get("passed") is not True or receipt.get("error") or not isinstance(operations, list)
            or any(not isinstance(op, dict) or type(op.get("status")) is not int or op["status"] != 0 for op in operations)
            or [(op.get("mode"), op.get("case")) for op in operations] != SEQUENCE):
        return report
    inputs = archive_members(path, plan["label"])
    report["collection"] = "completed_and_released"
    try:
        native, startup, returned = (pose(inputs[name]) for name in ("portrait", "portrait-launch", "portrait-return"))
        if inputs["portrait-source"] != b"native_idle_calibration\n" or native != startup:
            raise ValueError("Startup calibration did not match the live native reading")
        report["native_pose"] = {"before": native, "launch": startup, "return": returned,
                                 "relative_to_historical_down_1": {"U:3": "opposite_native_reference", "D:1": "same_native_reference"}.get(native, "different_reading")}
    except (ValueError, KeyError):
        startup = None
        report["native_pose"] = {"status": "insufficient"}
    try:
        sample = orientation_sample(inputs["request"].decode("ascii"), inputs["markers"].decode("ascii"))
        report["orientation_queries"] = {site: sample[site]["outcome"] for site in ("splash", "event_loop")}
    except ImpossibleOrientationTrace:
        raise
    except (ValueError, KeyError):
        sample = None
    report["splash"] = splash_evidence(inputs.get("splash", b""), inputs.get("splash-status"), sample, startup)
    report["reader_before"] = reader_frame(inputs, "panel.pgm")
    report["wake"] = wake_evidence(inputs.get("wake", b""))
    if report["wake"]["status"] == "observed_screensaver_wake":
        report["reader_resumed"] = reader_frame(inputs, "resumed.pgm")
        if all(report[which]["status"] == "matches_expected_0_degrees" for which in ("reader_before", "reader_resumed")):
            report["reader_comparison"] = panel_change(inputs["panel.pgm"], inputs["resumed.pgm"])
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("plan", "operations", "archive", "output"):
        parser.add_argument(arg, type=Path)
    args = parser.parse_args()
    result = analyze_visit(json.loads(args.plan.read_text()), json.loads(args.operations.read_text()), args.archive)
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
