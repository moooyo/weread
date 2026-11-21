"""Run one prepared Scribe candidate Reader visit, or recover only its own stage."""

import argparse
import json
from pathlib import Path
import re

from pose_visit import CONTRACT, verify_plan, verify_recovery
from run import run, transport


def schedule_for_visit(plan):
    if (not isinstance(plan, dict) or any(plan.get(key) != value for key, value in CONTRACT.items())
            or type(plan.get("expected_panel_alignment_degrees")) is not int
            or not isinstance(plan.get("label"), str) or not re.fullmatch(r"[0-9a-f]{16}", plan["label"])):
        raise ValueError("Not a single prepared Scribe candidate Reader visit")
    return CONTRACT["schedule"]


def execute(args):
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if plan == work or not plan.is_relative_to(work):
        raise ValueError("Use a private plan beneath the worker directory")
    config = (verify_recovery if args.recover else verify_plan)(plan)
    schedule_for_visit(config)
    transport((plan / "device.sh").read_text(encoding="ascii"), "admit", config["label"], "none")
    if args.preflight:
        print(json.dumps({"label": config["label"], "validated": True, "schedule": config["schedule"],
                          "device_accessed": False, "lease_requested": False}))
        return 0
    if not args.lease_tool or not args.owner:
        raise ValueError("A private lease utility and this session's owner are required")
    return run(args, pose_visit_schedule=schedule_for_visit)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("work", "plan"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--lease-tool", type=Path)
    parser.add_argument("--owner")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--preflight", action="store_true", help="Verify only the local plan and inputs")
    group.add_argument("--recover", type=Path, help="Close and verify the original attempt without another visit")
    parser.set_defaults(wait=0, admit_only=False)
    raise SystemExit(execute(parser.parse_args()))
