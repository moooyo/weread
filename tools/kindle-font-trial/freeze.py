"""Prepare the authorized temporary indexed experiment on the Linux worker."""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tarfile
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCHEDULE = ["A-library", "B-library", "A-reader", "B-reader", "A0", "B0"]
for pair in range(1, 7):
    SCHEDULE.extend(f"{lane}{pair}" for lane in ("AB" if pair % 2 else "BA"))


def launcher(source):
    command = '"$APP_DIR/weread" --kindle --device "$PROFILE" --data-dir "$DATA_DIR" >"$DATA_DIR/logs/application-bootstrap.log"'
    if source.count(command) != 1:
        raise ValueError("Research arguments must affect exactly the monitored application, not its probes")
    return source.replace(command, command.replace(' >', ' --demo --offline --screen "$WEREAD_TRIAL_ROUTE"'
        ' --exit-after-ms "$WEREAD_TRIAL_DWELL" ${WEREAD_TRIAL_CAPTURE:+--capture} ${WEREAD_TRIAL_CAPTURE:+"$WEREAD_TRIAL_CAPTURE"} >'))


def device_body():
    peers = (HERE / "peer-files.txt").read_text().strip()
    paths = [line.split()[1] for line in peers.splitlines() if re.fullmatch(r"[a-f0-9]{64} /[A-Za-z0-9_./-]+", line)]
    if len(set(paths)) != 12 or len(peers.splitlines()) != 12:
        raise ValueError("Incomplete owner-approved peer baseline")
    body = (HERE / "device.sh").read_text()
    if body.count("@PEER_FILES@") != 1:
        raise ValueError("Invalid device protection template")
    return body.replace("@PEER_FILES@", peers)


def prepare(work, run, helpers):
    run.mkdir(mode=0o700)
    label = secrets.token_hex(8)
    (run / "plan.json").write_text(json.dumps({"label": label, "schedule": SCHEDULE,
        "clock": "CLOCK_MONOTONIC", "stabilize_seconds": 8, "timed_dwell_ms": 6000}, indent=2) + "\n")
    (run / "device.sh").write_text(device_body())
    with tempfile.TemporaryDirectory(dir=run) as temporary:
        scratch = Path(temporary)
        document = (HERE / "document.sh.in").read_text().replace("@LABEL@", label)
        icon = (ROOT / "packaging/kindle/document-launcher.sh").read_text().splitlines()[2]
        if not icon.startswith("# Icon:"):
            raise ValueError("The reviewed entry no longer provides its public icon")
        (scratch / "document").write_text(document.replace("set -eu\n", icon + "\nset -eu\n", 1))
        (scratch / "launch.sh").write_text(launcher((ROOT / "packaging/kindle/launch.sh").read_text()))
        subprocess.run(["sh", "-n", str(scratch / "document")], check=True)
        subprocess.run(["sh", "-n", str(scratch / "launch.sh")], check=True)
        subprocess.run(["sh", "-n", str(run / "device.sh")], check=True)
        subprocess.run(["zig", "cc", "-target", "arm-linux-musleabihf", "-O2", "-static", "-s", "-std=c11",
                        "-Wall", "-Wextra", "-Werror", str(HERE / "measure.c"), "-o", str(scratch / "measure")], check=True)
        changed = subprocess.run(["qemu-arm", "-cpu", "cortex-a8", str(scratch / "measure"), "request", "bad"])
        if changed.returncode != 64:
            raise ValueError("Request helper accepted an invalid document name")
        seed = work / "fixtures/seed.tar.gz"
        with tarfile.open(seed) as archive:
            names = archive.getnames()
            if "demo/library.sqlite" not in names or "library-covers.json" not in names:
                raise ValueError("Expected the public seed")
            for member in archive:
                if (not (member.isfile() or member.isdir()) or Path(member.name).is_absolute()
                        or ".." in Path(member.name).parts or not (member.name == "library-covers.json"
                        or Path(member.name).parts[0] == "demo")):
                    raise ValueError("Unexpected seed path")
        with tarfile.open(run / "inputs.tgz", "x:gz", compresslevel=1) as archive:
            def put(path, name, mode):
                if not path.is_file() or path.is_symlink():
                    raise ValueError(f"Missing regular file for {name}")
                member = tarfile.TarInfo(name)
                member.size, member.mode = path.stat().st_size, mode
                with path.open("rb") as stream:
                    archive.addfile(member, stream)

            put(scratch / "document", "document", 0o755)
            put(scratch / "measure", "measure", 0o755)
            put(seed, "seed.tar.gz", 0o600)
            for lane, build in (("A", "baseline"), ("B", "candidate")):
                folder = work / "arm-pair" / build
                record = json.loads((folder / "result.json").read_text())
                if (folder / "weread").stat().st_size != record["elf"]["bytes"] or len(record["generated_receipts"]) != (0 if lane == "A" else 7):
                    raise ValueError("Wrong retained font arm")
                put(scratch / "launch.sh", f"{lane}/launch.sh", 0o755)
                put(folder / "weread", f"{lane}/app/weread", 0o755)
                for helper in ("weread-frame-guard", "weread-native-query"):
                    put(helpers / helper, f"{lane}/app/{helper}", 0o755)
        shutil.copy2(scratch / "document", run / "document.reference")
    print(f"Frozen {len(SCHEDULE)} launches; payload {os.path.getsize(run / 'inputs.tgz')} bytes.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("work", "run", "helpers"):
        parser.add_argument(arg, type=Path)
    args = parser.parse_args()
    prepare(args.work.resolve(strict=True), args.run.resolve(), args.helpers.resolve(strict=True))
