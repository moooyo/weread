"""Run or close one preflighted public Reader visit on the designated worker."""

import argparse
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import re
import runpy
import secrets
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile

from make_payload import EXECUTABLES, MEMBERS, PROFILE, ROOT, launcher
from seed import CHAPTERS, create_seed


HERE = Path(__file__).resolve().parent
COORD = Path("/var/tmp/kindle-build-coordination")
LIMIT = 200 * 1024**2
PUBLIC_PROTOCOL = ROOT / "docs/review/library-startup-latency-20261119/arm-admission/cpu-batch.py"


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def now():
    return datetime.now(timezone.utc).isoformat()


def payload_bytes(path):
    require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= LIMIT, "invalid_payload")
    content = path.read_bytes()
    with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as archive:
        members = archive.getmembers()
        require(len(members) == len(MEMBERS) and {member.name for member in members} == MEMBERS, "payload_members")
        require(all(member.isfile() and 0 < member.size <= 160*1024**2 and member.uid == member.gid == 0
                    and member.mode == (0o755 if member.name in EXECUTABLES else 0o600) for member in members), "payload_metadata")
        expanded = sum(member.size for member in members)
        require(expanded <= LIMIT, "payload_expanded")
        for name, expected in {**PROFILE, "launch.sh": launcher(), "launch-gate.sh": (HERE/"device.sh").read_bytes()}.items():
            require(archive.getmember(name).size == len(expected) and archive.extractfile(name).read() == expected, "payload_configuration")
        for name in EXECUTABLES - {"launch.sh"}:
            header = archive.extractfile(name).read(20)
            require(header[:7] == b"\x7fELF\x01\x01\x01" and header[18:20] == b"(\x00", "payload_architecture")
        require(archive.extractfile("data/library.sqlite").read(16) == b"SQLite format 3\x00", "payload_database")
        with tempfile.TemporaryDirectory() as temporary:
            fresh = Path(temporary) / "public"
            create_seed(fresh)
            require(archive.extractfile("data/library.sqlite").read() == (fresh / "library.sqlite").read_bytes(), "payload_not_public_seed")
    return content, expanded


def handoff(path, owner, directory=COORD):
    require(path is not None and not path.is_symlink() and path.resolve().parent == directory.resolve()
            and path.is_file() and 0 < path.stat().st_size < 16384, "missing_handoff")
    data = json.loads(path.read_bytes())
    require(data.get("project") == "WeRead" and data.get("owner") == owner and data.get("purpose") == "reader-turn"
            and data.get("ready") is True and data.get("peer_handoff_confirmed") is True, "unconfirmed_handoff")
    ends = datetime.fromisoformat(data["expires_at"])
    seconds = (ends - datetime.now(timezone.utc)).total_seconds()
    require(0 < seconds <= 3600, "handoff_expired_or_unbounded")
    for peer in ("aidoku", "bilicomics"):
        reference = data.get("peer_notices", {}).get(peer, "")
        require(re.fullmatch(peer + r"-[A-Za-z0-9_.-]{1,160}", reference), "invalid_peer_reference")
        notice = directory / reference
        require(not notice.is_symlink() and notice.is_file() and notice.stat().st_mtime_ns <= path.stat().st_mtime_ns, "missing_peer_reference")
        require(not any(item.is_file() and item.stat().st_mtime_ns > path.stat().st_mtime_ns for item in directory.glob(peer + "-*")), "peer_notice_changed")


