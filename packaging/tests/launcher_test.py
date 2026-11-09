"""Exercise launcher cleanup on Linux with isolated firmware/service fixtures."""

import errno
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest


SOURCE = Path(__file__).resolve().parents[1] / "kindle" / "launch.sh"


class LauncherLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="weread-launcher-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / "weread"
        (self.home / "app").mkdir(parents=True)
        (self.home / "data").mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.home / "data" / "device-profile").write_text("pw6\n")
        (self.home / "data" / "device-allowlist").write_text("pw6 5.20.1\n")
        (self.root / "firmware").write_text("Kindle 5.20.1 (example)\n")
        fixture_source = (
            SOURCE.read_text()
            .replace("/etc/prettyversion.txt", str(self.root / "firmware"))
            .replace("/dev/fb0", "/dev/null")
            .replace("/tmp/weread-launcher.lock", str(self.root / "lock"))
        )
        self.launcher = self.root / "launch.sh"
        self.launcher.write_text(fixture_source)
        self.env = os.environ.copy()
        self.env.update(
            WEREAD_HOME=str(self.home),
            PATH=f"{self.bin}:{self.env['PATH']}",
            TEST_TRACE=str(self.root / "trace"),
            TEST_ARGS=str(self.root / "args"),
            TEST_APPROVAL=str(self.root / "approval"),
            TEST_DISPLAY=str(self.root / "display"),
            TEST_STARTS=str(self.root / "starts"),
            TEST_NATIVE_STARTS=str(self.root / "native-starts"),
            TEST_NATIVE_VISIBLE=str(self.root / "native-visible"),
            TEST_NATIVE_VIEW=str(self.root / "native-view"),
            TEST_NATIVE_QUERIES=str(self.root / "native-queries"),
            TEST_NATIVE_WINDOW_POLLS=str(self.root / "native-window-polls"),
            TEST_APP_TRACE=str(self.root / "application-trace"),
            TEST_CHECKPOINT=str(self.root / "reader-checkpoint"),
            TEST_STATUSBAR="start/running, process 123",
            TEST_APP_EXIT="0",
            TEST_TERM_EXIT="0",
            TEST_STOP_DELAY="0",
            TEST_RESUME_HOLD="0",
            TEST_SKIP_ACK="0",
            TEST_ORDINARY_ON_STOP="0",
            TEST_INITIALIZATION_GENERATION="0",
            TEST_HOLD_UNTIL_GENERATION="0",
            TEST_POWER_STATE="active",
            TEST_EVENT="",
            TEST_SLEEP="0",
            TEST_NATIVE_KIND="",
            TEST_NATIVE_SKIP_ACK="0",
            TEST_NATIVE_ORDINARY="0",
            TEST_NATIVE_SET_FAIL="0",
            TEST_NATIVE_SET_FAIL_AFTER_MAP="0",
            TEST_NATIVE_MAP_ON_OPEN="1",
            TEST_NATIVE_APP_EXIT="0",
        )
        (self.root / "native-visible").write_text("none\n")
        (self.root / "native-view").write_text("KPP_HOME\n")
        self.script(self.bin / "status", 'printf "statusbar %s\\n" "$TEST_STATUSBAR"')
        for name in ("stop", "start", "xrefresh"):
            self.script(
                self.bin / name,
                f'printf "%s\\n" "{name}:$*" >>"$TEST_TRACE"',
            )
        self.script(self.bin / "pidof", "exit 1")
        self.script(
            self.bin / "lipc-wait-event",
            'if [ -n "$TEST_EVENT" ]; then sleep 0.1; printf "%s\\n" "$TEST_EVENT"; fi\n'
            'exec sleep 30',
        )
        self.script(
            self.bin / "lipc-get-prop",
            'case "$2:$3" in\n'
            '    com.lab126.appmgrd:activeApp|com.lab126.appmgrd:activeView)\n'
            '        visible=$(cat "$TEST_NATIVE_VISIBLE")\n'
            '        view=$(cat "$TEST_NATIVE_VIEW")\n'
            '        printf "%s|%s|%s\\n" "$3" "$visible" "$view" >>"$TEST_NATIVE_QUERIES"\n'
            '        if [ "$3" = activeApp ]; then printf "%s\\n" com.lab126.KPPMainApp; else printf "%s\\n" "$view"; fi ;;\n'
            '    com.lab126.powerd:state) printf "%s\\n" "$TEST_POWER_STATE" ;;\n'
            '    *) exit 1 ;;\n'
            'esac',
        )
        self.script(self.bin / "usleep", "exec sleep 0.01")
        self.script(
            self.bin / "xwininfo",
            'visible=$(cat "$TEST_NATIVE_VISIBLE")\n'
            'printf "%s\\n" "$visible" >>"$TEST_NATIVE_WINDOW_POLLS"\n'
            '[ "$visible" != error ] || exit 1\n'
            'printf "%s\\n" "Root window id: 0x50"\n'
            'printf "%s\\n" \'0x100 "L:A_N:application_ID:com.lab126.KPPMainApp" MapState=IsViewable\'\n'
            'if [ "$visible" = quick ]; then\n'
            '    printf "%s\\n" \'0x200 "L:D_N:dialog_ID:com.lab126.kppQuickSettings" MapState=IsViewable\'\n'
            'fi\n'
            'if [ "$visible" = wifi ]; then\n'
            '    printf "%s\\n" \'0x300 "L:D_N:dialog_A:pillow-wifi-wizard-dialog" MapState=IsViewable\'\n'
            'fi\n'
            'if [ "$visible" = modal ]; then\n'
            '    printf "%s\\n" \'0x400 "L:D_N:dialog_ID:native-password" MapState=IsViewable\'\n'
            'else\n'
            '    printf "%s\\n" \'0x400 "L:D_N:dialog_ID:native-password" MapState=IsUnMapped\'\n'
            'fi',
        )
        self.script(
            self.bin / "lipc-set-prop",
            'app_pid=$(cat "$WEREAD_HOME/../lock/application-pid" 2>/dev/null || :)\n'
            'if [ -n "$app_pid" ] && kill -0 "$app_pid" 2>/dev/null; then\n'
            '    printf "%s\\n" native-concurrent-application >>"$TEST_TRACE"\n'
            '    exit 77\n'
            'fi\n'
            'printf "native:%s\\n" "$*" >>"$TEST_TRACE"\n'
            'case "$*" in\n'
            '    *openedByTap*)\n'
            '        [ "$TEST_NATIVE_SET_FAIL" = 0 ] || exit 7\n'
            '        if [ "$TEST_NATIVE_MAP_ON_OPEN" = 1 ]; then printf "%s\\n" quick >"$TEST_NATIVE_VISIBLE"; fi\n'
            '        [ "$TEST_NATIVE_SET_FAIL_AFTER_MAP" = 0 ] || exit 7 ;;\n'
            '    *\'"show":true\'*)\n'
            '        [ "$TEST_NATIVE_SET_FAIL" = 0 ] || exit 7\n'
            '        if [ "$TEST_NATIVE_MAP_ON_OPEN" = 1 ]; then printf "%s\\n" wifi >"$TEST_NATIVE_VISIBLE"; fi\n'
            '        [ "$TEST_NATIVE_SET_FAIL_AFTER_MAP" = 0 ] || exit 7 ;;\n'
            '    *) exit 8 ;;\n'
            'esac',
        )
        self.script(
            self.home / "app" / "weread",
            'for probe_argument in "$@"; do\n'
            '    if [ "$probe_argument" = --probe-native-portrait-framebuffer ]; then\n'
            '        if [ "$#" -eq 4 ] && [ "$1:$2:$3" = --kindle:--probe-native-portrait-framebuffer:--device ]; then\n'
            '            case "$4" in pw6|scribe) printf "%s\\n" FIXED:0; exit 0 ;; esac\n'
            '        fi\n'
            '        exit 74\n'
            '    fi\n'
            'done\n'
            'app_child=""\n'
            'app_initializing=0\n'
            'stop_app() {\n'
            '    trap - HUP INT TERM\n'
            '    if [ -n "$app_child" ]; then\n'
            '        kill -TERM "$app_child" 2>/dev/null || :\n'
            '        wait "$app_child" 2>/dev/null || :\n'
            '    fi\n'
            '    if [ "$TEST_STOP_DELAY" != 0 ]; then sleep "$TEST_STOP_DELAY"; fi\n'
            '    if [ "$app_initializing" = 0 ]; then\n'
            '        printf "%s\\n" "book-7:3:42" >"$TEST_CHECKPOINT"\n'
            '        printf "saved:%s\\n" "$WEREAD_RESUME_SESSION" >>"$TEST_APP_TRACE"\n'
            '        if [ "$TEST_SKIP_ACK" = 0 ] && [ -r "$WEREAD_LAUNCH_CONTEXT_DIR/sleep-request" ] && [ "$(cat "$WEREAD_LAUNCH_CONTEXT_DIR/sleep-request")" = "$WEREAD_LAUNCH_GENERATION:$WEREAD_LAUNCH_TOKEN" ]; then\n'
            '            printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/resume-ready"\n'
            '        fi\n'
            '    else\n'
            '        printf "interrupted:%s\\n" "$WEREAD_RESUME_SESSION" >>"$TEST_APP_TRACE"\n'
            '    fi\n'
            '    if [ "$TEST_ORDINARY_ON_STOP" = 1 ]; then\n'
            '        printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/ordinary-exit"\n'
            '    fi\n'
            '    exit "$TEST_TERM_EXIT"\n'
            '}\n'
            'trap stop_app HUP INT TERM\n'
            'printf "%s\\n" "$@" >"$TEST_ARGS"\n'
            'printf "%s|%s" "$WEREAD_APPROVED_PROFILE" "$WEREAD_APPROVED_FIRMWARE" >"$TEST_APPROVAL"\n'
            'printf "%s" "${WEREAD_EXPERIMENTAL_DISPLAY-}" >"$TEST_DISPLAY"\n'
            'if [ "$TEST_INITIALIZATION_GENERATION" = "$WEREAD_LAUNCH_GENERATION" ]; then\n'
            '    app_initializing=1\n'
            'else\n'
            '    printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/state-open"\n'
            'fi\n'
            'printf "%s\\n" "$WEREAD_RESUME_SESSION" >>"$TEST_STARTS"\n'
            'printf "%s|%s|%s\\n" "$WEREAD_RESUME_SESSION" "$WEREAD_NATIVE_RETURN" "$WEREAD_NATIVE_RETURN_RESULT" >>"$TEST_NATIVE_STARTS"\n'
            'printf "started:%s\\n" "$WEREAD_RESUME_SESSION" >>"$TEST_APP_TRACE"\n'
            'if [ "$WEREAD_RESUME_SESSION" = 1 ] && [ "$app_initializing" = 0 ]; then\n'
            '    [ -r "$TEST_CHECKPOINT" ] || exit 9\n'
            '    printf "restored:%s\\n" "$(cat "$TEST_CHECKPOINT")" >>"$TEST_APP_TRACE"\n'
            'fi\n'
            'if [ -n "$TEST_NATIVE_KIND" ] && [ "$WEREAD_LAUNCH_GENERATION" = 1 ]; then\n'
            '    printf "%s\\n" "book-7:3:42" >"$TEST_CHECKPOINT"\n'
            '    printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/native-$TEST_NATIVE_KIND-request"\n'
            '    if [ "$TEST_NATIVE_SKIP_ACK" = 0 ]; then\n'
            '        printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/resume-ready"\n'
            '    fi\n'
            '    if [ "$TEST_NATIVE_ORDINARY" = 1 ]; then\n'
            '        printf "%s:%s\\n" "$WEREAD_LAUNCH_GENERATION" "$WEREAD_LAUNCH_TOKEN" >"$WEREAD_LAUNCH_CONTEXT_DIR/ordinary-exit"\n'
            '    fi\n'
            '    exit "$TEST_NATIVE_APP_EXIT"\n'
            'fi\n'
            'if [ "$TEST_SLEEP" = 1 ] && { [ "$WEREAD_RESUME_SESSION" = 0 ] || [ "$TEST_RESUME_HOLD" = 1 ]; } && { [ "$TEST_HOLD_UNTIL_GENERATION" = 0 ] || [ "$WEREAD_LAUNCH_GENERATION" -lt "$TEST_HOLD_UNTIL_GENERATION" ]; }; then\n'
            '    sleep 30 &\n'
            '    app_child=$!\n'
            '    wait "$app_child"\n'
            'fi\n'
            'exit "$TEST_APP_EXIT"',
        )

    @staticmethod
    def script(path, body):
        path.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
        path.chmod(0o755)

    def run_launcher(self, drive=None, managed_logs=False):
        arguments = ["sh", str(self.launcher), "--detached"]
        if managed_logs:
            arguments.append("--managed-logs")
        sequence = getattr(self, "launcher_capture_sequence", 0) + 1
        self.launcher_capture_sequence = sequence
        output_path = self.root / f"launcher-{sequence}.stdout"
        error_path = self.root / f"launcher-{sequence}.stderr"
        # A descendant retaining an output fd must not masquerade as a live
        # Main process. Process exit and group retirement are checked separately.
        with output_path.open("wb") as output_file, error_path.open("wb") as error_file:
            child = subprocess.Popen(
                arguments,
                env=self.env,
                stdout=output_file,
                stderr=error_file,
                text=True,
                start_new_session=True,
            )
            self.active_fixture_launcher = {"process": child, "output": output_path, "error": error_path}
            try:
                if drive is not None:
                    drive(child)
                child.wait(timeout=16)
            except subprocess.TimeoutExpired:
                detail = self.fixture_wait_details()
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.retire_fixture_event_group(child.pid)
                    child.wait(timeout=3)
                # The TERM grace can finish Main while an ignoring child still
                # pins its group. Retire that original group on this path too.
                members = self.retire_fixture_event_group(child.pid)
                self.fail(f"The fixture launcher did not finish in time. {detail} Remaining group: {members!r}")
            except BaseException:
                self.retire_fixture_event_group(child.pid)
                child.wait(timeout=3)
                raise
            finally:
                self.active_fixture_launcher = None
        members = self.retire_fixture_event_group(child.pid)
        if members is not None:
            self.fail(f"The fixture launcher left an event child running: {members!r}")
        return child.returncode, output_path.read_text(errors="replace"), error_path.read_text(errors="replace")

    def retire_fixture_event_group(self, group):
        try:
            os.killpg(group, 0)
        except ProcessLookupError:
            return None
        members = self.event_group_members(group)
        try:
            os.killpg(group, signal.SIGKILL)
        except ProcessLookupError:
            pass
        return members

    @staticmethod
    def fixture_output_tail(path, limit=2048):
        try:
            with Path(path).open("rb") as source:
                source.seek(0, os.SEEK_END)
                source.seek(max(0, source.tell() - limit))
                return source.read(limit).decode(errors="replace")
        except OSError:
            return ""

    def fixture_wait_details(self):
        active = getattr(self, "active_fixture_launcher", None)
        if active is None:
            return ""
        child = active["process"]
        values = {"launcher_status": child.poll(),
                  "group": self.event_group_members(child.pid),
                  "stdout": self.fixture_output_tail(active["output"]),
                  "stderr": self.fixture_output_tail(active["error"]),
                  "application_bootstrap": self.fixture_output_tail(
                      self.home / "data" / "logs" / "application-bootstrap.log"),
                  "trace": self.fixture_output_tail(self.root / "trace")}
        return json.dumps(values, sort_keys=True)

    @staticmethod
    def event_group_members(group):
        # Failure-only diagnostics contain no environment or argument values.
        # Bound retained output while identifying a real leftover or a zombie.
        members = []
        for directory in Path("/proc").iterdir():
            if not directory.name.isdigit():
                continue
            try:
                stat = (directory / "stat").read_text()
                fields = stat.rsplit(")", 1)[1].split()
                if int(fields[2]) != group:
                    continue
                members.append({"pid": int(directory.name), "birth": fields[19],
                                "parent": int(fields[1]), "state": fields[0],
                                "command": stat[stat.find("(") + 1:stat.rfind(")")]})
            except (OSError, IndexError, ValueError):
                continue
            if len(members) == 8:
                break
        return members

    def trace(self):
        path = self.root / "trace"
        return path.read_text().splitlines() if path.exists() else []

    def install_path_failure(self, command, record, remove_before_failure=False):
        original = shutil.which(command, path=self.env["PATH"])
        self.assertIsNotNone(original, "The isolated fixture requires " + command + ".")
        armed = self.root / "file-fault-armed"
        self.env.update(
            TEST_FILE_FAULT_ARMED=str(armed),
            TEST_FILE_FAULT_TARGET=str(self.root / "lock" / record),
            TEST_FILE_FAULT_REAL=original,
            TEST_FILE_FAULT_REMOVE_FIRST="1" if remove_before_failure else "0",
            TEST_FILE_FAULT_TRACE=str(self.root / "file-fault-events"),
        )
        self.script(self.bin / command, textwrap.dedent(r'''
            if [ -e "$TEST_FILE_FAULT_ARMED" ]; then
                for argument in "$@"; do
                    if [ "$argument" = "$TEST_FILE_FAULT_TARGET" ]; then
                        printf '%s\n' failed >>"$TEST_FILE_FAULT_TRACE"
                        if [ "$TEST_FILE_FAULT_REMOVE_FIRST" = 1 ]; then
                            "$TEST_FILE_FAULT_REAL" "$@"
                        fi
                        exit 74
                    fi
                done
            fi
            exec "$TEST_FILE_FAULT_REAL" "$@"
        ''').lstrip())
        self.observe_cleanup_descriptor()
        return armed

    def observe_cleanup_descriptor(self):
        self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"] = str(self.root / "cleanup-descriptor")
        source = self.launcher.read_text()
        anchor = '    if ! rm -f "$LOCK_DIR/pid"'
        self.assertIn(anchor, source)
        probe = (
            '    if [ -e "/proc/$$/fd/3" ]; then\n'
            '        printf "%s\\n" open >"$TEST_CLEANUP_DESCRIPTOR_TRACE"\n'
            '    else\n'
            '        printf "%s\\n" closed >"$TEST_CLEANUP_DESCRIPTOR_TRACE"\n'
            '    fi\n'
        )
        self.launcher.write_text(source.replace(anchor, probe + anchor, 1))

    @staticmethod
    def observed_identity(pid):
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return {"pid": int(pid), "birth": fields[19]}

    def wait_for_cleanup_fixture(self):
        directory = self.root / "lock"
        self.wait_until(
            lambda: self.managed_pids()
            and (directory / "application-owned").exists()
            and (directory / "state-open").is_file()
            and (directory / "observer-pid").exists()
            and (directory / "observer-pid").read_text().strip().isdigit()
        )
        return [
            self.observed_identity(self.managed_pids()[0]),
            self.observed_identity(int((directory / "observer-pid").read_text())),
        ]

    def assert_cleanup_resources_restored(self, awesome, identities, error, native_controls_blocked=False):
        self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
        self.assertIsNone(awesome.poll())
        if native_controls_blocked:
            # An intentionally broken private command record prevents releasing
            # any CLI. It cannot be treated as confirmation of system control.
            self.assertNotIn("start:statusbar", self.trace())
            self.assertNotIn("xrefresh:", self.trace())
            self.assertTrue("native theme command records" in error
                            or "native theme permission publication" in error, error)
        else:
            self.assertIn("start:statusbar", self.trace())
            self.assertIn("xrefresh:", self.trace())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        self.wait_until(lambda: all(not self.native_theme_identity_is_alive(entry) for entry in identities))
        self.assert_native_theme_helpers_finished()
        self.assertIn("Cleanup failed", error)
        self.assertLessEqual(error.count("Cleanup failed"), 12)
        self.assertLess(len(error.encode()), 8192)

    def assert_cleanup_record_failure(self, record, original_status=143, remove_before_failure=True):
        exit_gate = self.root / "application-exit-gate"
        awesome = self.native_dark_fixture(
            TEST_NATIVE_THEME_EXIT="hold" if original_status == 143 else "ordinary",
            TEST_NATIVE_THEME_EXIT_STATUS=str(original_status),
            TEST_NATIVE_THEME_EXIT_GATE=str(exit_gate),
        )
        armed = self.install_path_failure("rm", record, remove_before_failure)
        observed = []

        def drive(child):
            observed.extend(self.wait_for_cleanup_fixture())
            armed.touch()
            if original_status == 143:
                child.send_signal(signal.SIGTERM)
            else:
                exit_gate.touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, original_status or 1, error)
        self.assert_cleanup_resources_restored(awesome, observed, error)
        self.assertTrue(Path(self.env["TEST_FILE_FAULT_TRACE"]).exists())
        if remove_before_failure:
            self.assertFalse((self.root / "lock").exists())
        else:
            self.assertTrue((self.root / "lock" / record).is_file())

    def observe_theme_permission_helpers(self):
        self.env["TEST_THEME_PERMISSION_HELPERS"] = str(self.root / "permission-helpers")
        source = self.launcher.read_text()
        anchor = '            if [ "$native_theme_released" -eq 0 ]; then\n'
        self.assertIn(anchor, source)
        self.launcher.write_text(source.replace(
            anchor,
            anchor + '                if [ -e "$TEST_FILE_FAULT_ARMED" ]; then\n'
            '                    for fixture_stat in /proc/[0-9]*/stat; do\n'
            '                        IFS= read -r fixture_identity <"$fixture_stat" || continue\n'
            '                        fixture_pid=${fixture_identity%% *}\n'
            '                        fixture_tail=${fixture_identity##*) }\n'
            '                        set -- $fixture_tail\n'
            '                        [ "$#" -ge 20 ] && [ "$3" = "$NATIVE_THEME_COMMAND_PID" ] || continue\n'
            '                        shift 19\n'
            '                        printf "%s|%s\\n" "$fixture_pid" "$1" >>"$TEST_THEME_PERMISSION_HELPERS"\n'
            '                    done\n'
            '                fi\n',
            1,
        ))

    def assert_theme_permission_helpers_stopped(self):
        path = Path(self.env["TEST_THEME_PERMISSION_HELPERS"])
        self.assertTrue(path.exists(), "The permission-failure fixture did not observe a real private helper.")
        identities = [
            {"pid": int(row.split("|", 1)[0]), "birth": row.split("|", 1)[1]}
            for row in path.read_text().splitlines()
        ]
        self.assertTrue(identities)
        self.wait_until(lambda: all(not self.native_theme_identity_is_alive(entry) for entry in identities))

    def starts(self):
        path = self.root / "starts"
        return path.read_text().splitlines() if path.exists() else []

    def application_trace(self):
        path = self.root / "application-trace"
        return path.read_text().splitlines() if path.exists() else []

    def native_starts(self):
        path = self.root / "native-starts"
        return path.read_text().splitlines() if path.exists() else []

    def native_queries(self):
        path = self.root / "native-queries"
        return path.read_text().splitlines() if path.exists() else []

    def native_window_polls(self):
        path = self.root / "native-window-polls"
        return path.read_text().splitlines() if path.exists() else []

    def native_fixture(self, kind):
        (self.home / "data" / "device-profile").write_text("scribe\n")
        (self.home / "data" / "device-allowlist").write_text("scribe 5.19.5\n")
        (self.root / "firmware").write_text("Kindle 5.19.5 (fixture)\n")
        self.env["TEST_NATIVE_KIND"] = kind

    def wait_for_native_open(self, kind):
        target = "pillow-wifi-wizard-dialog" if kind == "wifi" else "com.lab126.kppQuickSettings"
        path = self.root / "lock" / "native-current"
        self.wait_until(lambda: path.exists() and target in path.read_text())

    def set_native_visibility(self, visible):
        path = self.root / "native-visible"
        pending = path.with_suffix(".pending")
        pending.write_text(f"{visible}\n")
        pending.replace(path)

    def close_native(self):
        self.set_native_visibility("none")

    def assert_visible_native_avoids_navigation_queries(self, kind):
        self.native_fixture(kind)

        def drive(_):
            self.wait_for_native_open(kind)
            baseline_queries = self.native_queries()
            self.assertEqual(
                baseline_queries,
                ["activeApp|none|KPP_HOME", "activeView|none|KPP_HOME"],
            )
            first_poll_count = len(self.native_window_polls())
            self.wait_until(lambda: len(self.native_window_polls()) >= first_poll_count + 8)
            self.assertEqual(self.native_queries(), baseline_queries)
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_starts(), ["0|0|closed", "1|1|closed"])
        closed_queries = self.native_queries()[2:]
        # Five observer confirmations precede two final ownership rechecks.
        self.assertGreaterEqual(closed_queries.count("activeApp|none|KPP_HOME"), 7)
        self.assertGreaterEqual(closed_queries.count("activeView|none|KPP_HOME"), 7)
        self.assertTrue(all("|none|" in query for query in closed_queries))

    def assert_native_open_failure_preserves_stock(self, kind, expected_visible="none"):
        self.native_fixture(kind)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1, error)
        self.assertIn("native display and input ownership are preserved", error)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.native_starts(), ["0|0|closed"])
        native_calls = [item for item in self.trace() if item.startswith("native:")]
        if kind == "wifi":
            expected_open = 'native:-s com.lab126.pillow customDialog {"name":"wifi_wizard_dialog","clientParams":{"show":true}}'
        else:
            expected_open = "native:-s com.lab126.kppchromebar configurePillState openedByTap"
        self.assertEqual(native_calls, [expected_open])
        self.assertFalse(any("configurePillState closed" in item for item in self.trace()))
        self.assertFalse(any('"dismiss":true' in item for item in self.trace()))
        self.assertNotIn("native-concurrent-application", self.trace())
        self.assertEqual(self.trace().count("stop:statusbar"), 1)
        self.assertEqual(self.trace().count("start:statusbar"), 1)
        self.assertEqual(self.trace().count("xrefresh:"), 1)
        self.assertEqual((self.root / "native-visible").read_text().strip(), expected_visible)
        self.assertFalse((self.root / "lock").exists())

    def wait_until(self, predicate, require_launcher_running=False, details=None):
        deadline = time.monotonic() + 8
        while not predicate():
            active = getattr(self, "active_fixture_launcher", None)
            if (require_launcher_running and active is not None
                    and active["process"].poll() is not None):
                self.fail("The fixture launcher exited before the expected lifecycle state. "
                          + self.fixture_wait_details())
            if time.monotonic() >= deadline:
                detail = details() if details is not None else self.fixture_wait_details()
                self.fail("The fixture did not reach the expected lifecycle state. " + detail)
            time.sleep(0.01)

    def wait_for_fixture_process_ready(self, child, ready, error_path, label):
        original = self.observed_identity(child.pid)

        def detail():
            values = {"fixture": label, "pid": original["pid"], "birth": original["birth"],
                      "status": child.poll(), "stderr": self.fixture_output_tail(error_path)}
            if self.original_process_exists(original):
                fields = Path(f"/proc/{child.pid}/stat").read_text().rsplit(")", 1)[1].split()
                values.update(state=fields[0], parent=fields[1], group=fields[2])
            return json.dumps(values, sort_keys=True)

        def started():
            if child.poll() is not None:
                self.fail("The fixture process exited before publishing readiness. " + detail())
            self.assertTrue(self.original_process_exists(original),
                            "The original fixture process identity changed before readiness.")
            return ready.is_file()

        self.wait_until(started, details=detail)
        return original

    def emit(self, *events):
        payload = ("\n".join(events) + "\n").encode()
        self.assertLessEqual(len(payload), 4096, "A fixture event batch must fit in PIPE_BUF.")
        deadline = time.monotonic() + 8
        channels = {"power-events" if event.startswith("_weread_") else "observer-events"
                    for event in events}
        self.assertEqual(len(channels), 1, "An atomic fixture batch must use one event channel.")
        # Firmware events must pass through the owned observer's interrupt and
        # epoch publication. App and timer hints still enter the main FIFO.
        path = self.root / "lock" / channels.pop()
        while True:
            try:
                descriptor = os.open(path, os.O_WRONLY | os.O_NONBLOCK)
                break
            except OSError as error:
                if error.errno not in (errno.ENOENT, errno.ENXIO, errno.EINTR):
                    raise
                if time.monotonic() >= deadline:
                    self.fail("The fixture power-event channel has no reader.")
                time.sleep(0.01)
        try:
            while True:
                try:
                    written = os.write(descriptor, payload)
                    self.assertEqual(written, len(payload), "A fixture event batch must be atomic.")
                    break
                except (BlockingIOError, InterruptedError):
                    if time.monotonic() >= deadline:
                        self.fail("The fixture power-event channel did not accept its event batch.")
                    time.sleep(0.01)
                except BrokenPipeError:
                    self.fail("The fixture power-event channel closed before its event batch.")
        finally:
            os.close(descriptor)

    @staticmethod
    def original_process_exists(identity):
        try:
            fields = Path(f"/proc/{identity['pid']}/stat").read_text().rsplit(")", 1)[1].split()
            # A zombie with the original birth still needs its parent's wait.
            return fields[19] == identity["birth"]
        except (OSError, IndexError):
            return False

    def wake_probe_fixture(self, term_ignoring=False, preserve_delay=False):
        awesome = self.native_dark_fixture(
            profile="pw6", firmware="5.20.1",
            TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="",
        )
        self.wake_probe_observed = []
        self.wake_probe_identity_path = self.root / "wake-sleep-identity"
        self.wake_probe_signal_path = self.root / "wake-sleep-signals"
        self.env.update(
            TEST_WAKE_SLEEP_IDENTITY=str(self.wake_probe_identity_path),
            TEST_WAKE_SLEEP_SIGNALS=str(self.wake_probe_signal_path),
        )
        self.wake_probe_started_arguments = None
        self.wake_probe_sleep_arguments = {b"2", b"10"} if preserve_delay else {b"30"}
        self.addCleanup(self.cleanup_wake_probe_fixture)
        source = self.launcher.read_text()
        anchor = '    sleep "$2" 3>&- &\n'
        self.assertEqual(source.count(anchor), 1)
        if term_ignoring:
            helper = self.root / "wake-sleep.py"
            helper.write_text(textwrap.dedent(r'''
                import json
                import os
                from pathlib import Path
                import signal
                import time

                identity_path = Path(os.environ["TEST_WAKE_SLEEP_IDENTITY"])
                signals_path = Path(os.environ["TEST_WAKE_SLEEP_SIGNALS"])
                def received(number, _frame):
                    with signals_path.open("a") as output:
                        output.write(str(number) + "\n")
                for number in (signal.SIGTERM, signal.SIGCONT, signal.SIGINT, signal.SIGHUP):
                    signal.signal(number, received)
                pid = os.getpid()
                parent = os.getppid()
                fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
                parent_fields = Path(f"/proc/{parent}/stat").read_text().rsplit(")", 1)[1].split()
                try:
                    os.readlink("/proc/self/fd/3")
                    fifo_open = True
                except FileNotFoundError:
                    fifo_open = False
                identity = {"pid": pid, "birth": fields[19], "parent": parent,
                            "wrapper_birth": parent_fields[19], "fifo_open": fifo_open}
                pending = identity_path.with_suffix(".pending")
                pending.write_text(json.dumps(identity))
                pending.replace(identity_path)
                # The cleanup path must escalate; this process never forks.
                while True:
                    time.sleep(0.02)
            ''').lstrip())
            replacement = (
                f'    {shlex.quote(sys.executable)} {shlex.quote(str(helper))} 3>&- &\n'
            )
            self.wake_probe_started_arguments = [os.fsencode(sys.executable), os.fsencode(helper)]
        else:
            # Preserve the production sleep command and descriptor redirection.
            # A longer fixture delay leaves time for deterministic STOP/fence tests.
            replacement = anchor if preserve_delay else '    set -- "$1" 30\n' + anchor
        self.launcher.write_text(source.replace(anchor, replacement, 1))
        self.observe_wake_probe_barriers()
        return awesome

    def cleanup_wake_probe_fixture(self):
        identities = list(getattr(self, "wake_probe_observed", []))
        path = getattr(self, "wake_probe_identity_path", None)
        if path is not None and path.exists():
            record = json.loads(path.read_text())
            identities.extend([
                {"pid": record["parent"], "birth": record["wrapper_birth"]},
                {"pid": record["pid"], "birth": record["birth"]},
            ])
        for identity in identities:
            if identity["pid"] <= 1 or not self.original_process_exists(identity):
                continue
            for number in (signal.SIGCONT, signal.SIGTERM):
                if self.original_process_exists(identity):
                    try:
                        os.kill(identity["pid"], number)
                    except ProcessLookupError:
                        pass
        deadline = time.monotonic() + 0.3
        while any(self.original_process_exists(entry) for entry in identities) and time.monotonic() < deadline:
            time.sleep(0.01)
        for identity in reversed(identities):
            if identity["pid"] > 1 and self.original_process_exists(identity):
                try:
                    os.kill(identity["pid"], signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def read_wake_probe_record(self, path=None):
        try:
            values = (path or self.root / "lock" / "wake-probe-child").read_text().strip().split(":")
        except OSError:
            return None
        if len(values) != 6 or not all(values[index].isdigit() for index in (0, 2, 3, 4, 5)):
            return None
        return {
            "cycle": values[0], "token": values[1],
            "wrapper": {"pid": int(values[2]), "birth": values[3]},
            "child": {"pid": int(values[4]), "birth": values[5]},
        }

    def wait_for_wake_probe(self, supervisor):
        self.wait_until(lambda: self.read_wake_probe_record() is not None)
        record = self.read_wake_probe_record()
        self.assertIsNotNone(record)
        wrapper, child = record["wrapper"], record["child"]
        for identity, parent in ((wrapper, supervisor.pid), (child, wrapper["pid"])):
            fields = Path(f"/proc/{identity['pid']}/stat").read_text().rsplit(")", 1)[1].split()
            self.assertEqual(fields[19], identity["birth"])
            self.assertEqual(int(fields[1]), parent)
            self.wake_probe_observed.append(identity.copy())
        self.assertEqual(record["token"], (self.root / "lock" / "nonce").read_text().strip())
        # The wrapper can publish a birth before its fork is scheduled. That
        # original child must finish its output redirection and exec before an
        # inherited-descriptor assertion or a STOP fence can inspect it.
        def child_started():
            self.assertTrue(self.original_process_exists(child),
                            "The original wake child exited before publishing startup proof.")
            try:
                fields = Path(f"/proc/{child['pid']}/stat").read_text().rsplit(")", 1)[1].split()
                arguments = [value for value in Path(f"/proc/{child['pid']}/cmdline").read_bytes().split(b"\0")
                             if value]
            except (OSError, IndexError):
                return False
            self.assertEqual(fields[19], child["birth"])
            self.assertEqual(int(fields[1]), wrapper["pid"])
            self.assertEqual(int(fields[2]), supervisor.pid)
            self.assertNotIn(fields[0], ("Z", "X"))
            try:
                os.readlink(f"/proc/{child['pid']}/fd/3")
            except FileNotFoundError:
                pass
            else:
                return False
            expected = self.wake_probe_started_arguments
            if expected is not None:
                if arguments != expected or not self.wake_probe_identity_path.is_file():
                    return False
                observed = json.loads(self.wake_probe_identity_path.read_text())
                self.assertEqual((observed["pid"], observed["birth"]), (child["pid"], child["birth"]))
                self.assertEqual((observed["parent"], observed["wrapper_birth"]),
                                 (wrapper["pid"], wrapper["birth"]))
                self.assertFalse(observed["fifo_open"])
                return True
            return (len(arguments) == 2 and Path(os.fsdecode(arguments[0])).name == "sleep"
                    and arguments[1] in self.wake_probe_sleep_arguments)

        self.wait_until(child_started)
        with self.assertRaises(FileNotFoundError):
            os.readlink(f"/proc/{child['pid']}/fd/3")
        # A published record can precede the parent's synchronous admission.
        # Consume a later FIFO item before manipulating the child or wrapper.
        barrier = f"_weread_fixture_wake_barrier admitted-{len(self.wake_probe_observed)}"
        self.emit(barrier)
        barrier_path = self.root / "wake-probe-barriers"
        self.wait_until(lambda: barrier_path.exists() and barrier in barrier_path.read_text().splitlines())
        self.assertEqual(self.read_wake_probe_record(), record)
        return record

    def capture_wake_fixture_child(self):
        self.wait_until(self.wake_probe_identity_path.exists)
        record = json.loads(self.wake_probe_identity_path.read_text())
        self.assertFalse(record["fifo_open"], "The wake delay inherited the power FIFO descriptor.")
        self.wake_probe_observed.extend([
            {"pid": record["parent"], "birth": record["wrapper_birth"]},
            {"pid": record["pid"], "birth": record["birth"]},
        ])
        return record

    def assert_wake_probe_reaped(self):
        self.assertTrue(self.wake_probe_observed)
        self.wait_until(lambda: all(not self.original_process_exists(entry) for entry in self.wake_probe_observed))
        self.assertFalse((self.root / "lock").exists())

    def observe_wake_probe_barriers(self):
        path = self.root / "wake-probe-barriers"
        self.env["TEST_WAKE_PROBE_BARRIERS"] = str(path)
        source = self.launcher.read_text()
        if "        '_weread_fixture_wake_barrier '*)\n" in source:
            self.assertEqual(source.count("        '_weread_fixture_wake_barrier '*)\n"), 1)
            return path
        anchor = "        '_weread_wake_check '*)\n"
        self.assertEqual(source.count(anchor), 1)
        barrier = (
            "        '_weread_fixture_wake_barrier '*)\n"
            '            printf "%s\\n" "$power_event" >>"$TEST_WAKE_PROBE_BARRIERS"\n'
            "            ;;\n"
        )
        self.launcher.write_text(source.replace(anchor, barrier + anchor, 1))
        return path

    def managed_fixture(self, **behavior):
        self.env.update(
            TEST_MANAGED_CAPABILITY="1",
            TEST_MANAGED_PREPARE="ready",
            TEST_MANAGED_WAKE="ready",
            TEST_MANAGED_EVENTS=str(self.root / "managed-events"),
            TEST_MANAGED_PIDS=str(self.root / "managed-pids"),
            TEST_NATIVE_THEME_FIXTURE="0",
        )
        self.env.update(behavior)
        # A Python event loop keeps the application alive across signals without
        # giving the fixture its own shell child that could outlive SIGSTOP.
        body = textwrap.dedent(r'''
            import os
            from pathlib import Path
            import signal
            import sys
            import time

            # The shadow binary has no trusted sensor by default. Its exact
            # metadata-only invocation precedes every managed App state read.
            if "--probe-native-portrait-framebuffer" in sys.argv[1:]:
                if (len(sys.argv) == 5
                        and sys.argv[1:4] == ["--kindle", "--probe-native-portrait-framebuffer", "--device"]
                        and sys.argv[4] in ("pw6", "scribe")):
                    print("FIXED:0")
                    sys.exit(0)
                sys.exit(74)

            context = Path(os.environ["WEREAD_LAUNCH_CONTEXT_DIR"])
            generation = os.environ["WEREAD_LAUNCH_GENERATION"]
            token = os.environ["WEREAD_LAUNCH_TOKEN"]
            base = generation + ":" + token
            resume = os.environ["WEREAD_RESUME_SESSION"]
            pid = os.getpid()
            pending = set()
            received_signals = set()
            prepared_request = ""
            previous_wake_request = ""

            def observed_process(number):
                try:
                    fields = Path("/proc/" + str(number) + "/stat").read_text().rsplit(")", 1)[1].split()
                    return fields[0], fields[19]
                except (OSError, IndexError):
                    return None

            def append(path, value):
                with Path(path).open("a") as output:
                    output.write(value + "\n")

            def record(kind, value=""):
                event = "managed:" + kind + ":" + str(pid)
                if value:
                    event += ":" + value
                append(os.environ["TEST_MANAGED_EVENTS"], event)
                append(os.environ["TEST_TRACE"], event)

            def marker(name, value):
                path = context / name
                temporary = path.with_suffix(".pending")
                temporary.write_text(value + "\n")
                temporary.replace(path)

            def read_marker(name):
                try:
                    return (context / name).read_text().strip()
                except OSError:
                    return ""

            def save_checkpoint():
                Path(os.environ["TEST_CHECKPOINT"]).write_text("book-7:3:42\n")
                if read_marker("sleep-request") == base:
                    marker("resume-ready", base)

            def received(number, _frame):
                received_signals.add(number)
                pending.add(number)

            def report_signals():
                observed = tuple(received_signals)
                record("signals", ",".join(sorted(signal.Signals(number).name for number in observed)))

            for number in (signal.SIGUSR1, signal.SIGUSR2, signal.SIGCONT,
                           signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                signal.signal(number, received)

            pid_file = Path(os.environ["TEST_MANAGED_PIDS"])
            if pid_file.exists():
                for previous in pid_file.read_text().splitlines():
                    previous_pid, previous_birth = previous.split("|", 1)
                    observed = observed_process(int(previous_pid))
                    if observed is not None and observed[0] not in ("Z", "X") and observed[1] == previous_birth:
                        record("concurrent", previous_pid)
                        sys.exit(73)
            birth = observed_process(pid)[1]
            append(pid_file, str(pid) + "|" + birth)
            append(os.environ["TEST_STARTS"], resume)
            append(os.environ["TEST_NATIVE_STARTS"], resume + "|0|closed")
            Path(os.environ["TEST_ARGS"]).write_text("\n".join(sys.argv[1:]) + "\n")
            marker("state-open", base)
            record("start", generation + ":" + resume)
            if os.environ.get("TEST_NATIVE_THEME_FIXTURE") == "1":
                record("native-snapshot", read_marker("native-epdc-mode"))
                target = os.environ.get("TEST_NATIVE_THEME_TARGET", "")
                if target:
                    scope = os.environ.get("TEST_NATIVE_THEME_TARGET_SCOPE", "current")
                    target_base = base
                    if scope == "stale-generation":
                        target_base = "0:" + token
                    elif scope == "stale-nonce":
                        different = ("1" if token[0] == "0" else "0") + token[1:]
                        target_base = generation + ":" + different
                    marker("native-dark-target", target_base + ":" + target)
                behavior = os.environ.get("TEST_NATIVE_THEME_EXIT", "hold")
                if behavior in ("ordinary", "hold-ordinary"):
                    marker("ordinary-exit", base)
                if behavior == "ordinary":
                    record("ordinary-exit")
                    exit_gate = os.environ.get("TEST_NATIVE_THEME_EXIT_GATE", "")
                    while exit_gate and not Path(exit_gate).exists():
                        time.sleep(0.01)
                    sys.exit(int(os.environ.get("TEST_NATIVE_THEME_EXIT_STATUS", "0")))
            if os.environ["TEST_MANAGED_CAPABILITY"] == "1":
                marker("managed-sleep-available", base)
            if generation != "1":
                if resume != "1" or not Path(os.environ["TEST_CHECKPOINT"]).exists():
                    record("cold-without-checkpoint")
                    sys.exit(9)
                record("cold-restored", Path(os.environ["TEST_CHECKPOINT"]).read_text().strip())
                sys.exit(0)

            while True:
                if pending.intersection((signal.SIGTERM, signal.SIGINT, signal.SIGHUP)):
                    save_checkpoint()
                    report_signals()
                    record("term")
                    sys.exit(0)
                if signal.SIGCONT in pending:
                    pending.discard(signal.SIGCONT)
                    record("continued")
                if signal.SIGUSR1 in pending:
                    pending.discard(signal.SIGUSR1)
                    request = read_marker("managed-sleep-request")
                    record("prepare", request)
                    previous_prepare = prepared_request
                    prepared_request = request
                    save_checkpoint()
                    mode = os.environ["TEST_MANAGED_PREPARE"]
                    if mode == "ordinary":
                        marker("ordinary-exit", base)
                        report_signals()
                        record("ordinary-exit")
                        sys.exit(0)
                    value = request
                    if mode == "stale-token":
                        fields = request.split(":")
                        fields[1] = ("1" if fields[1][0] == "0" else "0") + fields[1][1:]
                        value = ":".join(fields)
                    elif mode == "stale-cycle" and previous_prepare:
                        value = previous_prepare
                    if mode != "none":
                        marker("managed-sleep-ready", value)
                        record("sleep-ready", value)
                if signal.SIGUSR2 in pending:
                    pending.discard(signal.SIGUSR2)
                    request = read_marker("managed-wake-request")
                    record("wake", request)
                    if os.environ.get("TEST_NATIVE_THEME_FIXTURE") == "1":
                        record("native-resume-snapshot", read_marker("managed-native-epdc-mode"))
                    if request != prepared_request:
                        record("wake-mismatch", request)
                    else:
                        mode = os.environ["TEST_MANAGED_WAKE"]
                        if mode != "none":
                            value = previous_wake_request if mode == "stale-cycle" and previous_wake_request else request
                            marker("managed-wake-ready", value)
                            record("wake-ready", value)
                            previous_wake_request = request
                time.sleep(0.01)
        ''').lstrip()
        path = self.home / "app" / "weread"
        path.write_text("#!/usr/bin/env python3\n" + body)
        path.chmod(0o755)

    def native_dark_fixture(self, mode="Y8", stopped=False, profile="scribe", firmware="5.19.5", **behavior):
        self.managed_fixture()
        self.env.update(
            TEST_NATIVE_THEME_FIXTURE="1",
            TEST_NATIVE_THEME_EXIT="ordinary",
            TEST_NATIVE_THEME_TARGET="1",
            TEST_NATIVE_THEME_TARGET_SCOPE="current",
            TEST_NATIVE_THEME_EXIT_STATUS="0",
            TEST_NATIVE_THEME_MODE=str(self.root / "native-epdc-value"),
            TEST_NATIVE_THEME_PREF=str(self.root / "native-inverse-pref"),
            TEST_NATIVE_THEME_POWER=str(self.root / "native-power-state"),
            TEST_NATIVE_THEME_MODE_GETS=str(self.root / "native-mode-gets"),
            TEST_NATIVE_THEME_IDENTITIES=str(self.root / "native-helper-identities"),
            TEST_NATIVE_THEME_GET_TIMEOUT="0",
            TEST_NATIVE_THEME_SET_TIMEOUT="0",
            TEST_NATIVE_THEME_READBACK_MISMATCH="0",
            TEST_NATIVE_THEME_READBACK_VALUE="",
            TEST_NATIVE_THEME_PREF_MISMATCH="0",
            TEST_NATIVE_THEME_POWER_AFTER_MODE_GET="",
        )
        self.env.update(behavior)
        (self.home / "data" / "device-profile").write_text(profile + "\n")
        (self.home / "data" / "device-allowlist").write_text(profile + " " + firmware + "\n")
        (self.root / "firmware").write_text("Kindle " + firmware + " (fixture)\n")
        Path(self.env["TEST_NATIVE_THEME_MODE"]).write_text(mode + "\n")
        Path(self.env["TEST_NATIVE_THEME_PREF"]).write_text("Y8INV\n" if mode == "Y8INV" else "Y8\n")
        Path(self.env["TEST_NATIVE_THEME_POWER"]).write_text(self.env.get("TEST_POWER_STATE", "active") + "\n")
        Path(self.env["TEST_NATIVE_THEME_MODE_GETS"]).write_text("0\n")
        source = self.launcher.read_text()
        self.assertIn("NATIVE_THEME_TIMEOUT_TICKS=30", source)
        # Preserve a deadline while allowing an interpreter to start on a busy
        # shared worker. The initial fast polls add under 0.1 s to these ticks.
        self.launcher.write_text(source.replace("NATIVE_THEME_TIMEOUT_TICKS=30", "NATIVE_THEME_TIMEOUT_TICKS=120", 1))
        awesome_path = self.root / "fake-awesome.py"
        awesome_ready = self.root / "awesome-ready"
        awesome_error = self.root / "fake-awesome.stderr"
        awesome_ready.unlink(missing_ok=True)
        awesome_path.write_text(textwrap.dedent('''
            from pathlib import Path
            import signal
            import sys
            import time
            ready, trace = map(Path, sys.argv[1:])
            def continued(_number, _frame):
                with trace.open("a") as output:
                    output.write("wm:CONT\\n")
            signal.signal(signal.SIGCONT, continued)
            ready.touch()
            while True:
                time.sleep(0.02)
        ''').lstrip())
        with awesome_error.open("wb") as error_file:
            awesome = subprocess.Popen(
                [sys.executable, str(awesome_path), str(awesome_ready), self.env["TEST_TRACE"]],
                env=self.env,
                start_new_session=True,
                stdout=subprocess.DEVNULL,
                stderr=error_file,
            )

        def reap_awesome():
            if awesome.poll() is None:
                awesome.send_signal(signal.SIGCONT)
                awesome.kill()
            awesome.wait(timeout=3)

        self.addCleanup(reap_awesome)
        self.wait_for_fixture_process_ready(awesome, awesome_ready, awesome_error, "window manager")
        self.env["TEST_AWESOME_PID"] = str(awesome.pid)
        if stopped:
            awesome.send_signal(signal.SIGSTOP)
            self.wait_until(lambda: self.process_state(awesome.pid) == "T")
        self.script(self.bin / "pidof", '[ "${1:-}" = awesome ] || exit 1\nprintf "%s\\n" "$TEST_AWESOME_PID"')
        mock = textwrap.dedent(r'''
            import json
            import os
            from pathlib import Path
            import stat
            import subprocess
            import sys
            import time

            command = Path(sys.argv[0]).name
            arguments = sys.argv[1:]
            mode = Path(os.environ["TEST_NATIVE_THEME_MODE"])
            preference = Path(os.environ["TEST_NATIVE_THEME_PREF"])
            power = Path(os.environ["TEST_NATIVE_THEME_POWER"])
            mode_gets = Path(os.environ["TEST_NATIVE_THEME_MODE_GETS"])
            context = Path(os.environ["WEREAD_HOME"]).parent / "lock"

            def identity(pid, role):
                try:
                    fields = Path("/proc/" + str(pid) + "/stat").read_text().rsplit(")", 1)[1].split()
                    values = Path("/proc/" + str(pid) + "/cmdline").read_bytes().split(b"\0")
                    return {"pid":pid, "role":role, "birth":fields[19],
                            "group":int(fields[2]), "master":int((context / "pid").read_text()),
                            "arguments":[os.fsdecode(value) for value in values if value]}
                except (OSError, ValueError, IndexError):
                    return None

            def save_identity(pid, role):
                entry = identity(pid, role)
                if entry is not None:
                    with Path(os.environ["TEST_NATIVE_THEME_IDENTITIES"]).open("a") as output:
                        output.write(json.dumps(entry) + "\n")

            def state():
                try:
                    pid = int(os.environ["TEST_AWESOME_PID"])
                    return Path("/proc/" + str(pid) + "/stat").read_text().rsplit(")", 1)[1].split()[0]
                except (OSError, ValueError, IndexError):
                    return "missing"

            def trace(value):
                with Path(os.environ["TEST_TRACE"]).open("a") as output:
                    output.write(value + "\n")

            def observe_handoff(kind, value):
                if os.environ.get("TEST_NATIVE_THEME_HANDOFF") != "1":
                    return
                def process(number):
                    try:
                        fields = Path("/proc/" + str(number) + "/stat").read_text().rsplit(")", 1)[1].split()
                        return {"pid": int(number), "state": fields[0], "birth": fields[19]}
                    except (OSError, ValueError, IndexError):
                        return None
                try:
                    app = process((context / "application-pid").read_text().strip())
                except OSError:
                    app = None
                rows = Path(os.environ["TEST_TRACE"]).read_text().splitlines()
                entry = {"kind": kind, "value": value, "app": app,
                         "wm": process(os.environ["TEST_AWESOME_PID"]),
                         "statusbar_stopped": rows.count("stop:statusbar") > rows.count("start:statusbar")}
                with Path(os.environ["TEST_NATIVE_THEME_COMMAND_OBSERVATIONS"]).open("a") as output:
                    output.write(json.dumps(entry) + "\n")

            def blocker():
                # Keep the existing live group/child cleanup witness. A pipe
                # inside the new bounded capture must receive the getter's
                # natural EOF rather than an unrelated sleeping child's fd.
                stdout = subprocess.DEVNULL if stat.S_ISFIFO(os.fstat(1).st_mode) else None
                child = subprocess.Popen(["/bin/sleep", "30"], stdout=stdout)
                deadline = time.monotonic() + 1
                while True:
                    observed = identity(child.pid, "blocker")
                    if observed is not None and observed["arguments"] == ["/bin/sleep", "30"]:
                        save_identity(child.pid, "blocker")
                        return
                    if child.poll() is not None or time.monotonic() >= deadline:
                        if child.poll() is None:
                            child.kill()
                        child.wait()
                        raise SystemExit(75)
                    time.sleep(0.01)

            is_mode = arguments == ["-s", "com.lab126.winmgr", "epdcMode"]
            is_set = len(arguments) == 4 and arguments[:3] == ["-s", "com.lab126.winmgr", "epdcMode"]
            if command == "lipc-get-prop" and is_mode:
                save_identity(os.getpgrp(), "group")
                save_identity(os.getpid(), "command")
                blocker()
                value = mode.read_text().strip()
                trace("theme:get:" + value + ":wm=" + state())
                observe_handoff("get", value)
                count = int(mode_gets.read_text()) + 1
                mode_gets.write_text(str(count) + "\n")
                changed_power = os.environ["TEST_NATIVE_THEME_POWER_AFTER_MODE_GET"]
                if count >= 2 and changed_power:
                    power.write_text(changed_power + "\n")
                if os.environ["TEST_NATIVE_THEME_GET_TIMEOUT"] == "1":
                    time.sleep(30)
                print(value)
            elif command == "lipc-get-prop" and arguments == ["-s", "com.lab126.powerd", "state"]:
                save_identity(os.getpgrp(), "group")
                save_identity(os.getpid(), "command")
                blocker()
                value = power.read_text().strip()
                trace("theme:power:" + value + ":wm=" + state())
                print(value)
            elif command == "lipc-get-prop" and len(arguments) == 3 and arguments[1] == "com.lab126.appmgrd":
                print("com.lab126.KPPMainApp" if arguments[2] == "activeApp" else "KPP_HOME")
            elif command == "lipc-set-prop" and is_set:
                save_identity(os.getpgrp(), "group")
                save_identity(os.getpid(), "command")
                blocker()
                value = arguments[3]
                trace("theme:set:" + value + ":wm=" + state())
                observe_handoff("set", value)
                if os.environ["TEST_NATIVE_THEME_SET_TIMEOUT"] == "1":
                    time.sleep(30)
                readback = os.environ["TEST_NATIVE_THEME_READBACK_VALUE"]
                if readback:
                    mode.write_text(readback + "\n")
                elif os.environ["TEST_NATIVE_THEME_READBACK_MISMATCH"] != "1":
                    mode.write_text(value + "\n")
                dark = value == "Y8INV" and os.environ["TEST_NATIVE_THEME_PREF_MISMATCH"] != "1"
                preference.write_text("Y8INV\n" if dark else "Y8\n")
            elif command == "get-dynconf-value" and arguments == ["winmgr.colorinverse.pref"]:
                save_identity(os.getpgrp(), "group")
                save_identity(os.getpid(), "command")
                blocker()
                value = preference.read_text().strip()
                trace("theme:pref:" + value + ":wm=" + state())
                observe_handoff("pref", value)
                print(value)
            else:
                raise SystemExit(1)
        ''').lstrip()
        for name in ("lipc-get-prop", "lipc-set-prop", "get-dynconf-value"):
            path = self.bin / name
            path.write_text("#!/usr/bin/env python3\n" + mock)
            path.chmod(0o755)
        self.addCleanup(self.cleanup_native_theme_helpers)
        return awesome

    def set_native_portrait_inputs(self, metadata, directions):
        for name, values in (("metadata", metadata), ("direction", directions)):
            if isinstance(values, str):
                values = [values]
            path = self.root / ("portrait-" + name + "-responses")
            path.write_text(json.dumps({"position": 0, "values": list(values)}))

    def native_portrait_fixture(self):
        awesome = self.native_dark_fixture(
            TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET=""
        )
        self.env.update(
            TEST_NATIVE_PORTRAIT_PROFILE="scribe",
            TEST_NATIVE_PORTRAIT_METADATA=str(self.root / "portrait-metadata-responses"),
            TEST_NATIVE_PORTRAIT_DIRECTION=str(self.root / "portrait-direction-responses"),
            TEST_NATIVE_PORTRAIT_TRACE=str(self.root / "portrait-commands"),
        )
        self.set_native_portrait_inputs("AUTO:3\n", "U\n")
        # Only the four exact native-probe arguments take this early path. In
        # particular, an ordinary managed app still uses the existing signals,
        # owner records and checkpoint fixture; probe processes start no reader.
        prefix = textwrap.dedent(r'''
            import json
            import os
            from pathlib import Path
            import sys

            def portrait_next_response(variable):
                path = Path(os.environ[variable])
                response = json.loads(path.read_text())
                values = response["values"]
                if not values:
                    raise SystemExit(74)
                position = response["position"]
                value = values[min(position, len(values) - 1)]
                response["position"] = position + 1
                path.write_text(json.dumps(response))
                return value

            def portrait_command(kind, value):
                group = os.getpgrp()
                group_fields = Path("/proc/" + str(group) + "/stat").read_text().rsplit(")", 1)[1].split()
                wm_fields = Path("/proc/" + os.environ["TEST_AWESOME_PID"] + "/stat").read_text().rsplit(")", 1)[1].split()
                context = Path(os.environ["WEREAD_HOME"]).parent / "lock"
                event = {
                    "kind": kind, "value": value, "arguments": sys.argv[1:],
                    "group": group, "group_birth": group_fields[19],
                    "wm_state": wm_fields[0], "master": int((context / "pid").read_text()),
                }
                with Path(os.environ["TEST_NATIVE_PORTRAIT_TRACE"]).open("a") as output:
                    output.write(json.dumps(event) + "\n")

            probe_arguments = ["--kindle", "--probe-native-portrait-framebuffer", "--device",
                               os.environ["TEST_NATIVE_PORTRAIT_PROFILE"]]
            if "--probe-native-portrait-framebuffer" in sys.argv[1:]:
                if sys.argv[1:] != probe_arguments:
                    raise SystemExit(74)
                value = portrait_next_response("TEST_NATIVE_PORTRAIT_METADATA")
                portrait_command("metadata", value)
                sys.stdout.write(value)
                raise SystemExit(0)
            if Path(sys.argv[0]).name == "lipc-get-prop":
                if sys.argv[1:] == ["com.lab126.winmgr", "accelerometer"]:
                    value = portrait_next_response("TEST_NATIVE_PORTRAIT_DIRECTION")
                    portrait_command("direction", value)
                    sys.stdout.write(value)
                    raise SystemExit(0)
                if sys.argv[1:] == ["-s", "com.lab126.powerd", "state"]:
                    # The original native-dark fixture still produces the real
                    # response and owns its helper children. This records its
                    # current value and group for the complete capture order.
                    portrait_command("power", Path(os.environ["TEST_NATIVE_THEME_POWER"]).read_text())
        ''').lstrip()
        app = self.home / "app" / "weread"
        app_source = app.read_text()
        startup = '    record("native-snapshot", read_marker("native-epdc-mode"))\n'
        wake = '            record("native-resume-snapshot", read_marker("managed-native-epdc-mode"))\n'
        self.assertIn(startup, app_source)
        self.assertIn(wake, app_source)
        app_source = app_source.replace(
            startup, startup + '    record("portrait-snapshot", read_marker("native-portrait-calibration"))\n', 1
        ).replace(
            wake, wake + '            record("portrait-resume-snapshot", read_marker("managed-native-portrait-calibration"))\n', 1
        )
        for path, source in ((app, app_source), (self.bin / "lipc-get-prop", (self.bin / "lipc-get-prop").read_text())):
            shebang, body = source.split("\n", 1)
            path.write_text(shebang + "\n" + prefix + body)
            path.chmod(0o755)
        return awesome

    def native_portrait_commands(self):
        path = self.root / "portrait-commands"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def native_portrait_capture_groups(self):
        groups = []
        for entry in self.native_portrait_commands():
            if entry["kind"] == "metadata":
                key = (entry["group"], entry["group_birth"])
                if key not in groups:
                    groups.append(key)
        return groups

    def assert_native_portrait_capture(self, group, metadata, directions):
        entries = [entry for entry in self.native_portrait_commands()
                   if (entry["group"], entry["group_birth"]) == group]
        self.assertEqual([entry["kind"] for entry in entries],
                         ["power", "metadata", "direction", "metadata", "direction", "power"])
        self.assertEqual([entry["value"] for entry in entries if entry["kind"] == "metadata"], metadata)
        self.assertEqual([entry["value"] for entry in entries if entry["kind"] == "direction"], directions)
        self.assertEqual([entry["value"] for entry in entries if entry["kind"] == "power"], ["active\n", "active\n"])
        self.assertEqual([entry["arguments"] for entry in entries if entry["kind"] == "metadata"],
                         [["--kindle", "--probe-native-portrait-framebuffer", "--device", "scribe"]] * 2)
        self.assertEqual([entry["arguments"] for entry in entries if entry["kind"] == "direction"],
                         [["com.lab126.winmgr", "accelerometer"]] * 2)
        self.assertTrue(all(entry["wm_state"] not in ("T", "t", "Z", "X") for entry in entries))
        master = int((self.root / "lock" / "pid").read_text())
        self.assertTrue(all(entry["master"] == master for entry in entries))
        self.assertNotEqual(group[0], master)

    def test_native_portrait_capture_publishes_a_stable_owned_pair_before_app_start(self):
        awesome = self.native_portrait_fixture()
        observed = []

        def drive(_):
            application = self.wait_for_managed_application()
            directory = self.root / "lock"
            token = (directory / "nonce").read_text().strip()
            base = "1:" + token
            self.assertEqual((directory / "generation").read_text().strip(), base)
            record = directory / "native-portrait-calibration"
            self.assertEqual(record.read_text().strip(), base + ":U:3")
            self.assertEqual(record.stat().st_uid, os.geteuid())
            self.assertEqual(record.stat().st_mode & 0o077, 0)
            self.assertFalse(record.with_suffix(".pending").exists())
            self.assertEqual(self.native_theme_snapshots("portrait-snapshot"), [base + ":U:3"])
            groups = self.native_portrait_capture_groups()
            self.assertEqual(len(groups), 1)
            self.assert_native_portrait_capture(groups[0], ["AUTO:3\n", "AUTO:3\n"], ["U\n", "U\n"])
            self.assertNotEqual(groups[0][0], application)
            observed.append(application)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.managed_pids(), observed)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_portrait_capture_scopes_each_resident_wake_without_replacing_startup(self):
        awesome = self.native_portrait_fixture()
        expected = []

        def drive(_):
            application = self.wait_for_managed_application()
            directory = self.root / "lock"
            initial = (directory / "native-portrait-calibration").read_text()
            for cycle, direction, rotation in ((1, "D", 1), (2, "U", 3)):
                request = self.suspend_managed_application(application)
                self.assertTrue(request.endswith(":" + str(cycle)))
                self.assertFalse((directory / "managed-native-portrait-calibration").exists())
                self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
                self.set_native_portrait_inputs(f"AUTO:{rotation}\n", direction + "\n")
                self.wake_managed_application(application, request)
                value = request + f":{direction}:{rotation}"
                self.assertEqual((directory / "managed-native-portrait-calibration").read_text().strip(), value)
                self.assertEqual((directory / "native-portrait-calibration").read_text(), initial)
                self.assertFalse((directory / "managed-native-portrait-calibration.pending").exists())
                expected.append(value)
            groups = self.native_portrait_capture_groups()
            self.assertEqual(len(groups), 3)
            for group, direction, rotation in zip(groups, ("U", "D", "U"), (3, 1, 3)):
                self.assert_native_portrait_capture(group, [f"AUTO:{rotation}\n"] * 2, [direction + "\n"] * 2)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.native_theme_snapshots("portrait-resume-snapshot"), expected)
        self.assertEqual(len(self.managed_pids()), 1)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse(any(":concurrent:" in event or ":cold-restored:" in event for event in self.managed_events()))
        self.assertEqual(self.native_theme_events("set"), [])
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_portrait_capture_rejected_pairs_clear_old_cycle_and_keep_resident_reader(self):
        awesome = self.native_portrait_fixture()
        expected = []
        cases = (
            ("fixed", ["FIXED:1\n"], [], "FIXED:1", ["power", "metadata", "metadata", "power"]),
            ("malformed", ["AUTO:3\nextra\n"], [], None, ["power", "metadata"]),
            ("metadata-mismatch", ["AUTO:1\n", "AUTO:3\n"], ["D\n"], None,
             ["power", "metadata", "direction", "metadata"]),
            ("direction-mismatch", ["AUTO:3\n"], ["U\n", "D\n"], None,
             ["power", "metadata", "direction", "metadata", "direction"]),
        )

        def drive(_):
            application = self.wait_for_managed_application()
            directory = self.root / "lock"
            initial = (directory / "native-portrait-calibration").read_text()
            first = self.suspend_managed_application(application)
            self.set_native_portrait_inputs("AUTO:1\n", "D\n")
            self.wake_managed_application(application, first)
            self.assertEqual((directory / "managed-native-portrait-calibration").read_text().strip(), first + ":D:1")
            expected.append(first + ":D:1")
            for name, metadata, directions, payload, kinds in cases:
                request = self.suspend_managed_application(application)
                record = directory / "managed-native-portrait-calibration"
                self.assertFalse(record.exists(), name)
                self.set_native_portrait_inputs(metadata, directions)
                self.wake_managed_application(application, request)
                if payload is None:
                    self.assertFalse(record.exists(), name)
                    expected.append("")
                else:
                    self.assertEqual(record.read_text().strip(), request + ":" + payload, name)
                    expected.append(request + ":" + payload)
                self.assertEqual((directory / "native-portrait-calibration").read_text(), initial, name)
                self.assertFalse((directory / "managed-native-portrait-calibration.pending").exists(), name)
                group = self.native_portrait_capture_groups()[-1]
                entries = [entry for entry in self.native_portrait_commands()
                           if (entry["group"], entry["group_birth"]) == group]
                self.assertEqual([entry["kind"] for entry in entries], kinds, name)
                self.assertTrue(all(entry["wm_state"] not in ("T", "t", "Z", "X") for entry in entries), name)
            self.assertEqual((self.root / "reader-checkpoint").read_text(), "book-7:3:42\n")
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.native_theme_snapshots("portrait-resume-snapshot"), expected)
        self.assertEqual(len(self.managed_pids()), 1)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse(any(":concurrent:" in event or ":cold-restored:" in event for event in self.managed_events()))
        self.assertEqual(self.native_theme_events("set"), [])
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def native_theme_identities(self):
        path = self.root / "native-helper-identities"
        if not path.exists():
            return []
        entries = []
        for row in path.read_text().splitlines():
            try:
                entries.append(json.loads(row))
            except json.JSONDecodeError:
                continue
        return entries

    @staticmethod
    def native_theme_identity_is_alive(entry):
        try:
            fields = Path(f"/proc/{entry['pid']}/stat").read_text().rsplit(")", 1)[1].split()
            return fields[0] not in ("Z", "X") and fields[19] == entry["birth"]
        except (OSError, IndexError):
            return False

    def cleanup_native_theme_helpers(self):
        for entry in reversed(self.native_theme_identities()):
            if entry["pid"] <= 1 or not self.native_theme_identity_is_alive(entry):
                continue
            try:
                if entry["role"] == "group" and entry["pid"] == entry["group"] and entry["group"] != entry["master"]:
                    # The group leader may exec its final waiting command.
                    # Birth and group identity remain authoritative across exec.
                    if os.getpgid(entry["pid"]) == entry["group"]:
                        os.killpg(entry["group"], signal.SIGKILL)
                else:
                    arguments = tuple(value for value in Path(f"/proc/{entry['pid']}/cmdline").read_bytes().split(b"\0") if value)
                    if arguments == tuple(os.fsencode(value) for value in entry["arguments"]):
                        os.kill(entry["pid"], signal.SIGKILL)
            except OSError:
                pass

    def assert_native_theme_helpers_finished(self):
        entries = self.native_theme_identities()
        self.assertTrue(entries, "The native-property fixture must observe real helper groups.")
        try:
            self.wait_until(lambda: all(not self.native_theme_identity_is_alive(entry) for entry in entries))
        except AssertionError:
            remaining = []
            for entry in entries:
                if not self.native_theme_identity_is_alive(entry):
                    continue
                try:
                    fields = Path(f"/proc/{entry['pid']}/stat").read_text().rsplit(")", 1)[1].split()
                    remaining.append({
                        "pid": entry["pid"], "role": entry["role"],
                        "captured_group": entry["group"], "state": fields[0],
                        "parent": fields[1], "group": fields[2], "session": fields[3],
                    })
                except (OSError, IndexError):
                    continue
            self.fail("Native-property helpers remained alive: " + json.dumps(remaining, sort_keys=True))
        for entry in entries:
            if entry["role"] == "group":
                self.assertEqual(entry["pid"], entry["group"])
                self.assertNotEqual(entry["group"], entry["master"])

    def install_supervisor_query_fixture(self, command, behavior="blocked", gated=False, apply_before_block=False):
        self.assertIn(command, ("xwininfo", "lipc-get-prop", "status", "pidof", "start", "stop", "xrefresh"))
        self.assertIn(behavior, ("blocked", "released", "oversize", "flood", "empty"))
        original = self.root / "original-query" / command
        original.parent.mkdir(exist_ok=True)
        shutil.copy2(self.bin / command, original)
        armed = self.root / "supervisor-query-armed"
        ready = self.root / "supervisor-query-ready"
        output_bytes = self.root / "supervisor-query-output-bytes"
        self.env.update(
            TEST_SUPERVISOR_QUERY_ORIGINAL=str(original),
            TEST_SUPERVISOR_QUERY_COMMAND=command,
            TEST_SUPERVISOR_QUERY_BEHAVIOR=behavior,
            TEST_SUPERVISOR_QUERY_ARM=str(armed) if gated else "",
            TEST_SUPERVISOR_QUERY_READY=str(ready),
            TEST_SUPERVISOR_QUERY_RELEASE=str(self.root / "supervisor-query-release"),
            TEST_SUPERVISOR_QUERY_BYTES=str(output_bytes),
            TEST_SUPERVISOR_QUERY_APPLY_FIRST="1" if apply_before_block else "0",
        )
        body = textwrap.dedent(r'''
            import errno
            import json
            import os
            from pathlib import Path
            import signal
            import subprocess
            import sys
            import time

            arguments = sys.argv[1:]
            command = os.environ["TEST_SUPERVISOR_QUERY_COMMAND"]
            armed = os.environ["TEST_SUPERVISOR_QUERY_ARM"]
            applicable = command != "lipc-get-prop" or arguments == ["-s", "com.lab126.powerd", "state"]
            if not applicable or (armed and not Path(armed).exists()):
                original = os.environ["TEST_SUPERVISOR_QUERY_ORIGINAL"]
                os.execv(original, [original, *arguments])
            if os.environ["TEST_SUPERVISOR_QUERY_APPLY_FIRST"] == "1":
                original = os.environ["TEST_SUPERVISOR_QUERY_ORIGINAL"]
                subprocess.run([original, *arguments], check=True)

            context = Path(os.environ["WEREAD_HOME"]).parent / "lock"
            identities = Path(os.environ["TEST_NATIVE_THEME_IDENTITIES"])

            def descriptor(number):
                try:
                    return os.readlink("/proc/self/fd/" + str(number))
                except OSError:
                    return "closed"

            def save_identity(pid, role):
                fields = Path("/proc/" + str(pid) + "/stat").read_text().rsplit(")", 1)[1].split()
                values = Path("/proc/" + str(pid) + "/cmdline").read_bytes().split(b"\0")
                entry = {"pid": pid, "role": role, "birth": fields[19],
                         "group": int(fields[2]), "master": int((context / "pid").read_text()),
                         "arguments": [os.fsdecode(value) for value in values if value]}
                if role == "query-command":
                    entry.update(stdin=descriptor(0), stdout=descriptor(1),
                                 stderr=descriptor(2), fifo=descriptor(3))
                with identities.open("a") as output:
                    output.write(json.dumps(entry) + "\n")

            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
            save_identity(os.getpgrp(), "group")
            save_identity(os.getpid(), "query-command")
            child = subprocess.Popen(["/bin/sleep", "30"])
            save_identity(child.pid, "query-descendant")
            ready = Path(os.environ["TEST_SUPERVISOR_QUERY_READY"])
            count = Path(os.environ["TEST_SUPERVISOR_QUERY_BYTES"])
            behavior = os.environ["TEST_SUPERVISOR_QUERY_BEHAVIOR"]
            if behavior in ("blocked", "released"):
                count.write_text(str(os.fstat(1).st_size) + "\n")
                ready.touch()
                if behavior == "released":
                    release = Path(os.environ["TEST_SUPERVISOR_QUERY_RELEASE"])
                    deadline = time.monotonic() + 6
                    while not release.exists():
                        if time.monotonic() >= deadline:
                            sys.exit(76)
                        time.sleep(0.01)
                    original = os.environ["TEST_SUPERVISOR_QUERY_ORIGINAL"]
                    os.execv(original, [original, *arguments])
                else:
                    time.sleep(30)
            elif behavior == "empty":
                count.write_text(str(os.fstat(1).st_size) + "\n")
                ready.touch()
                sys.exit(1)
            elif behavior == "oversize":
                os.write(1, b"active\n" + b"x" * 8192 + b"\n")
                count.write_text(str(os.fstat(1).st_size) + "\n")
                ready.touch()
            else:
                # Return zero after reaching the kernel file limit. The owner
                # must still reject this deliberately truncated successful reply.
                try:
                    while True:
                        os.write(1, b"MapState=IsViewable\n" + b"x" * 8192)
                except OSError as error:
                    if error.errno != errno.EFBIG:
                        raise
                count.write_text(str(os.fstat(1).st_size) + "\n")
                ready.touch()
        ''').lstrip()
        path = self.bin / command
        path.write_text("#!/usr/bin/env python3\n" + body)
        path.chmod(0o755)
        return armed, ready, output_bytes

    def observe_native_capture_reads(self, limit):
        original = shutil.which("cat", path=self.env["PATH"])
        self.assertIsNotNone(original)
        self.env.update(
            TEST_CAPTURE_CAT_ORIGINAL=original,
            TEST_CAPTURE_FILE=str(self.root / "lock" / "native-theme-output"),
            TEST_CAPTURE_READ_LIMIT=str(limit),
            TEST_CAPTURE_OVERSIZE_READ=str(self.root / "oversize-capture-read"),
        )
        self.script(self.bin / "cat", textwrap.dedent(r'''
            if [ "$#" -eq 1 ] && [ "$1" = "$TEST_CAPTURE_FILE" ]; then
                size=$(wc -c <"$1")
                if [ "$size" -gt "$TEST_CAPTURE_READ_LIMIT" ]; then
                    printf '%s\n' "$size" >"$TEST_CAPTURE_OVERSIZE_READ"
                fi
            fi
            exec "$TEST_CAPTURE_CAT_ORIGINAL" "$@"
        ''').lstrip())

    def observe_display_cleanup_flags(self):
        self.env["TEST_DISPLAY_CLEANUP_FLAGS"] = str(self.root / "display-cleanup-flags")
        source = self.launcher.read_text()
        anchor = "    if ! release_display; then note_cleanup_failure 'native display'; fi\n"
        self.assertIn(anchor, source)
        before = '    printf "before:%s:%s:%s\\n" "$DISPLAY_OWNED" "$STATUSBAR_STOPPED" "$STATUSBAR_RESTORE_REQUIRED" >"$TEST_DISPLAY_CLEANUP_FLAGS"\n'
        after = '    printf "after:%s:%s:%s\\n" "$DISPLAY_OWNED" "$STATUSBAR_STOPPED" "$STATUSBAR_RESTORE_REQUIRED" >>"$TEST_DISPLAY_CLEANUP_FLAGS"\n'
        self.launcher.write_text(source.replace(anchor, before + anchor + after, 1))

    def display_cleanup_flags(self):
        return {
            fields[0]: tuple(int(value) for value in fields[1:])
            for fields in (row.split(":") for row in (self.root / "display-cleanup-flags").read_text().splitlines())
        }

    def assert_window_manager_stop_signal_window(self, number, overlap=False):
        awesome = self.native_dark_fixture()
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()
        observed = self.root / "wm-stop-window"
        self.env["TEST_WM_STOP_WINDOW"] = str(observed)
        source = self.launcher.read_text()
        anchor = (
            '            # The list owns the obligation before the pending slot is cleared.\n'
            if overlap else
            '        if [ "$(process_birth "$wm_pid" 2>/dev/null || :)" = "$wm_birth" ] &&\n'
            '            kill -STOP "$wm_pid" 2>/dev/null; then\n'
        )
        self.assertEqual(source.count(anchor), 1)
        hook = (
            '            fixture_wm_stop_checks=100\n'
            '            while [ "$fixture_wm_stop_checks" -gt 0 ]; do\n'
            '                fixture_wm_stop_state=$(sed -n \'s/^State:[[:space:]]*//p\' "/proc/$wm_pid/status")\n'
            '                case "$fixture_wm_stop_state" in T*|t*) break ;; esac\n'
            '                fixture_wm_stop_checks=$((fixture_wm_stop_checks - 1))\n'
            '                sleep 0.01\n'
            '            done\n'
            '            [ "$fixture_wm_stop_checks" -gt 0 ] || exit 75\n'
            '            printf "%s|%s|%s\\n" "$WM_STOP_PENDING_IDENTITY" "$PAUSED_WM_IDENTITIES" "$DISPLAY_OWNED" >"$TEST_WM_STOP_WINDOW"\n'
            f'            kill -{number} "$$"\n'
        )
        # Enter the successful STOP branch before injecting the signal. The
        # release path contains a similar case statement that must not match.
        replacement = hook + anchor if overlap else anchor + hook
        self.launcher.write_text(source.replace(anchor, replacement, 1))

        status, _, error = self.run_launcher()
        self.assertEqual(status, {"TERM": 143, "HUP": 129}[number], error)
        pending, confirmed, claimed = observed.read_text().strip().split("|", 2)
        self.assertEqual(int(pending.split(":", 1)[0]), awesome.pid)
        self.assertEqual(pending.split(":", 1)[1], self.observed_identity(awesome.pid)["birth"])
        self.assertEqual(claimed, "0")
        if overlap:
            self.assertEqual(confirmed.split(), [pending])
        else:
            self.assertEqual(confirmed, "")
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 1, 1), "after": (0, 0, 0)})
        self.wait_until(lambda: self.trace().count("wm:CONT") == 1)
        self.assertEqual(self.trace().count("wm:CONT"), 1)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def assert_supervisor_query_resources_released(self):
        entries = [entry for entry in self.native_theme_identities() if entry["role"] == "query-command"]
        self.assertTrue(entries, "The fixture must enter a real supervised query.")
        for entry in entries:
            self.assertEqual(entry["stdin"], "/dev/null", entry)
            self.assertEqual(entry["stderr"], "/dev/null", entry)
            self.assertEqual(entry["fifo"], "closed", entry)
            self.assertEqual(entry["stdout"], str(self.root / "lock" / "native-theme-output"), entry)
            self.assertNotEqual(entry["group"], entry["master"], entry)
        self.assert_native_theme_helpers_finished()
        groups = {entry["group"] for entry in entries}

        def active_members():
            members = []
            for path in Path("/proc").glob("[0-9]*/stat"):
                try:
                    fields = path.read_text().rsplit(")", 1)[1].split()
                    if fields[0] not in ("Z", "X") and int(fields[2]) in groups:
                        members.append({"pid": int(path.parent.name), "state": fields[0], "group": fields[2]})
                except (OSError, ValueError, IndexError):
                    continue
            return members

        self.wait_until(lambda: not active_members())
        self.assertFalse((self.root / "lock").exists())
        self.assertFalse((self.root / "oversize-capture-read").exists())
        descriptor = self.root / "cleanup-descriptor"
        if descriptor.exists():
            self.assertEqual(descriptor.read_text(), "closed\n")

    def native_theme_events(self, kind):
        prefix = "theme:" + kind + ":"
        return [event for event in self.trace() if event.startswith(prefix)]

    def assert_native_theme_restored(self, awesome, stopped=False):
        self.wait_until(lambda: self.process_state(awesome.pid) == "T" if stopped else self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
        self.assertIsNone(awesome.poll())
        trace = self.trace()
        self.assertIn("start:statusbar", trace)
        self.assertIn("xrefresh:", trace)
        self.assertFalse((self.root / "lock").exists())

    def native_theme_snapshots(self, kind):
        prefix = "managed:" + kind + ":"
        return [event.split(":", 3)[3] if len(event.split(":", 3)) == 4 else ""
                for event in self.managed_events() if event.startswith(prefix)]

    def assert_native_theme_applied_before_restore(self, mode, preference, error, persisted=True):
        trace = self.trace()
        setters = self.native_theme_events("set")
        self.assertEqual(len(setters), 1)
        self.assertTrue(setters[0].startswith("theme:set:" + mode + ":"))
        self.assertNotIn("wm=T", setters[0])
        changed = trace.index(setters[0])
        readback = next(index for index, event in enumerate(trace)
                        if index > changed and event.startswith("theme:get:" + mode + ":"))
        restored = max(index for index, event in enumerate(trace) if event == "start:statusbar")
        refreshed = max(index for index, event in enumerate(trace) if event == "xrefresh:")
        self.assertLess(changed, readback)
        self.assertLess(readback, restored)
        self.assertLess(restored, refreshed)
        if persisted:
            confirmation = next(index for index, event in enumerate(trace)
                                if index > readback and event.startswith("theme:pref:" + preference + ":"))
            self.assertLess(confirmation, restored)
        self.assertIn("synchronization confirmed", error)
        self.assertIn("persistence=" + ("confirmed" if persisted else "not_checked"), error)
        self.assertNotIn("persistence was not confirmed", error)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), mode)
        for event in self.native_theme_events("get"):
            self.assertNotIn("wm=T", event)
        self.assert_native_theme_helpers_finished()

    def assert_native_theme_failure_is_visible(self, error):
        text = error.lower()
        self.assertTrue(any(word in text for word in ("native", "epdc", "theme")), error)
        self.assertTrue(any(word in text for word in ("fail", "timeout", "timed", "unknown", "unsupported", "unavailable", "not complete", "not confirmed")), error)

    def managed_events(self):
        path = self.root / "managed-events"
        return path.read_text().splitlines() if path.exists() else []

    def managed_pids(self):
        path = self.root / "managed-pids"
        if not path.exists():
            return []
        return [int(row.split("|", 1)[0]) for row in path.read_text().splitlines()]

    def managed_received_signals(self, pid):
        prefix = f"managed:signals:{pid}:"
        reports = [event[len(prefix):] for event in self.managed_events() if event.startswith(prefix)]
        self.assertEqual(len(reports), 1, "A terminated fixture must preserve its received signal set.")
        return set(reports[0].split(","))

    @staticmethod
    def process_state(pid):
        try:
            return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
        except (OSError, IndexError):
            return None

    def wait_for_managed_application(self):
        directory = self.root / "lock"
        self.wait_until(
            lambda: self.managed_pids()
            and (directory / "managed-sleep-available").exists()
            and (directory / "application-birth").exists()
            and (directory / "application-owned").exists()
            and len((directory / "application-owned").read_text().strip().split(":")) == 4
        )
        pid = self.managed_pids()[0]
        owner = (directory / "application-owned").read_text().strip().split(":")
        self.assertEqual(owner[2], str(pid))
        self.assertEqual(owner[3], (directory / "application-birth").read_text().strip())
        return pid

    def suspend_managed_application(self, pid, duplicate=False):
        released = self.trace().count("xrefresh:")
        self.emit(*(["goingToScreenSaver 2"] * (2 if duplicate else 1)))
        self.wait_until(
            lambda: self.process_state(pid) == "T"
            and self.trace().count("xrefresh:") == released + 1
        )
        directory = self.root / "lock"
        request = (directory / "managed-sleep-request").read_text().strip()
        self.assertEqual((directory / "managed-sleep-ready").read_text().strip(), request)
        return request

    def wake_managed_application(self, pid, request):
        self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")
        path = self.root / "lock" / "managed-wake-ready"
        self.wait_until(
            lambda: path.exists() and path.read_text().strip() == request
            and self.process_state(pid) not in (None, "T", "Z", "X")
        )
        self.assertEqual(self.managed_pids(), [pid])
        self.assertEqual(self.starts(), ["0"])

    def assert_managed_prepare_fallback(self, mode):
        self.managed_fixture(TEST_MANAGED_PREPARE=mode)

        def drive(_):
            pid = self.wait_for_managed_application()
            self.emit("goingToScreenSaver 2")
            if mode != "none":
                self.wait_until(lambda: any(":sleep-ready:" in event for event in self.managed_events()))
                events = self.managed_events()
                self.assertNotEqual(
                    next(event.split(":", 3)[3] for event in events if ":sleep-ready:" in event),
                    next(event.split(":", 3)[3] for event in events if ":prepare:" in event),
                )
            self.wait_until(lambda: "xrefresh:" in self.trace())
            self.assertNotEqual(self.process_state(pid), "T")
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertFalse(any(":wake:" in event for event in self.managed_events()))
        self.assertTrue(any(":cold-restored:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_keeps_one_pid_and_generation_across_two_cycles(self):
        self.managed_fixture()
        observed = []

        def drive(_):
            pid = self.wait_for_managed_application()
            token = (self.root / "lock" / "nonce").read_text().strip()
            for cycle in (1, 2):
                request = self.suspend_managed_application(pid)
                self.assertEqual(request, f"1:{token}:{cycle}")
                observed.append(request)
                self.wake_managed_application(pid, request)
                self.assertEqual(
                    (self.root / "lock" / "generation").read_text().strip(), f"1:{token}"
                )
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(len(set(observed)), 2)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(len(self.managed_pids()), 1)
        self.assertTrue(any(":term:" in event for event in self.managed_events()))
        self.assertFalse(any(":concurrent:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_rejects_an_acknowledgment_with_an_old_token(self):
        self.assert_managed_prepare_fallback("stale-token")

    def test_managed_sleep_duplicate_power_events_do_not_change_the_resident_cycle(self):
        self.managed_fixture()

        def drive(_):
            pid = self.wait_for_managed_application()
            request = self.suspend_managed_application(pid, duplicate=True)
            self.wake_managed_application(pid, request)
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1", "exitingScreenSaver 1")
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(len([event for event in self.managed_events() if ":prepare:" in event]), 1)
        self.assertEqual(len([event for event in self.managed_events() if ":wake:" in event]), 1)
        self.assertFalse(any(":wake-mismatch:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_rejects_an_acknowledgment_from_an_old_cycle(self):
        self.managed_fixture(TEST_MANAGED_PREPARE="stale-cycle")

        def drive(_):
            pid = self.wait_for_managed_application()
            previous = self.suspend_managed_application(pid)
            self.wake_managed_application(pid, previous)
            self.emit("goingToScreenSaver 2")
            self.wait_until(
                lambda: len([event for event in self.managed_events() if ":sleep-ready:" in event]) == 2
            )
            events = self.managed_events()
            prepared = [event.split(":", 3)[3] for event in events if ":prepare:" in event]
            ready = [event.split(":", 3)[3] for event in events if ":sleep-ready:" in event]
            self.assertEqual(ready, [previous, previous])
            self.assertNotEqual(prepared[1], previous)
            self.assertGreater(int(previous.rsplit(":", 1)[1]), 0)
            self.wait_until(lambda: self.trace().count("xrefresh:") == 2)
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertFalse(any(":concurrent:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_prepare_timeout_uses_the_saved_cold_resume(self):
        self.assert_managed_prepare_fallback("none")

    def test_managed_sleep_capability_absence_keeps_the_cold_resume_path(self):
        self.managed_fixture(TEST_MANAGED_CAPABILITY="0")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2")
            self.wait_until(lambda: "xrefresh:" in self.trace())
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertFalse(any(":prepare:" in event for event in self.managed_events()))
        self.assertFalse(any(":wake:" in event for event in self.managed_events()))
        received = self.managed_received_signals(self.managed_pids()[0])
        self.assertNotIn("SIGUSR1", received)
        self.assertNotIn("SIGUSR2", received)

    def assert_managed_wake_fallback(self, mode):
        self.managed_fixture(TEST_MANAGED_WAKE=mode)

        def startup_failure_state(supervisor):
            import stat

            directory = self.root / "lock"
            names = ("pid", "nonce", "generation", "power-events", "observer-pid", "observer-owned",
                     "observer-go", "application-pid", "application-birth", "application-owned",
                     "state-open", "managed-sleep-available")
            records = {}
            for name in names:
                try:
                    metadata = (directory / name).lstat()
                    records[name] = {"present": True, "regular": stat.S_ISREG(metadata.st_mode),
                                     "bytes": metadata.st_size}
                except OSError:
                    records[name] = {"present": False}

            def small_record(name):
                if not records[name].get("regular", False):
                    return ""
                try:
                    with (directory / name).open("rb") as file:
                        return file.read(256).decode("ascii", errors="replace").strip()
                except OSError:
                    return ""

            generation = small_record("generation")
            nonce = small_record("nonce")
            owner = small_record("application-owned").split(":")
            consistency = {
                "nonce_format": len(nonce) == 32 and all(char in "0123456789abcdef" for char in nonce),
                "owner_fields": len(owner),
                "owner_generation_matches": len(owner) == 4 and owner[0] == generation.split(":")[0],
                "owner_token_matches": len(owner) == 4 and bool(nonce) and owner[1] == nonce,
                "owner_pid_matches": len(owner) == 4 and owner[2] == small_record("application-pid"),
                "owner_birth_matches": len(owner) == 4 and owner[3] == small_record("application-birth"),
                "managed_marker_matches": bool(generation) and small_record("managed-sleep-available") == generation,
            }

            def event_kinds(path):
                try:
                    with path.open("rb") as file:
                        size = file.seek(0, os.SEEK_END)
                        file.seek(max(0, size - 4096))
                        lines = file.read(4096).decode("utf-8", errors="replace").splitlines()[-8:]
                except OSError:
                    return []
                # Report kinds and PIDs only, never protocol marker payloads.
                return [":".join(line.split(":")[:3]) if line.startswith("managed:")
                        else line if line in ("stop:statusbar", "start:statusbar", "xrefresh:")
                        else "other-event" for line in lines]

            active = getattr(self, "active_fixture_launcher", None)
            source_messages = self.fixture_output_tail(active["error"], 4096) if active is not None else ""
            if nonce:
                source_messages = source_messages.replace(nonce, "[launcher-token]")
            return {"supervisor_exit": supervisor.poll(), "records": records,
                    "consistency": consistency, "group": self.event_group_members(supervisor.pid),
                    "managed_events": event_kinds(self.root / "managed-events"),
                    "trace": event_kinds(self.root / "trace"), "source_stderr": source_messages}

        def drive(supervisor):
            try:
                pid = self.wait_for_managed_application()
            except AssertionError as error:
                # A failed initial lifecycle is not a wake fallback. Keep the
                # original failure and collect bounded state before forced reap.
                state = startup_failure_state(supervisor)
                raise AssertionError(f"Initial managed ownership was not reached: {state!r}") from error
            if mode == "stale-cycle":
                previous = self.suspend_managed_application(pid)
                self.wake_managed_application(pid, previous)
                self.assertGreater(int(previous.rsplit(":", 1)[1]), 0)
            self.suspend_managed_application(pid)
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        pids = self.managed_pids()
        self.assertEqual(len(pids), 2)
        events = self.managed_events()
        self.assertLess(events.index(f"managed:term:{pids[0]}"), events.index(f"managed:start:{pids[1]}:2:1"))
        if mode == "stale-cycle":
            wakes = [event.split(":", 3)[3] for event in events if ":wake:" in event]
            ready = [event.split(":", 3)[3] for event in events if ":wake-ready:" in event]
            self.assertEqual(ready, [wakes[0], wakes[0]])
            self.assertNotEqual(wakes[1], wakes[0])
        self.assertFalse(any(":concurrent:" in event for event in events))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_wake_timeout_stops_the_old_process_before_cold_resume(self):
        self.assert_managed_wake_fallback("none")

    def test_managed_sleep_stale_wake_ack_stops_the_old_process_before_cold_resume(self):
        self.assert_managed_wake_fallback("stale-cycle")

    def test_managed_sleep_usb_cleanup_continues_the_stopped_app_before_termination(self):
        self.managed_fixture()
        observed = []

        def drive(_):
            pid = self.wait_for_managed_application()
            observed.append(pid)
            self.suspend_managed_application(pid)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertIn(f"managed:term:{observed[0]}", self.managed_events())
        self.assertIsNone(self.process_state(observed[0]))
        self.assertFalse(any(":wake:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_external_term_reaps_a_stopped_application(self):
        self.managed_fixture()
        observed = []

        def drive(child):
            pid = self.wait_for_managed_application()
            observed.append(pid)
            self.suspend_managed_application(pid)
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertIn(f"managed:term:{observed[0]}", self.managed_events())
        self.assertIsNone(self.process_state(observed[0]))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_observer_failure_reaps_a_stopped_application(self):
        self.managed_fixture()
        observed = []

        def drive(_):
            pid = self.wait_for_managed_application()
            observed.append(pid)
            self.suspend_managed_application(pid)
            self.emit("_weread_observer_exit fixture")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Power event observation ended", error)
        self.assertIn(f"managed:term:{observed[0]}", self.managed_events())
        self.assertIsNone(self.process_state(observed[0]))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_explicit_exit_during_preparation_wins_over_wake(self):
        self.managed_fixture(TEST_MANAGED_PREPARE="ordinary")

        def drive(_):
            self.wait_for_managed_application()
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertTrue(any(":ordinary-exit:" in event for event in self.managed_events()))
        self.assertFalse(any(":cold-restored:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_rejects_a_replaced_birth_record_before_signaling(self):
        self.managed_fixture()

        def drive(_):
            self.wait_for_managed_application()
            (self.root / "lock" / "application-birth").write_text("0\n")
            self.emit("goingToScreenSaver 2")
            self.wait_until(lambda: "xrefresh:" in self.trace())
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertFalse(any(":prepare:" in event for event in self.managed_events()))
        received = self.managed_received_signals(self.managed_pids()[0])
        self.assertNotIn("SIGUSR1", received)
        self.assertNotIn("SIGUSR2", received)
        self.assertFalse(any(":concurrent:" in event for event in self.managed_events()))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_never_signals_an_unrelated_pid_written_into_the_record(self):
        self.managed_fixture()
        foreign = self.root / "foreign.py"
        foreign.write_text(textwrap.dedent('''
            from pathlib import Path
            import signal
            import sys
            import time
            log, ready = map(Path, sys.argv[1:])
            def received(number, _frame):
                with log.open("a") as output:
                    output.write(str(number) + "\\n")
            for number in (signal.SIGTERM, signal.SIGUSR1, signal.SIGUSR2, signal.SIGCONT):
                signal.signal(number, received)
            ready.touch()
            while True:
                time.sleep(0.02)
        ''').lstrip())
        foreign_log = self.root / "foreign-signals"
        foreign_ready = self.root / "foreign-ready"
        child = subprocess.Popen(
            [sys.executable, str(foreign), str(foreign_log), str(foreign_ready)],
            start_new_session=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        def reap_foreign():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=3)

        self.addCleanup(reap_foreign)
        self.wait_until(foreign_ready.exists)
        original = []

        def drive(supervisor):
            original.append(self.wait_for_managed_application())
            (self.root / "lock" / "application-pid").write_text(f"{child.pid}\n")
            self.emit("goingToScreenSaver 2")
            self.wait_until(lambda: "xrefresh:" in self.trace() or supervisor.poll() is not None)
            self.assertIsNone(child.poll())
            self.assertNotEqual(self.process_state(child.pid), "T")
            self.assertFalse(foreign_log.exists())
            if supervisor.poll() is None:
                supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertIn(status, (1, 143), error)
        self.assertIsNone(child.poll())
        self.assertFalse(foreign_log.exists())
        self.assertIsNone(self.process_state(original[0]))
        self.assertFalse((self.root / "lock").exists())

    def test_managed_sleep_cleanup_reaps_the_captured_orphan_without_signaling_a_forged_record(self):
        self.managed_fixture()
        captured = {}
        fixture_argument = os.fsencode(self.home / "app" / "weread")

        def process_identity(pid):
            try:
                fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
                arguments = tuple(
                    argument for argument in Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
                    if argument
                )
                return {
                    "pid": pid,
                    "state": fields[0],
                    "parent": int(fields[1]),
                    "group": int(fields[2]),
                    "birth": fields[19],
                    "arguments": arguments,
                }
            except (OSError, ValueError, IndexError):
                return None

        def private_group_members(group):
            members = []
            for directory in Path("/proc").iterdir():
                if not directory.name.isdecimal():
                    continue
                try:
                    fields = (directory / "stat").read_text().rsplit(")", 1)[1].split()
                    if int(fields[2]) != group:
                        continue
                except (OSError, ValueError, IndexError):
                    continue
                identity = process_identity(int(directory.name))
                if identity is not None:
                    members.append(identity)
            return members

        def cleanup_captured_application():
            original = captured.get("application")
            if original is None:
                return
            # A failed assertion must not leave the intentionally orphaned app
            # stopped. Never select its cleanup target from the mutable record.
            for number in (signal.SIGCONT, signal.SIGKILL):
                current = process_identity(original["pid"])
                if current is None or current["state"] in ("Z", "X"):
                    return
                if (
                    current["birth"] != original["birth"]
                    or current["arguments"] != original["arguments"]
                    or fixture_argument not in current["arguments"]
                ):
                    return
                try:
                    os.kill(original["pid"], number)
                except ProcessLookupError:
                    return

        self.addCleanup(cleanup_captured_application)
        foreign = self.root / "orphan-foreign.py"
        foreign.write_text(textwrap.dedent('''
            from pathlib import Path
            import signal
            import sys
            import time
            log, ready = map(Path, sys.argv[1:])
            def received(number, _frame):
                with log.open("a") as output:
                    output.write(str(number) + "\\n")
            for number in (signal.SIGTERM, signal.SIGUSR1, signal.SIGUSR2, signal.SIGCONT):
                signal.signal(number, received)
            ready.touch()
            while True:
                time.sleep(0.02)
        ''').lstrip())
        foreign_log = self.root / "orphan-foreign-signals"
        foreign_ready = self.root / "orphan-foreign-ready"
        unrelated = subprocess.Popen(
            [sys.executable, str(foreign), str(foreign_log), str(foreign_ready)],
            start_new_session=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        def reap_unrelated():
            if unrelated.poll() is None:
                unrelated.kill()
            unrelated.wait(timeout=3)

        self.addCleanup(reap_unrelated)
        self.wait_until(foreign_ready.exists)
        supervisor = subprocess.Popen(
            ["sh", str(self.launcher), "--detached"],
            env=self.env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        try:
            pid = self.wait_for_managed_application()
            self.suspend_managed_application(pid)
            application = process_identity(pid)
            self.assertIsNotNone(application)
            self.assertEqual(application["state"], "T")
            self.assertIn(fixture_argument, application["arguments"])
            captured["application"] = application
            runner = process_identity(application["parent"])
            self.assertIsNotNone(runner)
            self.assertGreater(runner["pid"], 1)
            self.assertNotEqual(runner["pid"], supervisor.pid)
            self.assertEqual(runner["parent"], supervisor.pid)
            self.assertEqual(runner["group"], supervisor.pid)
            self.assertIn(os.fsencode(self.launcher), runner["arguments"])
            # The target is this fixture's independently verified runner, never
            # the master supervisor or the mutable application PID record.
            os.kill(runner["pid"], signal.SIGKILL)
            self.wait_until(
                lambda: (process_identity(pid) or {}).get("parent") == 1
            )
            self.assertEqual(self.process_state(pid), "T")
            (self.root / "lock" / "application-pid").write_text(f"{unrelated.pid}\n")
            self.emit("usbConfigured 1")
            output, error = supervisor.communicate(timeout=16)
            self.assertEqual(supervisor.returncode, 143, error)
            self.wait_until(lambda: self.process_state(pid) in (None, "Z", "X"))
            received = self.managed_received_signals(pid)
            # A stopped process cannot execute its TERM handler until continued.
            self.assertIn("SIGTERM", received)
            self.assertIn(f"managed:term:{pid}", self.managed_events())
            self.assertEqual(self.starts(), ["0"])
            self.assertEqual(self.managed_pids(), [pid])
            self.assertIsNone(unrelated.poll())
            self.assertNotEqual(self.process_state(unrelated.pid), "T")
            self.assertFalse(foreign_log.exists())
            self.assertFalse((self.root / "lock").exists())
            # PID 1 may not immediately reap the terminated app. Every other
            # member of this fixture's private session must already be gone.
            for member in private_group_members(supervisor.pid):
                self.assertEqual(member["pid"], pid)
                self.assertEqual(member["birth"], application["birth"])
                self.assertIn(member["state"], ("Z", "X"))
        finally:
            cleanup_captured_application()
            # The identity-checked orphan cleanup above is required even if its
            # original process group disappears or contains only a zombie.
            members = private_group_members(supervisor.pid)
            if supervisor.poll() is None or any(member["state"] not in ("Z", "X") for member in members):
                try:
                    os.killpg(supervisor.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            supervisor.communicate(timeout=3)

    def native_theme_handoff_fixture(self, mode="Y8", **behavior):
        awesome = self.native_dark_fixture(
            mode=mode, TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET=""
        )
        self.env.update(
            TEST_NATIVE_THEME_HANDOFF="1",
            TEST_NATIVE_THEME_READY="ready",
            TEST_NATIVE_THEME_DRIVER=str(self.root / "theme-driver"),
            TEST_NATIVE_THEME_COMMAND_OBSERVATIONS=str(self.root / "theme-command-observations"),
            TEST_NATIVE_THEME_RESULT_OBSERVATIONS=str(self.root / "theme-result-observations"),
        )
        self.env.update(behavior)
        app = self.home / "app" / "weread"
        source = app.read_text()
        anchor = "while True:\n    if pending.intersection("
        self.assertEqual(source.count(anchor), 1)
        driver = textwrap.dedent(r'''
            theme_driver_seen = ""
            theme_pending = ""

            def handle_theme_handoff():
                global theme_driver_seen, theme_pending
                try:
                    command = Path(os.environ["TEST_NATIVE_THEME_DRIVER"]).read_text()
                except OSError:
                    command = ""
                if command and command != theme_driver_seen:
                    theme_driver_seen = command
                    values = json.loads(command)
                    sequence = str(values["sequence"])
                    request = base + ":" + sequence + ":" + values["target"]
                    # The fixture implements only the App side of the file/FIFO
                    # protocol. Native changes remain real launcher operations.
                    record("theme-quiescent", request)
                    marker("native-theme-request", request)
                    theme_pending = sequence
                    record("theme-request", request)
                    descriptor = os.open(context / "power-events", os.O_WRONLY | os.O_NONBLOCK)
                    try:
                        payload = ("_weread_native_theme_request " + generation + " " + sequence + "\n").encode()
                        deadline = time.monotonic() + 2
                        while True:
                            try:
                                if os.write(descriptor, payload) != len(payload):
                                    raise SystemExit(77)
                                break
                            except (BlockingIOError, InterruptedError):
                                if time.monotonic() >= deadline:
                                    raise SystemExit(78)
                                time.sleep(0.01)
                    finally:
                        os.close(descriptor)
                result = read_marker("native-theme-result")
                if theme_pending and result.startswith(base + ":" + theme_pending + ":"):
                    wm = observed_process(int(os.environ["TEST_AWESOME_PID"]))
                    entry = {
                        "pid": pid, "birth": birth, "result": result,
                        "wm_pid": int(os.environ["TEST_AWESOME_PID"]),
                        "wm_state": wm[0] if wm else "missing",
                        "wm_birth": wm[1] if wm else "",
                        "native": read_marker("native-epdc-mode"),
                        "portrait": read_marker("native-theme-portrait-calibration"),
                        "interrupt": read_marker("native-theme-interrupt"),
                    }
                    append(os.environ["TEST_NATIVE_THEME_RESULT_OBSERVATIONS"], json.dumps(entry))
                    record("theme-result", result)
                    ready = base + ":" + theme_pending
                    mode = os.environ["TEST_NATIVE_THEME_READY"]
                    if mode == "ready":
                        marker("native-theme-ready", ready)
                        record("theme-ready", ready)
                    elif mode == "stale":
                        marker("native-theme-ready", base + ":0")
                        record("theme-stale-ready", base + ":0")
                    theme_pending = ""

        ''')
        source = source.replace(anchor, driver + anchor, 1)
        tick = "    time.sleep(0.01)\n"
        before, separator, after = source.rpartition(tick)
        self.assertTrue(separator)
        source = before + "    handle_theme_handoff()\n" + separator + after
        shebang, body = source.split("\n", 1)
        app.write_text(shebang + "\nimport json\n" + body)
        self.observe_native_theme_dispatch()
        return awesome

    def observe_native_theme_dispatch(self):
        source = self.launcher.read_text()
        anchor = (
            "                    fail 'Native theme handoff could not complete safely.'\n"
            "                fi\n"
        )
        self.assertEqual(source.count(anchor), 1)
        self.launcher.write_text(source.replace(
            anchor, anchor
            + '                printf "theme:dispatch:%s:%s\\n" "$2" "$3" >>"$TEST_TRACE"\n',
            1,
        ))

    def native_theme_continue_gate(self):
        window = self.root / "theme-continue-window"
        release = self.root / "theme-continue-release"
        self.env.update(TEST_THEME_CONTINUE_WINDOW=str(window), TEST_THEME_CONTINUE_RELEASE=str(release))
        source = self.launcher.read_text()
        anchor = "    NATIVE_THEME_STAGE=application-continue\n"
        self.assertEqual(source.count(anchor), 1)
        gate = textwrap.dedent(r'''
            printf '%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$NATIVE_THEME_REQUEST_SEQUENCE" >"$TEST_THEME_CONTINUE_WINDOW.pending" || return 1
            mv -f "$TEST_THEME_CONTINUE_WINDOW.pending" "$TEST_THEME_CONTINUE_WINDOW" || return 1
            fixture_theme_continue_checks=400
            while [ ! -f "$TEST_THEME_CONTINUE_RELEASE" ]; do
                [ "$fixture_theme_continue_checks" -gt 0 ] || return 1
                usleep 10000 || return 1
                fixture_theme_continue_checks=$((fixture_theme_continue_checks - 1))
            done
        ''')
        self.launcher.write_text(source.replace(anchor, anchor + gate, 1))
        self.theme_continue_release = release
        return window, release

    @staticmethod
    def write_fixture_record(path, value):
        pending = path.with_suffix(".fixture-pending")
        pending.write_text(value + "\n")
        pending.replace(path)

    def wait_for_native_theme_application(self):
        pid = self.wait_for_managed_application()
        directory = self.root / "lock"
        base = (directory / "generation").read_text().strip()
        self.wait_until(lambda: (directory / "native-theme-available").is_file())
        self.assertEqual((directory / "native-theme-available").read_text().strip(), base)
        self.assertEqual((directory / "native-theme-power-epoch").read_text().strip(), base.split(":")[1] + ":0")
        return pid, base

    def request_native_theme(self, pid, sequence, target):
        release = getattr(self, "theme_continue_release", None)
        if release is not None:
            release.unlink(missing_ok=True)
        directory = self.root / "lock"
        base = (directory / "generation").read_text().strip()
        command = json.dumps({"sequence": str(sequence), "target": target})
        self.write_fixture_record(self.root / "theme-driver", command)
        request = base + ":" + str(sequence) + ":" + target
        self.wait_until(lambda: request in self.native_theme_snapshots("theme-request"))
        try:
            self.assertEqual((directory / "native-theme-request").read_text().strip(), request)
        except FileNotFoundError:
            self.assertEqual((directory / "native-theme-ready").read_text().strip(), base + ":" + str(sequence))
        self.assertEqual(self.managed_pids(), [pid])
        return base + ":" + str(sequence)

    def wait_for_native_theme_ready(self, pid, ready):
        directory = self.root / "lock"
        self.wait_until(
            lambda: (directory / "native-theme-ready").is_file()
            and (directory / "native-theme-ready").read_text().strip() == ready
            and ready in self.native_theme_snapshots("theme-ready")
            and "theme:dispatch:" + ready.split(":")[0] + ":" + ready.split(":")[2] in self.trace()
            and not (directory / "native-theme-request").exists()
            and self.process_state(pid) not in (None, "T", "Z", "X")
        )
        return (directory / "native-theme-result").read_text().strip()

    def native_theme_command_observations(self):
        path = self.root / "theme-command-observations"
        return [json.loads(row) for row in path.read_text().splitlines()] if path.exists() else []

    def native_theme_result_observations(self):
        path = self.root / "theme-result-observations"
        return [json.loads(row) for row in path.read_text().splitlines()] if path.exists() else []

    def assert_native_theme_result_before_continue(self, pid, awesome, ready, actual, status, window):
        directory = self.root / "lock"
        self.wait_until(lambda: window.is_file() and window.read_text().strip() == ready)
        self.assertEqual(self.process_state(pid), "T")
        self.assertEqual(self.process_state(awesome.pid), "T")
        self.assertEqual((directory / "native-theme-result").read_text().strip(), ready + ":" + status + ":" + actual)
        self.assertEqual((directory / "native-epdc-mode").read_text().strip(), ":".join(ready.split(":")[:2]) + ":" + actual)
        self.assertFalse((directory / "native-theme-ready").exists())
        self.assertEqual(self.native_theme_snapshots("theme-result"), [])
        trace = self.trace()
        self.assertGreater(trace.count("stop:statusbar"), trace.count("start:statusbar"))

    def assert_native_theme_command_window(self, original, wm, target, actual, preference):
        rows = self.native_theme_command_observations()
        setter = next(index for index, row in enumerate(rows) if row["kind"] == "set" and row["value"] == target)
        readback = next(index for index, row in enumerate(rows)
                        if index > setter and row["kind"] == "get" and row["value"] == actual)
        persisted = next(index for index, row in enumerate(rows)
                         if index > readback and row["kind"] == "pref" and row["value"] == preference)
        self.assertLess(setter, readback)
        self.assertLess(readback, persisted)
        for row in (rows[setter], rows[readback], rows[persisted]):
            self.assertEqual(row["app"]["pid"], original["pid"])
            self.assertEqual(row["app"]["birth"], original["birth"])
            self.assertEqual(row["app"]["state"], "T")
            self.assertEqual(row["wm"]["pid"], wm["pid"])
            self.assertEqual(row["wm"]["birth"], wm["birth"])
            self.assertNotIn(row["wm"]["state"], ("T", "Z", "X"))
            self.assertTrue(row["statusbar_stopped"])

    def assert_native_theme_resident_cleanup(self, awesome, original):
        self.wait_until(lambda: not self.original_process_exists(original))
        self.assertEqual(self.managed_pids(), [original["pid"]])
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse(any(":concurrent:" in event or ":cold-restored:" in event for event in self.managed_events()))
        self.assertIn(f"managed:term:{original['pid']}", self.managed_events())
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_handoff_round_trip_keeps_the_original_app_and_window_manager(self):
        awesome = self.native_theme_handoff_fixture()
        window, release = self.native_theme_continue_gate()
        original = []
        wm = self.observed_identity(awesome.pid)

        def drive(supervisor):
            pid, base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            for sequence, target in (("1", "Y8INV"), ("2", "Y8")):
                ready = self.request_native_theme(pid, sequence, target)
                self.wait_until(lambda: window.is_file() and window.read_text().strip() == ready)
                self.assertEqual(self.process_state(pid), "T")
                self.assertEqual(self.process_state(awesome.pid), "T")
                self.assertEqual((self.root / "lock" / "native-theme-result").read_text().strip(), ready + ":ok:" + target)
                self.assertFalse((self.root / "lock" / "native-theme-ready").exists())
                self.assertEqual((self.root / "lock" / "generation").read_text().strip(), base)
                self.assertEqual(self.trace().count("start:statusbar"), 0)
                release.touch()
                self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":ok:" + target)
                self.assertEqual(self.managed_pids(), [pid])
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual([row["value"] for row in self.native_theme_command_observations() if row["kind"] == "set"],
                         ["Y8INV", "Y8"])
        self.assert_native_theme_command_window(original[0], wm, "Y8INV", "Y8INV", "Y8INV")
        self.assert_native_theme_command_window(original[0], wm, "Y8", "Y8", "Y8")
        results = self.native_theme_result_observations()
        self.assertEqual([row["result"].split(":")[-2:] for row in results], [["ok", "Y8INV"], ["ok", "Y8"]])
        for row in results:
            self.assertEqual((row["pid"], row["birth"]), (original[0]["pid"], original[0]["birth"]))
            self.assertEqual((row["wm_pid"], row["wm_birth"], row["wm_state"]), (wm["pid"], wm["birth"], "T"))
            self.assertEqual(row["interrupt"], "")
            self.assertTrue(row["native"].endswith(":" + row["result"].split(":")[-1]))
        self.assertEqual(error.count("Native theme handoff confirmed result=ok"), 2)
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def test_native_theme_handoff_known_readback_mismatch_returns_error_and_actual_mode(self):
        awesome = self.native_theme_handoff_fixture(TEST_NATIVE_THEME_READBACK_MISMATCH="1")
        window, release = self.native_theme_continue_gate()
        original = []
        wm = self.observed_identity(awesome.pid)

        def drive(supervisor):
            pid, _base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            ready = self.request_native_theme(pid, "1", "Y8INV")
            self.assert_native_theme_result_before_continue(pid, awesome, ready, "Y8", "error", window)
            release.touch()
            self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":error:Y8")
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(len(self.native_theme_events("set")), 1)
        self.assert_native_theme_command_window(original[0], wm, "Y8INV", "Y8", "Y8INV")
        self.assertEqual(self.native_theme_result_observations()[0]["result"].split(":")[-2:], ["error", "Y8"])
        self.assertIn("Native theme handoff confirmed result=error mode=Y8", error)
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def test_native_theme_handoff_persistence_mismatch_returns_error_with_known_actual(self):
        awesome = self.native_theme_handoff_fixture(TEST_NATIVE_THEME_PREF_MISMATCH="1")
        original = []
        wm = self.observed_identity(awesome.pid)

        def drive(supervisor):
            pid, _base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            ready = self.request_native_theme(pid, "1", "Y8INV")
            self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":error:Y8INV")
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assert_native_theme_command_window(original[0], wm, "Y8INV", "Y8INV", "Y8")
        self.assertIn("Native theme handoff confirmed result=error mode=Y8INV", error)
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def assert_native_theme_handoff_unconfirmed_cleanup(self, **behavior):
        awesome = self.native_theme_handoff_fixture(**behavior)
        original = []

        def drive(_supervisor):
            pid, _base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            self.request_native_theme(pid, "1", "Y8INV")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Native theme handoff could not complete safely", error)
        self.assertEqual(len(self.native_theme_events("set")), 1)
        self.assertEqual(self.native_theme_snapshots("theme-ready"), [])
        if behavior.get("TEST_NATIVE_THEME_READY") not in ("none", "stale"):
            self.assertEqual(self.native_theme_result_observations(), [])
        self.assert_native_theme_resident_cleanup(awesome, original[0])
        return error

    def test_native_theme_handoff_setter_timeout_exits_without_cold_restart(self):
        error = self.assert_native_theme_handoff_unconfirmed_cleanup(TEST_NATIVE_THEME_SET_TIMEOUT="1")
        self.assertIn("step=theme-set", error)
        self.assertEqual(len(self.native_theme_events("get")), 1)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")

    def test_native_theme_handoff_unknown_readback_exits_without_resuming_the_renderer(self):
        error = self.assert_native_theme_handoff_unconfirmed_cleanup(TEST_NATIVE_THEME_READBACK_VALUE="UNKNOWN")
        self.assertIn("stage=system-readback", error)
        self.assertEqual([event.split(":")[2] for event in self.native_theme_events("get")], ["Y8", "UNKNOWN"])
        self.assertEqual(self.native_theme_events("pref"), [])

    def test_native_theme_handoff_missing_renderer_acknowledgment_exits_without_cold_restart(self):
        error = self.assert_native_theme_handoff_unconfirmed_cleanup(TEST_NATIVE_THEME_READY="none")
        self.assertIn("renderer acknowledgment timed out", error)
        self.assertEqual(len(self.native_theme_result_observations()), 1)

    def test_native_theme_handoff_stale_renderer_acknowledgment_exits_without_cold_restart(self):
        error = self.assert_native_theme_handoff_unconfirmed_cleanup(TEST_NATIVE_THEME_READY="stale")
        self.assertIn("renderer acknowledgment timed out", error)
        self.assertEqual(len(self.native_theme_result_observations()), 1)

    def test_native_theme_handoff_rejects_stale_replayed_malformed_and_wrong_token_requests(self):
        awesome = self.native_theme_handoff_fixture()
        original = []

        def drive(supervisor):
            pid, base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            ready = self.request_native_theme(pid, "9", "Y8INV")
            self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":ok:Y8INV")
            self.wait_until(lambda: "theme:dispatch:1:9" in self.trace())
            generation, token = base.split(":")
            changed_token = ("1" if token[0] == "0" else "0") + token[1:]
            cases = [
                ("replayed", base + ":9:Y8", "9"),
                ("older", base + ":8:Y8", "8"),
                ("zero", base + ":0:Y8", "0"),
                ("empty-sequence", base + "::Y8", "10"),
                ("leading-zero", base + ":010:Y8", "010"),
                ("negative", base + ":-1:Y8", "-1"),
                ("nondigit", base + ":10x:Y8", "10x"),
                ("u64-overflow", base + ":18446744073709551616:Y8", "18446744073709551616"),
                ("oversized", base + ":" + "1" * 130 + ":Y8", "1" * 130),
                ("old-generation", "0:" + token + ":10:Y8", "10"),
                ("wrong-token", generation + ":" + changed_token + ":10:Y8", "10"),
                ("unknown-mode", base + ":10:UNKNOWN", "10"),
                ("extra-field", base + ":10:Y8:extra", "10"),
                ("hint-mismatch", base + ":10:Y8", "11"),
            ]
            expected_setters = len(self.native_theme_events("set"))
            for label, request, hint in cases:
                with self.subTest(request=label):
                    prior = self.trace().count("theme:dispatch:" + generation + ":" + hint)
                    self.write_fixture_record(self.root / "lock" / "native-theme-request", request)
                    self.emit("_weread_native_theme_request " + generation + " " + hint)
                    self.wait_until(lambda: self.trace().count("theme:dispatch:" + generation + ":" + hint) == prior + 1)
                    self.assertEqual(len(self.native_theme_events("set")), expected_setters)
                    self.assertIsNone(supervisor.poll())
                    self.assertEqual(self.managed_pids(), [pid])
            ready = self.request_native_theme(pid, "10", "Y8")
            self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":ok:Y8")
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(len(self.native_theme_events("set")), 2)
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def test_native_theme_handoff_accepts_adjacent_sequences_at_the_u64_limit(self):
        awesome = self.native_theme_handoff_fixture()
        original = []

        def drive(supervisor):
            pid, _base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            for sequence, target in (("18446744073709551614", "Y8INV"), ("18446744073709551615", "Y8")):
                ready = self.request_native_theme(pid, sequence, target)
                self.assertEqual(self.wait_for_native_theme_ready(pid, ready), ready + ":ok:" + target)
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(len(self.native_theme_events("set")), 2)
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def assert_native_theme_queued_power_interrupt(self, event):
        awesome = self.native_theme_handoff_fixture()
        armed, blocked, _count = self.install_supervisor_query_fixture("lipc-get-prop", behavior="released", gated=True)
        release = self.root / "supervisor-query-release"
        original = []

        def drive(_supervisor):
            pid, base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            armed.touch()
            self.request_native_theme(pid, "1", "Y8INV")
            self.wait_until(blocked.exists)
            prior_power_queries = len(self.native_theme_events("power"))
            self.assertNotEqual(self.process_state(pid), "T")
            self.assertEqual(self.process_state(awesome.pid), "T")
            self.emit(event)
            directory = self.root / "lock"
            self.wait_until(lambda: (directory / "native-theme-power-epoch").read_text().strip() == base.split(":")[1] + ":1"
                            and (directory / "native-theme-interrupt").is_file())
            self.assertEqual((directory / "native-theme-interrupt").read_text().strip(), base)
            self.assertEqual(self.native_theme_events("set"), [])
            self.assertFalse((directory / "native-theme-result").exists())
            # Model the wake-clear race without weakening the real observer:
            # its epoch remains newer than Main's consumed sleep generation.
            (directory / "native-theme-interrupt").unlink()
            self.assertEqual((directory / "native-theme-power-epoch").read_text().strip(), base.split(":")[1] + ":1")
            release.touch()
            self.wait_until(lambda: len(self.native_theme_events("power")) > prior_power_queries)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Native theme handoff could not complete safely", error)
        self.assertTrue(any(event.startswith("theme:power:active:") for event in self.native_theme_events("power")))
        self.assertNotIn("step=theme-power-state", error)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(self.native_theme_result_observations(), [])
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def test_native_theme_handoff_queued_screensaver_epoch_prevents_the_setter(self):
        self.assert_native_theme_queued_power_interrupt("goingToScreenSaver 2")

    def test_native_theme_handoff_queued_usb_epoch_prevents_the_setter(self):
        self.assert_native_theme_queued_power_interrupt("usbConfigured 1")

    def test_native_theme_handoff_replaced_app_record_cleans_the_original_without_signaling_a_foreign_pid(self):
        awesome = self.native_theme_handoff_fixture()
        unrelated, foreign, signals = self.unrelated_power_process()
        armed, blocked, _count = self.install_supervisor_query_fixture("lipc-get-prop", behavior="released", gated=True)
        original = []

        def drive(_supervisor):
            pid, _base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            armed.touch()
            self.request_native_theme(pid, "1", "Y8INV")
            self.wait_until(blocked.exists)
            directory = self.root / "lock"
            self.write_fixture_record(directory / "application-pid", str(foreign["pid"]))
            self.write_fixture_record(directory / "application-birth", foreign["birth"])
            (self.root / "supervisor-query-release").touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(self.native_theme_result_observations(), [])
        self.assertIsNone(unrelated.poll())
        self.assertFalse(signals.exists(), "A mutable App record authorized a signal to another process.")
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def test_native_theme_handoff_after_resident_wake_clears_the_old_power_interrupt(self):
        awesome = self.native_theme_handoff_fixture()
        original = []

        def drive(supervisor):
            pid, base = self.wait_for_native_theme_application()
            original.append(self.observed_identity(pid))
            first = self.request_native_theme(pid, "1", "Y8INV")
            self.assertEqual(self.wait_for_native_theme_ready(pid, first), first + ":ok:Y8INV")
            request = self.suspend_managed_application(pid)
            directory = self.root / "lock"
            self.assertEqual((directory / "native-theme-interrupt").read_text().strip(), base)
            self.assertEqual((directory / "native-theme-power-epoch").read_text().strip(), base.split(":")[1] + ":1")
            self.wake_managed_application(pid, request)
            self.wait_until(lambda: not (directory / "sleep-request").exists())
            self.assertFalse((directory / "native-theme-interrupt").exists())
            self.assertEqual((directory / "generation").read_text().strip(), base)
            second = self.request_native_theme(pid, "2", "Y8")
            self.assertEqual(self.wait_for_native_theme_ready(pid, second), second + ":ok:Y8")
            self.assertEqual(self.managed_pids(), [pid])
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(len(self.native_theme_events("set")), 2)
        self.assertEqual(self.managed_received_signals(original[0]["pid"]).intersection(
            {"SIGUSR1", "SIGUSR2", "SIGCONT"}), {"SIGUSR1", "SIGUSR2", "SIGCONT"})
        self.assert_native_theme_resident_cleanup(awesome, original[0])

    def assert_guarded_native_redraw_display(self, display):
        expected = ":0.0" if display is None else display
        self.env.update(TEST_XREFRESH_RECORD=str(self.root / "xrefresh-record"), TEST_XREFRESH_EXPECTED_DISPLAY=expected)
        if display is None:
            self.env.pop("DISPLAY", None)
        else:
            self.env["DISPLAY"] = display
        body = textwrap.dedent(r'''
            import json
            import os
            from pathlib import Path
            import sys
            context = Path(os.environ["WEREAD_HOME"]).parent / "lock"
            fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
            group = os.getpgrp()
            group_fields = Path("/proc/" + str(group) + "/stat").read_text().rsplit(")", 1)[1].split()
            try:
                fifo = os.readlink("/proc/self/fd/3")
            except FileNotFoundError:
                fifo = "closed"
            entry = {"pid": os.getpid(), "birth": fields[19], "group": group,
                     "group_birth": group_fields[19], "master": int((context / "pid").read_text()),
                     "display": os.environ.get("DISPLAY"), "fifo": fifo}
            Path(os.environ["TEST_XREFRESH_RECORD"]).write_text(json.dumps(entry))
            with Path(os.environ["TEST_TRACE"]).open("a") as output:
                output.write("xrefresh:\n")
            sys.exit(0 if entry["display"] == os.environ["TEST_XREFRESH_EXPECTED_DISPLAY"] else 79)
        ''').lstrip()
        path = self.bin / "xrefresh"
        path.write_text("#!/usr/bin/env python3\n" + body)
        path.chmod(0o755)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        entry = json.loads((self.root / "xrefresh-record").read_text())
        self.assertEqual(entry["display"], expected)
        self.assertEqual(entry["fifo"], "closed")
        self.assertNotEqual(entry["group"], entry["master"])
        self.assertFalse(self.native_theme_identity_is_alive(entry))
        self.assertFalse(self.native_theme_identity_is_alive({"pid": entry["group"], "birth": entry["group_birth"]}))
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertNotIn("step=native-redraw", error)
        self.assertFalse((self.root / "lock").exists())

    def test_native_redraw_defaults_unset_display_for_the_guarded_helper(self):
        self.assert_guarded_native_redraw_display(None)

    def test_native_redraw_preserves_explicit_display_for_the_guarded_helper(self):
        self.assert_guarded_native_redraw_display(":8.1")

    def test_native_theme_snapshots_follow_each_resident_wake_without_setting_properties(self):
        awesome = self.native_dark_fixture(mode="Y8INV", TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        requests = []

        def drive(_):
            pid = self.wait_for_managed_application()
            for mode in ("Y8", "Y8INV"):
                request = self.suspend_managed_application(pid)
                self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
                Path(self.env["TEST_NATIVE_THEME_MODE"]).write_text(mode + "\n")
                Path(self.env["TEST_NATIVE_THEME_PREF"]).write_text(mode + "\n")
                requests.append(request + ":" + mode)
                self.wake_managed_application(pid, request)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        initial = self.native_theme_snapshots("native-snapshot")
        self.assertEqual(len(initial), 1)
        fields = initial[0].split(":")
        self.assertEqual((fields[0], fields[-1]), ("1", "Y8INV"))
        self.assertRegex(fields[1], r"^[0-9a-f]{32}$")
        self.assertEqual(self.native_theme_snapshots("native-resume-snapshot"), requests)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual([event.split(":")[2] for event in self.native_theme_events("get")], ["Y8INV", "Y8", "Y8INV"])
        for event in self.native_theme_events("get"):
            self.assertNotIn("wm=T", event)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_ordinary_success_applies_dark_and_verifies_before_statusbar_restore(self):
        awesome = self.native_dark_fixture()
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertTrue(self.native_theme_snapshots("native-snapshot")[0].endswith(":Y8"))
        self.assert_native_theme_applied_before_restore("Y8INV", "Y8INV", error)
        self.assert_native_theme_restored(awesome)

    def test_native_theme_unknown_wake_clears_the_previous_cycle_snapshot(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        snapshots = []

        def drive(_):
            pid = self.wait_for_managed_application()
            previous = self.suspend_managed_application(pid)
            Path(self.env["TEST_NATIVE_THEME_MODE"]).write_text("Y8INV\n")
            self.wake_managed_application(pid, previous)
            snapshots.append(previous + ":Y8INV")
            current = self.suspend_managed_application(pid)
            Path(self.env["TEST_NATIVE_THEME_MODE"]).write_text("UNKNOWN\n")
            self.wake_managed_application(pid, current)
            self.assertFalse((self.root / "lock" / "managed-native-epdc-mode").exists())
            snapshots.append("")
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.native_theme_snapshots("native-resume-snapshot"), snapshots)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_explicit_zero_clears_the_native_dark_preference(self):
        awesome = self.native_dark_fixture(mode="Y8INV", TEST_NATIVE_THEME_TARGET="0")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assert_native_theme_applied_before_restore("Y8", "Y8", error)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_PREF"]).read_text().strip(), "Y8")
        self.assert_native_theme_restored(awesome)

    def test_native_theme_failed_application_exit_cannot_apply_its_target(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT_STATUS="7")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 7, error)
        self.assertTrue(self.native_theme_snapshots("native-snapshot")[0].endswith(":Y8"))
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_sleep_and_usb_never_apply_a_pending_target(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold")

        def drive(_):
            pid = self.wait_for_managed_application()
            request = self.suspend_managed_application(pid)
            self.assertEqual(self.native_theme_events("set"), [])
            self.wake_managed_application(pid, request)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_external_term_does_not_apply_an_existing_ordinary_marker(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold-ordinary")

        def drive(child):
            self.wait_for_managed_application()
            self.assertTrue((self.root / "lock" / "ordinary-exit").exists())
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def assert_native_theme_target_is_rejected(self, scope="current", value="1"):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_TARGET_SCOPE=scope, TEST_NATIVE_THEME_TARGET=value)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertTrue(self.native_theme_snapshots("native-snapshot")[0].endswith(":Y8"))
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_stale_generation_target_is_rejected(self):
        self.assert_native_theme_target_is_rejected(scope="stale-generation")

    def test_native_theme_stale_nonce_target_is_rejected(self):
        self.assert_native_theme_target_is_rejected(scope="stale-nonce")

    def test_native_theme_target_must_be_an_exact_boolean_digit(self):
        self.assert_native_theme_target_is_rejected(value="2")

    def test_native_theme_unknown_mode_never_becomes_a_snapshot_or_a_setter(self):
        awesome = self.native_dark_fixture(mode="UNKNOWN")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_theme_snapshots("native-snapshot"), [""])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "UNKNOWN")
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_getter_timeout_reaps_its_group_and_restores_native_control(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_GET_TIMEOUT="1")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_theme_snapshots("native-snapshot"), [""])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_startup_window_query_timeout_reaps_its_group_and_continues(self):
        awesome = self.native_dark_fixture()
        _, ready, _ = self.install_supervisor_query_fixture("xwininfo")
        self.observe_cleanup_descriptor()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertIn("Native settings are unavailable", error)
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_startup_window_query_term_closes_fifo_before_display_ownership(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        _, ready, _ = self.install_supervisor_query_fixture("xwininfo")
        self.observe_cleanup_descriptor()

        def drive(child):
            self.wait_until(ready.exists)
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("stop:statusbar", self.trace())
        self.assertNotIn("start:statusbar", self.trace())
        self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
        self.assertIsNone(awesome.poll())
        self.assert_supervisor_query_resources_released()

    def test_supervisor_window_flood_rejects_a_zero_exit_truncated_capture_before_reading(self):
        awesome = self.native_dark_fixture()
        _, ready, count = self.install_supervisor_query_fixture("xwininfo", behavior="flood")
        self.observe_native_capture_reads(65536)
        self.observe_cleanup_descriptor()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 0, error)
        # dash/ash expose different block units. Both fixed kernel limits stay
        # above the logical tree quota and at or below this capture hard cap.
        self.assertGreater(int(count.read_text()), 65536)
        self.assertLessEqual(int(count.read_text()), 262144)
        self.assertIn("Native settings are unavailable", error)
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_wake_power_query_term_reaps_the_group_and_restores_native_control(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        armed, ready, _ = self.install_supervisor_query_fixture("lipc-get-prop", gated=True)
        self.observe_cleanup_descriptor()

        def drive(child):
            pid = self.wait_for_managed_application()
            self.suspend_managed_application(pid)
            armed.touch()
            self.emit("outOfScreenSaver 1")
            self.wait_until(ready.exists)
            self.assertFalse(any(event.startswith("managed:wake:") for event in self.managed_events()))
            self.assertNotEqual(self.process_state(awesome.pid), "T")
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_wake_power_query_observer_eof_still_cleans_fifo_and_native_control(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        armed, ready, _ = self.install_supervisor_query_fixture("lipc-get-prop", gated=True)
        observer_exit = self.root / "query-observer-eof"
        self.env["TEST_QUERY_OBSERVER_EOF"] = str(observer_exit)
        self.script(self.bin / "lipc-wait-event", textwrap.dedent(r'''
            while [ ! -e "$TEST_QUERY_OBSERVER_EOF" ]; do sleep 0.01; done
            exit 1
        ''').lstrip())
        self.observe_cleanup_descriptor()

        def drive(_):
            pid = self.wait_for_managed_application()
            self.suspend_managed_application(pid)
            armed.touch()
            self.emit("outOfScreenSaver 1")
            self.wait_until(ready.exists)
            observer_exit.touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Power event observation ended unexpectedly", error)
        self.assertFalse(any(event.startswith("managed:wake:") for event in self.managed_events()))
        self.assertEqual(self.starts(), ["0"])
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_oversize_power_reply_defers_wake_and_allows_a_later_valid_query(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold", TEST_NATIVE_THEME_TARGET="")
        armed, ready, count = self.install_supervisor_query_fixture("lipc-get-prop", behavior="oversize", gated=True)
        self.observe_native_capture_reads(4096)
        self.observe_cleanup_descriptor()

        def drive(_):
            pid = self.wait_for_managed_application()
            request = self.suspend_managed_application(pid)
            armed.touch()
            self.emit("outOfScreenSaver 1")
            self.wait_until(ready.exists)
            self.wait_until(lambda: all(
                not self.native_theme_identity_is_alive(entry)
                for entry in self.native_theme_identities() if entry["role"] == "query-command"
            ))
            self.assertGreater(int(count.read_text()), 4096)
            self.assertFalse(any(event.startswith("managed:wake:") for event in self.managed_events()))
            self.assertNotEqual(self.process_state(awesome.pid), "T")
            armed.unlink()
            self.wait_until(lambda: f"managed:wake-ready:{pid}:{request}" in self.managed_events())
            self.wait_until(lambda: self.process_state(pid) not in (None, "T", "Z", "X"))
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)

    def test_supervisor_status_timeout_aborts_preparation_without_claiming_display(self):
        awesome = self.native_dark_fixture()
        _, ready, _ = self.install_supervisor_query_fixture("status")
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 1, error)
        self.assertIn("statusbar state is unavailable", error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("stop:statusbar", self.trace())
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 0, 0), "after": (0, 0, 0)})
        self.assertIsNone(awesome.poll())
        self.assertNotEqual(self.process_state(awesome.pid), "T")
        self.assert_supervisor_query_resources_released()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_supervisor_pidof_timeout_is_not_a_legitimate_empty_process_snapshot(self):
        awesome = self.native_dark_fixture()
        _, ready, _ = self.install_supervisor_query_fixture("pidof")
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 1, error)
        self.assertIn("process query is unavailable", error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("stop:statusbar", self.trace())
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 0, 0), "after": (0, 0, 0)})
        self.assertIsNone(awesome.poll())
        self.assertNotEqual(self.process_state(awesome.pid), "T")
        self.assert_supervisor_query_resources_released()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_supervisor_pidof_exit_one_with_zero_output_preserves_its_empty_success_semantics(self):
        awesome = self.native_dark_fixture()
        _, ready, count = self.install_supervisor_query_fixture("pidof", behavior="empty")
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 0, error)
        self.assertEqual(int(count.read_text()), 0)
        self.assertEqual(self.starts(), ["0"])
        self.assertNotIn("Display preparation failed", error)
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (1, 1, 1), "after": (0, 0, 0)})
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_supervisor_unconfirmed_statusbar_stop_restores_its_observed_running_baseline(self):
        awesome = self.native_dark_fixture()
        _, ready, _ = self.install_supervisor_query_fixture("stop", apply_before_block=True)
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 1, error)
        self.assertIn("statusbar stop was not confirmed", error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 0, 1), "after": (0, 0, 0)})
        self.assertIn("stop:statusbar", self.trace())
        self.assert_supervisor_query_resources_released()
        self.assert_native_theme_restored(awesome)
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_supervisor_statusbar_start_timeout_retains_its_flag_and_continues_other_cleanup(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT_STATUS="7")
        _, ready, _ = self.install_supervisor_query_fixture("start")
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 7, error)
        self.assertIn("Cleanup failed for native display", error)
        self.assertEqual(self.starts(), ["0"])
        self.assertNotIn("start:statusbar", self.trace())
        self.assertIn("xrefresh:", self.trace())
        self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
        self.assertIsNone(awesome.poll())
        self.assertEqual(self.display_cleanup_flags(), {"before": (1, 1, 1), "after": (1, 1, 1)})
        self.assert_supervisor_query_resources_released()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_supervisor_xrefresh_timeout_preserves_prior_error_and_releases_groups_fifo_and_wm(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT_STATUS="7")
        _, ready, _ = self.install_supervisor_query_fixture("xrefresh")
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()

        status, _, error = self.run_launcher(lambda _: self.wait_until(ready.exists))
        self.assertEqual(status, 7, error)
        self.assertIn("Cleanup failed for native display", error)
        self.assertEqual(self.starts(), ["0"])
        self.assertIn("start:statusbar", self.trace())
        self.assertNotIn("xrefresh:", self.trace())
        self.wait_until(lambda: self.process_state(awesome.pid) not in (None, "T", "Z", "X"))
        self.assertIsNone(awesome.poll())
        self.assertEqual(self.display_cleanup_flags(), {"before": (1, 1, 1), "after": (1, 0, 0)})
        self.assert_supervisor_query_resources_released()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_window_manager_stop_pending_identity_is_restored_on_term(self):
        self.assert_window_manager_stop_signal_window("TERM")

    def test_window_manager_stop_pending_identity_is_restored_on_hup(self):
        self.assert_window_manager_stop_signal_window("HUP")

    def test_window_manager_stop_transfer_overlap_continues_once(self):
        self.assert_window_manager_stop_signal_window("TERM", overlap=True)

    def test_window_manager_failed_stop_drops_pending_and_restores_partial_preparation(self):
        awesome = self.native_dark_fixture()
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()
        failed = self.root / "wm-stop-failed"
        self.env["TEST_WM_STOP_FAILED"] = str(failed)
        source = self.launcher.read_text()
        anchor = "\ntake_display\nstart_application 0\n"
        self.assertIn(anchor, source)
        replacement = textwrap.dedent(r'''
            kill() {
                if [ "$#" -eq 2 ] && [ "$1" = -STOP ] && [ "$2" = "$TEST_AWESOME_PID" ]; then
                    : >"$TEST_WM_STOP_FAILED"
                    return 1
                fi
                command kill "$@"
            }
        ''').lstrip()
        self.launcher.write_text(source.replace(anchor, "\n" + replacement + anchor, 1))

        status, _, error = self.run_launcher()
        self.assertEqual(status, 1, error)
        self.assertTrue(failed.exists(), "The fixture must reject the actual WM STOP.")
        self.assertIn("window manager stop was not confirmed", error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 1, 1), "after": (0, 0, 0)})
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_window_manager_changed_identity_before_stop_is_never_signalled(self):
        awesome = self.native_dark_fixture()
        self.observe_cleanup_descriptor()
        self.observe_display_cleanup_flags()
        armed = self.root / "wm-identity-changed"
        self.env.update(
            TEST_WM_IDENTITY_CHANGED=str(armed),
            TEST_WM_WRONG_BIRTH=str(int(self.observed_identity(awesome.pid)["birth"]) + 1),
        )
        source = self.launcher.read_text()
        self.assertIn("process_birth() {", source)
        source = source.replace("process_birth() {", "fixture_original_process_birth() {", 1)
        shim = textwrap.dedent(r'''
            process_birth() {
                if [ "$1" = "$TEST_AWESOME_PID" ] && [ -e "$TEST_WM_IDENTITY_CHANGED" ]; then
                    printf '%s\n' "$TEST_WM_WRONG_BIRTH"
                else
                    fixture_original_process_birth "$@"
                fi
            }
        ''').lstrip()
        original = "fixture_original_process_birth() {\n"
        self.assertEqual(source.count(original), 1)
        # Power startup now checks birth before taking the display. Install the
        # delegating shim before any consumer, then change only the WM identity
        # after its original running identity has actually been registered.
        source = source.replace(original, shim + "\n" + original, 1)
        pending = '        WM_STOP_PENDING_IDENTITY="$wm_pid:$wm_birth"\n'
        self.assertIn(pending, source)
        source = source.replace(pending, pending + '        : >"$TEST_WM_IDENTITY_CHANGED"\n', 1)
        self.launcher.write_text(source)

        status, _, error = self.run_launcher()
        self.assertEqual(status, 1, error)
        self.assertTrue(armed.exists(), "The fixture must change identity after pre-registration.")
        self.assertIn("window manager stop was not confirmed", error)
        self.assertEqual(self.starts(), [])
        self.assertNotIn("wm:CONT", self.trace())
        self.assertEqual(self.display_cleanup_flags(), {"before": (0, 1, 1), "after": (0, 0, 0)})
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")

    def test_native_theme_setter_timeout_reaps_its_group_and_restores_native_control(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_SET_TIMEOUT="1")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(len(self.native_theme_events("set")), 1)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_readback_mismatch_is_reported_without_repeating_the_setter(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_READBACK_MISMATCH="1")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(len(self.native_theme_events("set")), 1)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_persistent_preference_mismatch_is_reported_after_active_readback(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_PREF_MISMATCH="1")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(len(self.native_theme_events("set")), 1)
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8INV")
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_PREF"]).read_text().strip(), "Y8")
        self.assertEqual(len(self.native_theme_events("pref")), 1)
        self.assert_native_theme_failure_is_visible(error)
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_previously_stopped_awesome_is_not_sampled_continued_or_changed(self):
        awesome = self.native_dark_fixture(stopped=True)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_theme_snapshots("native-snapshot"), [""])
        self.assertEqual(self.native_theme_events("get"), [])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(self.native_theme_identities(), [])
        self.assertNotIn("wm:CONT", self.trace())
        self.assert_native_theme_restored(awesome, stopped=True)

    def assert_native_theme_unsupported(self, profile, firmware):
        awesome = self.native_dark_fixture(profile=profile, firmware=firmware)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_theme_snapshots("native-snapshot"), [""])
        self.assertEqual(self.native_theme_events("get"), [])
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(self.native_theme_identities(), [])
        self.assert_native_theme_restored(awesome)

    def test_native_theme_unreviewed_profile_does_not_access_the_property(self):
        self.assert_native_theme_unsupported("pw6", "5.20.1")

    def test_native_theme_unreviewed_firmware_does_not_access_the_property(self):
        self.assert_native_theme_unsupported("scribe", "5.18.6")

    def test_native_theme_inactive_power_state_skips_the_setter(self):
        awesome = self.native_dark_fixture(TEST_POWER_STATE="screenSaver")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertTrue(self.native_theme_snapshots("native-snapshot")[0].endswith(":Y8"))
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_power_change_during_mode_query_prevents_the_setter(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_POWER_AFTER_MODE_GET="screenSaver")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertTrue(self.native_theme_snapshots("native-snapshot")[0].endswith(":Y8"))
        self.assertEqual(self.native_theme_events("set"), [])
        power = self.native_theme_events("power")
        self.assertTrue(any(event.startswith("theme:power:active:") for event in power))
        self.assertTrue(power[-1].startswith("theme:power:screenSaver:"))
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def test_native_theme_dynconf_helper_is_optional_after_active_readback(self):
        awesome = self.native_dark_fixture()
        (self.bin / "get-dynconf-value").unlink()
        system = self.root / "without-dynconf"
        system.mkdir()
        for name in ("sh", "awk", "sed", "head", "tr", "mkdir", "cat", "od", "mkfifo",
                     "sleep", "rm", "rmdir", "mv", "grep", "dirname", "basename", "env", "setsid", "wc"):
            command = shutil.which(name)
            self.assertIsNotNone(command, "The isolated fixture requires the standard command " + name)
            (system / name).symlink_to(command)
        (system / "python3").symlink_to(sys.executable)
        self.env["PATH"] = str(self.bin) + ":" + str(system)
        self.assertIsNone(shutil.which("get-dynconf-value", path=self.env["PATH"]))
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assert_native_theme_applied_before_restore("Y8INV", "Y8INV", error, persisted=False)
        self.assertEqual(self.native_theme_events("pref"), [])
        self.assert_native_theme_restored(awesome)

    def test_unaccepted_firmware_changes_no_services(self):
        (self.home / "data" / "device-allowlist").write_text("# Empty until acceptance.\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1)
        self.assertIn("has not been accepted", error)
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "args").exists())
        self.assertFalse((self.root / "lock").exists())

    def test_success_restores_running_statusbar_and_passes_config(self):
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertEqual((self.root / "approval").read_text(), "pw6|5.20.1")
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(
            (self.root / "args").read_text().splitlines(),
            ["--kindle", "--device", "pw6", "--data-dir", str(self.home / "data")],
        )
        self.assertFalse((self.root / "lock").exists())

    def test_pid_write_failure_releases_launcher_lock_on_every_attempt(self):
        self.launcher.write_text(
            self.launcher.read_text().replace('>"$LOCK_DIR/pid"', '>/dev/full', 1)
        )
        for _ in range(8):
            status, _, error = self.run_launcher()
            self.assertNotEqual(status, 0, error)
            self.assertFalse((self.root / "lock").exists())
            self.assertEqual(self.trace(), [])
            self.assertEqual(self.starts(), [])

    def observe_lock_cleanup(self):
        self.lock_cleanup_entry = self.root / "lock-cleanup-entry"
        self.lock_cleanup_entered = self.root / "owned-cleanup-entered"
        self.env.update(TEST_LOCK_CLEANUP_ENTRY=str(self.lock_cleanup_entry),
                        TEST_OWNED_CLEANUP_ENTERED=str(self.lock_cleanup_entered))
        source = self.launcher.read_text()
        entry = "    saved_status=$1\n"
        self.assertEqual(source.count(entry), 1)
        source = source.replace(entry, entry
                                + '    printf "%s:%s:%s:%s:%s:%s\\n" "$LOCK_OWNED" "$EVENT_FD_OPEN" "$DISPLAY_OWNED" "$STATUSBAR_STOPPED" "$STATUSBAR_RESTORE_REQUIRED" "$LAUNCH_STARTUP_SIGNAL" >"$TEST_LOCK_CLEANUP_ENTRY"\n', 1)
        owned = "    CLEANUP_RUNNING=1\n"
        self.assertEqual(source.count(owned), 1)
        self.launcher.write_text(source.replace(owned, owned + '    : >"$TEST_OWNED_CLEANUP_ENTERED"\n', 1))
        self.observe_cleanup_descriptor()

    def lock_cleanup_state(self):
        return tuple(int(field) for field in self.lock_cleanup_entry.read_text().strip().split(":"))

    def foreign_display_lock(self, owner_pid):
        directory = self.root / "lock"
        directory.mkdir(mode=0o700)
        (directory / "foreign-private").mkdir(mode=0o700)
        records = {"pid": str(owner_pid).encode() + b"\n", "nonce": b"foreign-nonce\n",
                   "generation": b"foreign-generation\n", "power-events": b"foreign-event-state\n",
                   "foreign-private/state": b"\x00foreign private state\xff"}
        self.foreign_display_paths = [directory, directory / "foreign-private"]
        for name, content in records.items():
            path = directory / name
            path.write_bytes(content)
            path.chmod(0o600)
            self.foreign_display_paths.append(path)
        return self.foreign_display_snapshot()

    def foreign_display_snapshot(self):
        return {str(path.relative_to(self.root)): (path.stat().st_ino, path.stat().st_mode,
                                                   path.read_bytes() if path.is_file() else None)
                for path in self.foreign_display_paths}

    def assert_lock_owner_assignment_signal(self, number):
        self.observe_lock_cleanup()
        baseline = self.launcher.read_text()
        ready = self.root / "lock-owner-assignment-ready"
        self.env["TEST_LOCK_OWNER_READY"] = str(ready)
        assignment = "    LOCK_OWNED=1\n"
        self.assertEqual(baseline.count(assignment), 1)
        hook = textwrap.dedent(r'''
            [ -d "$LOCK_DIR" ] && [ "$LOCK_OWNED" -eq 0 ] || exit 81
            : >"$TEST_LOCK_OWNER_READY"
            fixture_lock_checks=200
            while [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                [ "$fixture_lock_checks" -gt 0 ] || exit 82
                usleep 10000 || :
                fixture_lock_checks=$((fixture_lock_checks - 1))
            done
        ''')
        self.launcher.write_text(baseline.replace(assignment, hook + assignment, 1))
        def drive(supervisor):
            self.wait_until(ready.exists)
            directory = self.root / "lock"
            self.assertTrue(directory.is_dir())
            self.assertEqual(list(directory.iterdir()), [], "A process or record started before lock registration.")
            supervisor.send_signal(number)
        status, _, error = self.run_launcher(drive)
        expected = 128 + number
        self.assertEqual(status, expected, error)
        self.assertEqual(self.lock_cleanup_state(), (1, 0, 0, 0, 0, expected))
        self.assertTrue(self.lock_cleanup_entered.exists())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        self.assertFalse((self.root / "lock").exists())
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        # Removing an actual empty lock must allow a later normal launch.
        self.launcher.write_text(baseline)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertFalse((self.root / "lock").exists())
        self.assertEqual(self.starts(), ["0"])

    def test_lock_success_term_before_owner_registration_reclaims_the_empty_directory(self):
        self.assert_lock_owner_assignment_signal(signal.SIGTERM)

    def test_lock_success_hup_before_owner_registration_reclaims_the_empty_directory(self):
        self.assert_lock_owner_assignment_signal(signal.SIGHUP)

    def assert_definition_phase_signal(self, number, foreign=False):
        self.observe_lock_cleanup()
        unrelated = identity = signals = snapshot = None
        if foreign:
            unrelated, identity, signals = self.unrelated_power_process()
            snapshot = self.foreign_display_lock(unrelated.pid)
        baseline = self.launcher.read_text()
        ready = self.root / "definition-phase-ready"
        self.env["TEST_DEFINITION_PHASE_READY"] = str(ready)
        anchor = "APP_RUNNER_PID=''\n"
        self.assertIn(anchor, baseline)
        hook = textwrap.dedent(r'''
            [ "$LOCK_OWNED" -eq 0 ] || exit 81
            : >"$TEST_DEFINITION_PHASE_READY"
            # No child process is created before the full cleanup definitions.
            # A finite builtin loop holds the parent at this exact source phase.
            fixture_definition_checks=20000000
            while [ "$fixture_definition_checks" -gt 0 ]; do
                fixture_definition_checks=$((fixture_definition_checks - 1))
            done
            exit 82
        ''')
        self.launcher.write_text(baseline.replace(anchor, hook + anchor, 1))
        def drive(supervisor):
            self.wait_until(ready.exists)
            if foreign:
                self.assertEqual(self.foreign_display_snapshot(), snapshot)
            else:
                self.assertFalse((self.root / "lock").exists())
            supervisor.send_signal(number)
        status, _, error = self.run_launcher(drive)
        # Before trap registration the shell may retain the kernel signal exit.
        self.assertIn(status, (-number, 128 + number), error)
        self.assertFalse(self.lock_cleanup_entry.exists())
        self.assertFalse(self.lock_cleanup_entered.exists())
        self.assertFalse(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).exists())
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        if foreign:
            self.assertEqual(self.foreign_display_snapshot(), snapshot)
            self.assertIsNone(unrelated.poll())
            self.assertTrue(self.original_process_exists(identity))
            self.assertFalse(signals.exists())
        else:
            self.assertFalse((self.root / "lock").exists())
            self.launcher.write_text(baseline)
            status, _, error = self.run_launcher()
            self.assertEqual(status, 0, error)
            self.assertFalse((self.root / "lock").exists())

    def test_definition_phase_term_leaves_no_lock_and_allows_a_later_launch(self):
        self.assert_definition_phase_signal(signal.SIGTERM)

    def test_definition_phase_hup_never_changes_a_foreign_display_lock(self):
        self.assert_definition_phase_signal(signal.SIGHUP, foreign=True)

    def assert_failed_lock_signal(self, number):
        self.observe_lock_cleanup()
        unrelated, identity, signals = self.unrelated_power_process()
        snapshot = self.foreign_display_lock(unrelated.pid)
        attempted = self.root / "foreign-lock-rejected"
        self.env["TEST_FOREIGN_LOCK_REJECTED"] = str(attempted)
        baseline = self.launcher.read_text()
        acquisition = 'begin_owned_registration\nif mkdir "$LOCK_DIR" 2>/dev/null; then\n'
        self.assertEqual(baseline.count(acquisition), 1)
        first = "TERM" if number == signal.SIGTERM else "HUP"
        second = "HUP" if first == "TERM" else "TERM"
        wrapper = textwrap.dedent(r'''
            mkdir() {
                if [ "$#" -eq 1 ] && [ "$1" = "$LOCK_DIR" ]; then
                    if command mkdir "$@"; then return 0; else fixture_mutex_status=$?; fi
                    : >"$TEST_FOREIGN_LOCK_REJECTED"
                    kill -FIRST "$$"
                    kill -SECOND "$$"
                    return "$fixture_mutex_status"
                fi
                command mkdir "$@"
            }
        ''').replace("-FIRST", "-" + first).replace("-SECOND", "-" + second)
        self.launcher.write_text(baseline.replace(acquisition, wrapper + acquisition, 1))
        status, _, error = self.run_launcher()
        expected = 128 + number
        self.assertEqual(status, expected, error)
        self.assertTrue(attempted.exists(), "The actual foreign-lock mkdir failure was not exercised.")
        self.assertEqual(self.lock_cleanup_state(), (0, 0, 0, 0, 0, expected))
        self.assertFalse(self.lock_cleanup_entered.exists(), "Cleanup entered a lock owned by another launcher.")
        self.assertFalse(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).exists())
        self.assertEqual(self.foreign_display_snapshot(), snapshot)
        self.assertIsNone(unrelated.poll())
        self.assertTrue(self.original_process_exists(identity))
        self.assertFalse(signals.exists())
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        # A normal rejected launch must preserve the same foreign lock as well.
        self.launcher.write_text(baseline)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1, error)
        self.assertIn("Another launcher owns the display lock", error)
        self.assertEqual(self.foreign_display_snapshot(), snapshot)
        self.assertFalse(self.lock_cleanup_entered.exists())
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])

    def test_failed_lock_acquisition_term_preserves_foreign_state_and_the_first_signal(self):
        self.assert_failed_lock_signal(signal.SIGTERM)

    def test_failed_lock_acquisition_hup_preserves_foreign_state_and_the_first_signal(self):
        self.assert_failed_lock_signal(signal.SIGHUP)

    def test_first_terminal_signal_survives_a_failed_initial_owner_record_write(self):
        self.observe_lock_cleanup()
        baseline = self.launcher.read_text()
        record = 'printf \'%s\\n\' "$$" >"$LOCK_DIR/pid"\n'
        self.assertEqual(baseline.count(record), 1)
        hook = textwrap.dedent(r'''
            begin_owned_registration
            kill -TERM "$$"
            kill -HUP "$$"
        ''')
        self.launcher.write_text(baseline.replace(record, hook + record.replace('>"$LOCK_DIR/pid"', '>/dev/full'), 1))
        status, _, error = self.run_launcher()
        self.assertEqual(status, 143, error)
        self.assertEqual(self.lock_cleanup_state(), (1, 0, 0, 0, 0, 143))
        self.assertTrue(self.lock_cleanup_entered.exists())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        self.assertFalse((self.root / "lock").exists())
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        self.launcher.write_text(baseline)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertFalse((self.root / "lock").exists())

    def test_cleanup_observer_unlink_failure_still_reaps_application_and_restores_services(self):
        self.assert_cleanup_record_failure("observer-pid")

    def test_cleanup_application_record_failure_still_restores_native_control(self):
        self.assert_cleanup_record_failure("application-pid")

    def test_cleanup_final_marker_failure_turns_an_original_success_into_failure(self):
        self.assert_cleanup_record_failure("pid", original_status=0)

    def test_cleanup_retained_final_marker_preserves_original_failure_and_restores_services(self):
        self.assert_cleanup_record_failure("pid", original_status=7, remove_before_failure=False)

    def test_cleanup_native_record_failure_still_reaps_its_live_child(self):
        application = (self.home / "app" / "weread").read_text()
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_TARGET="")
        (self.home / "app" / "weread").write_text(application)
        self.native_fixture("wifi")
        self.env["TEST_NATIVE_BLOCKER_READY"] = str(self.root / "native-blocker-ready")
        self.script(
            self.bin / "lipc-set-prop",
            'trap "" HUP INT TERM\n'
            ': >"$TEST_NATIVE_BLOCKER_READY"\n'
            'exec sleep 30',
        )
        armed = self.install_path_failure("rm", "native-child-pid", remove_before_failure=True)
        observed = []

        def drive(child):
            path = self.root / "lock" / "native-child-pid"
            self.wait_until(
                lambda: path.exists() and path.read_text().strip().isdigit()
                and Path(self.env["TEST_NATIVE_BLOCKER_READY"]).exists()
            )
            observed.append(self.observed_identity(int(path.read_text())))
            observer = self.root / "lock" / "observer-pid"
            observed.append(self.observed_identity(int(observer.read_text())))
            armed.touch()
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assert_cleanup_resources_restored(awesome, observed, error)
        self.assertFalse((self.root / "lock").exists())
        self.assertTrue(Path(self.env["TEST_FILE_FAULT_TRACE"]).exists())

    def test_cleanup_preserves_an_unexpected_marker_directory_and_restores_resources(self):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT="hold")
        self.observe_cleanup_descriptor()
        unexpected = self.root / "lock" / "state-open.pending"
        observed = []

        def drive(child):
            observed.extend(self.wait_for_cleanup_fixture())
            unexpected.mkdir()
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assert_cleanup_resources_restored(awesome, observed, error)
        self.assertTrue(unexpected.is_dir(), "Cleanup must not recursively remove an unexpected object.")

    def test_cleanup_native_theme_stale_records_cannot_release_another_command(self):
        exit_gate = self.root / "application-exit-gate"
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT_GATE=str(exit_gate))
        armed = self.install_path_failure("rm", "native-theme-done")
        self.observe_theme_permission_helpers()
        observed = []
        startup_power = []

        def drive(child):
            observed.extend(self.wait_for_cleanup_fixture())
            startup_power.extend(self.assert_startup_portrait_power_reads())
            directory = self.root / "lock"
            (directory / "native-theme-done").write_text("0\n")
            (directory / "native-theme-output").write_text("active\n")
            (directory / "native-theme-go").write_text("1:0:stale-token\n")
            armed.touch()
            exit_gate.touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assert_cleanup_resources_restored(awesome, observed, error, native_controls_blocked=True)
        self.assertFalse(Path(self.env["TEST_THEME_PERMISSION_HELPERS"]).exists())
        self.assertEqual(self.native_theme_events("power"), startup_power)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_PREF"]).read_text().strip(), "Y8")
        self.assertEqual((self.root / "lock" / "native-theme-done").read_text(), "0\n")

    def assert_startup_portrait_power_reads(self):
        # The current launcher pairs startup portrait metadata before this
        # cleanup fault is armed. The fault window must add zero property calls.
        power = self.native_theme_events("power")
        self.assertEqual(len(power), 2)
        for event in power:
            self.assertRegex(event, r"^theme:power:active:wm=[RSDI]$")
        return power

    def assert_native_theme_permission_publication_failure(self, failure):
        exit_gate = self.root / "application-exit-gate"
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_EXIT_GATE=str(exit_gate))
        if failure == "rename":
            armed = self.install_path_failure("mv", "native-theme-go")
        else:
            armed = self.root / "file-fault-armed"
            self.env["TEST_FILE_FAULT_ARMED"] = str(armed)
            self.observe_cleanup_descriptor()
            source = self.launcher.read_text()
            anchor = "                if ! printf '%s:%s:%s\\n' \"$NATIVE_THEME_COMMAND_PID\" \"$NATIVE_THEME_COMMAND_SEQUENCE\" \"$LAUNCH_TOKEN\" >\"$LOCK_DIR/native-theme-go.pending\" ||\n"
            self.assertIn(anchor, source)
            replacement = (
                '                fixture_permission_path="$LOCK_DIR/native-theme-go.pending"\n'
                '                if [ -e "$TEST_FILE_FAULT_ARMED" ]; then fixture_permission_path=/dev/full; fi\n'
                + anchor.replace('>"$LOCK_DIR/native-theme-go.pending"', '>"$fixture_permission_path"')
            )
            self.launcher.write_text(source.replace(anchor, replacement, 1))
        self.observe_theme_permission_helpers()
        observed = []
        startup_power = []

        def drive(child):
            observed.extend(self.wait_for_cleanup_fixture())
            startup_power.extend(self.assert_startup_portrait_power_reads())
            armed.touch()
            exit_gate.touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assert_cleanup_resources_restored(awesome, observed, error, native_controls_blocked=True)
        self.assert_theme_permission_helpers_stopped()
        self.assertEqual(self.native_theme_events("power"), startup_power)
        self.assertEqual(self.native_theme_events("set"), [])
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_MODE"]).read_text().strip(), "Y8")
        self.assertEqual(Path(self.env["TEST_NATIVE_THEME_PREF"]).read_text().strip(), "Y8")
        self.assertIn("permission publication", error)
        self.assertFalse((self.root / "lock").exists())

    def test_cleanup_native_theme_go_rename_failure_reaps_its_unreleased_group(self):
        self.assert_native_theme_permission_publication_failure("rename")

    def test_cleanup_native_theme_go_write_failure_reaps_its_unreleased_group(self):
        self.assert_native_theme_permission_publication_failure("write")

    def assert_failed_child_record_is_reaped(self, record, fixture, identity_line):
        if record == "observer-pid":
            return self.assert_power_observer_publication_failure("pid-write", attempts=4)
        self.env["TEST_RECORD_CHILD_PID"] = str(self.root / "record-child-pid")
        self.env["TEST_RECORD_CHILD_READY"] = str(self.root / "record-child-ready")
        self.script(
            fixture,
            'trap "" HUP INT TERM\n'
            'IFS= read -r identity </proc/self/stat\n'
            'printf "%s\\n" "${identity%% *}" >"$TEST_RECORD_CHILD_PID"\n'
            ': >"$TEST_RECORD_CHILD_READY"\n'
            'exec sleep 30',
        )
        source = self.launcher.read_text()
        source = source.replace(f'>"$LOCK_DIR/{record}"', '>/dev/full', 1)
        # Ensure the injected failure observes a live child that ignores TERM.
        source = source.replace(
            identity_line,
            identity_line
            + '    while [ ! -e "$TEST_RECORD_CHILD_READY" ]; do usleep 10000; done\n',
            1,
        )
        self.launcher.write_text(source)
        for attempt in range(4):
            with self.subTest(record=record, attempt=attempt):
                Path(self.env["TEST_RECORD_CHILD_PID"]).unlink(missing_ok=True)
                Path(self.env["TEST_RECORD_CHILD_READY"]).unlink(missing_ok=True)
                status, _, error = self.run_launcher()
                self.assertNotEqual(status, 0, error)
                self.assertFalse((self.root / "lock").exists())
                child = int(Path(self.env["TEST_RECORD_CHILD_PID"]).read_text().strip())
                self.assertIsNone(self.process_state(child), "A failed identity write left its child behind.")

    def test_application_pid_write_failure_reaps_its_unrecorded_child(self):
        self.assert_failed_child_record_is_reaped(
            "application-pid", self.home / "app" / "weread",
            "    runner_child_parent=$WRAPPER_PID\n",
        )

    def test_application_birth_write_failure_reaps_its_unrecorded_child(self):
        self.assert_failed_child_record_is_reaped(
            "application-birth", self.home / "app" / "weread",
            "    runner_child_parent=$WRAPPER_PID\n",
        )

    def inject_child_birth_failures(self, target_line, target, count=1):
        failures = self.root / "birth-sample-failures"
        self.env["TEST_BIRTH_SAMPLE_FAILURES"] = str(failures)
        source = self.launcher.read_text()
        sampling = "read_wrapper_child_birth() {\n    WRAPPER_CHILD_BIRTH=''\n"
        self.assertIn(sampling, source)
        source = source.replace(
            sampling,
            sampling
            + '    if [ "${fixture_birth_failures:-0}" -gt 0 ] && [ "$1" = "${fixture_birth_target:-}" ]; then\n'
            + '        fixture_birth_failures=$((fixture_birth_failures - 1))\n'
            + '        printf "%s\\n" "$1" >>"$TEST_BIRTH_SAMPLE_FAILURES"\n'
            + '        return 1\n'
            + '    fi\n',
            1,
        )
        self.assertIn(target_line, source)
        source = source.replace(
            target_line,
            target_line
            + f"    fixture_birth_failures={count}\n"
            + f"    fixture_birth_target=${target}\n",
            1,
        )
        self.launcher.write_text(source)
        return failures

    def test_application_retries_an_initial_birth_read_before_publishing_started(self):
        failures = self.inject_child_birth_failures(
            "    runner_child_parent=$WRAPPER_PID\n", "runner_child"
        )
        self.env["TEST_SLEEP"] = "1"
        observed = {}

        def drive(child):
            owned = self.root / "lock" / "application-owned"
            self.wait_until(owned.exists)
            fields = owned.read_text().strip().split(":")
            self.assertEqual(len(fields), 4)
            self.assertTrue(fields[2].isdigit() and fields[3].isdigit())
            observed["pid"] = fields[2]
            self.assertEqual(
                (self.root / "lock" / "application-birth").read_text().strip(), fields[3]
            )
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(failures.read_text().splitlines(), [observed["pid"]])
        self.assertFalse((self.root / "lock").exists())

    def test_application_initial_birth_read_failure_still_reaps_a_term_ignoring_child(self):
        failures = self.inject_child_birth_failures(
            "    runner_child_parent=$WRAPPER_PID\n", "runner_child"
        )
        self.assert_failed_child_record_is_reaped(
            "application-pid", self.home / "app" / "weread",
            "    runner_child_parent=$WRAPPER_PID\n",
        )
        self.assertEqual(len(failures.read_text().splitlines()), 4)

    def test_runner_birth_recovery_retries_after_the_startup_identity_was_unavailable(self):
        failures = self.inject_child_birth_failures(
            "    APP_RUNNER_PID=$!\n", "APP_RUNNER_PID"
        )
        self.launcher.write_text(
            self.launcher.read_text().replace(
                '    APP_RUNNER_BIRTH=$(process_birth "$APP_RUNNER_PID" 2>/dev/null || :)\n',
                "    APP_RUNNER_BIRTH=''\n",
                1,
            )
        )
        self.env["TEST_SLEEP"] = "1"
        observed = {}

        def drive(child):
            record = self.root / "lock" / "application-pid"
            self.wait_until(lambda: record.exists() and self.starts() == ["0"])
            application_pid = int(record.read_text().strip())
            fields = Path(f"/proc/{application_pid}/stat").read_text().rsplit(")", 1)[1].split()
            observed["runner"] = fields[1]
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(failures.read_text().splitlines(), [observed["runner"]])
        self.assertFalse((self.root / "lock").exists())

    def test_failed_birth_capture_never_publishes_started_and_cleanup_recovers_the_child(self):
        failures = self.inject_child_birth_failures(
            "    runner_child_parent=$WRAPPER_PID\n", "runner_child", count=3
        )
        published = self.root / "unproven-started"
        self.env["TEST_UNPROVEN_STARTED"] = str(published)
        source = self.launcher.read_text().replace(
            "    wrapper_capture_checks=$(managed_wait_budget 1)\n",
            "    wrapper_capture_checks=2\n",
            1,
        )
        source = source.replace(
            "        printf '_weread_app_started ",
            '        : >"$TEST_UNPROVEN_STARTED"\n'
            + "        printf '_weread_app_started ",
            1,
        )
        self.launcher.write_text(source)
        self.assert_failed_child_record_is_reaped(
            "application-pid", self.home / "app" / "weread",
            "    runner_child_parent=$WRAPPER_PID\n",
        )
        self.assertEqual(len(failures.read_text().splitlines()), 12)
        self.assertFalse(published.exists(), "An unproven child was advertised as started.")

    def test_stop_application_refuses_to_wait_for_a_live_runner_with_a_wrong_birth(self):
        source = self.launcher.read_text()
        entry = 'trap \'cleanup "$?"\' 0\n'
        self.assertIn(entry, source)
        source = source.split(entry, 1)[0] + textwrap.dedent(r'''
            trap '' TERM
            # This standalone helper scenario does not run Main's lock entry.
            mkdir "$LOCK_DIR" || exit 80
            LOCK_OWNED=1
            sleep 30 &
            fixture_runner=$!
            capture_wrapper_child_birth "$fixture_runner" "$$" || exit 81
            fixture_runner_birth=$WRAPPER_CHILD_BIRTH
            APP_RUNNER_PID=$fixture_runner
            APP_RUNNER_BIRTH=0
            stop_application
            [ "$APP_STATUS" -eq 1 ] || exit 82
            owned_child_is_running "$fixture_runner" "$fixture_runner_birth" "$$" || exit 83
            printf '%s\n' refused-unproven-wait >>"$TEST_TRACE"
            signal_owned_process "$fixture_runner" "$fixture_runner_birth" "$$" KILL || exit 84
            fixture_exit_checks=100
            while ! owned_child_has_exited "$fixture_runner" "$fixture_runner_birth" "$$" &&
                [ "$fixture_exit_checks" -gt 0 ]; do
                usleep 10000
                fixture_exit_checks=$((fixture_exit_checks - 1))
            done
            owned_child_has_exited "$fixture_runner" "$fixture_runner_birth" "$$" || exit 85
            wait "$fixture_runner" 2>/dev/null || :
            rmdir "$LOCK_DIR"
            exit 0
        ''')
        self.launcher.write_text(source)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertIn("blocking wait was refused", error)
        self.assertEqual(self.trace(), ["refused-unproven-wait"])
        self.assertFalse((self.root / "lock").exists())

    def test_event_pid_write_failure_reaps_its_unrecorded_child_and_notifies_supervisor(self):
        self.assert_failed_child_record_is_reaped(
            "observer-pid", self.bin / "lipc-wait-event",
            "    observer_child_birth=$WRAPPER_CHILD_BIRTH\n",
        )

    def test_event_initial_birth_read_failure_still_reaps_its_term_ignoring_child(self):
        failures = self.inject_child_birth_failures("    observer_child=$!\n", "observer_child")
        self.assert_failed_child_record_is_reaped(
            "observer-pid", self.bin / "lipc-wait-event",
            "    observer_child_birth=$WRAPPER_CHILD_BIRTH\n",
        )
        self.assertEqual(len(failures.read_text().splitlines()), 4)

    def test_native_pid_write_failure_reaps_its_unrecorded_child(self):
        self.native_fixture("wifi")
        self.assert_failed_child_record_is_reaped(
            "native-child-pid", self.bin / "lipc-set-prop",
            "        native_child_birth=$WRAPPER_CHILD_BIRTH\n",
        )

    def test_native_initial_birth_read_failure_still_reaps_its_term_ignoring_child(self):
        self.native_fixture("wifi")
        failures = self.inject_child_birth_failures("        native_child=$!\n", "native_child")
        self.assert_failed_child_record_is_reaped(
            "native-child-pid", self.bin / "lipc-set-prop",
            "        native_child_birth=$WRAPPER_CHILD_BIRTH\n",
        )
        self.assertEqual(len(failures.read_text().splitlines()), 4)

    def test_managed_launcher_diagnostics_stay_bounded_in_the_open_log_inode(self):
        logs = self.home / "data" / "logs"
        logs.mkdir(exist_ok=True)
        log_path = logs / "launcher.log"
        log_path.write_bytes(b"old-log\n" * 20000)
        retained_log = log_path.open("rb")
        self.addCleanup(retained_log.close)
        original_log = os.fstat(retained_log.fileno())
        self.env["TEST_LAUNCHER_MESSAGE"] = "diagnostic:" + "x" * 2048
        source = self.launcher.read_text()
        anchor = "command -v lipc-wait-event >/dev/null 2>&1 || fail 'Power event observation is required.'"
        source = source.replace(
            anchor,
            'diagnostic_count=0\n'
            'while [ "$diagnostic_count" -lt 96 ]; do\n'
            '    launcher_log "%s\\n" "$TEST_LAUNCHER_MESSAGE"\n'
            '    diagnostic_count=$((diagnostic_count + 1))\n'
            'done\n'
            'launcher_log "%s\\n" "fixture-log-end"\n'
            + anchor,
            1,
        )
        self.launcher.write_text(source)
        status, _, error = self.run_launcher(managed_logs=True)
        content = log_path.read_bytes()
        context = f"status={status}, stderr={error!r}, log_size={len(content)}, log_tail={content[-2048:]!r}"
        self.assertEqual(status, 0, context)
        # The marker is written before observers start. Their subsequent
        # shutdown diagnostics are valid lifecycle records, not leaked logs.
        self.assertTrue(b"fixture-log-end\n" in content.splitlines(keepends=True), context)
        self.assertLessEqual(len(content), 65536 + len(self.env["TEST_LAUNCHER_MESSAGE"]) + 1, context)
        self.assertNotIn(b"old-log", content, context)
        current_log = log_path.stat()
        self.assertEqual((current_log.st_dev, current_log.st_ino),
                         (original_log.st_dev, original_log.st_ino), context)
        retained_log.seek(0)
        self.assertEqual(retained_log.read(), content, context)
        self.assertFalse((self.root / "lock").exists(), context)

    def test_normal_entry_uses_managed_logs_and_overwrites_bootstrap_output(self):
        real_setsid = shutil.which("setsid", path=self.env["PATH"])
        self.assertIsNotNone(real_setsid, "The isolated entry fixture requires setsid.")
        identity_path = self.root / "detached-identity"
        finished_path = self.root / "detached-finished"
        self.env["TEST_DETACHED_IDENTITY"] = str(identity_path)
        self.env["TEST_DETACHED_FINISHED"] = str(finished_path)
        self.env["TEST_DETACHED_LAUNCHER"] = str(self.launcher.resolve())
        anchor = self.root / "detached-anchor.py"
        anchor.write_text(textwrap.dedent(r'''
            import json
            import os
            from pathlib import Path
            import signal
            import subprocess
            import sys
            import time

            pid = os.getpid()
            fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
            identity = {"pid": pid, "birth": fields[19],
                        "group": os.getpgrp(), "session": os.getsid(0)}
            if identity["group"] != pid or identity["session"] != pid:
                raise RuntimeError("The detached fixture must own a private session.")
            # Keep a birth-verified group leader alive until test cleanup, so a
            # failed supervisor cannot leave an unpinned numeric group identity.
            signal.signal(signal.SIGTERM, lambda *_: None)
            identity_path = Path(os.environ["TEST_DETACHED_IDENTITY"])
            pending = identity_path.with_suffix(".pending")
            pending.write_text(json.dumps(identity))
            pending.replace(identity_path)
            child = subprocess.Popen(sys.argv[1:])
            deadline = time.monotonic() + 15
            reported = False
            while time.monotonic() < deadline:
                status = child.poll()
                if status is not None and not reported:
                    Path(os.environ["TEST_DETACHED_FINISHED"]).write_text(str(status))
                    reported = True
                time.sleep(0.01)
            # This still-live leader pins its own private group even if the
            # test process is interrupted before running registered cleanup.
            os.killpg(pid, signal.SIGKILL)
        '''))
        self.script(
            self.bin / "setsid",
            # Only the top-level ordinary entry uses the retained anchor. The
            # owned sh -c helpers must keep their actual group-leader identity.
            'if [ "$#" -eq 4 ] && [ "$1" = sh ] && [ "$2" = "$TEST_DETACHED_LAUNCHER" ] && '
            '[ "$3" = --detached ] && [ "$4" = --managed-logs ]; then\n'
            f'    exec {shlex.quote(real_setsid)} {shlex.quote(sys.executable)} '
            f'{shlex.quote(str(anchor))} "$@"\n'
            'fi\n'
            f'exec {shlex.quote(real_setsid)} "$@"',
        )

        def cleanup_detached_group():
            deadline = time.monotonic() + 1
            while not identity_path.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            if not identity_path.exists():
                return
            entry = json.loads(identity_path.read_text())
            if not self.native_theme_identity_is_alive(entry):
                return
            try:
                if (entry["group"] == entry["session"] == entry["pid"]
                        and os.getpgid(entry["pid"]) == entry["group"]
                        and os.getsid(entry["pid"]) == entry["session"]):
                    os.killpg(entry["group"], signal.SIGTERM)
                    time.sleep(0.1)
                    if self.native_theme_identity_is_alive(entry):
                        os.killpg(entry["group"], signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.wait_until(lambda: not self.native_theme_identity_is_alive(entry))

        self.addCleanup(cleanup_detached_group)
        logs = self.home / "data" / "logs"
        logs.mkdir(exist_ok=True)
        (logs / "last-launch.log").write_bytes(b"stale startup\n" * 10000)
        (logs / "application-bootstrap.log").write_bytes(b"stale bootstrap\n" * 10000)
        log_path = logs / "launcher.log"
        log_path.write_bytes(b"stale supervisor\n" * 10000)
        retained_log = log_path.open("rb")
        self.addCleanup(retained_log.close)
        original_log = os.fstat(retained_log.fileno())
        self.env["TEST_NORMAL_ENTRY_READY"] = str(self.root / "normal-entry-ready")
        self.script(
            self.home / "app" / "weread",
            'printf "%s\\n" "fixture bootstrap" >&2\n'
            ': >"$TEST_NORMAL_ENTRY_READY"\n'
            'exit 0',
        )
        result = subprocess.run(
            ["sh", str(self.launcher)], env=self.env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=8,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.wait_until(
            lambda: Path(self.env["TEST_NORMAL_ENTRY_READY"]).exists()
            and not (self.root / "lock").exists()
            and finished_path.exists()
        )
        self.assertEqual(finished_path.read_text(), "0")
        entry = json.loads(identity_path.read_text())
        live_members = []
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            try:
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
            except (OSError, IndexError):
                continue
            if (int(fields[2]) == entry["group"] and fields[0] not in ("Z", "X")
                    and int(path.name) != entry["pid"]):
                live_members.append(int(path.name))
        self.assertEqual(live_members, [], "The detached supervisor left a live group member.")
        self.assertEqual((logs / "application-bootstrap.log").read_text(), "fixture bootstrap\n")
        content = log_path.read_bytes()
        bootstrap = (logs / "last-launch.log").read_bytes()
        context = f"log_size={len(content)}, log_tail={content[-2048:]!r}, bootstrap_tail={bootstrap[-2048:]!r}"
        # A normal launch clears old content while retaining legitimate
        # supervisor/observer diagnostics produced during cleanup.
        self.assertNotIn(b"stale supervisor", content, context)
        self.assertLessEqual(len(content), 65536, context)
        current_log = log_path.stat()
        self.assertEqual((current_log.st_dev, current_log.st_ino),
                         (original_log.st_dev, original_log.st_ino), context)
        retained_log.seek(0)
        self.assertEqual(retained_log.read(), content, context)
        self.assertNotIn(b"stale startup", bootstrap, context)
        self.assertLess(len(bootstrap), 65536, context)

    def test_failure_preserves_exit_code_and_restores_services(self):
        self.env["TEST_APP_EXIT"] = "7"
        status, _, _ = self.run_launcher()
        self.assertEqual(status, 7)
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertFalse((self.root / "lock").exists())

    def test_previously_stopped_statusbar_is_left_stopped(self):
        self.env["TEST_STATUSBAR"] = "stop/waiting"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.trace(), ["xrefresh:"])

    def test_windows_configuration_line_endings_are_accepted(self):
        (self.home / "data" / "device-profile").write_bytes(b"pw6\r\n")
        (self.home / "data" / "device-allowlist").write_bytes(b"pw6 5.20.1\r\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual((self.root / "approval").read_text(), "pw6|5.20.1")

    def test_usb_takeover_terminates_the_supervisor_without_resuming(self):
        self.env.update(TEST_EVENT="usbConfigured example", TEST_SLEEP="1")
        status, _, _ = self.run_launcher()
        self.assertEqual(status, 143)
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse((self.root / "lock").exists())

    def test_sleep_releases_resources_and_wake_resumes_the_saved_reader(self):
        self.env["TEST_SLEEP"] = "1"

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2")
            self.wait_until(lambda: "xrefresh:" in self.trace())
            self.assertTrue((self.root / "lock").exists())
            self.assertEqual(self.application_trace(), ["started:0", "saved:0"])
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertEqual(
            self.application_trace(),
            ["started:0", "saved:0", "started:1", "restored:book-7:3:42"],
        )
        self.assertEqual(
            self.trace(),
            ["stop:statusbar", "start:statusbar", "xrefresh:"] * 2,
        )
        self.assertFalse((self.root / "lock").exists())

    def test_wake_arriving_during_application_cleanup_is_not_lost(self):
        self.env.update(TEST_SLEEP="1", TEST_STOP_DELAY="0.2")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertEqual(
            self.application_trace(),
            ["started:0", "saved:0", "started:1", "restored:book-7:3:42"],
        )

    def test_sleep_before_state_open_retries_the_original_normal_start(self):
        self.env.update(
            TEST_SLEEP="1",
            TEST_INITIALIZATION_GENERATION="1",
            TEST_HOLD_UNTIL_GENERATION="2",
        )
        (self.root / "reader-checkpoint").write_text("stale-book:1:99\n")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertIn("Initialization interrupted", error)
        self.assertEqual(self.starts(), ["0", "0"])
        self.assertEqual(self.application_trace(), ["started:0", "interrupted:0", "started:0"])

    def test_early_term_before_state_open_retries_instead_of_losing_the_launch(self):
        self.env.update(
            TEST_SLEEP="1",
            TEST_TERM_EXIT="143",
            TEST_INITIALIZATION_GENERATION="1",
            TEST_HOLD_UNTIL_GENERATION="2",
        )

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "0"])

    def test_sleep_during_resume_initialization_retries_the_saved_reader_intent(self):
        self.env.update(
            TEST_SLEEP="1",
            TEST_RESUME_HOLD="1",
            TEST_INITIALIZATION_GENERATION="2",
            TEST_HOLD_UNTIL_GENERATION="3",
        )

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")
            self.wait_until(lambda: self.starts() == ["0", "1"])
            self.assertFalse((self.root / "lock" / "state-open").exists())
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1", "1"])
        self.assertEqual(
            self.application_trace(),
            [
                "started:0", "saved:0", "started:1", "interrupted:1",
                "started:1", "restored:book-7:3:42",
            ],
        )

    def test_an_initialization_failure_is_not_treated_as_a_sleep_cancellation(self):
        self.env.update(TEST_SLEEP="1", TEST_TERM_EXIT="7", TEST_INITIALIZATION_GENERATION="1")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, _ = self.run_launcher(drive)
        self.assertEqual(status, 7)
        self.assertEqual(self.starts(), ["0"])

    def test_repeated_sleep_wake_and_duplicate_events_start_one_app_per_cycle(self):
        self.env.update(TEST_SLEEP="1", TEST_RESUME_HOLD="1")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            for expected in (["0", "1"], ["0", "1", "1"]):
                self.emit(
                    "goingToScreenSaver 2",
                    "goingToScreenSaver 2",
                    "outOfScreenSaver 1",
                    "outOfScreenSaver 1",
                    "exitingScreenSaver 1",
                    "exitingScreenSaver 1",
                )
                self.wait_until(lambda: self.starts() == expected)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0", "1", "1"])
        self.assertEqual(
            self.trace(),
            ["stop:statusbar", "start:statusbar", "xrefresh:"] * 3,
        )

    def test_a_new_sleep_cancels_the_previous_wake_intent(self):
        self.env.update(TEST_SLEEP="1", TEST_STOP_DELAY="0.2")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit(
                "goingToScreenSaver 2",
                "outOfScreenSaver 1",
                "goingToScreenSaver 2",
                "exitingScreenSaver 1",
            )
            self.wait_until(lambda: "xrefresh:" in self.trace())
            time.sleep(0.2)
            self.assertEqual(self.starts(), ["0"])
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])

    def test_wake_probe_stopped_wrapper_and_sleep_are_reaped_before_resident_resume(self):
        awesome = self.wake_probe_fixture()

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            request = self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            record = self.wait_for_wake_probe(supervisor)
            for identity in (record["child"], record["wrapper"]):
                os.kill(identity["pid"], signal.SIGSTOP)
                self.wait_until(lambda: self.process_state(identity["pid"]) == "T")
            self.emit("exitingScreenSaver 1")
            wake_ready = self.root / "lock" / "managed-wake-ready"
            self.wait_until(
                lambda: wake_ready.exists() and wake_ready.read_text().strip() == request
                and self.process_state(app_pid) not in (None, "T", "Z", "X")
            )
            self.wait_until(lambda: all(
                not self.original_process_exists(identity)
                for identity in (record["wrapper"], record["child"])
            ))
            self.assertEqual(self.managed_pids(), [app_pid])
            self.assertEqual(self.starts(), ["0"])
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_wake_probe_term_ignoring_sleep_is_killed_and_reaped_on_usb(self):
        awesome = self.wake_probe_fixture(term_ignoring=True)

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            self.wait_for_wake_probe(supervisor)
            self.capture_wake_fixture_child()
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertIn(str(signal.SIGTERM), self.wake_probe_signal_path.read_text().splitlines())
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_wake_probe_unresponsive_wrapper_uses_its_verified_child_cleanup_fallback(self):
        awesome = self.wake_probe_fixture(term_ignoring=True)
        ready = self.root / "wake-wrapper-unresponsive"
        self.env["TEST_WAKE_WRAPPER_UNRESPONSIVE"] = str(ready)
        source = self.launcher.read_text()
        anchor = "    while :; do\n        probe_interrupted=0\n"
        self.assertEqual(source.count(anchor), 1)
        self.launcher.write_text(source.replace(
            anchor,
            '    trap "" HUP INT TERM\n'
            '    : >"$TEST_WAKE_WRAPPER_UNRESPONSIVE"\n' + anchor,
            1,
        ))

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            self.wait_for_wake_probe(supervisor)
            self.capture_wake_fixture_child()
            self.wait_until(ready.exists)
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertIn(str(signal.SIGTERM), self.wake_probe_signal_path.read_text().splitlines())
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def assert_wake_probe_child_publication_failure(self, failure):
        awesome = self.wake_probe_fixture(term_ignoring=True)
        source = self.launcher.read_text()
        captured = "        probe_child_birth=$WRAPPER_CHILD_BIRTH\n"
        self.assertEqual(source.count(captured), 1)
        # Prove that cleanup sees a live, TERM-ignoring child even when the
        # durable publication has never succeeded.
        source = source.replace(
            captured,
            captured
            + '        fixture_ready_checks=200\n'
            + '        while [ ! -e "$TEST_WAKE_SLEEP_IDENTITY" ]; do\n'
            + '            [ "$fixture_ready_checks" -gt 0 ] || exit 1\n'
            + '            usleep 10000\n'
            + '            fixture_ready_checks=$((fixture_ready_checks - 1))\n'
            + '        done\n',
            1,
        )
        if failure == "write":
            anchor = '>"$LOCK_DIR/wake-probe-child.pending"'
            self.assertEqual(source.count(anchor), 1)
            source = source.replace(anchor, '>/dev/full', 1)
        self.launcher.write_text(source)
        armed = self.install_path_failure("mv", "wake-probe-child") if failure == "rename" else None

        def drive(_supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            if armed is not None:
                armed.touch()
            self.emit("outOfScreenSaver 1")
            self.capture_wake_fixture_child()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertTrue(any(message in error for message in (
            "Wake probe startup failed; native ownership is preserved.",
            "Wake probe failed; native ownership is preserved.",
        )), error)
        self.assertEqual(self.starts(), ["0"])
        self.assertIn(str(signal.SIGTERM), self.wake_probe_signal_path.read_text().splitlines())
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)
        if armed is not None:
            self.assertTrue(Path(self.env["TEST_FILE_FAULT_TRACE"]).exists())
            self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")

    def test_wake_probe_child_record_write_failure_reaps_its_unpublished_sleep(self):
        self.assert_wake_probe_child_publication_failure("write")

    def test_wake_probe_child_record_rename_failure_reaps_its_unpublished_sleep(self):
        self.assert_wake_probe_child_publication_failure("rename")

    def assert_wake_probe_registration_record(self, supervisor, record, delay):
        self.assertIsNotNone(record)
        for identity, parent in ((record["wrapper"], supervisor.pid),
                                 (record["child"], record["wrapper"]["pid"])):
            fields = Path(f"/proc/{identity['pid']}/stat").read_text().rsplit(")", 1)[1].split()
            self.assertEqual(fields[19], identity["birth"])
            self.assertEqual(int(fields[1]), parent)
            self.assertEqual(int(fields[2]), supervisor.pid)
            self.assertNotIn(fields[0], ("Z", "X"))
            self.wake_probe_observed.append(identity.copy())
        self.assertEqual(record["token"], (self.root / "lock" / "nonce").read_text().strip())
        self.assertEqual(os.readlink(f"/proc/{record['wrapper']['pid']}/fd/3"),
                         str(self.root / "lock" / "power-events"))

        def timer_started():
            if not self.original_process_exists(record["child"]):
                return False
            try:
                arguments = Path(f"/proc/{record['child']['pid']}/cmdline").read_bytes().split(b"\0")
                return (len(arguments) == 3 and arguments[-1] == b""
                        and Path(os.fsdecode(arguments[0])).name == "sleep"
                        and arguments[1] == str(delay).encode())
            except OSError:
                return False

        self.wait_until(timer_started)
        with self.assertRaises(FileNotFoundError):
            os.readlink(f"/proc/{record['child']['pid']}/fd/3")
        self.assertTrue(self.original_process_exists(record["child"]))

    def assert_wake_probe_registration_followup(self, awesome):
        # A new launch reuses the durable checkpoint and native service fixture.
        # Only the test's historical PID list is reset before its next live lookup.
        (self.root / "managed-pids").unlink()
        Path(self.env["TEST_NATIVE_THEME_POWER"]).write_text("active\n")
        start_count = len(self.starts())
        observed = []

        def drive(supervisor):
            application = self.wait_for_managed_application()
            observed.append(self.observed_identity(application))
            self.assertEqual(self.starts(), ["0"] * (start_count + 1))
            request = self.suspend_managed_application(application)
            self.emit("outOfScreenSaver 1")
            record = self.wait_for_wake_probe(supervisor)
            self.assert_wake_probe_registration_record(supervisor, record, 2)
            ready = self.root / "lock" / "managed-wake-ready"
            self.wait_until(lambda: ready.is_file() and ready.read_text().strip() == request
                            and self.process_state(application) not in (None, "T", "Z", "X"))
            self.wait_until(lambda: all(not self.original_process_exists(identity)
                                       for identity in (record["wrapper"], record["child"])))
            self.assertEqual(self.managed_pids(), [application])
            self.assertEqual(self.starts(), ["0"] * (start_count + 1))
            self.assertFalse(any(":concurrent:" in event for event in self.managed_events()))
            supervisor.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.wait_until(lambda: all(not self.original_process_exists(identity) for identity in observed))
        self.assert_wake_probe_reaped()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")
        self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)

    def assert_wake_probe_registration_signal(self, number, delay=2, inner=False, second=None):
        awesome = self.wake_probe_fixture(preserve_delay=True)
        Path(self.env["TEST_NATIVE_THEME_POWER"]).write_text("screenSaver\n")
        self.observe_cleanup_descriptor()
        window = self.root / "wake-registration-window"
        window_state = self.root / "wake-registration-window-state"
        latched = self.root / "wake-registration-latched"
        release = self.root / "wake-registration-release"
        completed = self.root / "wake-registration-completed"
        signal_events = self.root / "wake-registration-signals"
        self.env.update(
            TEST_WAKE_REGISTRATION_WINDOW=str(window),
            TEST_WAKE_REGISTRATION_STATE=str(window_state),
            TEST_WAKE_REGISTRATION_LATCHED=str(latched),
            TEST_WAKE_REGISTRATION_RELEASE=str(release),
            TEST_WAKE_REGISTRATION_COMPLETED=str(completed),
            TEST_WAKE_REGISTRATION_SIGNALS=str(signal_events),
            TEST_WAKE_REGISTRATION_DELAY=str(delay),
        )
        original = self.launcher.read_text()
        source = original
        signal_anchor = '    if [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; then LAUNCH_STARTUP_SIGNAL=$1; fi\n'
        self.assertEqual(source.count(signal_anchor), 1)
        source = source.replace(signal_anchor, signal_anchor + textwrap.dedent(r'''
            if [ -f "$TEST_WAKE_REGISTRATION_WINDOW" ]; then
                printf '%s|%s|%s\n' "$1" "$LAUNCH_STARTUP_SIGNAL" "$LAUNCH_REGISTRATION_ACTIVE" >>"$TEST_WAKE_REGISTRATION_SIGNALS"
            fi
        '''), 1)
        finish_anchor = 'finish_owned_registration() {\n    LAUNCH_REGISTRATION_ACTIVE=0\n'
        self.assertEqual(source.count(finish_anchor), 1)
        source = source.replace(finish_anchor, 'finish_owned_registration() {\n' + textwrap.dedent(r'''
            if [ -f "$TEST_WAKE_REGISTRATION_WINDOW" ]; then
                printf '%s|%s|%s|%s|%s|%s|%s\n' "$LAUNCH_STARTUP_SIGNAL" "$LAUNCH_REGISTRATION_ACTIVE" "$wake_probe_registration_status" "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$WAKE_PROBE_CHILD_PID" "$WAKE_PROBE_CHILD_BIRTH" >"$TEST_WAKE_REGISTRATION_COMPLETED"
            fi
        ''') + '    LAUNCH_REGISTRATION_ACTIVE=0\n', 1)

        if inner:
            assignment = '    probe_child=$!\n'
            self.assertEqual(source.count(assignment), 1)
            hook = textwrap.dedent(r'''
                fixture_probe_timer=$!
                capture_wrapper_child_birth "$fixture_probe_timer" 2>/dev/null || exit 81
                printf '%s:%s:%s:%s:%s:%s\n' "$probe_cycle" "$LAUNCH_TOKEN" "$WRAPPER_PID" "$WRAPPER_BIRTH" "$fixture_probe_timer" "$WRAPPER_CHILD_BIRTH" >"$TEST_WAKE_REGISTRATION_WINDOW.pending"
                printf '%s|%s\n' "$probe_child" "$probe_stopping" >"$TEST_WAKE_REGISTRATION_STATE"
                mv -f "$TEST_WAKE_REGISTRATION_WINDOW.pending" "$TEST_WAKE_REGISTRATION_WINDOW"
                fixture_probe_checks=200
                while [ "$probe_stopping" -eq 0 ]; do
                    [ "$fixture_probe_checks" -gt 0 ] || exit 82
                    usleep 10000 || :
                    fixture_probe_checks=$((fixture_probe_checks - 1))
                done
                printf '%s\n' "$probe_stopping" >"$TEST_WAKE_REGISTRATION_LATCHED"
                fixture_probe_checks=200
                while [ ! -f "$TEST_WAKE_REGISTRATION_RELEASE" ]; do
                    [ "$fixture_probe_checks" -gt 0 ] || exit 83
                    usleep 10000 || :
                    fixture_probe_checks=$((fixture_probe_checks - 1))
                done
            ''')
        else:
            assignment = '    WAKE_PROBE_PID=$!\n'
            self.assertEqual(source.count(assignment), 1)
            hook = textwrap.dedent(r'''
                if [ "$2" = "$TEST_WAKE_REGISTRATION_DELAY" ]; then
                    fixture_probe_checks=200
                    while [ ! -f "$LOCK_DIR/wake-probe-child" ]; do
                        [ "$fixture_probe_checks" -gt 0 ] || exit 81
                        usleep 10000 || :
                        fixture_probe_checks=$((fixture_probe_checks - 1))
                    done
                    IFS= read -r fixture_probe_record <"$LOCK_DIR/wake-probe-child" || exit 82
                    printf '%s\n' "$fixture_probe_record" >"$TEST_WAKE_REGISTRATION_WINDOW.pending"
                    printf '%s|%s|%s\n' "$2" "$WAKE_PROBE_PID" "$LAUNCH_REGISTRATION_ACTIVE" >"$TEST_WAKE_REGISTRATION_STATE"
                    mv -f "$TEST_WAKE_REGISTRATION_WINDOW.pending" "$TEST_WAKE_REGISTRATION_WINDOW"
                    fixture_probe_checks=200
                    while [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                        [ "$fixture_probe_checks" -gt 0 ] || exit 83
                        usleep 10000 || :
                        fixture_probe_checks=$((fixture_probe_checks - 1))
                    done
                    printf '%s|%s\n' "$LAUNCH_STARTUP_SIGNAL" "$LAUNCH_REGISTRATION_ACTIVE" >"$TEST_WAKE_REGISTRATION_LATCHED"
                    fixture_probe_checks=200
                    while [ ! -f "$TEST_WAKE_REGISTRATION_RELEASE" ]; do
                        [ "$fixture_probe_checks" -gt 0 ] || exit 84
                        usleep 10000 || :
                        fixture_probe_checks=$((fixture_probe_checks - 1))
                    done
                fi
            ''')
        source = source.replace(assignment, hook + assignment, 1)
        self.launcher.write_text(source)
        observed = []

        def drive(supervisor):
            application = self.wait_for_managed_application()
            observed.append(self.observed_identity(application))
            self.suspend_managed_application(application)
            self.emit("outOfScreenSaver 1")
            previous_wrapper = None
            if delay == 10:
                def next_probe_published():
                    record = self.read_wake_probe_record()
                    return record is not None and record["wrapper"] != previous_wrapper

                for _ in range(3):
                    self.wait_until(next_probe_published)
                    previous = self.wait_for_wake_probe(supervisor)
                    self.assert_wake_probe_registration_record(supervisor, previous, 2)
                    previous_wrapper = previous["wrapper"]
            self.wait_until(lambda: window.is_file() and window_state.is_file())
            record = self.read_wake_probe_record(window)
            self.assert_wake_probe_registration_record(supervisor, record, delay)
            self.assertEqual(window_state.read_text().strip(), "|0" if inner else f"{delay}||1")
            # Freeze the real, independently identified sleep. Natural timer exit
            # must not hide a missing cleanup owner at this exact fork boundary.
            os.kill(record["child"]["pid"], signal.SIGSTOP)
            self.wait_until(lambda: self.process_state(record["child"]["pid"]) == "T")
            if delay == 2:
                # This fixture uses an unsupported theme profile, and its first
                # wake timer is still frozen before any active-state query.
                self.assertEqual((self.home / "data" / "device-profile").read_text().strip(), "pw6")
                self.assertEqual(Path(self.env["TEST_NATIVE_THEME_POWER"]).read_text().strip(), "screenSaver")
                self.assertEqual(self.native_theme_identities(), [])
                for kind in ("get", "power", "set"):
                    self.assertEqual(self.native_theme_events(kind), [])
            supervisor.send_signal(number)
            if inner:
                os.kill(record["wrapper"]["pid"], number)
            self.wait_until(latched.is_file)
            expected = {signal.SIGTERM: 143, signal.SIGHUP: 129}[number]
            self.assertEqual(latched.read_text().strip(), "1" if inner else f"{expected}|1")
            self.wait_until(lambda: signal_events.is_file()
                            and f"{expected}|{expected}|1" in signal_events.read_text().splitlines())
            if second is not None:
                supervisor.send_signal(second)
                second_status = {signal.SIGTERM: 143, signal.SIGHUP: 129}[second]
                self.wait_until(lambda: f"{second_status}|{expected}|1" in signal_events.read_text().splitlines())
            release.touch()

        status, _, error = self.run_launcher(drive)
        expected = {signal.SIGTERM: 143, signal.SIGHUP: 129}[number]
        self.assertEqual(status, expected, error)
        fields = completed.read_text().strip().split("|")
        self.assertEqual(len(fields), 7)
        self.assertEqual(fields[:3], [str(expected), "1", "1" if inner else "0"])
        record = self.read_wake_probe_record(window)
        self.assertEqual(fields[3:5], [str(record["wrapper"]["pid"]), record["wrapper"]["birth"]])
        self.assertEqual(fields[5:7], ["", ""] if inner else
                         [str(record["child"]["pid"]), record["child"]["birth"]])
        self.assertEqual(self.starts(), ["0"])
        self.wait_until(lambda: all(not self.original_process_exists(identity) for identity in observed))
        self.assert_wake_probe_reaped()
        self.assertEqual((self.root / "cleanup-descriptor").read_text(), "closed\n")
        if delay == 2:
            # Early terminal cleanup cannot invent a property helper or query.
            # The delay10 path and the successful followup retain the strict
            # nonempty identity and helper-cleanup assertions.
            self.assertEqual(self.native_theme_identities(), [])
            for kind in ("get", "power", "set"):
                self.assertEqual(self.native_theme_events(kind), [])
        else:
            self.assert_native_theme_helpers_finished()
        self.assert_native_theme_restored(awesome)
        self.launcher.write_text(original)
        self.assert_wake_probe_registration_followup(awesome)

    def test_wake_probe_fork_assignment_term_keeps_the_two_second_timer_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGTERM)

    def test_wake_probe_fork_assignment_hup_keeps_the_two_second_timer_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGHUP)

    def test_wake_probe_replacement_term_keeps_the_ten_second_timer_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGTERM, delay=10)

    def test_wake_probe_replacement_hup_keeps_the_ten_second_timer_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGHUP, delay=10)

    def test_wake_probe_timer_fork_term_keeps_unpublished_child_cleanup_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGTERM, inner=True)

    def test_wake_probe_timer_fork_hup_keeps_unpublished_child_cleanup_owned(self):
        self.assert_wake_probe_registration_signal(signal.SIGHUP, inner=True)

    def test_wake_probe_registration_keeps_first_hup_when_term_arrives_before_completion(self):
        self.assert_wake_probe_registration_signal(signal.SIGHUP, second=signal.SIGTERM)

    def test_wake_probe_self_identity_failure_is_reported_after_parent_capture(self):
        awesome = self.wake_probe_fixture()
        captured_path = self.root / "wake-parent-captured"
        self.env["TEST_WAKE_PARENT_CAPTURED"] = str(captured_path)
        source = self.launcher.read_text()
        parent_capture = "    capture_wake_probe_identity || return 1\n"
        self.assertEqual(source.count(parent_capture), 1)
        source = source.replace(
            parent_capture,
            parent_capture
            + '    printf "%s:%s\\n" "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" >"$TEST_WAKE_PARENT_CAPTURED"\n',
            1,
        )
        wrapper_capture = "    trap stop_probe_child HUP INT TERM\n    capture_wrapper_identity || exit 1\n"
        self.assertEqual(source.count(wrapper_capture), 1)
        source = source.replace(
            wrapper_capture,
            "    trap stop_probe_child HUP INT TERM\n"
            # The supervisor must already know this real PID and birth before
            # the wrapper's own observation fails without usable event fields.
            + '    capture_wrapper_identity() {\n'
            + '        fixture_parent_checks=200\n'
            + '        while [ ! -e "$TEST_WAKE_PARENT_CAPTURED" ]; do\n'
            + '            [ "$fixture_parent_checks" -gt 0 ] || return 1\n'
            + '            usleep 10000\n'
            + '            fixture_parent_checks=$((fixture_parent_checks - 1))\n'
            + '        done\n'
            + '        return 1\n'
            + '    }\n'
            + '    capture_wrapper_identity || exit 1\n',
            1,
        )
        self.launcher.write_text(source)
        self.observe_cleanup_descriptor()

        def parent_captured():
            if not captured_path.exists():
                return False
            values = captured_path.read_text().strip().split(":")
            return len(values) == 2 and all(value.isdigit() for value in values)

        def drive(_supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            self.wait_until(parent_captured)
            pid, birth = captured_path.read_text().strip().split(":")
            self.wake_probe_observed.append({"pid": int(pid), "birth": birth})

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Wake probe startup failed; native ownership is preserved.", error)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse(self.wake_probe_identity_path.exists())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_wake_probe_forged_child_record_never_signals_an_unrelated_live_process(self):
        awesome = self.wake_probe_fixture(term_ignoring=True)
        ready = self.root / "unrelated-wake-ready"
        signals = self.root / "unrelated-wake-signals"
        unrelated_path = self.root / "unrelated-wake.py"
        unrelated_path.write_text(textwrap.dedent(r'''
            from pathlib import Path
            import signal
            import sys
            import time
            ready, signals = map(Path, sys.argv[1:])
            def received(number, _frame):
                with signals.open("a") as output:
                    output.write(str(number) + "\n")
            for number in (signal.SIGTERM, signal.SIGCONT, signal.SIGINT, signal.SIGHUP):
                signal.signal(number, received)
            ready.touch()
            while True:
                time.sleep(0.02)
        ''').lstrip())
        unrelated = subprocess.Popen(
            [sys.executable, str(unrelated_path), str(ready), str(signals)],
            start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )

        def reap_unrelated():
            if unrelated.poll() is None:
                unrelated.kill()
            unrelated.wait(timeout=3)

        self.addCleanup(reap_unrelated)
        self.wait_until(ready.exists)
        unrelated_identity = self.observed_identity(unrelated.pid)

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            record = self.wait_for_wake_probe(supervisor)
            self.capture_wake_fixture_child()
            # wait_for_wake_probe's FIFO barrier proves that startup cached the
            # actual child before the mutable record is replaced.
            foreign_fields = Path(f"/proc/{unrelated.pid}/stat").read_text().rsplit(")", 1)[1].split()
            self.assertNotEqual(int(foreign_fields[1]), record["wrapper"]["pid"])
            values = [record["cycle"], record["token"], str(record["wrapper"]["pid"]),
                      record["wrapper"]["birth"], str(unrelated.pid), unrelated_identity["birth"]]
            path = self.root / "lock" / "wake-probe-child"
            pending = path.with_suffix(".fixture")
            pending.write_text(":".join(values) + "\n")
            pending.replace(path)
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertIsNone(unrelated.poll(), "A forged timer record selected an unrelated process.")
        self.assertTrue(self.original_process_exists(unrelated_identity))
        self.assertFalse(signals.exists(), "A forged timer record sent a signal outside its ancestry.")
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_wake_probe_stale_cycle_pid_and_birth_events_cannot_complete_a_new_probe(self):
        awesome = self.wake_probe_fixture()
        barriers = self.observe_wake_probe_barriers()

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            old = self.wait_for_wake_probe(supervisor)
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1")

            def replacement_ready():
                record = self.read_wake_probe_record()
                return record is not None and int(record["cycle"]) > int(old["cycle"])

            self.wait_until(replacement_ready)
            current = self.wait_for_wake_probe(supervisor)
            self.wait_until(lambda: all(not self.original_process_exists(identity)
                                       for identity in (old["wrapper"], old["child"])))
            wrapper = current["wrapper"]
            self.assertNotEqual(old["wrapper"]["pid"], wrapper["pid"])
            admitted_barriers = len(barriers.read_text().splitlines())
            variants = [
                ("cycle", old["cycle"], wrapper["pid"], wrapper["birth"]),
                ("pid", current["cycle"], old["wrapper"]["pid"], wrapper["birth"]),
                ("birth", current["cycle"], wrapper["pid"], str(int(wrapper["birth"]) + 1)),
            ]
            for index, (label, cycle, pid, birth) in enumerate(variants, 1):
                with self.subTest(fence=label):
                    self.emit(
                        f"_weread_wake_check {cycle} {pid} {birth}",
                        f"_weread_wake_probe_error {cycle} {pid} {birth}",
                        f"_weread_fixture_wake_barrier {label}",
                    )
                    self.wait_until(lambda: barriers.exists() and len(barriers.read_text().splitlines())
                                    == admitted_barriers + index)
                    self.assertIsNone(supervisor.poll())
                    self.assertEqual(self.read_wake_probe_record(), current)
                    self.assertTrue(self.original_process_exists(wrapper))
                    self.assertTrue(self.original_process_exists(current["child"]))
                    self.assertEqual(self.process_state(app_pid), "T")
                    self.assertEqual(self.starts(), ["0"])
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_wake_probe_current_error_event_fails_without_resuming_the_reader(self):
        awesome = self.wake_probe_fixture()

        def drive(supervisor):
            app_pid = self.wait_for_managed_application()
            self.suspend_managed_application(app_pid)
            self.emit("outOfScreenSaver 1")
            record = self.wait_for_wake_probe(supervisor)
            wrapper = record["wrapper"]
            self.emit(f"_weread_wake_probe_error {record['cycle']} {wrapper['pid']} {wrapper['birth']}")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertIn("Wake probe failed", error)
        self.assertEqual(self.starts(), ["0"])
        self.assert_wake_probe_reaped()
        self.assert_native_theme_restored(awesome)

    def test_out_event_without_exiting_requires_an_active_native_state(self):
        self.env["TEST_SLEEP"] = "1"

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])

    def test_timer_wake_or_native_unlock_in_progress_does_not_reclaim_input(self):
        self.env.update(TEST_SLEEP="1", TEST_POWER_STATE="screenSaver")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "wakeupFromSuspend 0", "exitingScreenSaver 1")
            self.wait_until(lambda: "xrefresh:" in self.trace())
            time.sleep(0.2)
            self.assertEqual(self.starts(), ["0"])
            self.emit("outOfScreenSaver 1")
            time.sleep(2.2)
            self.assertEqual(self.starts(), ["0"])
            self.emit("usbConfigured 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])

    def test_a_failed_sleep_save_exits_without_reopening_a_stale_checkpoint(self):
        self.env.update(TEST_SLEEP="1", TEST_TERM_EXIT="7")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, _ = self.run_launcher(drive)
        self.assertEqual(status, 7)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])

    def test_a_zero_exit_without_the_current_save_acknowledgment_does_not_resume(self):
        self.env.update(TEST_SLEEP="1", TEST_SKIP_ACK="1")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            # A previous generation cannot acknowledge this sleep request.
            nonce = (self.root / "lock" / "nonce").read_text().strip()
            (self.root / "lock" / "resume-ready").write_text(f"0:{nonce}\n")
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1)
        self.assertIn("did not acknowledge", error)
        self.assertEqual(self.starts(), ["0"])

    def test_an_explicit_exit_during_sleep_cleanup_wins_over_the_pending_wake(self):
        self.env.update(TEST_SLEEP="1", TEST_ORDINARY_ON_STOP="1", TEST_STOP_DELAY="0.2")

        def drive(_):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2", "outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse((self.root / "lock").exists())

    def test_external_term_while_sleeping_exits_the_supervisor(self):
        self.env["TEST_SLEEP"] = "1"

        def drive(child):
            self.wait_until(lambda: self.starts() == ["0"])
            self.emit("goingToScreenSaver 2")
            self.wait_until(lambda: "xrefresh:" in self.trace())
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        # The trace precedes the owned command's final confirmation. EXIT can
        # retry that one uncertain refresh while keeping service toggles unique.
        self.assertIn(self.trace(), (
            ["stop:statusbar", "start:statusbar", "xrefresh:"],
            ["stop:statusbar", "start:statusbar", "xrefresh:", "xrefresh:"],
        ))
        self.assertFalse((self.root / "lock").exists())

    def test_repeated_signals_during_cleanup_preserve_the_first_status_and_restore_services(self):
        self.env.update(
            TEST_SLEEP="1",
            TEST_CLEANUP_ENTERED=str(self.root / "cleanup-entered"),
            TEST_CLEANUP_RELEASE=str(self.root / "cleanup-release"),
        )
        self.script(
            self.bin / "start",
            ': >"$TEST_CLEANUP_ENTERED"\n'
            'while [ ! -e "$TEST_CLEANUP_RELEASE" ]; do sleep 0.01; done\n'
            'printf "start:%s\\n" "$*" >>"$TEST_TRACE"',
        )

        def drive(child):
            self.wait_until(lambda: self.starts() == ["0"])
            child.send_signal(signal.SIGINT)
            self.wait_until(lambda: (self.root / "cleanup-entered").exists())
            for repeated in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
                child.send_signal(repeated)
            time.sleep(0.1)
            self.assertIsNone(child.poll())
            (self.root / "cleanup-release").touch()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 130, error)
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertFalse((self.root / "lock").exists())

    def test_inherited_resume_does_not_reopen_a_book_on_an_ordinary_launch(self):
        self.env["WEREAD_RESUME_SESSION"] = "1"
        self.env["WEREAD_NATIVE_RETURN"] = "1"
        self.env["WEREAD_NATIVE_RETURN_RESULT"] = "failed"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.native_starts(), ["0|0|closed"])

    def test_native_wifi_releases_the_app_before_opening_and_resumes_after_close(self):
        self.native_fixture("wifi")

        def drive(_):
            self.wait_for_native_open("wifi")
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_starts(), ["0|0|closed", "1|1|closed"])
        trace = self.trace()
        opened = next(index for index, value in enumerate(trace) if '"show":true' in value)
        self.assertLess(trace.index("start:statusbar"), opened)
        self.assertLess(trace.index("xrefresh:"), opened)
        self.assertNotIn("native-concurrent-application", trace)
        self.assertIn("restored:book-7:3:42", self.application_trace())
        self.assertFalse((self.root / "lock").exists())

    def test_native_quick_settings_waits_for_its_mapped_window_to_close(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            time.sleep(0.15)
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_starts(), ["0|0|closed", "1|1|closed"])
        self.assertTrue(any("configurePillState openedByTap" in item for item in self.trace()))

    def test_native_wifi_only_queries_navigation_after_its_window_closes(self):
        self.assert_visible_native_avoids_navigation_queries("wifi")

    def test_native_quick_only_queries_navigation_after_its_window_closes(self):
        self.assert_visible_native_avoids_navigation_queries("quick")

    def test_native_child_dialog_prevents_a_premature_return(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            baseline_queries = self.native_queries()
            first_modal_count = self.native_window_polls().count("modal")
            self.set_native_visibility("modal")
            self.wait_until(lambda: self.native_window_polls().count("modal") >= first_modal_count + 8)
            self.assertEqual(self.native_queries(), baseline_queries)
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])

    def test_native_all_settings_keeps_ownership_until_its_original_view_returns(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            (self.root / "native-view").write_text("KPP_SETTINGS\n")
            self.close_native()
            self.wait_until(
                lambda: self.native_queries().count("activeView|none|KPP_SETTINGS") >= 5
            )
            self.assertEqual(self.starts(), ["0"])
            (self.root / "native-view").write_text("KPP_HOME\n")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])
        self.assertGreaterEqual(self.native_queries().count("activeView|none|KPP_HOME"), 8)

    def test_native_request_requires_the_current_saved_checkpoint_acknowledgment(self):
        self.native_fixture("wifi")
        self.env["TEST_NATIVE_SKIP_ACK"] = "1"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1)
        self.assertIn("did not acknowledge a saved native", error)
        self.assertFalse(any(item.startswith("native:") for item in self.trace()))
        self.assertEqual(self.starts(), ["0"])

    def test_failed_native_save_does_not_open_or_resume_a_previous_checkpoint(self):
        self.native_fixture("quick")
        self.env["TEST_NATIVE_APP_EXIT"] = "7"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 7, error)
        self.assertFalse(any(item.startswith("native:") for item in self.trace()))
        self.assertEqual(self.starts(), ["0"])

    def test_an_explicit_exit_wins_over_a_native_handoff(self):
        self.native_fixture("wifi")
        self.env["TEST_NATIVE_ORDINARY"] = "1"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertFalse(any(item.startswith("native:") for item in self.trace()))
        self.assertEqual(self.starts(), ["0"])

    def test_failed_native_wifi_open_preserves_stock_without_canceling_or_restarting(self):
        self.env["TEST_NATIVE_SET_FAIL"] = "1"
        self.assert_native_open_failure_preserves_stock("wifi")

    def test_failed_native_quick_open_preserves_stock_without_canceling_or_restarting(self):
        self.env["TEST_NATIVE_SET_FAIL"] = "1"
        self.assert_native_open_failure_preserves_stock("quick")

    def test_unmapped_native_wifi_open_exits_without_canceling_or_restarting(self):
        self.env["TEST_NATIVE_MAP_ON_OPEN"] = "0"
        self.assert_native_open_failure_preserves_stock("wifi")

    def test_unmapped_native_quick_open_exits_without_canceling_or_restarting(self):
        self.env["TEST_NATIVE_MAP_ON_OPEN"] = "0"
        self.assert_native_open_failure_preserves_stock("quick")

    def test_native_wifi_mapped_before_open_error_remains_visible(self):
        self.env["TEST_NATIVE_SET_FAIL_AFTER_MAP"] = "1"
        self.assert_native_open_failure_preserves_stock("wifi", expected_visible="wifi")

    def test_native_quick_mapped_before_open_error_remains_visible(self):
        self.env["TEST_NATIVE_SET_FAIL_AFTER_MAP"] = "1"
        self.assert_native_open_failure_preserves_stock("quick", expected_visible="quick")

    def test_native_close_while_sleeping_waits_for_native_dismissal_before_return(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            self.emit("goingToScreenSaver 2")
            self.close_native()
            time.sleep(0.2)
            self.assertEqual(self.starts(), ["0"])
            self.emit("exitingScreenSaver 1")
            time.sleep(0.1)
            self.assertEqual(self.starts(), ["0"])
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.native_starts(), ["0|0|closed", "1|1|closed"])

    def test_native_panel_remapped_during_sleep_is_not_replaced_by_an_old_close(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            self.emit("goingToScreenSaver 2")
            self.close_native()
            time.sleep(0.2)
            self.set_native_visibility("quick")
            self.emit("outOfScreenSaver 1", "exitingScreenSaver 1")
            time.sleep(0.2)
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])

    def test_native_query_ignoring_term_is_reaped_on_supervisor_exit(self):
        self.native_fixture("wifi")
        original_query = self.bin / "xwininfo-real"
        (self.bin / "xwininfo").rename(original_query)
        self.env.update(
            TEST_NATIVE_QUERY_REAL=str(original_query),
            TEST_NATIVE_QUERY_HOLD=str(self.root / "native-query-hold"),
            TEST_NATIVE_QUERY_READY=str(self.root / "native-query-ready"),
        )
        self.script(
            self.bin / "xwininfo",
            'if [ -e "$TEST_NATIVE_QUERY_HOLD" ]; then\n'
            '    trap "" TERM\n'
            '    : >"$TEST_NATIVE_QUERY_READY"\n'
            '    exec sleep 30\n'
            'fi\n'
            'exec "$TEST_NATIVE_QUERY_REAL" "$@"',
        )

        def drive(child):
            self.wait_for_native_open("wifi")
            (self.root / "native-query-hold").touch()
            self.wait_until(lambda: (self.root / "native-query-ready").exists())
            child.send_signal(signal.SIGTERM)

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse((self.root / "lock").exists())

    def test_stale_native_close_cannot_reclaim_an_open_panel(self):
        self.native_fixture("quick")

        def drive(_):
            self.wait_for_native_open("quick")
            self.emit("_weread_native_closed 0 closed")
            time.sleep(0.15)
            self.assertEqual(self.starts(), ["0"])
            self.close_native()

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assertEqual(self.starts(), ["0", "1"])

    def test_usb_takeover_during_native_settings_exits_without_resuming(self):
        self.native_fixture("wifi")

        def drive(_):
            self.wait_for_native_open("wifi")
            self.emit("usbConfigured fixture")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 143, error)
        self.assertEqual(self.starts(), ["0"])
        self.assertFalse((self.root / "lock").exists())

    def test_native_observation_failure_preserves_stock_ownership(self):
        self.native_fixture("wifi")

        def drive(_):
            self.wait_for_native_open("wifi")
            self.set_native_visibility("error")

        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1)
        self.assertIn("native display and input ownership are preserved", error)
        self.assertEqual(self.starts(), ["0"])
        self.assertEqual(self.trace().count("stop:statusbar"), 1)
        self.assertEqual(self.trace().count("start:statusbar"), 1)

    def test_named_emulator_display_is_passed_from_local_configuration(self):
        (self.home / "data" / "experimental-display").write_bytes(b"pw6-5201-emulator\r\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual((self.root / "display").read_text(), "pw6-5201-emulator")

    def test_inherited_emulator_display_is_not_implicit_approval(self):
        self.env["WEREAD_EXPERIMENTAL_DISPLAY"] = "pw6-5201-emulator"
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual((self.root / "display").read_text(), "")

    def test_unknown_display_changes_no_services(self):
        (self.home / "data" / "experimental-display").write_text("unknown\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1)
        self.assertIn("Unknown experimental display", error)
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "lock").exists())

    def test_display_rejects_a_different_firmware_before_service_changes(self):
        (self.root / "firmware").write_text("Kindle 5.18.6 (example)\n")
        (self.home / "data" / "device-allowlist").write_text("pw6 5.18.6\n")
        (self.home / "data" / "experimental-display").write_text("pw6-5201-emulator\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1)
        self.assertIn("requires pw6 firmware 5.20.1", error)
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "lock").exists())

    def test_display_rejects_a_different_profile_before_service_changes(self):
        (self.home / "data" / "device-profile").write_text("pw5\n")
        (self.home / "data" / "device-allowlist").write_text("pw5 5.20.1\n")
        (self.home / "data" / "experimental-display").write_text("pw6-5201-emulator\n")
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1)
        self.assertIn("requires pw6 firmware 5.20.1", error)
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "lock").exists())

    def power_observer_fixture(self, term_ignoring=False):
        self.power_observer_identity = self.root / "power-subscriber-identity"
        self.power_observer_signals = self.root / "power-subscriber-signals"
        self.power_observer_observed = []
        self.env.update(
            TEST_POWER_SUBSCRIBER_IDENTITY=str(self.power_observer_identity),
            TEST_POWER_SUBSCRIBER_SIGNALS=str(self.power_observer_signals),
            TEST_POWER_SUBSCRIBER_IGNORE="1" if term_ignoring else "0",
        )
        body = textwrap.dedent(r'''
            import json
            import os
            from pathlib import Path
            import signal
            import sys
            import time

            identity_path = Path(os.environ["TEST_POWER_SUBSCRIBER_IDENTITY"])
            signals_path = Path(os.environ["TEST_POWER_SUBSCRIBER_SIGNALS"])
            def received(number, _frame):
                with signals_path.open("a") as output:
                    output.write(str(number) + "\n")
                if number != signal.SIGCONT and os.environ["TEST_POWER_SUBSCRIBER_IGNORE"] != "1":
                    sys.exit(0)
            for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGCONT):
                signal.signal(number, received)
            fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
            parent = os.getppid()
            parent_fields = Path(f"/proc/{parent}/stat").read_text().rsplit(")", 1)[1].split()
            try:
                os.readlink("/proc/self/fd/3")
                duplicate_fifo = True
            except FileNotFoundError:
                duplicate_fifo = False
            identity = {"pid": os.getpid(), "birth": fields[19], "parent": parent,
                        "wrapper_birth": parent_fields[19], "duplicate_fifo": duplicate_fifo}
            pending = identity_path.with_suffix(".pending")
            pending.write_text(json.dumps(identity))
            pending.replace(identity_path)
            # This replacement subscriber never forks or emits synthetic power events.
            while True:
                time.sleep(0.02)
        ''').lstrip()
        path = self.bin / "lipc-wait-event"
        path.write_text("#!/usr/bin/env python3\n" + body)
        path.chmod(0o755)
        self.addCleanup(self.cleanup_power_observer_fixture)
        self.observe_cleanup_descriptor()

    def cleanup_power_observer_fixture(self):
        identities = list(getattr(self, "power_observer_observed", []))
        path = getattr(self, "power_observer_identity", None)
        if path is not None and path.exists():
            entry = json.loads(path.read_text())
            identities.extend([
                {"pid": entry["parent"], "birth": entry["wrapper_birth"]},
                {"pid": entry["pid"], "birth": entry["birth"]},
            ])
        for identity in identities:
            if identity["pid"] <= 1 or not self.original_process_exists(identity):
                continue
            if self.process_state(identity["pid"]) in ("Z", "X", None):
                continue
            for number in (signal.SIGCONT, signal.SIGKILL):
                if not self.original_process_exists(identity):
                    break
                try:
                    os.kill(identity["pid"], number)
                except ProcessLookupError:
                    break

    def read_power_observer_record(self, path=None):
        try:
            values = (path or self.root / "lock" / "observer-owned").read_text().strip().split(":")
        except OSError:
            return None
        if len(values) != 7 or not all(value.isdigit() for value in values[1:]):
            return None
        return {
            "token": values[0],
            "owner": {"pid": int(values[1]), "birth": values[2]},
            "wrapper": {"pid": int(values[3]), "birth": values[4]},
            "child": {"pid": int(values[5]), "birth": values[6]},
        }

    def remember_power_observer_record(self, record):
        self.assertIsNotNone(record)
        self.power_observer_observed.extend([record["wrapper"], record["child"]])

    def wait_for_power_observer(self, supervisor):
        permission = self.root / "lock" / "observer-go"
        self.wait_until(lambda: self.read_power_observer_record() is not None
                        and permission.is_file() and self.power_observer_identity.is_file()
                        and (self.root / "lock" / "observer-stream-ready").is_file())
        record = self.read_power_observer_record()
        self.remember_power_observer_record(record)
        for identity in (record["wrapper"], record["child"]):
            self.assertTrue(self.original_process_exists(identity))
        self.assertEqual(record["owner"]["pid"], supervisor.pid)
        self.assertEqual(record["owner"]["birth"], self.observed_identity(supervisor.pid)["birth"])
        observed = json.loads(self.power_observer_identity.read_text())
        self.assertEqual(record["child"], {"pid": observed["pid"], "birth": observed["birth"]})
        self.assertEqual(record["wrapper"], {"pid": observed["parent"], "birth": observed["wrapper_birth"]})
        self.assertFalse(observed["duplicate_fifo"])
        expected = ":".join((record["token"], str(record["owner"]["pid"]), record["owner"]["birth"],
                             str(record["wrapper"]["pid"]), record["wrapper"]["birth"],
                             str(record["child"]["pid"]), record["child"]["birth"]))
        self.assertEqual((self.root / "lock" / "observer-stream-ready").read_text().strip(), expected)
        self.assertEqual(os.readlink(f"/proc/{record['child']['pid']}/fd/1"),
                         str(self.root / "lock" / "observer-events"))
        for descriptor in (4, 5):
            with self.assertRaises(FileNotFoundError):
                os.readlink(f"/proc/{record['child']['pid']}/fd/{descriptor}")
        return record

    def assert_power_observer_retired(self):
        self.assertTrue(self.power_observer_observed)
        self.wait_until(lambda: all(not self.original_process_exists(identity)
                                   for identity in self.power_observer_observed))
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        self.assertFalse((self.root / "lock").exists())

    def unrelated_power_process(self):
        ready = self.root / "unrelated-power-ready"
        signals = self.root / "unrelated-power-signals"
        path = self.root / "unrelated-power.py"
        error_path = self.root / "unrelated-power.stderr"
        ready.unlink(missing_ok=True)
        path.write_text(textwrap.dedent(r'''
            from pathlib import Path
            import signal
            import sys
            import time

            ready, signals = map(Path, sys.argv[1:])
            def received(number, _frame):
                with signals.open("a") as output:
                    output.write(str(number) + "\n")
            for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGCONT):
                signal.signal(number, received)
            ready.touch()
            while True:
                time.sleep(0.02)
        ''').lstrip())
        with error_path.open("wb") as error_file:
            child = subprocess.Popen([sys.executable, str(path), str(ready), str(signals)],
                                     env=self.env, start_new_session=True, stdout=subprocess.DEVNULL,
                                     stderr=error_file)
        def reap():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=3)
        self.addCleanup(reap)
        original = self.wait_for_fixture_process_ready(child, ready, error_path, "unrelated signal witness")
        return child, original, signals

    def assert_power_assignment_signal(self, number):
        self.power_observer_fixture()
        window = self.root / "power-assignment-window"
        self.env["TEST_POWER_ASSIGNMENT_WINDOW"] = str(window)
        source = self.launcher.read_text()
        assignment = "        EVENT_SOURCE_PID=$!\n"
        self.assertEqual(source.count(assignment), 1)
        source = source.replace(assignment, textwrap.dedent(r'''
                fixture_assignment_checks=200
                while [ ! -f "$LOCK_DIR/observer-owned" ]; do
                    [ "$fixture_assignment_checks" -gt 0 ] || exit 81
                    usleep 10000 || :
                    fixture_assignment_checks=$((fixture_assignment_checks - 1))
                done
                IFS= read -r fixture_assignment_record <"$LOCK_DIR/observer-owned"
                printf '%s\n' "$fixture_assignment_record" >"$TEST_POWER_ASSIGNMENT_WINDOW"
                [ -z "$EVENT_SOURCE_PID" ] || exit 82
                fixture_assignment_checks=200
                while [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                    [ "$fixture_assignment_checks" -gt 0 ] || exit 83
                    usleep 10000 || :
                    fixture_assignment_checks=$((fixture_assignment_checks - 1))
                done
        ''') + assignment, 1)
        self.launcher.write_text(source)
        def drive(supervisor):
            self.wait_until(window.exists)
            record = self.read_power_observer_record(window)
            self.remember_power_observer_record(record)
            for identity in (record["wrapper"], record["child"]):
                self.assertTrue(self.original_process_exists(identity))
            self.assertEqual(record["owner"]["pid"], supervisor.pid)
            self.assertFalse(self.power_observer_identity.exists(), "Subscription ran before Main captured identity.")
            self.assertFalse((self.root / "lock" / "observer-go").exists())
            supervisor.send_signal(number)
        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, {signal.SIGTERM: 143, signal.SIGHUP: 129}[number], error)
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        self.assertFalse(self.power_observer_identity.exists())
        self.assert_power_observer_retired()

    def test_power_observer_fork_assignment_term_is_deferred_until_owned_registration(self):
        self.assert_power_assignment_signal(signal.SIGTERM)

    def test_power_observer_fork_assignment_hup_is_deferred_until_owned_registration(self):
        self.assert_power_assignment_signal(signal.SIGHUP)

    def assert_registration_boundary_signals(self, boundary):
        self.power_observer_fixture()
        handled = self.root / "registration-boundary-handled"
        record = self.root / "registration-boundary-record"
        self.env.update(TEST_REGISTRATION_BOUNDARY_HANDLED=str(handled),
                        TEST_REGISTRATION_BOUNDARY_RECORD=str(record))
        source = self.launcher.read_text()
        if boundary == "entry":
            anchor = "    LAUNCH_REGISTRATION_ACTIVE=1\n"
            self.assertEqual(source.count(anchor), 1)
            source = source.replace(anchor, anchor + textwrap.dedent(r'''
                kill -HUP "$$"
                [ "$LAUNCH_STARTUP_SIGNAL" -eq 129 ] || exit 81
                kill -TERM "$$"
                [ "$LAUNCH_STARTUP_SIGNAL" -eq 129 ] || exit 82
                : >"$TEST_REGISTRATION_BOUNDARY_HANDLED"
            '''), 1)
            expected = 129
        else:
            anchor = "finish_owned_registration() {\n    LAUNCH_REGISTRATION_ACTIVE=0\n"
            self.assertEqual(source.count(anchor), 1)
            source = source.replace(anchor, "finish_owned_registration() {\n" + textwrap.dedent(r'''
                if [ -n "$EVENT_SOURCE_CHILD_PID" ]; then
                    printf '%s:%s:%s:%s:%s:%s:%s\n' "$LAUNCH_TOKEN" "$$" "$POWER_OWNER_BIRTH" "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" >"$TEST_REGISTRATION_BOUNDARY_RECORD"
                    kill -TERM "$$"
                    [ "$LAUNCH_STARTUP_SIGNAL" -eq 143 ] || exit 81
                    : >"$TEST_REGISTRATION_BOUNDARY_HANDLED"
                    LAUNCH_REGISTRATION_ACTIVE=0
                    # The second handler runs after deferral has ended. It must
                    # use the already latched TERM status, not its own HUP status.
                    kill -HUP "$$"
                fi
                LAUNCH_REGISTRATION_ACTIVE=0
            '''), 1)
            expected = 143
        self.launcher.write_text(source)
        status, _, error = self.run_launcher()
        self.assertEqual(status, expected, error)
        self.assertTrue(handled.exists(), "Both distinct terminal signal intents were not exercised.")
        self.assertEqual(self.starts(), [])
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "lock").exists())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
        if boundary == "entry":
            self.assertFalse(self.power_observer_identity.exists())
        else:
            self.remember_power_observer_record(self.read_power_observer_record(record))
            self.assert_power_observer_retired()

    def test_owned_registration_entry_preserves_the_first_of_distinct_terminal_signals(self):
        self.assert_registration_boundary_signals("entry")

    def test_owned_registration_exit_preserves_the_latch_after_deferral_ends(self):
        self.assert_registration_boundary_signals("exit")

    def assert_app_assignment_signal(self, number):
        awesome = self.native_dark_fixture(TEST_NATIVE_THEME_TARGET="", TEST_NATIVE_THEME_EXIT="hold")
        self.power_observer_fixture()
        lease = self.root / "fixture-touch-lease"
        ready = self.root / "app-lease-ready"
        released = self.root / "app-lease-released"
        window = self.root / "app-assignment-window"
        self.env.update(TEST_APP_LEASE=str(lease), TEST_APP_LEASE_READY=str(ready),
                        TEST_APP_LEASE_RELEASED=str(released), TEST_APP_ASSIGNMENT_WINDOW=str(window))
        app = self.home / "app" / "weread"
        app.write_text("#!/usr/bin/env python3\n" + textwrap.dedent(r'''
            import fcntl
            import json
            import os
            from pathlib import Path
            import signal
            import sys
            import time

            # Cache an initialized launcher context and own a real file lease.
            # Losing its directory alone will not end this active input proxy.
            directory = Path(os.environ["WEREAD_LAUNCH_CONTEXT_DIR"])
            base = os.environ["WEREAD_LAUNCH_GENERATION"] + ":" + os.environ["WEREAD_LAUNCH_TOKEN"]
            for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                signal.signal(number, lambda *_: sys.exit(0))
            signal.signal(signal.SIGCONT, lambda *_: None)
            descriptor = os.open(os.environ["TEST_APP_LEASE"], os.O_CREAT | os.O_RDWR, 0o600)
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                (directory / "state-open").write_text(base + "\n")
                with Path(os.environ["TEST_STARTS"]).open("a") as output:
                    output.write(os.environ["WEREAD_RESUME_SESSION"] + "\n")
                fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
                parent_fields = Path(f"/proc/{os.getppid()}/stat").read_text().rsplit(")", 1)[1].split()
                ready = Path(os.environ["TEST_APP_LEASE_READY"])
                pending = ready.with_suffix(".pending")
                pending.write_text(json.dumps({
                    "pid": os.getpid(), "birth": fields[19], "wrapper": os.getppid(),
                    "wrapper_birth": parent_fields[19], "descriptor": descriptor,
                }))
                pending.replace(ready)
                while True:
                    time.sleep(0.02)
            finally:
                os.close(descriptor)
                Path(os.environ["TEST_APP_LEASE_RELEASED"]).touch()
        ''').lstrip())
        app.chmod(0o755)
        source = self.launcher.read_text()
        assignment = "    APP_RUNNER_PID=$!\n"
        self.assertEqual(source.count(assignment), 1)
        source = source.replace(assignment, textwrap.dedent(r'''
            fixture_app_checks=200
            while [ ! -e "$TEST_APP_LEASE_READY" ] || [ ! -f "$LOCK_DIR/application-birth" ]; do
                [ "$fixture_app_checks" -gt 0 ] || exit 81
                usleep 10000 || :
                fixture_app_checks=$((fixture_app_checks - 1))
            done
            [ -z "$APP_RUNNER_PID" ] || exit 82
            : >"$TEST_APP_ASSIGNMENT_WINDOW"
            fixture_app_checks=200
            while [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                [ "$fixture_app_checks" -gt 0 ] || exit 83
                usleep 10000 || :
                fixture_app_checks=$((fixture_app_checks - 1))
            done
        ''') + assignment, 1)
        self.launcher.write_text(source)
        observed = []
        def drive(supervisor):
            self.wait_until(window.exists)
            self.wait_for_power_observer(supervisor)
            self.assertEqual(self.process_state(awesome.pid), "T", "The native input owner was not paused.")
            entry = json.loads(ready.read_text())
            observed.extend([
                {"pid": entry["pid"], "birth": entry["birth"]},
                {"pid": entry["wrapper"], "birth": entry["wrapper_birth"]},
            ])
            for identity in observed:
                self.assertTrue(self.original_process_exists(identity))
            self.assertTrue(Path(f"/proc/{entry['pid']}/fd/{entry['descriptor']}").exists())
            supervisor.send_signal(number)
        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, {signal.SIGTERM: 143, signal.SIGHUP: 129}[number], error)
        self.wait_until(lambda: all(not self.original_process_exists(identity) for identity in observed))
        self.assertTrue(released.exists(), "The active application's input lease was not released.")
        import fcntl
        with lease.open("r+b") as file:
            fcntl.flock(file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.assert_power_observer_retired()
        self.assert_native_theme_restored(awesome)
        trace = self.trace()
        self.assertEqual(trace.count("stop:statusbar"), 1)
        self.assertEqual(trace.count("start:statusbar"), 1)
        self.assertEqual(trace.count("xrefresh:"), 1)

    def test_app_runner_fork_assignment_term_retires_the_initialized_input_owner(self):
        self.assert_app_assignment_signal(signal.SIGTERM)

    def test_app_runner_fork_assignment_hup_retires_the_initialized_input_owner(self):
        self.assert_app_assignment_signal(signal.SIGHUP)

    def test_power_observer_lost_wrapper_retires_only_its_captured_stopped_orphan_on_explicit_exit(self):
        self.power_observer_fixture(term_ignoring=True)
        unrelated, foreign, signals = self.unrelated_power_process()
        self.env.update(TEST_SLEEP="1", TEST_ORDINARY_ON_STOP="1")
        def drive(supervisor):
            record = self.wait_for_power_observer(supervisor)
            application = self.root / "lock" / "application-owned"
            self.wait_until(application.is_file)
            app_pid = int(application.read_text().strip().split(":")[2])
            # All mutable records are replaced after the real startup proof.
            # They may neither replace Main's child identity nor signal a stranger.
            directory = self.root / "lock"
            values = [record["token"], str(supervisor.pid), record["owner"]["birth"],
                      str(record["wrapper"]["pid"]), record["wrapper"]["birth"],
                      str(foreign["pid"]), foreign["birth"]]
            (directory / "observer-owned").write_text(":".join(values) + "\n")
            (directory / "observer-pid").write_text(str(foreign["pid"]) + "\n")
            (directory / "observer-go").write_text("stale-permission\n")
            os.kill(record["wrapper"]["pid"], signal.SIGKILL)
            self.wait_until(lambda: Path(f"/proc/{record['child']['pid']}/stat").read_text()
                            .rsplit(")", 1)[1].split()[1] == "1")
            os.kill(record["child"]["pid"], signal.SIGSTOP)
            self.wait_until(lambda: self.process_state(record["child"]["pid"]) == "T")
            os.kill(app_pid, signal.SIGTERM)
        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 0, error)
        self.assert_power_observer_retired()
        received = self.power_observer_signals.read_text().splitlines()
        self.assertIn(str(signal.SIGINT), received)
        self.assertIn(str(signal.SIGTERM), received)
        self.assertIsNone(unrelated.poll())
        self.assertTrue(self.original_process_exists(foreign))
        self.assertFalse(signals.exists(), "A mutable observer record signalled an unrelated process.")
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])

    def assert_power_observer_publication_failure(self, failure, attempts=1):
        self.power_observer_fixture(term_ignoring=True)
        ready = self.root / "power-preexec-ready"
        identity = self.root / "power-preexec-identity"
        blocker = self.root / "power-preexec-blocker.py"
        blocker.write_text("#!/usr/bin/env python3\n" + textwrap.dedent(r'''
            import os
            from pathlib import Path
            import signal
            import time

            for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                signal.signal(number, signal.SIG_IGN)
            fields = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
            identity = Path(os.environ["TEST_POWER_PREEXEC_IDENTITY"])
            pending = identity.with_suffix(".pending")
            pending.write_text(str(os.getpid()) + ":" + fields[19] + "\n")
            pending.replace(identity)
            Path(os.environ["TEST_POWER_PREEXEC_READY"]).touch()
            # Exec preserves the gate's PID and birth. This unresponsive child
            # never forks a short timer that could outlive a killed test shell.
            while True:
                time.sleep(0.02)
        ''').lstrip())
        blocker.chmod(0o755)
        self.env.update(TEST_POWER_PREEXEC_READY=str(ready), TEST_POWER_PREEXEC_IDENTITY=str(identity),
                        TEST_POWER_PREEXEC_BLOCKER=str(blocker))
        source = self.launcher.read_text()
        captured = "    subscriber_birth=$WRAPPER_BIRTH\n"
        self.assertEqual(source.count(captured), 1)
        source = source.replace(captured, captured + '    exec "$TEST_POWER_PREEXEC_BLOCKER"\n', 1)
        startup = "            power_start_checks=$(managed_wait_budget 3)\n"
        self.assertEqual(source.count(startup), 1)
        # The default fixture compresses 100 ms into 10 ms. Keep a finite wait
        # for this interpreter-backed child before injecting publication failure.
        source = source.replace(startup, "            power_start_checks=200\n", 1)
        wrapper_capture = "        observer_child_birth=$WRAPPER_CHILD_BIRTH\n"
        self.assertEqual(source.count(wrapper_capture), 1)
        source = source.replace(wrapper_capture, wrapper_capture + textwrap.dedent(r'''
            fixture_power_ready_checks=200
            while [ ! -e "$TEST_POWER_PREEXEC_READY" ]; do
                [ "$fixture_power_ready_checks" -gt 0 ] || exit 81
                usleep 10000 || :
                fixture_power_ready_checks=$((fixture_power_ready_checks - 1))
            done
        '''), 1)
        targets = {"pid-write": "observer-pid", "owned-write": "observer-owned.pending",
                   "go-write": "observer-go.pending"}
        if failure in targets:
            anchor = f'>"$LOCK_DIR/{targets[failure]}"'
            self.assertEqual(source.count(anchor), 1)
            source = source.replace(anchor, '>/dev/full', 1)
        self.launcher.write_text(source)
        if failure in ("owned-rename", "go-rename"):
            target = "observer-owned" if failure == "owned-rename" else "observer-go"
            self.install_path_failure("mv", target).touch()
        for attempt in range(attempts):
            with self.subTest(failure=failure, attempt=attempt):
                ready.unlink(missing_ok=True)
                identity.unlink(missing_ok=True)
                status, _, error = self.run_launcher()
                self.assertEqual(status, 1, error)
                pid, birth = identity.read_text().strip().split(":")
                self.assertFalse(self.original_process_exists({"pid": int(pid), "birth": birth}),
                                 "Failed publication left the original pre-exec child behind.")
                self.assertFalse(self.power_observer_identity.exists(), "Failed publication started liblipc.")
                self.assertFalse((self.root / "lock").exists())
                self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")
                self.assertEqual(self.trace(), [])
                if failure in ("owned-rename", "go-rename"):
                    self.assertTrue(Path(self.env["TEST_FILE_FAULT_TRACE"]).exists(),
                                    "The actual permission or ownership rename was not rejected.")

    def test_power_observer_owned_record_write_failure_reaps_its_unreleased_child(self):
        self.assert_power_observer_publication_failure("owned-write")

    def test_power_observer_owned_record_rename_failure_reaps_its_unreleased_child(self):
        self.assert_power_observer_publication_failure("owned-rename")

    def test_power_observer_permission_write_failure_reaps_its_captured_unreleased_child(self):
        self.assert_power_observer_publication_failure("go-write")

    def test_power_observer_permission_rename_failure_reaps_its_captured_unreleased_child(self):
        self.assert_power_observer_publication_failure("go-rename")

    def test_power_observer_startup_rejects_missing_spoofed_and_wrong_birth_child_records(self):
        self.power_observer_fixture()
        unrelated, foreign, signals = self.unrelated_power_process()
        self.env.update(TEST_POWER_FOREIGN_PID=str(foreign["pid"]),
                        TEST_POWER_FOREIGN_BIRTH=foreign["birth"])
        source = self.launcher.read_text()
        anchor = "capture_power_observer_child() {\n"
        self.assertEqual(source.count(anchor), 1)
        # Inject in the publisher before the atomic rename. An entry-only reader
        # hook races publication between its first test and the real reader's
        # later test, allowing valid identity to be captured without injection.
        publisher = textwrap.dedent(r'''
            fixture_publish_power_record() {
                [ "$#" -eq 2 ] || return 1
                IFS= read -r fixture_original_record <"$1" || return 1
                IFS=: read -r fixture_token fixture_owner fixture_owner_birth fixture_wrapper fixture_wrapper_birth fixture_child fixture_child_birth fixture_extra <"$1" || return 1
                [ -z "$fixture_extra" ] || return 1
                process_matches "$fixture_wrapper" "$fixture_wrapper_birth" "$fixture_owner" &&
                    process_matches "$fixture_child" "$fixture_child_birth" "$fixture_wrapper" || return 1
                [ ! -e "$LOCK_DIR/observer-go" ] || return 1
                printf '%s\n' "$fixture_original_record" >"$TEST_POWER_ORIGINAL_RECORD.pending" || return 1
                mv -f "$TEST_POWER_ORIGINAL_RECORD.pending" "$TEST_POWER_ORIGINAL_RECORD" || return 1
                case "$TEST_POWER_BAD_RECORD" in
                    missing)
                        rm -f "$1" || return 1
                        return 0 ;;
                    spoof)
                        printf '%s:%s:%s:%s:%s:%s:%s\n' "$fixture_token" "$fixture_owner" "$fixture_owner_birth" "$fixture_wrapper" "$fixture_wrapper_birth" "$TEST_POWER_FOREIGN_PID" "$TEST_POWER_FOREIGN_BIRTH" >"$1" || return 1 ;;
                    birth)
                        printf '%s:%s:%s:%s:%s:%s:%s\n' "$fixture_token" "$fixture_owner" "$fixture_owner_birth" "$fixture_wrapper" "$fixture_wrapper_birth" "$fixture_child" "$((fixture_child_birth + 1))" >"$1" || return 1 ;;
                    *) return 1 ;;
                esac
                mv -f "$1" "$2"
            }
        ''').lstrip()
        source = source.replace(anchor, publisher + "\n" + anchor, 1)
        publication = '            ! mv -f "$LOCK_DIR/observer-owned.pending" "$LOCK_DIR/observer-owned"; then\n'
        self.assertEqual(source.count(publication), 1)
        source = source.replace(publication,
                                '            ! fixture_publish_power_record "$LOCK_DIR/observer-owned.pending" "$LOCK_DIR/observer-owned"; then\n', 1)
        self.launcher.write_text(source)
        for mode in ("missing", "spoof", "birth"):
            with self.subTest(mode=mode):
                original = self.root / f"power-original-record-{mode}"
                self.power_observer_identity = self.root / f"power-subscriber-identity-{mode}"
                self.power_observer_signals = self.root / f"power-subscriber-signals-{mode}"
                self.env.update(TEST_POWER_BAD_RECORD=mode, TEST_POWER_ORIGINAL_RECORD=str(original),
                                TEST_POWER_SUBSCRIBER_IDENTITY=str(self.power_observer_identity),
                                TEST_POWER_SUBSCRIBER_SIGNALS=str(self.power_observer_signals))
                def drive(_supervisor):
                    self.wait_until(original.exists)
                    self.remember_power_observer_record(self.read_power_observer_record(original))
                status, _, error = self.run_launcher(drive)
                self.assertEqual(status, 1, error)
                self.assertIn("Power observer startup failed", error)
                self.assertFalse(self.power_observer_identity.exists())
                self.assertEqual(self.starts(), [])
                self.assertEqual(self.trace(), [])
                self.assert_power_observer_retired()
                self.assertIsNone(unrelated.poll())
                self.assertFalse(signals.exists(), "An unproven startup record selected another process.")

    def assert_power_permission_rejected(self, mode):
        self.power_observer_fixture()
        released = self.root / "power-permission-released"
        original = self.root / "power-permission-original"
        self.env.update(TEST_POWER_PERMISSION_RELEASED=str(released),
                        TEST_POWER_PERMISSION_ORIGINAL=str(original), TEST_SLEEP="1")
        source = self.launcher.read_text()
        waiting = "    subscriber_checks=$(managed_wait_budget 5)\n"
        self.assertEqual(source.count(waiting), 1)
        source = source.replace(waiting, textwrap.dedent(r'''
            fixture_permission_checks=200
            while [ ! -e "$TEST_POWER_PERMISSION_RELEASED" ]; do
                [ "$fixture_permission_checks" -gt 0 ] || exit 81
                usleep 10000 || :
                fixture_permission_checks=$((fixture_permission_checks - 1))
            done
        ''') + waiting, 1)
        committed = "                        power_stream_checks=$(managed_wait_budget 5)\n"
        self.assertEqual(source.count(committed), 1)
        adjustment = (
            '                        IFS= read -r fixture_permission_record <"$LOCK_DIR/observer-owned"\n'
            '                        printf "%s\\n" "$fixture_permission_record" >"$TEST_POWER_PERMISSION_ORIGINAL"\n'
        )
        if mode == "missing":
            adjustment += '                        rm -f "$LOCK_DIR/observer-go"\n'
        else:
            adjustment += '                        printf "%s\\n" stale-permission >"$LOCK_DIR/observer-go"\n'
        adjustment += '                        : >"$TEST_POWER_PERMISSION_RELEASED"\n'
        self.launcher.write_text(source.replace(committed, adjustment + committed, 1))
        def drive(_supervisor):
            self.wait_until(original.exists)
            self.remember_power_observer_record(self.read_power_observer_record(original))
        status, _, error = self.run_launcher(drive)
        self.assertEqual(status, 1, error)
        self.assertFalse(self.power_observer_identity.exists(), "An absent or stale permission started liblipc.")
        self.assertIn("Power observer startup failed", error)
        self.assert_power_observer_retired()
        self.assertEqual(self.trace(), [])

    def test_power_observer_subscriber_rejects_a_missing_committed_permission(self):
        self.assert_power_permission_rejected("missing")

    def test_power_observer_subscriber_rejects_a_spoofed_committed_permission(self):
        self.assert_power_permission_rejected("spoof")

    def test_power_observer_wrapper_identity_failure_after_main_capture_starts_no_subscriber(self):
        self.power_observer_fixture()
        captured = self.root / "power-main-captured"
        self.env["TEST_POWER_MAIN_CAPTURED"] = str(captured)
        source = self.launcher.read_text()
        parent = "        if capture_power_observer_identity; then\n"
        self.assertEqual(source.count(parent), 1)
        source = source.replace(parent, parent
                                + '            printf "%s:%s\\n" "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" >"$TEST_POWER_MAIN_CAPTURED"\n', 1)
        wrapper = "    trap stop_observer_child HUP INT TERM\n    capture_wrapper_identity || exit 1\n"
        self.assertEqual(source.count(wrapper), 1)
        source = source.replace(wrapper,
                                "    trap stop_observer_child HUP INT TERM\n" + textwrap.dedent(r'''
            capture_wrapper_identity() {
                fixture_parent_checks=200
                while [ ! -e "$TEST_POWER_MAIN_CAPTURED" ]; do
                    [ "$fixture_parent_checks" -gt 0 ] || return 1
                    usleep 10000 || :
                    fixture_parent_checks=$((fixture_parent_checks - 1))
                done
                return 1
            }
            capture_wrapper_identity || exit 1
        '''), 1)
        self.launcher.write_text(source)
        status, _, error = self.run_launcher()
        self.assertEqual(status, 1, error)
        pid, birth = captured.read_text().strip().split(":")
        self.assertFalse(self.original_process_exists({"pid": int(pid), "birth": birth}))
        self.assertFalse(self.power_observer_identity.exists())
        self.assertEqual(self.trace(), [])
        self.assertFalse((self.root / "lock").exists())
        self.assertEqual(Path(self.env["TEST_CLEANUP_DESCRIPTOR_TRACE"]).read_text(), "closed\n")

    def test_power_observer_cleanup_refuses_a_wrong_wrapper_birth_or_foreign_pid(self):
        unrelated, foreign, signals = self.unrelated_power_process()
        self.env.update(TEST_POWER_FOREIGN_PID=str(foreign["pid"]),
                        TEST_POWER_FOREIGN_BIRTH=foreign["birth"])
        source = self.launcher.read_text()
        entry = 'trap \'cleanup "$?"\' 0\n'
        self.assertEqual(source.count(entry), 1)
        prefix = source.split(entry, 1)[0]
        fixture = textwrap.dedent(r'''
            trap '' HUP INT TERM
            # This standalone helper scenario does not run Main's lock entry.
            mkdir "$LOCK_DIR" || exit 80
            LOCK_OWNED=1
            sleep 30 >/dev/null 2>&1 3>&- &
            fixture_original=$!
            capture_wrapper_child_birth "$fixture_original" "$$" || exit 81
            fixture_original_birth=$WRAPPER_CHILD_BIRTH
            if [ "$TEST_POWER_WRONG_WRAPPER" = birth ]; then
                EVENT_SOURCE_PID=$fixture_original
                EVENT_SOURCE_BIRTH=0
            else
                EVENT_SOURCE_PID=$TEST_POWER_FOREIGN_PID
                EVENT_SOURCE_BIRTH=$TEST_POWER_FOREIGN_BIRTH
            fi
            fixture_selected=$EVENT_SOURCE_PID
            fixture_selected_birth=$EVENT_SOURCE_BIRTH
            if stop_observer; then exit 82; fi
            [ "$EVENT_SOURCE_PID:$EVENT_SOURCE_BIRTH" = "$fixture_selected:$fixture_selected_birth" ] || exit 83
            owned_child_is_running "$fixture_original" "$fixture_original_birth" "$$" || exit 84
            printf '%s\n' refused-power-observer-identity >>"$TEST_TRACE"
            signal_owned_process "$fixture_original" "$fixture_original_birth" "$$" KILL || exit 85
            fixture_exit_checks=100
            while ! owned_child_has_exited "$fixture_original" "$fixture_original_birth" "$$" &&
                [ "$fixture_exit_checks" -gt 0 ]; do
                usleep 10000
                fixture_exit_checks=$((fixture_exit_checks - 1))
            done
            owned_child_has_exited "$fixture_original" "$fixture_original_birth" "$$" || exit 86
            wait "$fixture_original" 2>/dev/null || :
            rmdir "$LOCK_DIR"
            exit 0
        ''')
        self.launcher.write_text(prefix + fixture)
        for mode in ("birth", "pid"):
            with self.subTest(mode=mode):
                self.env["TEST_POWER_WRONG_WRAPPER"] = mode
                status, _, error = self.run_launcher()
                self.assertEqual(status, 0, error)
                self.assertIn("blocking wait was refused", error)
                self.assertFalse((self.root / "lock").exists())
                self.assertIsNone(unrelated.poll())
                self.assertFalse(signals.exists(), "A wrong wrapper identity authorized signalling a stranger.")
        self.assertEqual(self.trace(), ["refused-power-observer-identity"] * 2)

    def test_repeated_fast_exit_does_not_strand_event_children(self):
        self.launcher.write_text(self.launcher.read_text().replace("\nsleep 2\n", "\n"))
        for attempt in range(24):
            with self.subTest(attempt=attempt):
                status, _, error = self.run_launcher()
                self.assertEqual(status, 0, error)
                self.assertFalse((self.root / "lock").exists())

    def test_event_observer_ignoring_term_is_reaped_on_app_exit(self):
        self.launcher.write_text(self.launcher.read_text().replace("\nsleep 2\n", "\n"))
        self.env["TEST_EVENT_READY"] = str(self.root / "event-ready")
        self.script(
            self.bin / "lipc-wait-event",
            'trap "" TERM\n'
            ': >"$TEST_EVENT_READY"\n'
            'exec sleep 30',
        )
        self.script(
            self.home / "app" / "weread",
            'while [ ! -e "$TEST_EVENT_READY" ]; do sleep 0.01; done\n'
            'exit 0',
        )
        status, _, error = self.run_launcher()
        self.assertEqual(status, 0, error)
        self.assertEqual(self.trace(), ["stop:statusbar", "start:statusbar", "xrefresh:"])
        self.assertFalse((self.root / "lock").exists())


if __name__ == "__main__":
    unittest.main()
