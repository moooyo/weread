"""Measure the existing offscreen ARM Reader fixture on an exclusively leased Scribe."""

import argparse
import gzip
import json
import os
from pathlib import Path
import re
import runpy
import secrets
import shlex
import signal
import struct
import subprocess
import sys
import tempfile

from visit import HERE, PUBLIC_PROTOCOL, handoff, now, require


TEST = "app::reading::render_tests::measure_ordinary_pointer_turns"
ARGS = ["--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"]
TIMES = ("press_ns", "release_ns", "draw_ns", "release_to_draw_ns")
COUNTS = ("press_model_publications", "release_model_publications", "model_publications", "catalog_projections", "weight_builds", "weight_uid_inserts", "weight_chapter_lookups")
FLAG = b"READER_TURN_JSON "


def observations(output, lane):
    values = []
    require(0 < len(output) <= 262144, "oversize_measurement")
    for line in output.splitlines():
        if FLAG not in line:
            continue
        entry = json.loads(line.partition(FLAG)[2]); sample_set = entry["samples"]; count = entry["chapters"]
        require(set(entry) == {"samples", "chapters", "warmups", "p50_ns", "p95_ns"} and count in (24,1863) and entry["warmups"] == 3 and len(sample_set)==24, "sample_count")
        expected = (1,2,3,7,4,4*count,4*count) if lane == "A" else (0,1,1,2,2,2*count,2*count)
        for sample in sample_set:
            require(set(sample) == set(TIMES+COUNTS+("page","anchor")) and all(type(n) is int and 0<=n<2**63 for n in sample.values()), "sample_fields")
            require(tuple(sample[key] for key in COUNTS) == expected and sample["page"] == 3 and sample["anchor"] > 0, "wrong_lane_or_page")
            require(sample["release_ns"] > 0 and sample["draw_ns"]>0 and sample["release_to_draw_ns"] == sample["release_ns"]+sample["draw_ns"], "bad_timing")
        require(len({sample["anchor"] for sample in sample_set}) == 1, "changing_anchor")
        for percent in (50,95):
            expected_quantiles = {key: sorted(sample[key] for sample in sample_set)[(24*percent+99)//100-1] for key in TIMES}
            require(entry[f"p{percent}_ns"] == expected_quantiles, "quantile_mismatch")
        values.append(entry)
    require([entry["chapters"] for entry in values] == [24,1863], "missing_catalog_sizes")
    return values


def static_elf(path):
    require(path.is_file() and not path.is_symlink() and 52 <= path.stat().st_size <= 192*1024**2, "invalid_executable")
    content = path.read_bytes()
    require(content[:7] == b"\x7fELF\x01\x01\x01" and content[18:20] == b"(\0", "not_arm32")
    offset, = struct.unpack_from("<I",content,28)
    size, count = struct.unpack_from("<HH",content,42)
    require(size >= 32 and 0<count<256 and offset+size*count<=len(content), "program_headers")
    require(all(struct.unpack_from("<I",content,offset+index*size)[0] != 3 for index in range(count)), "not_static")
    return content


def execute(args):
    os.umask(0o077)
    protocol = runpy.run_path(str(PUBLIC_PROTOCOL)); trusted=Path("/root/.config/kindle-physical")
    ssh=protocol["ssh_command"](trusted/"ssh_config",trusted/"known_hosts","kindle-physical")
    lease=protocol["load_lease"](args.lease_tool.resolve(strict=True))
    runs=args.runs.resolve(strict=True); require(runs.is_dir() and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}",args.owner),"bad_owner_or_runs")
    if args.recover:
        attempt=args.recover.resolve(strict=True)
        require(attempt.parent==runs and re.fullmatch(r"cpu-[0-9a-f]{16}",attempt.name),"bad_recovery")
        report=json.loads((attempt/"receipt.json").read_bytes()); label=report["label"]
        require(attempt.name=="cpu-"+label and report["owner"]==args.owner and not report["released"],"bad_recovery_owner")
        script=(attempt/"device.sh").read_bytes(); payload=b""
    else:
        require(args.lane in ("A","B") and args.app is not None,"missing_arm")
        script=(HERE/"software_device.sh").read_bytes(); content=static_elf(args.app); size=len(content)
        payload=gzip.compress(content,compresslevel=1,mtime=0); del content
        if args.preflight:
            print(f"cpu_preflight arm={args.lane} bytes={size} compressed={len(payload)}"); return 0
        handoff(args.handoff,args.owner); require(not protocol["LEASE"].exists(),"lease_busy_no_connection")
        label=secrets.token_hex(8); attempt=runs/("cpu-"+label); attempt.mkdir(mode=0o700)
        (attempt/"device.sh").write_bytes(script)
        report=dict(label=label,owner=args.owner,lane=args.lane,mode="offscreen",clock="device Instant/monotonic",started=now(),
                    bytes=size,steps=[],passed=False,released=False,connection_closed=True,stage_removed=False,recovery_required=False)
    def persist():
        pending=attempt/"receipt.next"; pending.write_text(json.dumps(report,indent=2)+"\n"); pending.replace(attempt/"receipt.json")
    persist(); print("attempt",attempt,flush=True)
    require(script.isascii() and 0<len(script)<32768,"bad_script")
    password=sys.stdin.buffer.readline(1024).rstrip(b"\r\n").decode(); require(password,"missing_authentication")
    request=lease.LeaseRequest(protocol["LEASE"],attempt/"token",args.owner,"weread-turn-cpu-"+label)
    with tempfile.TemporaryDirectory(dir=attempt) as temporary:
        helper=Path(temporary)/"askpass"; helper.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n'); helper.chmod(0o700)
        environment=dict(os.environ,SSH_ASKPASS=str(helper),SSH_ASKPASS_REQUIRE="force",DISPLAY="unused",WEREAD_TRANSIENT_PASSWORD=password)
        def call(mode):
            require(lease.status(request).get("ownership_verified") is True,"ownership_changed")
            header=script.rstrip(b"\n")
            command=(f'export LC_ALL=C; body=$(dd bs=1 count={len(header)} 2>/dev/null); [ "${{#body}}" = {len(header)} ] || exit 64; exec sh -c "$body" '+
                     shlex.join(["weread-offscreen",mode,label,str(report["bytes"])]))
            step={"mode":mode,"at":now()}; report["steps"].append(step); report["connection_closed"]=False; persist()
            try:
                result=subprocess.run([*ssh,command],input=header+(payload if mode=="stage" else b""),stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,
                                      env=environment,start_new_session=True,timeout={"stage":180,"run":240,"close":45}.get(mode,25))
                step["status"]=result.returncode
                if result.returncode:
                    detail=re.fullmatch(rb"refused ([a-z_]{1,60})\n",result.stdout)
                    step["reason"]=detail[1].decode() if detail else "transport_or_receipt"; return b""
                return result.stdout
            finally:
                report["connection_closed"]=True; persist()
        acquired=False; touched=bool(args.recover); samples=None
        try:
            if args.recover:
                require(lease.status(request).get("ownership_verified") is True,"wrong_recovery_token")
            else:
                handoff(args.handoff,args.owner); result=lease.acquire(request); acquired=True
                require(result.get("ok") is True and result.get("state")=="held","unconfirmed_acquisition")
            acquired=True
            if not args.recover:
                admission=call("admit"); require(admission in (b"admitted active\n",b"admitted screenSaver\n"),"admission")
                report["power_state"]=admission.decode().split()[1]; touched=True
                require(call("stage")==b"staged\n","stage")
                samples=observations(call("run"),args.lane)
        except (Exception,KeyboardInterrupt) as error:
            report["failure"]=getattr(error,"code",str(error) if isinstance(error,ValueError) and re.fullmatch(r"[a-z_]+",str(error)) else type(error).__name__)
            if not acquired and (request.token_file.exists() or any(getattr(error,"details",{}).get(key) for key in ("lease_directory_created","private_token_created","owner_record_created"))): report["recovery_required"]=True
        finally:
            try:
                if acquired:
                    close=call("close") if touched else b"absent\n"
                    cleared=close in (b"closed\n",b"closed_state_changed\n",b"absent\n") and (not touched or call("absent")==b"absent\n")
                    if cleared and report["connection_closed"]:
                        released=lease.release(request,native_restored_and_connections_closed=True)
                        require(released.get("owner_and_token_checked") and released.get("only_owned_owner_json_removed_then_rmdir"),"unconfirmed_release")
                        request.token_file.unlink(); report.update(released=True,released_at=released["released_at_utc"],stage_removed=touched,recovery_required=False)
                        report["passed"] = close==b"closed\n" and samples is not None
                        if report["passed"]: (attempt/"offscreen.json").write_text(json.dumps(samples,separators=(",",":"))+"\n")
                    else: report["recovery_required"]=True
            finally: persist()
    print("passed",report["passed"],"released",report["released"],flush=True)
    return 0 if report["passed"] or args.recover and report["released"] else 1


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("runs","lease-tool"): parser.add_argument("--"+name,type=Path,required=True)
    for name in ("app","recover","handoff"): parser.add_argument("--"+name,type=Path)
    parser.add_argument("--owner",required=True); parser.add_argument("--lane",choices=("A","B")); parser.add_argument("--preflight",action="store_true")
    args=parser.parse_args()
    def interrupt(*unused): raise KeyboardInterrupt
    signal.signal(signal.SIGTERM,interrupt)
    try: return execute(args)
    except (OSError,ValueError,RuntimeError,subprocess.SubprocessError) as error:
        print("offscreen visit refused:",str(error) if isinstance(error,ValueError) else type(error).__name__,file=sys.stderr); return 1


if __name__=="__main__":
    sys.exit(main())
