"""Freeze an offline orientation pilot or seal a separate fixed-pose timing plan."""

import argparse
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import secrets
import shutil
import subprocess
import tarfile
import tempfile

from freeze import HERE, SCHEDULE, launcher
from orientation_contract import CANDIDATE, CONTROL, PILOT as PICTURES, require_pilot_closeout, schedule_for
from orientation_device import render_device


PILOT = "startup-orientation-pilot"
FORMAL = "startup-orientation-reuse"
FACES = ["SANS_REGULAR", "SANS_MEDIUM", "SANS_BOLD", "SERIF_LIGHT", "SERIF_REGULAR", "SERIF_MEDIUM", "SERIF_BOLD"]
HELPERS = ("weread-frame-guard", "weread-native-query")
DEVICE = "Kindle Scribe 2022"
FIRMWARE = "5.19.5"


def local(work, path, *, directory=False):
    path = path.absolute()
    if path.resolve(strict=True) != path or not path.is_relative_to(work) or not (
            path.is_dir() if directory else path.is_file()):
        raise ValueError(f"Expected a regular project-local {'directory' if directory else 'file'}: {path.name}")
    return path


def read_record(path):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 256 * 1024:
        raise ValueError(f"Missing or unbounded record: {path.name}")
    return json.loads(path.read_bytes())


def publish(path, record):
    with path.open("x", encoding="utf-8", newline="\n") as output:
        output.write(json.dumps(record, indent=2) + "\n")


def trial_document(source, label):
    template = (HERE / "document.sh.in").read_text()
    for before, after in [("# Name: WeRead Font Trial", "# Name: WeRead Orientation Trial"),
                          ("/mnt/us/.weread-indexed-@LABEL@", "/mnt/us/.weread-orientation-@LABEL@")]:
        if template.count(before) != 1:
            raise ValueError("Unrecognized one-shot public document template")
        template = template.replace(before, after)
    icons = [line for line in (source / "packaging/kindle/document-launcher.sh").read_text().splitlines()
             if line.startswith("# Icon:")]
    if len(icons) != 1 or template.count("set -eu\n") != 1 or not re.fullmatch(r"[a-f0-9]{16}", label):
        raise ValueError("Invalid public icon or trial label")
    return template.replace("@LABEL@", label).replace("set -eu\n", icons[0] + "\nset -eu\n").encode("ascii")


def arm_inputs(work):
    source = local(work, work / "src", directory=True)
    inspect_elf = runpy.run_path(str(source / "docs/review/library-startup-latency-20261119/rebuild-arm.py"))["inspect_elf"]
    arms = {}
    for name, lane, features in [("A", "baseline", CONTROL), ("B", "candidate", CANDIDATE)]:
        folder = work / "arm-pair" / lane
        record = read_record(local(work, folder / "result.json"))
        expected = {"source": "/work/src", "lane": lane, "abi": "armv7-unknown-linux-musleabihf",
                    "both_recompiled": True, "backend_features": [] if name == "A" else [CANDIDATE[-1]],
                    "generated_receipts": FACES}
        if (any(record.get(key) != value for key, value in expected.items())
                or sorted(record.get("application_features", [])) != sorted(features)):
            raise ValueError(f"Unmatched source, features or seven font receipts for {name}")
        executable = local(work, folder / "weread")
        elf = inspect_elf(executable)
        if record.get("elf") != elf:
            raise ValueError(f"Retained ELF differs from the {name} build receipt")
        arms[name] = {"executable": str(executable.relative_to(work)), "bytes": elf["bytes"],
                      "features": features, "backend_features": record["backend_features"], "font_receipts": len(FACES)}
    if arms["A"]["bytes"] == arms["B"]["bytes"]:
        raise ValueError("The two independently recorded executable lengths must distinguish these retained builds")
    return arms


def public_seed(source, path):
    if path.stat().st_size > 16 * 1024**2:
        raise ValueError("Unexpected public seed size")
    bindings = read_record(source / "docs/review/catalog-entry-adapter-20261118/library-cover-bindings.json")
    covers = {item["thumbnail"] for item in bindings["targets"]}
    if len(covers) != 15:
        raise ValueError("Expected the approved 15-cover binding set")
    expected = covers | {"demo/library.sqlite", "library-covers.json", "demo/images/cache-index.sqlite3",
                         "demo/covers/cache-index.sqlite3"}
    files, seen, size = set(), set(), 0
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            name = member.name
            size += member.size
            if (name in seen or PurePosixPath(name).as_posix() != name or len(seen) >= 32
                    or size > 16 * 1024**2 or not (member.isfile() or member.isdir())):
                raise ValueError("Invalid public seed archive member")
            seen.add(name)
            if member.isdir():
                if name not in {"demo", "demo/images", "demo/covers"}:
                    raise ValueError("Unexpected public seed directory")
                continue
            if name not in expected and not re.fullmatch(r"demo/images/[0-9a-f]{64}-[0-9a-f]{64}\.png", name):
                raise ValueError("Unexpected public seed file")
            files.add(name)
            if name == "library-covers.json" and (member.size > 64 * 1024 or json.load(archive.extractfile(member)) != bindings):
                raise ValueError("The seed does not contain the approved public cover bindings")
            if name == "demo/library.sqlite" and archive.extractfile(member).read(16) != b"SQLite format 3\0":
                raise ValueError("The public shelf is not a SQLite database")
    if not expected <= files or len(files - expected) != 1:
        raise ValueError("Incomplete 15-cover public fixture")


