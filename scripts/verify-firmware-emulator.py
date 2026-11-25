#!/usr/bin/env python3
"""Run pinned Kindle firmware-emulator cases on the Linux verification host."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import threading
import time

from PIL import Image


CASES = ("interactive", "reader", "recovery", "startup-1", "startup-2", "startup-3")
PROFILES = {
    "pw6": {
        "model": "kindle-paperwhite-6",
        "experimental_display": "pw6-5201-emulator",
        "physical_size": (1272, 1696),
        "design_size": [632, 840],
        "logical_size": [636, 848],
        "library_pill": (100, 1580, 212, 1640),
        "settings_pill": (1060, 1580, 1176, 1640),
        "settings_tap": (1116, 1616),
        # Exit sits at the right end of the row above the navigation.
        "exit_tap": (1104, 1504),
        "confirm_tap": (1084, 998),
        "detail_tap": (222, 620),
        "reader_tap": (360, 520),
    },
    "scribe": {
        "model": "kindle-scribe-1",
        "experimental_display": None,
        "physical_size": (1860, 2480),
        "design_size": [930, 1240],
        "logical_size": [930, 1240],
        "library_pill": (174, 2350, 286, 2410),
        "settings_pill": (1576, 2350, 1688, 2410),
        "settings_tap": (1632, 2380),
        "exit_tap": (1668, 2256),
        "confirm_tap": (1362, 1388),
        "detail_tap": (220, 600),
        # Scribe Detail retains the primary action in its independent left pane.
        "reader_tap": (300, 1320),
    },
}
LOCK = "/tmp/weread-launcher.lock"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emulator-root", type=Path, required=True)
    parser.add_argument("--guest-ssh", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True, help="Host path of the deployed candidate")
    parser.add_argument("--deployed-name", help="Expected immutable guest ELF basename from the deployment record")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--guest-home", default="/mnt/us/weread",
                        help="Absolute application home in this disposable guest")
    parser.add_argument("--guest-entry", default="/mnt/us/documents/weread.sh",
                        help="Absolute document launcher path in this disposable guest")
    parser.add_argument("--machine", default="weread-pw6-5201")
    parser.add_argument("--device-profile", choices=tuple(PROFILES), default="pw6")
    parser.add_argument("--case", choices=("all", *CASES, "walkthrough"), default="all")
    parser.add_argument("--only", help="Comma-separated walkthrough groups; requires --case walkthrough")
    parser.add_argument("--sample-memory", action="store_true")
    args = parser.parse_args()
    if args.only and args.case != "walkthrough":
        parser.error("--only requires --case walkthrough")
    if args.sample_memory and args.case == "walkthrough":
        parser.error("--sample-memory is for the launcher smoke cases")
    if not args.binary.is_file():
        parser.error("--binary must identify the deployed host executable")
    for path in (args.guest_home, args.guest_entry):
        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", path) or ".." in Path(path).parts:
            parser.error("guest paths must be absolute with no parent traversal")
    logs = f"{args.guest_home}/data/logs"
    entry = [str(args.guest_ssh), "env", f"WEREAD_HOME={args.guest_home}",
             "sh", args.guest_entry, "--detached"]
    profile = PROFILES[args.device_profile]
    args.output_dir = args.output_dir.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "machine": args.machine,
        "model": profile["model"],
        "firmware": "5.20.1",
        "experimental_display": profile["experimental_display"],
        "host_binary": str(args.binary.resolve()),
        "binary_bytes": args.binary.stat().st_size,
        "cases": [],
    }

    def save_report():
        (args.output_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        for record in report["cases"]:
            path = args.output_dir / record["case"] / "result.json"
            path.write_text(json.dumps(record, indent=2) + "\n")

    def guest(*command, check=True, data=None):
        return subprocess.run(
            [str(args.guest_ssh), *map(str, command)], input=data, capture_output=True,
            check=check, timeout=20,
        )

    def qmp(*command):
        return subprocess.run(
            [sys.executable, str(args.emulator_root / "scripts/eink-qmp.py"),
             "--machine", args.machine, *map(str, command)],
            capture_output=True, check=True, timeout=20,
        )

    def capture(directory, name):
        path = (directory / f"{name}.png").resolve()
        qmp("screendump", path)
        with Image.open(path) as image:
            assert image.size == profile["physical_size"], image.size
        return path

    def dark_pixels(path, box):
        with Image.open(path) as image:
            return sum(value < 64 for value in image.convert("L").crop(box).tobytes())

    def read_pid(path):
        value = guest("cat", path).stdout.decode().strip()
        assert value.isdecimal() and int(value) > 1, value
        return value

    def sample_memory(pid, stop, path, samples, errors):
        fields = {"VmRSS", "VmHWM", "RssAnon", "RssFile", "VmSwap", "MemAvailable"}
        with path.open("w") as output:
            while not stop.is_set():
                try:
                    result = guest("cat", f"/proc/{pid}/status", "/proc/meminfo", check=False)
                    if result.returncode:
                        if result.returncode == 255:
                            errors.append(result.stderr.decode(errors="replace").strip())
                        break
                    sample = {"elapsed_seconds": round(time.monotonic() - started, 3)}
                    for line in result.stdout.decode().splitlines():
                        key, _, value = line.partition(":")
                        if key in fields:
                            sample[key] = int(value.split()[0])
                    samples.append(sample)
                    output.write(json.dumps(sample) + "\n")
                    output.flush()
                except Exception as error:
                    errors.append(str(error))
                    output.write(json.dumps({"sampling_error": str(error)}) + "\n")
                    break
                stop.wait(0.2)

    def restored():
        guest("test", "!", "-d", LOCK)
        status = guest("status", "statusbar").stdout.decode()
        assert "start/running" in status, status
        for pid in guest("pidof", "awesome").stdout.decode().split():
            assert pid.isdecimal(), pid
            state = guest("cat", f"/proc/{pid}/status").stdout.decode()
            state = next(line for line in state.splitlines() if line.startswith("State:"))
            assert state.split()[1] not in ("T", "t"), state

    firmware = guest("cat", "/etc/prettyversion.txt").stdout.decode().strip()
    assert firmware.startswith("Kindle 5.20.1 "), firmware
    report["firmware_identity"] = firmware
    binary_name = guest("cat", f"{logs}/test-binary").stdout.decode().strip()
    assert re.fullmatch(r"weread-[0-9a-f]{12}", binary_name), binary_name
    if args.deployed_name is not None:
        assert binary_name == args.deployed_name, binary_name
    guest_bytes = int(guest("stat", "-c", "%s", f"{args.guest_home}/app/{binary_name}").stdout.strip())
    assert guest_bytes == report["binary_bytes"], (guest_bytes, report["binary_bytes"])
    report["deployed_binary"] = binary_name

    if args.case == "walkthrough":
        from firmware_walkthrough import Walkthrough

        directory = args.output_dir / "walkthrough"
        directory.mkdir(exist_ok=False)
        record = {"case": "walkthrough", "result": "running"}
        report["cases"].append(record)
        save_report()
        try:
            walk = Walkthrough(args, profile, directory, record, guest, qmp, capture, restored, read_pid, logs, LOCK, entry)
            record["result"] = "passed" if walk.run() else "failed"
        except Exception as error:
            record["result"] = "failed"
            record["error"] = str(error)
            raise
        finally:
            save_report()
        print(json.dumps({"report": str(args.output_dir / "report.json"), "result": record["result"]}), flush=True)
        return 0 if record["result"] == "passed" else 1

    for case in CASES if args.case == "all" else (args.case,):
        directory = args.output_dir / case
        directory.mkdir(parents=True, exist_ok=True)
        record = {"case": case, "result": "running"}
        report["cases"].append(record)
        save_report()
        process = None
        memory_stop = threading.Event()
        memory_thread = None
        memory_samples = []
        memory_errors = []
        started = time.monotonic()
        try:
            guest("test", "!", "-d", LOCK)
            guest("tee", f"{logs}/test-case", data=(case + "\n").encode())
            guest("tee", f"{logs}/application.log", data=b"")
            guest("rm", "-f", f"{logs}/final-native.png",
                  f"{logs}/final-native.metadata.json", f"{logs}/native.pid")
            with (directory / "launcher.log").open("wb") as output:
                process = subprocess.Popen(
                    entry,
                    stdout=output, stderr=subprocess.STDOUT,
                )
            if args.sample_memory:
                for _ in range(90):
                    if process.poll() is not None:
                        raise RuntimeError(f"Application exited before PID sampling: {process.returncode}")
                    result = guest("cat", f"{logs}/native.pid", check=False)
                    pid = result.stdout.decode().strip()
                    if result.returncode == 0 and pid.isdecimal() and int(pid) > 1:
                        memory_thread = threading.Thread(
                            target=sample_memory,
                            args=(pid, memory_stop, directory / "memory.jsonl", memory_samples, memory_errors),
                            daemon=True,
                        )
                        memory_thread.start()
                        break
                    time.sleep(0.2)
                assert memory_thread is not None, "The native PID was not available"
            if not case.startswith("startup-"):
                ready = False
                for _ in range(90):
                    if process.poll() is not None:
                        raise RuntimeError(f"Application exited before readiness: {process.returncode}")
                    path = capture(directory, "library")
                    if dark_pixels(path, profile["library_pill"]) > 4000:
                        ready = True
                        break
                    time.sleep(0.5)
                assert ready, "The native library did not reach the emulated display"

                if case == "interactive":
                    qmp("tap", *profile["settings_tap"], "--hold", 0.1)
                    time.sleep(1)
                    path = capture(directory, "settings")
                    assert dark_pixels(path, profile["settings_pill"]) > 4000
                    qmp("tap", *profile["exit_tap"], "--hold", 0.1)
                    time.sleep(1)
                    capture(directory, "exit-confirmation")
                    qmp("tap", *profile["confirm_tap"], "--hold", 0.1)
                elif case == "reader":
                    qmp("tap", *profile["detail_tap"], "--hold", 0.1)
                    time.sleep(1)
                    capture(directory, "detail")
                    qmp("tap", *profile["reader_tap"], "--hold", 0.1)
                    time.sleep(1)
                    capture(directory, "reader")
                    guest("kill", "-TERM", read_pid(f"{logs}/native.pid"))
                else:
                    guest("kill", "-TERM", read_pid(f"{LOCK}/pid"))

            status = process.wait(timeout=45)
            record["exit_code"] = status
            assert status == (143 if case == "recovery" else 0), status
            restored()
            time.sleep(1)
            capture(directory, "restored-home")
            if case != "recovery":
                raw_metadata = guest("cat", f"{logs}/final-native.metadata.json").stdout
                (directory / "native.metadata.json").write_bytes(raw_metadata)
                metadata = json.loads(raw_metadata)
                expected_route = {"interactive": "settings", "reader": "reader"}.get(case, "library")
                assert metadata["route"] == expected_route, metadata
                assert metadata["backend"] == "kindle", metadata
                assert metadata["profile"] == args.device_profile, metadata
                assert metadata["design_logical_size"] == profile["design_size"], metadata
                assert metadata["logical_size"] == profile["logical_size"], metadata
                assert metadata["window_size"] == metadata["snapshot_size"] == list(profile["physical_size"]), metadata
                assert metadata["scale_factor"] == 2, metadata
                (directory / "native.png").write_bytes(guest("cat", f"{logs}/final-native.png").stdout)
                record["metadata"] = metadata
            record["result"] = "passed"
        except Exception as error:
            record["result"] = "failed"
            record["error"] = str(error)
            if process is not None and process.poll() is None:
                try:
                    guest("kill", "-TERM", read_pid(f"{LOCK}/pid"), check=False)
                    process.wait(timeout=15)
                except Exception:
                    pass
            raise
        finally:
            memory_stop.set()
            if memory_thread is not None:
                memory_thread.join(timeout=25)
            if memory_thread is not None:
                rss = [sample["VmRSS"] for sample in memory_samples if "VmRSS" in sample]
                hwm = [sample["VmHWM"] for sample in memory_samples if "VmHWM" in sample]
                available = [sample["MemAvailable"] for sample in memory_samples if "MemAvailable" in sample]
                record["memory_observation"] = {
                    "samples": len(memory_samples),
                    "observed_peak_rss_kb": max(rss) if rss else None,
                    "observed_high_water_kb": max(hwm) if hwm else None,
                    "minimum_available_kb": min(available) if available else None,
                    "sampling_errors": memory_errors,
                    "coverage": "Periodic /proc sampling; not an exhaustive peak measurement",
                }
            record["elapsed_seconds"] = round(time.monotonic() - started, 3)
            result = guest("cat", f"{logs}/application.log", check=False)
            (directory / "application.log").write_bytes(result.stdout + result.stderr)
            save_report()
            print(json.dumps(record), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
