import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from orientation_contract import CANDIDATE, CONTROL, PILOT, schedule_for
import run


class WorkerTransactionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.plan = self.work / "pilot"
        self.plan.mkdir()
        self.label = "0123456789abcdef"
        self.config = dict(purpose="startup-orientation-pilot", label=self.label, schedule=PILOT,
                           device="Kindle Scribe 2022", firmware="5.19.5", clock="CLOCK_MONOTONIC",
                           launcher_wait_seconds=2, control_features=CONTROL, candidate_features=CANDIDATE)
        (self.plan / "plan.json").write_text(json.dumps(self.config))
        (self.plan / "device.sh").write_text("exit 1\n")
        (self.plan / "inputs.tgz").write_bytes(b"public fixture")
        self.args = SimpleNamespace(work=self.work, plan=self.plan, owner="fixture-owner", wait=0,
                                    lease_tool=self.plan / "device.sh", recover=None, admit_only=False)
        self.lease = Mock()
        self.lease.LeaseRequest.side_effect = lambda path, token, *_: SimpleNamespace(token_file=token)
        self.lease.acquire.side_effect = lambda request: request.token_file.touch(exist_ok=False)
        self.lease.status.return_value = {"ownership_verified": True}
        self.lease.release.return_value = {"owner_and_token_checked": True, "only_owned_owner_json_removed_then_rmdir": True,
                                          "released_at_utc": "2026-11-21T02:00:00+00:00"}
        self.calls = []
        self.bad_ack = set()

    def simulate(self, command, **kwargs):
        if command[0] == "/opt/kindle-toolchain/ubuntu-run":
            self.assertTrue(self.lease.release.called)
            self.calls.append("offline-analysis")
            return subprocess.CompletedProcess(command, 0)
        self.assertEqual(command[0], "fixture-transport")
        mode, _, case = command[-1].split()[-3:]
        self.calls.append(mode if mode != "case" else case)
        replies = dict(admit="admitted", stage="indexed", case="completed " + case, restore="restored", close="closed")
        response = replies.get(mode, "") if mode not in self.bad_ack else "unexpected"
        if mode == "collect":
            kwargs["stdout"].write(b"simulated archive")
        return subprocess.CompletedProcess(command, 0, stdout=response.encode("ascii"), stderr=b"")

    def execute(self):
        regular = Path.is_file
        def file_exists(path):
            return str(path) == "/opt/kindle-toolchain/ubuntu-run" or regular(path)

        with patch.object(Path, "is_file", file_exists), patch.object(run, "LEASE", self.work / "lock"), \
             patch.dict(run.UTIL, ssh_command=lambda *_: ["fixture-transport"], load_lease=lambda *_: self.lease), \
             patch.object(run.sys, "stdin", io.StringIO("fixture\n")), patch.object(run.signal, "signal"), \
             patch.object(run.subprocess, "run", side_effect=self.simulate), redirect_stdout(io.StringIO()):
            return run.run(self.args, orientation_schedule=schedule_for)

    def test_pilot_releases_before_analyzing_and_never_touches_formal_cases(self):
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, ["admit", "stage", *PILOT, "restore", "collect", "close", "offline-analysis"])
        attempt, = self.plan.glob("batch-*")
        receipt = json.loads((attempt / "operations.json").read_text())
        self.assertEqual((receipt["label"], receipt["passed"]), (self.label, True))
        self.assertFalse((attempt / "private-token").exists())

    def test_wrong_close_ack_keeps_ownership_then_recovery_cannot_relaunch(self):
        self.bad_ack = {"close"}
        self.assertEqual(self.execute(), 1)
        self.lease.release.assert_not_called()
        attempt, = self.plan.glob("batch-*")
        first = (attempt / "operations.json").read_bytes()
        self.assertTrue(json.loads(first)["recovery_required"])
        self.calls.clear()
        self.bad_ack.clear()
        self.args.recover = attempt
        self.assertEqual(self.execute(), 0)
        self.lease.acquire.assert_called_once()
        self.lease.release.assert_called_once()
        self.assertEqual(self.calls, ["restore", "collect", "close"])
        self.assertEqual(first, (attempt / "operations.json").read_bytes())

    def test_occupied_lease_and_rejected_admission_never_stage(self):
        (self.work / "lock").mkdir()
        with self.assertRaisesRegex(ValueError, "occupied"):
            self.execute()
        self.lease.acquire.assert_not_called()
        self.assertEqual(self.calls, [])
        (self.work / "lock").rmdir()
        self.bad_ack = {"admit"}
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls, ["admit"])
        self.lease.release.assert_called_once()


if __name__ == "__main__":
    unittest.main()
