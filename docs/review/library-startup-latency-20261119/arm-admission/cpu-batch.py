"""Run a guarded font probe or read-only entry audit in an agreed device window."""

import argparse
from datetime import datetime, timezone
import getpass
import gzip
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import signal
import statistics
import subprocess
import sys
import tempfile


LEASE = Path("/tmp/codex-kindle-physical-test.lock")
IDLE = "scribe_2022 fw_5.19.5 native_idle stage_absent"
ROUND = re.compile(r"round=([0-6]) full_ns=([1-9][0-9]{0,11}) receipt_ns=([1-9][0-9]{0,11})")
ENTRY = [r"document=(?:regular|absent|other)", r"reference=(?:equal|different|not_compared)",
         r"supervisor=(?:regular|absent|other)", r"application=(?:regular|absent|other)",
         r"selected_rows=(?:[0-9]{1,6}|not_sampled)"]


def samples(lines):
    if len(lines) != 8 or lines[-1] != "stage_removed":
        raise ValueError("Incomplete batch")
    rows = []
    for index, line in enumerate(lines[:-1]):
        match = ROUND.fullmatch(line)
        if not match or int(match[1]) != index:
            raise ValueError("Unexpected round")
        rows.append({"round": index, "full_first": index == 0 or index % 2 == 1,
                     "full_ns": int(match[2]), "receipt_ns": int(match[3])})
    measured = rows[1:]
    return {"warmup": rows[0], "measured": measured,
            "median_full_ns": statistics.median(row["full_ns"] for row in measured),
            "median_receipt_ns": statistics.median(row["receipt_ns"] for row in measured),
            "median_paired_saving_ns": statistics.median(row["full_ns"] - row["receipt_ns"] for row in measured)}


def execute(call, release, report, *, recover=False):
    may_have_staged = recover
    ready_to_release = False
    try:
        if not recover:
            if call("check") != (0, [IDLE]):
                return
            may_have_staged = True
            status, lines = call("sample")
            if status == 0:
                report["timing"] = samples(lines)
    finally:
        try:
            if may_have_staged:
                if call("close") == (0, ["stage_absent"]):
                    ready_to_release = call("check") == (0, [IDLE])
            else:
                ready_to_release = True
        finally:
            if ready_to_release:
                report["released_at_utc"] = release()
                report["passed"] = "timing" in report and not recover
            else:
                report["recovery_required"] = True


def audit_entry(call, release, report):
    passed = False
    try:
        if call("check") != (0, [IDLE]):
            return
        status, lines = call("entry")
        idle = call("check") == (0, [IDLE])
        passed = status == 0 and idle and len(lines) == len(ENTRY) and all(
            re.fullmatch(pattern, line) for pattern, line in zip(ENTRY, lines))
    finally:
        report["released_at_utc"] = release()
        report["passed"] = passed


def load_lease(path):
    spec = importlib.util.spec_from_file_location("device_lease", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    module.PROJECT = "WeRead"
    return module


def ssh_command(config, known, alias):
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]*", alias):
        raise ValueError("Expected a configured SSH alias")
    for path in (config, known):
        if not path.is_file() or path.is_symlink() or path.stat().st_mode & 0o022:
            raise ValueError("Missing or writable SSH trust input")
    flags = ["-F", str(config), "-T", "-oStrictHostKeyChecking=yes", "-oUpdateHostKeys=no",
             f"-oUserKnownHostsFile={known}", "-oGlobalKnownHostsFile=/dev/null", "-oForwardAgent=no",
             "-oClearAllForwardings=yes", "-oControlPath=none", "-oControlMaster=no",
             "-oNumberOfPasswordPrompts=1", "-oConnectionAttempts=1", "-oConnectTimeout=6",
             "-oServerAliveInterval=4", "-oServerAliveCountMax=2"]
    values = subprocess.check_output(["ssh", "-G", *flags, alias], text=True, timeout=5)
    resolved = dict(line.split(" ", 1) for line in values.splitlines() if " " in line)
    name = resolved["hostname"] if resolved["port"] == "22" else f"[{resolved['hostname']}]:{resolved['port']}"
    subprocess.run(["ssh-keygen", "-F", resolved.get("hostkeyalias", name), "-f", str(known)],
                   stdout=subprocess.DEVNULL, check=True, timeout=5)
    return ["ssh", *flags, alias]


