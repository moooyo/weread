"""Qualify a closed orientation pilot or the four pictures before timing, offline."""

import argparse
import io
import json
from pathlib import Path
import tarfile

from PIL import Image

from analyze import cropped, mae, picture
from analyze_orientation import SITES, orientation_sample
from freeze import SCHEDULE
from orientation_contract import GateError, PILOT as PILOT_CASES, observed_portrait, require_pilot_closeout, schedule_for


PILOT = "startup-orientation-pilot"
FULL = "startup-orientation-reuse"
PICTURE_CASES = SCHEDULE[:4]
LIMITS = {"request": 96, "markers": 64 * 1024, "portrait": 4, "portrait-launch": 4,
          "portrait-return": 4, "portrait-source": 32, "final.metadata.json": 4096,
          "thumb.pgm": 4096, "final.png": 12 * 1024**2, "panel.pgm": 1860 * 2480 + 128}


def require_plan(plan, purpose):
    if not isinstance(plan, dict) or plan.get("purpose") != purpose:
        raise GateError("Wrong picture gate for the frozen plan")
    try:
        schedule_for(plan)
    except ValueError as error:
        raise GateError(str(error)) from None


def public_inputs(path, label, cases):
    label_key = "out/orientation-batch"
    wanted = {f"out/{case}/{leaf}": maximum for case in cases for leaf, maximum in LIMITS.items()}
    wanted[label_key] = 17
    inputs = {}
    seen = set()
    total = 0
    with tarfile.open(path, "r|gz") as archive:
        for member in archive:
            parts = member.name.split("/")
            context = f"{parts[1]}: " if len(parts) >= 2 and parts[0] == "out" and parts[1] in SCHEDULE else ""
            total += member.size
            if member.name in seen or len(seen) >= 128 or member.size < 0 or total > 80 * 1024**2:
                raise GateError(f"{context}ambiguous or oversized archive")
            seen.add(member.name)
            if member.isdir() and (parts == ["out"] or (len(parts) == 2 and parts[0] == "out" and parts[1] in cases)):
                continue
            if member.name not in wanted:
                raise GateError(f"{context}archive contains an unexpected case or public entry")
            if not member.isfile() or not 0 < member.size <= wanted[member.name]:
                raise GateError(f"{parts[1]}: invalid public evidence size or type")
            inputs[member.name] = archive.extractfile(member).read()
    if inputs.get(label_key) != (label + "\n").encode("ascii"):
        raise GateError("Raw capture label does not match the frozen plan")
    for case in cases:
        if any(f"out/{case}/{leaf}" not in inputs for leaf in LIMITS):
            raise GateError(f"{case}: incomplete public picture evidence")
    return lambda case, leaf: inputs[f"out/{case}/{leaf}"]


def qualify(plan, archive, cases, expected_pose=None, alignment=None):
    read = public_inputs(archive, plan["label"], cases)
    qualification = {}
    pairs = {}
    previous_send = None
    thumbs = {}
    for case in cases:
        try:
            sample = orientation_sample(read(case, "request").decode("ascii"), read(case, "markers").decode("ascii"))
        except (UnicodeError, ValueError, KeyError):
            raise GateError(f"{case}: invalid orientation trace or device clock") from None
        if any((sample[site]["outcome"] == "reused") != (case[0] == "B") for site in SITES):
            outcomes = ", ".join(f"{site}={sample[site]['outcome']}" for site in SITES)
            raise GateError(f"{case}: picture did not exercise the declared orientation path ({outcomes})")
        if previous_send is not None and sample["request_ns"] <= previous_send:
            raise GateError(f"{case}: overlapping or out-of-order device timestamps")
        previous_send = sample["request_ns"] + sample["t_from_request_ns"]["first_application_frame_send"]
        try:
            pose = observed_portrait(read, case)
        except ValueError:
            raise GateError(f"{case}: native, post-SEND launch and restored portrait evidence does not agree") from None
        if case == "A-library" and expected_pose is None:
            expected_pose = pose
        if pose != expected_pose:
            raise GateError(f"{case}: pose or framebuffer rotation changed")
        try:
            if not read(case, "final.png").startswith(b"\x89PNG\r\n\x1a\n") or not read(case, "panel.pgm").startswith(b"P5\n"):
                raise ValueError()
            software, scanout, observation = picture(read, case)
            with Image.open(io.BytesIO(read(case, "thumb.pgm"))) as thumb:
                if thumb.format != "PPM" or thumb.mode != "L" or thumb.size != (48, 64):
                    raise ValueError()
                thumbs[case] = cropped(thumb)
        except (ValueError, KeyError, TypeError, OSError, Image.DecompressionBombError):
            raise GateError(f"{case}: invalid route, capture geometry, contrast or image agreement") from None
        if case == "A-library" and alignment is None:
            alignment = observation["aligned_degrees"]
        if observation["aligned_degrees"] != alignment:
            raise GateError(f"{case}: scanout alignment differs from the independent control reference")
        qualification[case] = observation
        if case[0] == "A":
            control_software, control_scanout = software, scanout
        else:
            software_error, scanout_error = mae(control_software, software), mae(control_scanout, scanout)
            if software_error > 0.1 or scanout_error > 0.5:
                raise GateError(f"{case}: candidate and control picture bodies differ")
            pairs[case[2:]] = dict(software_body_mae=software_error, panel_body_mae=scanout_error)
    if cases == PICTURE_CASES:
        for arm in "AB":
            if mae(thumbs[arm + "-library"], thumbs[arm + "-reader"]) < 3:
                raise GateError(f"{arm}-reader: route thumbnails cannot distinguish Library from Reader")
    return expected_pose, alignment, qualification, pairs


def pilot_reference(plan, operations, archive):
    require_plan(plan, PILOT)
    released = require_pilot_closeout(plan, operations)
    (holding, rotation), alignment, _, _ = qualify(plan, archive, PILOT_CASES)
    return dict(purpose=PILOT, label=plan["label"], source_case="A-library", holding=holding,
                raw_rotation=rotation, expected_panel_alignment_degrees=alignment, passed=True,
                device=plan["device"], firmware=plan["firmware"], released=released, route="library",
                software_scanout_qualified=True)


def qualify_pictures(plan, archive):
    require_plan(plan, FULL)
    pose = plan["holding"], plan["raw_rotation"]
    _, alignment, qualification, pairs = qualify(plan, archive, PICTURE_CASES, pose, plan["expected_panel_alignment_degrees"])
    return dict(purpose="startup-orientation-pictures", label=plan["label"], pictures_passed=True,
                scope="picture qualification only", holding=pose[0], raw_rotation=pose[1],
                expected_panel_alignment_degrees=alignment, qualification=qualification, capture_pairs=pairs)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    for mode in ("pilot", "pictures"):
        command = commands.add_parser(mode)
        command.add_argument("plan", type=Path)
        if mode == "pilot":
            command.add_argument("operations", type=Path)
        command.add_argument("archive", type=Path)
        command.add_argument("output", type=Path)
    args = parser.parse_args(argv)
    try:
        plan = json.loads(args.plan.read_text())
        if args.mode == "pilot":
            result = pilot_reference(plan, json.loads(args.operations.read_text()), args.archive)
        else:
            result = qualify_pictures(plan, args.archive)
        with args.output.open("x") as output:
            output.write(json.dumps(result, indent=2) + "\n")
    except GateError as error:
        parser.exit(1, f"Picture gate refused: {error}\n")
    except (OSError, EOFError, ValueError, tarfile.TarError):
        parser.exit(1, "Picture gate refused: unreadable inputs or unavailable fresh output\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
