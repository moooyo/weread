"""Exercise the child permission gate and recovery in worker-only scratch trees."""

import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


SOURCE = Path(__file__).with_name("device.sh")
LABEL = "0123456789abcdef"


def birth(pid):
    return Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()[19]


class StagedSupervisorTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        text = SOURCE.read_text().replace("volume=/mnt/us", "volume=" + str(root))
        start = text.index("state() {")
        end = text.index("matches()", start)
        self.program = root / "fixture.sh"
        text = text[:start] + "state() { printf 'retained_native_state\\n'; }\n" + text[end:]
        self.program.write_text(text)
        self.stage = root / (".weread-turn-" + LABEL)
        self.stage.mkdir()
        (self.stage / "owner").write_text(LABEL + "\n")
        (self.stage / "before").write_text("retained_native_state\n")
        (self.stage / "launch-gate.sh").write_text(text)
        (self.stage / "launch.sh").write_text('printf "%s\\n" "$WEREAD_READER_TURN_EXPECTED_ARM" > "$WEREAD_HOME/observed-launch"\n')

    def call(self, mode):
        return subprocess.run(["sh", str(self.program), mode, LABEL, "20", "60", "B"],
                              capture_output=True, timeout=20)

    def test_registered_child_runs_only_the_expected_lane(self):
        completed = self.call("run")
        self.assertEqual((completed.returncode, completed.stdout), (0, b"exited\n"), completed.stderr)
        self.assertEqual((self.stage/"observed-launch").read_text(), "B\n")
        self.assertEqual(self.call("close").stdout, b"closed\n")
        self.assertFalse(self.stage.exists())

    def test_child_exits_without_permission_after_parent_disconnects(self):
        (self.stage/"started").touch()
        parent = """
case_dir=$1; label=$2
read -r rec < /proc/$$/stat; rest=${rec##*) }; set -- $rest; shift 19
printf '%s %s\\n' "$$" "$1" >"$case_dir/run-parent"
sh "$case_dir/launch-gate.sh" launch-gate "$label" 20 60 B >"$case_dir/gate.output" 2>&1 &
printf '%s\\n' "$!" >"$case_dir/gate.pid"
wait
"""
        environment = dict(os.environ, WEREAD_HOME=str(self.stage), WEREAD_READER_TURN_TRIAL="1", WEREAD_READER_TURN_EXPECTED_ARM="B")
        process = subprocess.Popen(["sh", "-c", parent, "fixture", str(self.stage), LABEL], env=environment)
        try:
            for tick in range(160):
                if (self.stage/"gate.pid").exists(): break
                time.sleep(.025)
            self.assertTrue((self.stage/"gate.pid").is_file())
            process.kill()
            process.wait(timeout=2)
            for tick in range(160):
                if (self.stage/"gate.output").exists() and b"refused parent_gone" in (self.stage/"gate.output").read_bytes(): break
                time.sleep(.025)
            self.assertFalse((self.stage/"observed-launch").exists())
            self.assertEqual((self.stage/"gate.output").read_bytes(), b"refused parent_gone\n")
            self.assertEqual(self.call("close").stdout, b"closed\n")
        finally:
            if process.poll() is None:
                process.kill(); process.wait()

    def test_close_does_not_delete_missing_registration_or_touch_foreign_process(self):
        (self.stage/"permission").write_text(f"{LABEL}:1 1\n")
        self.assertEqual(self.call("close").stdout, b"refused ownership\n")
        self.assertTrue(self.stage.is_dir())
        with subprocess.Popen(["sleep", "6"]) as other:
            try:
                saved = f"{other.pid} {int(birth(other.pid)) + 1}"
                (self.stage/"supervisor").write_text(saved+"\n")
                (self.stage/"permission").write_text(f"{LABEL}:{saved}\n")
                self.assertEqual(self.call("close").stdout, b"closed\n")
                self.assertIsNone(other.poll())
            finally:
                other.terminate(); other.wait()

    def test_collection_reports_only_fixed_public_failure_categories(self):
        logs = self.stage / "data/logs"
        logs.mkdir(parents=True)
        failures = {"unexpected-target-at-draw": "draw_target", "target-did-not-submit-in-release-iteration": "no_release_send",
                    "target-not-written-in-ordinary-frame": "ordinary_write", "invalid-target-submission": "send_target",
                    "target-changed": "changed_target", "wait-marker-mismatch": "wait_marker",
                    "Reader trial marker replaced": "stage_marker", "Reader trial binary does not match the scheduled arm": "lane",
                    "Reader trial arguments rejected": "route", "unknown locally generated error": "unclassified"}
        for error, public in failures.items():
            (logs / "application-bootstrap.log").write_text(f"local-only prefix: {error}\n")
            with self.subTest(error=error):
                self.assertEqual(self.call("collect").stdout, f"refused no_result_{public}\n".encode())


if __name__ == "__main__":
    unittest.main()