def recorded_call(command, payload, mode, env, report):
    try:
        result = subprocess.run(command, input=payload, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=env, timeout=180 if mode == "sample" else 20)
        status = result.returncode
        raw = result.stdout
    except subprocess.TimeoutExpired as error:
        status, raw = 124, error.stdout or b""
    lines = []
    for line in raw.decode("ascii", "replace").splitlines()[:16]:
        if (line in (IDLE, "stage_absent", "stage_removed") or ROUND.fullmatch(line)
                or re.fullmatch(r"refused [a-z_]{1,32}", line) or any(re.fullmatch(pattern, line) for pattern in ENTRY)):
            lines.append(line)
        else:
            lines.append("unexpected_output")
    report["operations"].append({"operation": mode, "status": status, "lines": lines})
    return status, lines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("runs", "lease-tool", "ssh-config", "known-hosts"):
        parser.add_argument("--" + name, type=Path, required=True)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--probe", type=Path)
    action.add_argument("--inspect-entry", type=Path, help="known public document bytes, compared without execution")
    parser.add_argument("--alias", required=True)
    parser.add_argument("--owner", required=True)
    parser.add_argument("--recover", type=Path, help="existing private run directory, cleanup only")
    args = parser.parse_args()
    if sys.platform != "linux" or not Path("/opt/kindle-toolchain/ubuntu-run").is_file():
        parser.error("Use the designated worker host; its physical lease is outside the build userspace")
    os.umask(0o077)
    ssh = ssh_command(args.ssh_config, args.known_hosts, args.alias)
    lease = load_lease(args.lease_tool)
    body = Path(__file__).with_name("cpu-stage.sh").read_text()
    size = 1
    entry_body = ""
    if args.inspect_entry:
        if args.recover or not 0 < args.inspect_entry.stat().st_size <= 256 * 1024:
            parser.error("A read-only audit takes one bounded reference, not a recovery directory")
        entry_body = Path(__file__).with_name("entry-audit.sh").read_text()
        payload = args.inspect_entry.read_bytes()
    else:
        probe = args.probe.resolve(strict=True)
        size = probe.stat().st_size
        if not 0 < size <= 64 * 1024**2:
            parser.error("Unexpected probe size")
        payload = b"" if args.recover else gzip.compress(probe.read_bytes(), compresslevel=1, mtime=0)
    runs = args.runs.resolve(strict=True)
    if args.recover:
        batch = args.recover.resolve(strict=True)
        if batch.parent != runs:
            parser.error("Use this experiment's private run directory")
        label = (batch / "label").read_text().strip()
        if not re.fullmatch(r"[0-9a-f]{16}", label):
            parser.error("Invalid run label")
    else:
        label = secrets.token_hex(8)
        batch = Path(tempfile.mkdtemp(dir=runs, prefix="font-cpu-"))
        (batch / "label").write_text(label + "\n")
    scope = "Read-only indexed document audit, no launch" if args.inspect_entry else "Guarded CPU-only font admission, no UI, indexed entry or display timing"
    report = {"scope": scope,
              "started_at_utc": datetime.now(timezone.utc).isoformat(), "operations": [], "passed": False}
    request = lease.LeaseRequest(LEASE, batch / "token", args.owner, "weread-font-" + label)
    output = batch / ("recovery.json" if args.recover else "result.json")
    with tempfile.TemporaryDirectory(dir=batch) as temp:
        askpass = Path(temp) / "askpass"
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        if sys.stdin.isatty():
            password = getpass.getpass("Device password (not saved): ")
        else:
            password = sys.stdin.readline().rstrip("\r\n")
        if not password:
            parser.error("No private credential supplied; no lease acquired")
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force",
                   DISPLAY="unused", WEREAD_TRANSIENT_PASSWORD=password)
        print(f"Private result: {output}", flush=True)

        def call(mode):
            if not lease.status(request)["ownership_verified"]:
                raise RuntimeError("Physical lease ownership changed")
            remote = shlex.join(["sh", "-c", body, "weread-font-cpu", mode, label, str(size)])
            if mode == "entry":
                remote = shlex.join(["sh", "-c", entry_body, "weread-entry-audit"])
            return recorded_call([*ssh, remote], payload if mode in ("sample", "entry") else b"", mode, env, report)

        def release():
            receipt = lease.release(request, native_restored_and_connections_closed=True)
            if not receipt["owner_and_token_checked"] or not receipt["only_owned_owner_json_removed_then_rmdir"]:
                raise RuntimeError("Lease release not confirmed")
            request.token_file.unlink()
            return receipt["released_at_utc"]

        def interrupt(*_args):
            raise KeyboardInterrupt

        signal.signal(signal.SIGTERM, interrupt)
        try:
            if not args.recover:
                lease.acquire(request)
            if args.inspect_entry:
                audit_entry(call, release, report)
            else:
                execute(call, release, report, recover=bool(args.recover))
        except (Exception, KeyboardInterrupt) as error:
            report["error"] = getattr(error, "code", type(error).__name__)
        finally:
            output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)
    return 0 if report["passed"] or (args.recover and "released_at_utc" in report) else 1


if __name__ == "__main__":
    raise SystemExit(main())
