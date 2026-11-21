"""Shared, device-free admission rules for the two orientation plans."""

from datetime import datetime
import re

from freeze import SCHEDULE


CONTROL = ["kindle", "bundled-font-preflight"]
CANDIDATE = [*CONTROL, "startup-orientation-reuse"]
PILOT = ["A-library", "B-library"]


class GateError(ValueError):
    pass


def require_pilot_closeout(plan, operations):
    if (not isinstance(operations, dict) or operations.get("label") != plan["label"]
            or operations.get("purpose", plan["purpose"]) != "startup-orientation-pilot"
            or operations.get("passed") is not True or operations.get("recovery_required") or operations.get("error")):
        raise GateError("Missing or unsuccessful matching pilot closeout")
    released = operations.get("released")
    if not isinstance(released, str) or not re.fullmatch(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?(?:Z|[+-][0-9]{2}:[0-9]{2})", released
    ):
        raise GateError("Pilot lease release timestamp is missing or invalid")
    try:
        datetime.fromisoformat(released.replace("Z", "+00:00"))
    except ValueError:
        raise GateError("Invalid pilot lease release timestamp") from None
    expected = [("admit", "none"), ("stage", "none"), *(("case", case) for case in PILOT),
                ("restore", "none"), ("collect", "none"), ("close", "none")]
    observed = operations.get("operations")
    if not isinstance(observed, list) or len(observed) != len(expected):
        raise GateError("Incomplete pilot operation sequence")
    for (mode, case), item in zip(expected, observed):
        if (not isinstance(item, dict) or (item.get("mode"), item.get("case")) != (mode, case)
                or type(item.get("status")) is not int or item["status"] != 0):
            raise GateError(f"Missing, failed or unexpected operation at {mode}/{case}")
    return released


def observed_portrait(read, case):
    raw = read(case, "portrait")
    match = re.fullmatch(rb"([UD]):([0-3])\n", raw)
    if (not match or read(case, "portrait-source") != b"native_idle_calibration\n"
            or any(read(case, name) != raw for name in ("portrait-launch", "portrait-return"))):
        raise ValueError(f"{case}: missing or inconsistent native portrait evidence")
    return ("up" if match[1] == b"U" else "down", int(match[2]))


def schedule_for(plan):
    purpose = plan.get("purpose")
    if purpose not in ("startup-orientation-pilot", "startup-orientation-reuse"):
        raise ValueError("Not an orientation trial")
    pilot = purpose == "startup-orientation-pilot"
    schedule = PILOT if pilot else SCHEDULE
    expected = {"device": "Kindle Scribe 2022", "firmware": "5.19.5", "clock": "CLOCK_MONOTONIC",
                "schedule": schedule, "control_features": CONTROL, "candidate_features": CANDIDATE,
                "launcher_wait_seconds": 2}
    for key, value in expected.items():
        if plan.get(key) != value:
            raise ValueError(f"Not the matched orientation experiment: {key}")
    if not isinstance(plan.get("label"), str) or not re.fullmatch(r"[a-f0-9]{16}", plan["label"]):
        raise ValueError("Invalid orientation batch label")
    if pilot:
        if any(key in plan for key in ("holding", "expected_panel_alignment_degrees", "raw_rotation")):
            raise ValueError("Pilot orientation must be observed, not preselected")
    else:
        if (plan.get("holding") not in ("up", "down") or type(plan.get("raw_rotation")) is not int
                or plan["raw_rotation"] not in range(4)):
            raise ValueError("A batch needs one declared fixed holding direction and framebuffer rotation")
        if (type(plan.get("expected_panel_alignment_degrees")) is not int
                or plan["expected_panel_alignment_degrees"] not in (0, 180)):
            raise ValueError("Predeclare the reference scanout alignment for the fixed holding direction")
    return schedule
