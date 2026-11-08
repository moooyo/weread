#!/usr/bin/env python3
"""Run pinned Kindle firmware-emulator cases on the Linux verification host."""

import argparse
import hashlib
import json
from pathlib import Path
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
        "exit_tap": (170, 1135),
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
        "exit_tap": (200, 1136),
        "confirm_tap": (1362, 1388),
        "detail_tap": (220, 600),
        "reader_tap": (784, 640),
    },
}
LOGS = "/mnt/us/weread/data/logs"
LOCK = "/tmp/weread-launcher.lock"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emulator-root", type=Path, required=True)
    parser.add_argument("--guest-ssh", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--machine", default="weread-pw6-5201")
    parser.add_argument("--device-profile", choices=tuple(PROFILES), default="pw6")
    parser.add_argument("--case", choices=("all", *CASES), default="all")
    parser.add_argument("--sample-memory", action="store_true")
    args = parser.parse_args()
    profile = PROFILES[args.device_profile]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "model": profile["model"],
        "firmware": "5.20.1",
        "experimental_display": profile["experimental_display"],
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "cases": [],
    }

    def save_report():
        (args.output_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        for record in report["cases"]:
            path = args.output_dir / record["case"] / "result.json"
            path.write_text(json.dumps(record, indent=2) + "\n")

    def guest(*command, check=True, data=None):
        return subprocess.run(
            [str(args.guest_ssh), *command], input=data, capture_output=True,
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
    binary_name = f"weread-{report['binary_sha256'][:12]}"
    configured_binary = guest("cat", f"{LOGS}/test-binary").stdout.decode().strip()
    assert configured_binary == binary_name, configured_binary
    guest_hash = guest("sha256sum", f"/mnt/us/weread/app/{binary_name}").stdout.decode().split()[0]
    assert guest_hash == report["binary_sha256"], guest_hash

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
            guest("tee", f"{LOGS}/test-case", data=(case + "\n").encode())
            guest("tee", f"{LOGS}/application.log", data=b"")
            guest("rm", "-f", f"{LOGS}/final-native.png",
                  f"{LOGS}/final-native.metadata.json", f"{LOGS}/native.pid")
            with (directory / "launcher.log").open("wb") as output:
                process = subprocess.Popen(
                    [str(args.guest_ssh), "sh", "/mnt/us/documents/weread.sh", "--detached"],
                    stdout=output, stderr=subprocess.STDOUT,
                )
            if args.sample_memory:
                for _ in range(90):
                    if process.poll() is not None:
                        raise RuntimeError(f"Application exited before PID sampling: {process.returncode}")
                    result = guest("cat", f"{LOGS}/native.pid", check=False)
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
                    guest("kill", "-TERM", read_pid(f"{LOGS}/native.pid"))
                else:
                    guest("kill", "-TERM", read_pid(f"{LOCK}/pid"))

            status = process.wait(timeout=45)
            record["exit_code"] = status
            assert status == (143 if case == "recovery" else 0), status
            restored()
            time.sleep(1)
            capture(directory, "restored-home")
            if case != "recovery":
                raw_metadata = guest("cat", f"{LOGS}/final-native.metadata.json").stdout
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
                (directory / "native.png").write_bytes(guest("cat", f"{LOGS}/final-native.png").stdout)
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
            result = guest("cat", f"{LOGS}/application.log", check=False)
            (directory / "application.log").write_bytes(result.stdout + result.stderr)
            save_report()
            print(json.dumps(record), flush=True)


if __name__ == "__main__":
    main()