def check_request_helper(measure, label, parent):
    with tempfile.TemporaryDirectory(dir=parent) as temporary:
        stub = Path(temporary) / "lipc-set-prop"
        stub.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
        stub.chmod(0o700)
        emulator = shutil.which("qemu-arm")
        if not emulator:
            raise ValueError("The offline worker requires qemu-arm to check the actual request namespace")
        arguments = [emulator, "-cpu", "cortex-a8", str(measure), "request"]
        environment = dict(os.environ, PATH=temporary)
        valid = subprocess.run([*arguments, label], env=environment, capture_output=True, check=True, timeout=15)
        lines = valid.stdout.decode("ascii").splitlines()
        if (len(lines) != 4 or not re.fullmatch(r"REQUEST [0-9]+\.[0-9]{9}", lines[0]) or lines[1:] != [
                "com.lab126.appmgrd", "start", f"file:///mnt/us/documents/weread-orientation-trial-{label}.sh"]):
            raise ValueError("The request helper does not exclusively target the orientation document")
        invalid = subprocess.run([*arguments, "bad"], env=environment, capture_output=True, timeout=15)
        if invalid.returncode != 64 or invalid.stdout:
            raise ValueError("The orientation request helper accepted an invalid document name")


def payload(work, label, seed, helpers):
    arms = arm_inputs(work)
    source = work / "src"
    document = trial_document(source, label)
    supervisor = launcher((source / "packaging/kindle/launch.sh").read_text()).encode()
    device = render_device(arms["A"]["bytes"], arms["B"]["bytes"]).encode("ascii")
    entries = {"document": document, "measure": local(work, helpers / "measure"), "seed.tar.gz": local(work, seed)}
    for name in arms:
        entries[f"{name}/launch.sh"] = supervisor
        entries[f"{name}/app/weread"] = work / arms[name]["executable"]
        for helper in HELPERS:
            entries[f"{name}/app/{helper}"] = local(work, helpers / helper)
    sizes = {name: len(value) if isinstance(value, bytes) else value.stat().st_size for name, value in entries.items()}
    if any(size <= 0 for size in sizes.values()):
        raise ValueError("All eleven orientation package members must be nonempty")
    basis = {"label": label, "arms": arms, "helpers": str(helpers.relative_to(work)),
             "seed": str(seed.relative_to(work)), "members": sizes}
    return basis, entries, device


def base_plan(preparation):
    purpose = preparation["purpose"]
    if purpose not in (PILOT, FORMAL):
        raise ValueError("Unsupported orientation experiment")
    return {"purpose": purpose, "label": preparation["label"], "schedule": PICTURES if purpose == PILOT else SCHEDULE,
            "device": DEVICE, "firmware": FIRMWARE, "clock": "CLOCK_MONOTONIC", "launcher_wait_seconds": 2,
            "control_features": CONTROL, "candidate_features": CANDIDATE, "arms": preparation["arms"],
            "common_inputs": {"seed": preparation["seed"], "helpers": preparation["helpers"],
                             "bytes": {name: preparation["members"][name] for name in (
                                 "seed.tar.gz", "measure", "A/launch.sh", *(f"A/app/{helper}" for helper in HELPERS))}},
            "stabilize_seconds": 8, "timed_dwell_ms": 6000}


def freeze(work, run, seed, helpers, *, pilot=False):
    work = work.resolve(strict=True)
    run = run.absolute()
    local(work, run.parent, directory=True)
    if run.is_relative_to(work / "src") or run.is_relative_to(work / "arm-pair"):
        raise ValueError("Place the fresh plan outside the sources and retained ARM builds")
    label = secrets.token_hex(8)
    basis, entries, device = payload(work, label, local(work, seed), local(work, helpers, directory=True))
    public_seed(work / "src", seed)
    run.mkdir(mode=0o700)
    check_request_helper(entries["measure"], label, run)
    for program in (device, entries["document"], entries["A/launch.sh"]):
        subprocess.run(["sh", "-n"], input=program, check=True)
    (run / "document.reference").write_bytes(entries["document"])
    (run / "device.sh").write_bytes(device)
    with tarfile.open(run / "inputs.tgz", "x:gz", compresslevel=1) as archive:
        for name, value in entries.items():
            member = tarfile.TarInfo(name)
            member.mode = 0o600 if name == "seed.tar.gz" else 0o755
            member.size = basis["members"][name]
            with (io.BytesIO(value) if isinstance(value, bytes) else value.open("rb")) as content:
                archive.addfile(member, content)
    preparation = dict(basis, purpose=PILOT if pilot else FORMAL, archive_bytes=(run / "inputs.tgz").stat().st_size)
    publish(run / "preparation.json", preparation)
    if pilot:
        publish(run / "plan.json", base_plan(preparation))
    return preparation


