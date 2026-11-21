"""Lease the designated worker's physical Kindle only for an already prepared batch."""

import argparse
from datetime import datetime, timezone
import getpass
import json
import os
from pathlib import Path
import re
import runpy
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from freeze import ROOT, SCHEDULE


UTIL = runpy.run_path(str(ROOT / "docs/review/library-startup-latency-20261119/arm-admission/cpu-batch.py"))
LEASE = UTIL["LEASE"]


def stamp():
    return datetime.now(timezone.utc).isoformat()


def transport(script, mode, label, name):
    header = script.rstrip("\n").encode("ascii")
    if not 0 < len(header) <= 32768:
        raise ValueError("Invalid trial program size")
    command = (f'export LC_ALL=C; program=$(dd bs=1 count={len(header)} 2>/dev/null); '
               f'[ "${{#program}}" = {len(header)} ] || exit 64; exec sh -c "$program" ' +
               shlex.join(["weread-indexed-trial", mode, label, name]))
    return command, header


def not_operated(attempt):
    try:
        report = json.loads((attempt / "operations.json").read_text())
        if (report.get("error") in ("operation_busy", "lease_exists") and report["operations"] == []
                and not (attempt / "private-token").exists()):
            return True
        return (bool(report.get("released")) and not report["passed"] and
                [(item["mode"], item["status"] != 0) for item in report["operations"]] == [("admit", True)])
    except (OSError, ValueError, KeyError):
        return False


def is_installed_retest(config):
    if "purpose" not in config:
        return False
    if config["purpose"] == "installed-retest":
        return True
    raise ValueError("Unsupported plan purpose for the font trial runner")


def close_transaction(call, archive):
    restored = call("restore") or call("restore")
    try:
        collected = call("collect", output=archive)
    except (Exception, KeyboardInterrupt):
        collected = False
    finally:
        closed = restored and call("close")
    return closed, collected


