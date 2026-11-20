"""Stage the specifically qualified candidate without copying any private data."""

import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tarfile
import tempfile

from analyze import analyze
from freeze import HERE, ROOT, SCHEDULE, device_body


def release_gate(operations, analysis):
    observed = [item["case"] for item in operations["operations"] if item["mode"] == "case"]
    if not operations.get("passed") or not operations.get("released") or observed != SCHEDULE:
        raise ValueError("The fixed device schedule and closeout are required")
    if any(item["status"] != 0 for item in operations["operations"]):
        raise ValueError("An operation failed in the proposed promotion batch")
    values = analysis["request_to_send_ms"]
    if values["improved_pairs"] < 5 or values["median_paired_saving"] <= 0 or analysis["font_ms"]["median_paired_saving"] <= 0:
        raise ValueError("Insufficient measured benefit")


def prepare(work, trial, attempt, plan, old_document):
    operations = json.loads((attempt / "operations.json").read_text())
    analysis = analyze(attempt / "results.tgz")
    release_gate(operations, analysis)
    for lane, count in (("default", 56), ("preflight", 58)):
        if f"test result: ok. {count} passed; 0 failed;" not in (trial / f"main-{lane}.log").read_text():
            raise ValueError("Missing host font regression evidence")
    script = (ROOT / "scripts/build-kindle.sh").read_text()
    if '--features kindle,bundled-font-preflight --target "$TARGET"' not in script:
        raise ValueError("The production package must select the measured font feature")
    plan.mkdir(mode=0o700)
    (plan / "plan.json").write_text(json.dumps({"purpose": "installed-retest", "label": secrets.token_hex(8),
        "schedule": ["normal"], "source": "qualified retained B", "private_data": "untouched by installer"}, indent=2) + "\n")
    prefix, boundary, _ = device_body().partition('case "$mode" in\n')
    if not boundary:
        raise ValueError("Missing common device admission functions")
    (plan / "device.sh").write_text(prefix + (HERE / "rollout.sh").read_text())
    subprocess.run(["sh", "-n", str(plan / "device.sh")], check=True)
    with tempfile.TemporaryDirectory(dir=plan) as temporary:
        stage = Path(temporary)
        abi = "armv7-unknown-linux-musleabihf"
        release = stage / "target" / abi / "release"
        release.mkdir(parents=True)
        os.link(work / "arm-pair/candidate/weread", release / "weread")
        for name in ("weread-frame-guard", "weread-native-query"):
            os.link(trial / "helpers" / name, release / name)
        package = stage / "package"
        anchor = 'mkdir -p "$STAGED_APP/config"'
        if script.count(anchor) != 1:
            raise ValueError("Unrecognized packaging contract")
        tail = script[script.index(anchor):]
        subprocess.run(["sh", "-eu"], input=tail, text=True, check=True, env=dict(os.environ,
            REPO_ROOT=str(ROOT), CARGO_TARGET_DIR=str(stage / "target"), TARGET=abi,
            STAGING_ROOT=str(package), STAGED_APP=str(package / "weread/app")))
        if any(path.is_symlink() for path in package.rglob("*")) or (package / "weread/data").exists():
            raise ValueError("Unexpected package contents")
        with tarfile.open(plan / "inputs.tgz", "x:gz", compresslevel=1, dereference=True) as archive:
            archive.add(package, arcname="package")
            for name, file in (("old-document", old_document), ("document", package / "documents/weread.sh"),
                               ("launch.sh", package / "weread/launch.sh")):
                archive.add(file, arcname="ref/" + name, recursive=False)
    print("Gated release staged with the measured candidate and production packaging files.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("work", "trial", "attempt", "plan", "old-document"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    prepare(args.work, args.trial, args.attempt, args.plan, args.old_document)