def check_payload(run, work):
    local(work, run, directory=True)
    preparation = read_record(run / "preparation.json")
    seed = local(work, work / preparation["seed"])
    helpers = local(work, work / preparation["helpers"], directory=True)
    basis, entries, device = payload(work, preparation["label"], seed, helpers)
    if (any(preparation.get(key) != value for key, value in basis.items())
            or local(work, run / "device.sh").read_bytes() != device
            or local(work, run / "document.reference").read_bytes() != entries["document"]):
        raise ValueError("Frozen orientation inputs changed")
    path = local(work, run / "inputs.tgz")
    if path.stat().st_size != preparation["archive_bytes"]:
        raise ValueError("Frozen orientation archive length changed")
    with tarfile.open(path, "r:gz") as archive:
        seen = set()
        for member in archive:
            if (not member.isfile() or member.name in seen or member.name not in entries
                    or member.size != basis["members"][member.name]
                    or member.mode != (0o600 if member.name == "seed.tar.gz" else 0o755)):
                raise ValueError("Invalid frozen orientation archive member")
            seen.add(member.name)
        if seen != set(entries):
            raise ValueError("Missing frozen orientation archive member")
    return preparation


def from_reference(work, preparation, reference):
    reference = local(work, reference)
    attempt = reference.parent
    pilot = read_record(local(work, attempt.parent / "plan.json"))
    operations = read_record(local(work, attempt / "operations.json"))
    proof = read_record(reference)
    label = proof.get("label")
    if (reference.name != "pilot-reference.json" or not isinstance(label, str) or not re.fullmatch(r"[a-f0-9]{16}", label)
            or label == preparation["label"] or pilot != base_plan(dict(preparation, purpose=PILOT, label=label))):
        raise ValueError("A separate frozen orientation pilot with the matching builds is required")
    expected = {"purpose": PILOT, "source_case": "A-library", "route": "library", "device": DEVICE, "firmware": FIRMWARE}
    if (any(proof.get(key) != value for key, value in expected.items()) or proof.get("passed") is not True
            or proof.get("software_scanout_qualified") is not True or proof.get("holding") not in ("up", "down")
            or type(proof.get("raw_rotation")) is not int or proof["raw_rotation"] not in range(4)
            or type(proof.get("expected_panel_alignment_degrees")) is not int or proof["expected_panel_alignment_degrees"] not in (0, 180)):
        raise ValueError("Invalid independent A-library fixed-pose reference")
    if require_pilot_closeout(pilot, operations) != proof.get("released"):
        raise ValueError("The reference does not match the independently closed pilot")
    return {key: proof[key] for key in ("holding", "raw_rotation", "expected_panel_alignment_degrees")} | {
        "pilot_label": label, "pilot_reference": str(reference.relative_to(work))}


def seal(work, run, reference):
    work = work.resolve(strict=True)
    preparation = check_payload(run, work)
    if preparation["purpose"] != FORMAL:
        raise ValueError("Only the separate formal timing plan may be sealed")
    publish(run / "plan.json", base_plan(preparation) | from_reference(work, preparation, reference))


def verify_prepared(run, work):
    work = work.resolve(strict=True)
    preparation = check_payload(run, work)
    plan = read_record(local(work, run / "plan.json"))
    expected = base_plan(preparation)
    if expected["purpose"] == FORMAL:
        expected.update(from_reference(work, preparation, work / plan["pilot_reference"]))
    if plan != expected:
        raise ValueError("The plan is not the frozen pilot or independently sealed timing plan")
    schedule_for(plan)
    return plan


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    for action in ("freeze", "seal", "verify"):
        command = commands.add_parser(action)
        command.add_argument("--work", type=Path, required=True)
        command.add_argument("--run", type=Path, required=True)
        if action == "freeze":
            command.add_argument("--seed", type=Path, required=True)
            command.add_argument("--helpers", type=Path, required=True)
            command.add_argument("--pilot", action="store_true")
        if action == "seal":
            command.add_argument("--reference", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    if args.action == "freeze":
        record = freeze(args.work, args.run, args.seed, args.helpers, pilot=args.pilot)
        print(f"Frozen {record['purpose']}: {record['label']}")
    elif args.action == "seal":
        seal(args.work, args.run, args.reference)
        print("Sealed the formal timing plan from the independent pilot control.")
    else:
        print(f"Verified {verify_prepared(args.run, args.work)['purpose']}.")
