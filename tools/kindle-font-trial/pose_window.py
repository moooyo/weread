"""Classify one observed Scribe pose change offline, not screen, touch or hardware acceptance.

Supply two independently bound JSON receipts. Each has a `launch` object containing the
public batch label, visit, boot_id, gui_pid, gui_birth and clock. The application receipt
has raw `request` and `markers` text from the indexed orientation collector. The sensor
receipt has source `evdev_abs_pressure_snapshot` and `samples` in acquisition order:
[sequence (starting at 1), before_ns, after_ns, raw ABS_PRESSURE]. Bracket each fresh
snapshot of the same validated descriptor on CLOCK_MONOTONIC; retain gaps or invalid
reads (null), never interpolate. The caller must establish the provenance and binding
on capture; copying an identity onto unrelated observations cannot establish either.
Only the classification and timing of the observed change are public output.

A hit requires the entire transition bracket strictly after startup_feedback_ready and
before portrait_opened, which runs before the first business draw. The first SEND
marker is written after driver acceptance: a change between these last two markers
cannot be classified more precisely without another witness. A miss refers only to the
observed change outside the marker window, not to the absence of all physical motion.
"""

import argparse
import json
from pathlib import Path
import re

from analyze_orientation import orientation_sample


MAX_NS = 2**63 - 1
HOLDING = {15: "up", 17: "up", 16: "down", 18: "down"}


def positive(value):
    return type(value) is int and 0 < value <= MAX_NS


def bound(receipt):
    identity = receipt["launch"]
    if not (isinstance(identity, dict) and identity.get("clock") == "CLOCK_MONOTONIC"
            and re.fullmatch(r"[0-9a-f]{16}", identity["label"])
            and re.fullmatch(r"[A-Za-z0-9_-]{1,48}", identity["visit"])
            and re.fullmatch(r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", identity["boot_id"])
            and positive(identity["gui_pid"]) and positive(identity["gui_birth"])):
        raise ValueError("Invalid launch binding")
    return identity


def classify(application, sensor):
    def insufficient(reason):
        return {"coverage": "insufficient", "reason": reason}

    try:
        if bound(application) != bound(sensor) or sensor["source"] != "evdev_abs_pressure_snapshot":
            return insufficient("unbound_sensor")
        trace = orientation_sample(application["request"], application["markers"])
        start, before_draw, sent = (trace["request_ns"] + trace["t_from_request_ns"][stage] for stage in
                                    ("startup_feedback_ready", "portrait_opened", "first_application_frame_send"))
        if not 0 < start < before_draw < sent <= MAX_NS:
            return insufficient("ambiguous_markers")
        changes = []
        previous = None
        for expected, (sequence, begin, end, raw) in enumerate(sensor["samples"], 1):
            if (not positive(sequence) or sequence != expected or not positive(begin) or not positive(end)
                    or begin > end or (previous and begin <= previous[1]) or type(raw) is not int or raw not in HOLDING):
                return insufficient("discontinuous_sensor")
            current = (begin, end, HOLDING[raw])
            if previous and previous[2] != current[2]:
                changes.append((previous[0], end, previous[2], current[2]))
            previous = current
    except (AttributeError, KeyError, TypeError, ValueError):
        return insufficient("invalid_or_missing_input")
    if len(changes) != 1:
        return insufficient("no_single_observed_change")
    earliest, latest, origin, destination = changes[0]
    if start < earliest and latest < before_draw:
        outcome = "hit"
    elif latest < start or sent < earliest:
        outcome = "missed"
    else:
        return insufficient("boundary_overlap_or_post_draw")
    return {"coverage": outcome, "from": origin, "to": destination, "observed_change_ns": [earliest, latest]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("application", type=Path)
    parser.add_argument("sensor", type=Path)
    args = parser.parse_args()
    try:
        result = classify(json.loads(args.application.read_text()), json.loads(args.sensor.read_text()))
    except (OSError, ValueError):
        result = {"coverage": "insufficient", "reason": "unreadable_input"}
    print(json.dumps(result))
    raise SystemExit({"hit": 0, "missed": 1, "insufficient": 2}[result["coverage"]])
