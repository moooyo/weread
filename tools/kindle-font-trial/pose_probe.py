"""Render and interpret a read-only native Scribe posture probe. This module never connects."""

from pathlib import Path
import re

from orientation_device import section


HERE = Path(__file__).resolve().parent
COUNT = 10
TRACE_COUNT = 12
MAX_NS = 2**63 - 1
NATIVE = r"(?:[UD]:[0-3]|FIXED:[0-3]|unavailable)"
DIRECTION = {15: "up", 17: "up", 16: "down", 18: "down"}


def render_device(helper_bytes, *, display_trace=False):
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
    def variant(template, tag, keep):
        begin, end = "@" + tag + "_BEGIN@\n", "@" + tag + "_END@\n"
        if template.count(begin) != 1 or template.count(end) != 1:
            raise ValueError("Ambiguous posture template section")
        first, rest = template.split(begin)
        optional, last = rest.split(end)
        return first + (optional if keep else "") + last

    for tag in ("TRACE_FUNCTIONS", "DISPLAY_READS", "DISPLAY_RESULT"):
        script = variant(script, tag, display_trace)
    for tag in ("ORDINARY_READS", "ORDINARY_RESULT"):
        script = variant(script, tag, not display_trace)
    if display_trace:
        script = script.replace('"$stage/owner"|"$stage/snapshot")', '"$stage/owner"|"$stage/snapshot"|"$stage/winmgr.out")')
        script = script.replace('rm -f "$stage/snapshot" &&', 'rm -f "$stage/snapshot" "$stage/winmgr.out" &&')
    script = script.replace("@TRACE_TIMER@\n", '        trace_timer_ready || fail timeout_unavailable\n' if display_trace else '')
    return (script.replace("@READERS@", "\n".join(readers)).replace("@BYTES@", str(helper_bytes))
            .replace("@COUNT@", str(COUNT)).replace("@TRACE_COUNT@", str(TRACE_COUNT)))


def parse_observation(text, expected_label, *, display_trace=False):
    if isinstance(text, bytes):
        text = text.decode("ascii")
    if not isinstance(text, str) or not re.fullmatch(r"[0-9a-f]{16}", expected_label):
        raise ValueError("Invalid public visit label or output")
    if not text.isascii() or not text.endswith("\n") or len(text) > (4096 if display_trace else 2048):
        raise ValueError("Incomplete posture output")
    lines = text[:-1].split("\n")
    if display_trace:
        return parse_display_rows(lines, expected_label)
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


def parse_display_group(line, sequence, previous_end=0):
    edge = r"([1-9][0-9]{0,18}) ([1-9][0-9]{0,18}) (1[5-8]) ([0-3])"
    row = re.fullmatch(r"read ([1-9][0-9]?) " + edge + r" ([LRUD]|unknown) " + edge, line)
    if not row or int(row[1]) != sequence:
        raise ValueError("Invalid native display group")
    left_start, left_end, left_raw, left_fb = map(int, row.groups()[1:5])
    right_start, right_end, right_raw, right_fb = map(int, row.groups()[6:10])
    if not previous_end < left_start <= left_end < right_start <= right_end <= MAX_NS:
        raise ValueError("Native observations are not in device monotonic clock order")
    return {"sequence": sequence,
            "before": {"interval_ns": [left_start, left_end], "raw": left_raw, "framebuffer_rotation": left_fb},
            "winmgr": {"raw": row[6], "interval_ns": [left_end, right_start]},
            "after": {"interval_ns": [right_start, right_end], "raw": right_raw, "framebuffer_rotation": right_fb}}


def parse_display_rows(lines, label):
    if (not 4 <= len(lines) <= TRACE_COUNT + 3 or lines[0] != "pose_display_trace " + label
            or lines[-1] != "protected_state unchanged" or lines[-2] not in ("trace_end complete", "trace_end budget")):
        raise ValueError("Unbound, unverified or incomplete native display diagnostic")
    end = lines[-2].split()[1]
    if (len(lines) == TRACE_COUNT + 3) != (end == "complete"):
        raise ValueError("Unexpected trace count for its stopping condition")
    groups = []
    previous_end = 0
    for sequence, line in enumerate(lines[1:-2], 1):
        group = parse_display_group(line, sequence, previous_end)
        previous_end = group['after']['interval_ns'][1]
        groups.append(group)
    return {"label": label, "clock": "CLOCK_MONOTONIC", "classification": "display_diagnostic_only",
            "app_admitted": False, "trace_end": end, "samples": groups, "protected_state_unchanged": True}
