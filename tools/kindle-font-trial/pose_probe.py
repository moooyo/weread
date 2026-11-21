"""Render and interpret a read-only native Scribe posture probe. This module never connects."""

from pathlib import Path
import re

from orientation_device import section


HERE = Path(__file__).resolve().parent
COUNT = 10
MAX_NS = 2**63 - 1
NATIVE = r"(?:[UD]:[0-3]|FIXED:[0-3]|unavailable)"
DIRECTION = {15: "up", 17: "up", 16: "down", 18: "down"}


def render_device(helper_bytes):
    if type(helper_bytes) is not int or not 1 <= helper_bytes <= 512 * 1024:
        raise ValueError("Invalid posture helper length")
    source = (HERE.parent / "startup-observer/daily_render_device.sh").read_text(encoding="ascii")
    def lines(text, begin, end):
        return section(text, "\n" + begin, "\n" + end).lstrip("\n")

    readers = [lines(source, begin, end) for begin, end in [
        ("absent() {", "tag() {"), ("number() {", "token() {"), ("identity() {", "argv() {"),
        ("no_peer() {", "save_document() {"), ("state() {", "ready_volume() {"),
    ]]
    font = (HERE / "device.sh").read_text(encoding="ascii")
    orientation = (HERE / "orientation-device.sh").read_text(encoding="ascii")
    readers.extend([lines(orientation, "db=/var/local/cc.db", "@FONT_COMMON@"),
                    lines(font, "sql() {", "count_doc() {"), lines(font, "light() {", "peer_proof() {"),
                    lines(orientation, "catalog_one() {", "state() {"),
                    lines(orientation, "state() {", "new_stage() {").replace("state() {", "peer_projection() {", 1)])
    script = (HERE / "pose-probe.sh").read_text(encoding="ascii")
    return script.replace("@READERS@", "\n".join(readers)).replace("@BYTES@", str(helper_bytes)).replace("@COUNT@", str(COUNT))


def parse_observation(text, expected_label):
    if isinstance(text, bytes):
        text = text.decode("ascii")
    if not isinstance(text, str) or not re.fullmatch(r"[0-9a-f]{16}", expected_label):
        raise ValueError("Invalid public visit label or output")
    if not text.isascii() or not text.endswith("\n") or len(text) > 2048:
        raise ValueError("Incomplete posture output")
    lines = text[:-1].split("\n")
    if (len(lines) != COUNT + 4 or lines[0] != "pose_probe " + expected_label
            or lines[-1] != "protected_state unchanged"):
        raise ValueError("Unbound visit, incomplete output or unverified native state")
    before = re.fullmatch(r"native_before (" + NATIVE + ")", lines[1])
    after = re.fullmatch(r"native_after (" + NATIVE + ")", lines[-2])
    if not before or not after:
        raise ValueError("Invalid calibration records")
    before, after = before[1], after[1]
    samples = []
    for sequence, line in enumerate(lines[2:-2], 1):
        if not re.fullmatch(r"[1-9][0-9]? [1-9][0-9]{0,18} [1-9][0-9]{0,18} (?:15|16|17|18)", line):
            raise ValueError("Unexpected posture sample")
        index, start, end, raw = map(int, line.split())
        if index != sequence or not (samples[-1][2] if samples else 0) < start <= end <= MAX_NS:
            raise ValueError("Pose samples are incomplete or not in device clock order")
        samples.append([index, start, end, raw])
    directions = {DIRECTION[row[3]] for row in samples}
    if before != after:
        reason = "native_observations_differ"
    elif before == "unavailable" or before.startswith("FIXED:"):
        reason = "native_direction_not_available"
    elif len(directions) != 1:
        reason = "sensor_directions_differ"
    elif directions != {"up" if before[0] == "U" else "down"}:
        reason = "native_sensor_disagreement"
    else:
        reason = "matching_observations"
    return {"label": expected_label, "clock": "CLOCK_MONOTONIC", "source": "evdev_abs_pressure_snapshot",
            "native_before": before, "native_after": after, "samples": samples,
            "classification": "stable_native_reference" if reason == "matching_observations" else "inconclusive",
            "reason": reason, "protected_state_unchanged": True}
