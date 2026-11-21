"""Compare two isolated detached launcher fixtures on the Linux verification host."""

import argparse
import json
from pathlib import Path
import socket
import statistics
import sys
import time


def require(condition, label):
    if not condition:
        raise ValueError(label)


def run_case(module, launcher, arm, delay, row):
    module.SOURCE = launcher
    fixture = module.LauncherLifecycleTests()
    try:
        fixture.setUp()
        fixture.native_fixture("")
        app = fixture.home / "app" / "weread"
        original_app = app.read_text()
        awesome = fixture.cover_sleep_fixture()
        require("\nsleep 2\n" in fixture.launcher.read_text() and not (fixture.bin / "sleep").exists(),
                "settling guard not using real sleep")
        probe = fixture.bin / "xwininfo"
        source = probe.read_text()
        require(source.startswith("#!/bin/sh\nset -eu\n"), "unknown X probe fixture")
        if delay:
            probe.write_text(source.replace("set -eu\n", 'set -eu\nsleep 0.200\n', 1))
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as witness:
            witness.bind(str(fixture.root / "app-entry.sock"))
            witness.settimeout(1)
            fixture.env["BENCH_ENTRY_SOCKET"] = str(fixture.root / "app-entry.sock")
            app_entry = fixture.bin / "app-entry"
            app_entry.write_text(
                f"#!{sys.executable}\nimport time\nstamp = time.monotonic_ns()\nimport os, socket\n"
                "with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as witness:\n"
                "    witness.sendto(str(stamp).encode(), os.environ['BENCH_ENTRY_SOCKET'])\n"
            )
            app_entry.chmod(0o755)
            anchor = '\ndone\napp_child=""\n'
            require(original_app.count(anchor) == 1, "unknown mock application entry")
            app.write_text(original_app.replace(anchor, '\ndone\napp-entry\napp_child=""\n'))
            start = time.monotonic_ns()
            status, _, _ = fixture.run_launcher()
            finish = time.monotonic_ns()
            reached = int(witness.recv(80))
            require(start < reached < finish, "invalid monotonic entry time")
            row["entry_ms"] = (reached - start) / 1e6
            row["exit_status"] = status
            require(status == 0, "supervisor exit failed")
        trace = fixture.trace()
        row["starts"] = len(fixture.starts())
        row["xwininfo"] = len(fixture.native_window_polls())
        row["blanket"] = (fixture.root / "blanket-queries").read_text().splitlines().count("load")
        row["statusbar_stops"] = trace.count("stop:statusbar")
        row["statusbar_starts"] = trace.count("start:statusbar")
        require(fixture.starts() == ["0"] and fixture.application_trace() == ["started:0"], "normal app starts")
        require((fixture.root / "approval").read_text() == "scribe|5.19.5", "profile mismatch")
        require(row["xwininfo"] == (1 if arm == "A" else 0) and row["blanket"] == 1, "wrong query counts")
        require(row["statusbar_stops"] == row["statusbar_starts"] == 1 and
                trace.index("stop:statusbar") < trace.index("start:statusbar"), "statusbar did not restore")
        fixture.assert_native_theme_restored(awesome)
        require("wm:CONT" in trace, "no window-manager continuation")
        fixture.assert_frame_guards_reaped(1)
        row["native_restored"] = True
    finally:
        require(fixture.doCleanups(), "fixture cleanup failure")


def summarize(samples):
    by_case = {row["case"]: row["entry_ms"] for row in samples}
    baseline, candidate = ([by_case[f"{arm}{number}"] for number in range(1, 7)] for arm in "AB")
    paired = [left - right for left, right in zip(baseline, candidate)]
    return {"control_median_ms": statistics.median(baseline), "candidate_median_ms": statistics.median(candidate),
            "paired_savings_ms": paired, "median_paired_saving_ms": statistics.median(paired),
            "faster_pairs": sum(value > 0 for value in paired)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("workspace", "control", "candidate", "output"):
        parser.add_argument(argument, type=Path)
    args = parser.parse_args()
    require(sys.platform == "linux" and Path("/proc/self/stat").exists(), "use the Linux verification worker")
    sys.path.insert(0, str(args.workspace.resolve(strict=True) / "packaging/tests"))
    import launcher_test

    require(not args.output.exists(), "report already exists; preserve every attempt")
    output = {"scope": "Linux detached supervisor to mock application entry, not Kindle or Library SEND.",
              "settle_seconds": 2, "fixture_usleep_ms": 10,
              "fixture": "Scribe 5.19.5 profile, fake services and window manager, /dev/null framebuffer, no real input.",
              "endpoint": "Parent monotonic clock immediately before run_launcher to timestamp in a Python mock-app entry witness (includes its startup); no polling or SSH receipt.",
              "arms": {"A": "pre-change launcher", "B": "on-demand settings query"}, "cohorts": {}}
    schedule = ["A0", "B0"]
    for number in range(1, 7):
        schedule.extend(f"{arm}{number}" for arm in ("AB" if number % 2 else "BA"))
    for name, delay in (("unchanged_x_probe", 0), ("controlled_200ms_x_probe", 200)):
        cohort = {"added_query_delay_ms": delay, "samples": []}
        output["cohorts"][name] = cohort
        for case in schedule:
            arm = case[0]
            row = {"case": case, "warmup": case.endswith("0"), "passed": False}
            cohort["samples"].append(row)
            try:
                run_case(launcher_test, (args.control if arm == "A" else args.candidate).resolve(strict=True), arm, delay, row)
                row["passed"] = True
            except Exception as error:
                row["failed_check"] = str(error) if isinstance(error, ValueError) else type(error).__name__
                raise
            finally:
                args.output.write_text(json.dumps(output, indent=2) + "\n")
            print(name, case, round(row["entry_ms"], 3), flush=True)
        cohort["measured"] = summarize(cohort["samples"])
        args.output.write_text(json.dumps(output, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