def run(args, *, orientation_schedule=None):
    if sys.platform != "linux" or not Path("/opt/kindle-toolchain/ubuntu-run").is_file():
        raise ValueError("Use the worker host, not the build userspace")
    os.umask(0o077)
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if not plan.is_relative_to(work):
        raise ValueError("Expected this worker's private frozen plan")
    config = json.loads((plan / "plan.json").read_text())
    label = config["label"]
    if orientation_schedule is None:
        rollout = is_installed_retest(config)
        schedule = ["normal"] if rollout else SCHEDULE
    else:
        rollout = False
        schedule = orientation_schedule(config)
    if not re.fullmatch(r"[a-f0-9]{16}", label) or config["schedule"] != schedule:
        raise ValueError("Invalid frozen schedule")
    script = (plan / "device.sh").read_text()
    transport(script, "admit", label, "none")
    if not args.recover and not args.admit_only:
        payload = plan / "inputs.tgz"
        if not payload.is_file() or payload.is_symlink() or payload.stat().st_size == 0:
            raise ValueError("Missing frozen payload")
    trust = Path("/root/.config/kindle-physical")
    ssh = UTIL["ssh_command"](trust / "ssh_config", trust / "known_hosts", "kindle-physical")
    lease = UTIL["load_lease"](args.lease_tool.resolve(strict=True))
    if not args.recover and not args.wait and LEASE.exists():
        raise ValueError("Device lease is occupied; no Kindle connection or reservation")
    if args.recover:
        attempt = args.recover.resolve(strict=True)
        if attempt.parent != plan or not (attempt / "private-token").is_file():
            raise ValueError("No matching interrupted attempt")
    elif not args.admit_only and any(not not_operated(other) for other in plan.glob("batch-*")):
        raise ValueError("Use a new label after a write or an uncertain attempt")
    secret = getpass.getpass("Device password (not saved): ") if sys.stdin.isatty() else sys.stdin.readline().rstrip("\r\n")
    if not secret:
        raise ValueError("Missing private authentication")
    if not args.recover:
        attempt = Path(tempfile.mkdtemp(dir=plan, prefix="admission-" if args.admit_only else "batch-"))
    kind = "weread-orientation-" if orientation_schedule else "weread-indexed-"
    request = lease.LeaseRequest(LEASE, attempt / "private-token", args.owner, kind + label)
    report = {"label": label, "started": stamp(), "operations": [], "passed": False}
    report_path = attempt / ("recovery.json" if args.recover else "operations.json")

    def save():
        pending = report_path.with_suffix(".pending")
        pending.write_text(json.dumps(report, indent=2) + "\n")
        pending.replace(report_path)

    def interrupt(*_args):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupt)
    with tempfile.TemporaryDirectory(dir=attempt) as local:
        askpass = Path(local) / "askpass"
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", DISPLAY="unused",
                   WEREAD_TRANSIENT_PASSWORD=secret)

        def call(mode, name="none", output=None):
            if not lease.status(request)["ownership_verified"]:
                raise RuntimeError("Physical lease ownership changed")
            remote, header = transport(script, mode, label, name)
            command = [*ssh, remote]
            with tempfile.TemporaryFile(dir=attempt) as data:
                data.write(header)
                if mode == "stage":
                    with (plan / "inputs.tgz").open("rb") as source:
                        shutil.copyfileobj(source, data, length=1024**2)
                data.seek(0)
                sink = output.open("wb") if output else None
                try:
                    result = subprocess.run(command, env=env, stdin=data, stdout=sink or subprocess.PIPE, stderr=subprocess.PIPE,
                                            timeout={"stage": 450, "case": 125, "collect": 150, "restore": 180, "close": 60}.get(mode, 45))
                    code = result.returncode
                    stdout = "" if sink else result.stdout.decode("ascii", "replace").strip()
                    errors = result.stderr.decode("ascii", "replace")
                except subprocess.TimeoutExpired:
                    code, stdout, errors = 124, "", "transport timeout"
                finally:
                    if sink:
                        sink.close()
            expected = {"admit": "admitted", "stage": "indexed", "case": f"completed {name}", "restore": "restored", "close": "closed"}
            if code == 0 and mode in expected and stdout != expected[mode]:
                code, errors = 65, "refused acknowledgement"
            message = next((line for line in errors.splitlines() if re.fullmatch(r"refused [a-z_]+", line)), "")
            report["operations"].append({"at": stamp(), "mode": mode, "case": name, "status": code,
                                         "diagnostic": message or ("transport_or_command_failed" if code else "")})
            save()
            print(f"{mode} {name} {code} {message}", flush=True)
            if code:
                (attempt / f"private-{mode}-{name}.stderr").write_text(errors[:4096])
            return code == 0

        def analyze_on_worker(tool, *arguments):
            subprocess.run(["/opt/kindle-toolchain/ubuntu-run", str(work), "python3", "-B",
                            "/work/src/tools/kindle-font-trial/" + tool, *arguments], check=True, timeout=70)

        def picture_gate():
            archive = attempt / "qualifications.tgz"
            if not call("collect", output=archive):
                raise ValueError("Qualification collection failed")
            rel = "/work/" + str(attempt.relative_to(work))
            if orientation_schedule:
                analyze_on_worker("orientation_gate.py", "pictures", rel + "/../plan.json", rel + "/qualifications.tgz",
                                  rel + "/qualification.json")
            else:
                analyze_on_worker("analyze.py", rel + "/qualifications.tgz", rel + "/qualification.json", "--qualifications")

        attempted = bool(args.recover)
        acquired = False
        completed = False
        print(f"Private attempt: {attempt}", flush=True)
        try:
            if not args.recover:
                if args.wait:
                    end = time.monotonic() + args.wait
                    while LEASE.exists() and time.monotonic() < end:
                        time.sleep(3)
                lease.acquire(request)
            acquired = True
            if not args.recover:
                if not call("admit"):
                    raise ValueError("Read-only admission refused")
                if not args.admit_only:
                    attempted = True
                    if not call("stage"):
                        raise ValueError("Temporary entry was not indexed")
                    for index, name in enumerate(schedule):
                        if not call("case", name):
                            raise ValueError("Case failed")
                        if not rollout and index == 3:
                            picture_gate()
                completed = True
        except (Exception, KeyboardInterrupt) as error:
            report["error"] = getattr(error, "code", type(error).__name__)
            if not acquired:
                if request.token_file.exists():
                    report["recovery_required"] = True
                save()
            print(str(error) if isinstance(error, ValueError) else report["error"], flush=True)
        finally:
            if acquired:
                closed = not attempted
                try:
                    if attempted:
                        archive = "recovery-results.tgz" if args.recover else "results.tgz"
                        closed, collected = close_transaction(call, attempt / archive)
                        completed = completed and collected
                finally:
                    if closed:
                        receipt = lease.release(request, native_restored_and_connections_closed=True)
                        if not receipt["owner_and_token_checked"] or not receipt["only_owned_owner_json_removed_then_rmdir"]:
                            raise RuntimeError("Release not confirmed")
                        request.token_file.unlink()
                        report["released"] = receipt["released_at_utc"]
                        print("Released exact physical lease.", flush=True)
                    else:
                        report["recovery_required"] = True
                    report["passed"] = completed and closed
                    save()
    if report["passed"] and not args.admit_only and not rollout and not args.recover:
        rel = "/work/" + str(attempt.relative_to(work))
        if orientation_schedule:
            inputs = [rel + "/../plan.json", rel + "/operations.json", rel + "/results.tgz"]
            if config["purpose"] == "startup-orientation-pilot":
                analyze_on_worker("orientation_gate.py", "pilot", *inputs, rel + "/pilot-reference.json")
            else:
                analyze_on_worker("analyze_orientation.py", *inputs, rel + "/orientation.json")
        else:
            analyze_on_worker("analyze.py", rel + "/results.tgz", rel + "/analysis.json")
    return 0 if report["passed"] or (args.recover and "released" in report) else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("work", "plan", "lease-tool"):
        parser.add_argument("--" + arg, type=Path, required=True)
    parser.add_argument("--owner", required=True)
    parser.add_argument("--wait", type=int, default=0)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--recover", type=Path)
    group.add_argument("--admit-only", action="store_true")
    raise SystemExit(run(parser.parse_args()))