def run_visit(args):
    os.umask(0o077)
    protocol = runpy.run_path(str(PUBLIC_PROTOCOL))
    script = (HERE / "device.sh").read_bytes()
    require(script.isascii() and 0 < len(script) <= 32768, "unsupported_device_program")
    require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}", args.owner), "invalid_owner")
    trust = Path("/root/.config/kindle-physical")
    ssh = protocol["ssh_command"](trust / "ssh_config", trust / "known_hosts", "kindle-physical")
    lease = protocol["load_lease"](args.lease_tool.resolve(strict=True))
    runs = args.runs.resolve(strict=True)
    require(runs.is_dir(), "invalid_runs_directory")
    if not args.recover:
        require(args.lane in ("A", "B") and args.chapters == CHAPTERS and args.payload is not None, "missing_arm")
        content, expanded = payload_bytes(args.payload)
        if args.preflight:
            print(f"preflight_passed arm={args.lane} chapters={args.chapters} bytes={len(content)} expanded={expanded}")
            return 0
        handoff(args.handoff, args.owner)
        require(not protocol["LEASE"].exists(), "lease_busy_no_connection")
        label = secrets.token_hex(8)
        attempt = runs / ("visit-" + label)
        attempt.mkdir(mode=0o700)
        (attempt / "device.sh").write_bytes(script)
        report = dict(label=label, owner=args.owner, lane=args.lane, chapters=args.chapters, mode="native", started=now(),
                      bytes=len(content), expanded=expanded, passed=False, steps=[], released=False,
                      native_restored=False, connection_closed=True, recovery_required=False)
    else:
        attempt = args.recover.resolve(strict=True)
        require(attempt.parent == runs and re.fullmatch(r"visit-[0-9a-f]{16}", attempt.name), "invalid_recovery_directory")
        report = json.loads((attempt / "receipt.json").read_bytes())
        label = report["label"]
        require(attempt.name == "visit-"+label and report["owner"] == args.owner and not report["released"], "invalid_recovery_owner")
        script = (attempt / "device.sh").read_bytes()
        content = b""
    receipt = attempt / "receipt.json"
    def persist():
        temporary = attempt / "receipt.next"
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(receipt)
    persist()
    print("attempt", attempt, flush=True)
    request = lease.LeaseRequest(protocol["LEASE"], attempt / "token", args.owner, "weread-turn-"+label)
    password = sys.stdin.buffer.readline(1024).rstrip(b"\r\n").decode()
    require(password, "missing_private_authentication")
    with tempfile.TemporaryDirectory(dir=attempt) as temp:
        askpass = Path(temp)/"askpass"
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", DISPLAY="unused", WEREAD_TRANSIENT_PASSWORD=password)
        def call(mode, expected=None):
            require(lease.status(request).get("ownership_verified") is True, "ownership_changed")
            header = script.rstrip(b"\n")
            command = (f'export LC_ALL=C; body=$(dd bs=1 count={len(header)} 2>/dev/null); '
                       f'[ "${{#body}}" = {len(header)} ] || exit 64; exec sh -c "$body" ' +
                       shlex.join(["weread-reader-turn", mode, label, str(report["bytes"]), str(report["expanded"]), report["lane"]]))
            report["steps"].append({"mode": mode, "at": now()}); step=report["steps"][-1]
            report["connection_closed"] = False; persist()
            try:
                result = subprocess.run([*ssh, command], input=header+(content if mode=="stage" else b""),
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
                                        timeout={"stage":180, "run":250, "close":115}.get(mode, 30), start_new_session=True)
                step["status"] = result.returncode
                valid = result.returncode == 0 and (expected is None or result.stdout in expected)
                if not valid:
                    reason = re.fullmatch(rb"refused ([a-z_]{1,60})\n", result.stdout)
                    step["reason"] = reason[1].decode() if reason else "transport_or_receipt"
                return result.stdout if valid else None
            finally:
                report["connection_closed"] = True; persist()
        acquired = False; touched = bool(args.recover); successful = False
        try:
            if args.recover:
                require(lease.status(request).get("ownership_verified") is True, "wrong_recovery_token")
            else:
                handoff(args.handoff, args.owner)
                acquired_result = lease.acquire(request)
                acquired = True
                require(acquired_result.get("ok") is True and acquired_result.get("state") == "held"
                        and lease.status(request).get("ownership_verified") is True, "lease_not_acquired")
            acquired = True
            if not args.recover:
                require(call("admit", [b"admitted\n"]) is not None, "device_admission")
                touched = True
                require(call("stage", [b"staged\n"]) is not None, "stage")
                require(call("run", [b"exited\n"]) is not None, "run")
                data = call("collect")
                require(data and len(data) <= 131072 and len(data.splitlines()) == 24, "result_count")
                fields = {"sequence", "warmup", "chapter", "source_page", "target_page", "source_anchor", "target_anchor",
                          "source_line_start", "target_line_start", "press_ns", "release_ns", "model_ns", "draw_begin_ns",
                          "draw_end_ns", "written_ns", "sent_ns", "marker", "full", "rect", "waveform", "wait_begin_ns",
                          "wait_end_ns", "wait_marker", "wait_status"}
                for line in data.splitlines():
                    row = json.loads(line)
                    require(isinstance(row, dict) and set(row) == fields and
                            row["wait_status"] in ("acknowledged", "unavailable", "not-witnessed", "timed-out", "worker-unavailable") and
                            isinstance(row["rect"], list) and len(row["rect"]) == 4 and all(type(value) is int for value in row["rect"]) and
                            all(value is None or type(value) in (bool, int) for key, value in row.items() if key not in ("rect", "wait_status")), "result_schema")
                (attempt/"turn-result.jsonl").write_bytes(data)
                (attempt/"turn-complete").write_text("24\n")
                successful = True
        except (Exception, KeyboardInterrupt) as error:
            report["failure"] = getattr(error,"code",str(error) if isinstance(error,ValueError) and re.fullmatch(r"[a-z_]+",str(error)) else type(error).__name__)
            if not acquired and (request.token_file.exists() or any(getattr(error, "details", {}).get(field) for field in ("lease_directory_created", "private_token_created", "owner_record_created"))):
                report["recovery_required"] = True
        finally:
            if acquired:
                try:
                    safe = not touched or (call("close", [b"closed\n", b"absent\n"]) is not None and call("absent", [b"returned\n"]) is not None)
                    if safe and report["connection_closed"]:
                        report["native_restored"] = touched
                        released = lease.release(request, native_restored_and_connections_closed=True)
                        require(released.get("owner_and_token_checked") is True and released.get("only_owned_owner_json_removed_then_rmdir") is True, "unconfirmed_release")
                        report.update(released=True,released_at=released["released_at_utc"],recovery_required=False)
                        request.token_file.unlink()
                        report["passed"] = successful and not args.recover
                    else:
                        report["recovery_required"] = True
                finally:
                    persist()
            else:
                persist()
    print("passed", report["passed"], "returned", report["released"], flush=True)
    return 0 if report["passed"] or args.recover and report["released"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["native"], default="native")
    for name in ("runs", "lease-tool"):
        parser.add_argument("--"+name,type=Path,required=True)
    for name in ("payload", "handoff", "recover"):
        parser.add_argument("--"+name,type=Path)
    parser.add_argument("--owner", required=True)
    parser.add_argument("--lane", choices=("A","B"))
    parser.add_argument("--chapters",type=int,choices=(CHAPTERS,))
    parser.add_argument("--preflight",action="store_true")
    args=parser.parse_args()
    def interrupted(*unused):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM,interrupted)
    try:
        return run_visit(args)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print("reader visit refused:", str(error) if isinstance(error, ValueError) else type(error).__name__, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
