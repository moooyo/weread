import io
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

from freeze import HERE, SCHEDULE, device_body
from prepare_rollout import prepare, release_gate


class ReleaseGateTest(unittest.TestCase):
    def test_archived_font_only_binary_cannot_replace_the_current_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            plan = root / "plan"
            with self.assertRaisesRegex(ValueError, "archived font-only candidate"):
                prepare(root, root, root, plan, root / "old-document")
            self.assertFalse(plan.exists())

    def test_partial_or_slow_study_cannot_promote(self):
        operations = {"passed": True, "released": "recorded",
                      "operations": [{"mode": "case", "case": case, "status": 0} for case in SCHEDULE]}
        metrics = {"request_to_send_ms": {"improved_pairs": 5, "median_paired_saving": 10},
                   "font_ms": {"median_paired_saving": 10}}
        release_gate(operations, metrics)
        metrics["request_to_send_ms"]["improved_pairs"] = 4
        with self.assertRaises(ValueError):
            release_gate(operations, metrics)
        metrics["request_to_send_ms"]["improved_pairs"] = 6
        operations["operations"].pop()
        with self.assertRaises(ValueError):
            release_gate(operations, metrics)


class RolloutFixture(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.volume = Path(temp.name)
        self.home = self.volume / "weread"
        self.daily = self.volume / "documents/weread.sh"
        self.daily.parent.mkdir()
        self.daily.write_bytes(b"old")
        (self.home / "app").mkdir(parents=True)
        (self.home / "app/weread").write_bytes(b"old elf")
        (self.home / "app/weread-frame-guard").touch()
        (self.home / "data").mkdir()
        (self.home / "data/private-sentinel").write_bytes(b"unchanged")
        self.label = "0011223344556677"
        self.stage = self.volume / (".weread-rollout-" + self.label)
        code = device_body().partition('case "$mode" in\n')[0] + (HERE / "rollout.sh").read_text()
        code = code.replace("/mnt/us", str(self.volume))
        stubs = '''
native() { :; }
settled() { :; }
peer_proof() { :; }
sql() { echo 1; }
rollout_state() { echo unchanged; }
scan() { :; }
rollout_admission() { original_install && absent "$stage" || fail admission; }
'''
        self.script = self.volume / "operator"
        self.script.write_text(code.replace('case "$mode" in\n', stubs + 'case "$mode" in\n'))

    def payload(self):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as archive:
            for name, value in {"package/documents/weread.sh": b"new", "ref/document": b"new",
                                "ref/old-document": b"old", "ref/launch.sh": b"supervisor",
                                "package/weread/launch.sh": b"supervisor",
                                "package/weread/app/weread": b"new elf",
                                "package/weread/app/weread-frame-guard": b"guard",
                                "package/weread/app/weread-native-query": b"query"}.items():
                member = tarfile.TarInfo(name)
                member.size, member.mode = len(value), 0o755
                archive.addfile(member, io.BytesIO(value))
        return stream.getvalue()

    def call(self, operation):
        run = subprocess.run(["sh", self.script, operation, self.label], input=self.payload(), capture_output=True)
        self.assertEqual((self.home / "data/private-sentinel").read_bytes(), b"unchanged")
        return run.returncode

    def test_rejected_normal_launch_rolls_back_public_files_only(self):
        self.assertEqual(self.call("stage"), 0)
        self.assertEqual(self.daily.read_bytes(), b"new")
        self.assertEqual(self.call("restore"), 0)
        self.assertEqual((self.home / "app/weread").read_bytes(), b"old elf")
        self.assertFalse((self.home / "launch.sh").exists())
        self.assertEqual(self.daily.read_bytes(), b"old")
        self.assertEqual(self.call("close"), 0)
        self.assertFalse(self.stage.exists())

    def test_accepted_install_retains_new_package(self):
        self.assertEqual(self.call("stage"), 0)
        (self.stage / "promoted").touch()
        self.assertEqual(self.call("restore"), 0)
        self.assertEqual(self.call("close"), 0)
        self.assertEqual((self.home / "app/weread").read_bytes(), b"new elf")
        self.assertEqual(self.daily.read_bytes(), b"new")
        self.assertFalse(self.stage.exists())


if __name__ == "__main__":
    unittest.main()
