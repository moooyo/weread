import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import types
import unittest
from unittest.mock import patch

import make_payload
import seed
import visit


class NativeVisitTests(unittest.TestCase):
    def test_rejects_extra_data_and_accepts_only_the_generated_seed(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            seed.create_seed(root / "seed")
            fake_arm = b"\x7fELF\x01\x01\x01" + bytes(11) + b"(\x00"
            for name in ("weread", "weread-frame-guard", "weread-native-query"):
                (root / name).write_bytes(fake_arm)
            archive = root / "lane.tgz"
            with patch.object(sys, "argv", ["make_payload", "--app", str(root / "weread"), "--helpers", str(root), "--seed", str(root / "seed/library.sqlite"), "--output", str(archive)]):
                make_payload.main()
            payload, size = visit.payload_bytes(archive)
            self.assertGreater(size, len(payload))
            for extra in (False, True):
                wrong = root / f"wrong-{extra}.tgz"
                with tarfile.open(archive, "r:gz") as source, tarfile.open(wrong, "w:gz") as dest:
                    for member in source.getmembers():
                        body = source.extractfile(member).read()
                        if member.name == "data/library.sqlite" and not extra:
                            body = body[:-1] + bytes([body[-1] ^ 1])
                        make_payload.write_member(dest, member.name, io.BytesIO(body), len(body))
                    if extra:
                        make_payload.write_member(dest, "extra", io.BytesIO(b"x"), 1)
                with self.assertRaises(ValueError):
                    visit.payload_bytes(wrong)

    def transaction(self, script, busy=False):
        with tempfile.TemporaryDirectory() as scratch:
            runs = Path(scratch)
            shared = runs / "lease"
            if busy:
                shared.mkdir()
            events = []
            def acquire(request):
                events.append("acquire"); request.token_file.write_text("fixture")
                return {"ok": True, "state": "held"}
            def release(request, native_restored_and_connections_closed):
                self.assertTrue(native_restored_and_connections_closed)
                events.append("release")
                return {"owner_and_token_checked": True, "only_owned_owner_json_removed_then_rmdir": True, "released_at_utc": "fixture"}
            lease = types.SimpleNamespace(LeaseRequest=lambda lock, token, owner, run: types.SimpleNamespace(token_file=token),
                                          acquire=acquire, status=lambda _request: {"ownership_verified": True}, release=release)
            protocol = {"LEASE": shared, "load_lease": lambda _path: lease, "ssh_command": lambda *_: ["ssh"]}
            def simulate(command, **kwargs):
                self.assertEqual(command[-1].split()[-1], "A")
                mode = command[-1].split()[-5]
                events.append(mode)
                status, stdout = script[mode]
                return types.SimpleNamespace(returncode=status, stdout=stdout)
            args = types.SimpleNamespace(owner="we-read", runs=runs, lease_tool=Path(__file__), recover=None,
                                         lane="A", chapters=1863, payload=runs/"lane", preflight=False, handoff=runs/"handoff")
            with patch.object(visit.runpy, "run_path", return_value=protocol), patch.object(visit, "payload_bytes", return_value=(b"public", 20)), \
                 patch.object(visit, "handoff"), patch.object(visit.subprocess,"run",side_effect=simulate), \
                 patch.object(visit.sys, "stdin", types.SimpleNamespace(buffer=io.BytesIO(b"test\n"))):
                if busy:
                    with self.assertRaisesRegex(ValueError, "lease_busy"):
                        visit.run_visit(args)
                    self.assertEqual(events, [])
                    self.assertEqual(list(runs.glob("visit-*")), [])
                    return None, events
                self.assertEqual(visit.run_visit(args), 1)
            attempt, = runs.glob("visit-*")
            report = json.loads((attempt/"receipt.json").read_text())
            self.assertEqual((attempt/"token").exists(), not report["released"])
            return report, events

    def test_busy_means_no_device_operations(self):
        self.transaction({}, busy=True)

    def test_readonly_refusal_releases_without_writing(self):
        report, steps = self.transaction({"admit": (1,b"refused power_state\n")})
        self.assertEqual(steps, ["acquire","admit","release"])
        self.assertTrue(report["released"]); self.assertFalse(report["native_restored"])

    def test_failure_requires_close_and_independent_absence(self):
        base = {"admit":(0,b"admitted\n"),"stage":(0,b"staged\n"),"run":(1,b"refused gui_exit_target\n"), "close":(0,b"closed\n"),"absent":(0,b"returned\n")}
        report, steps = self.transaction(base)
        self.assertTrue(report["released"] and report["native_restored"])
        self.assertEqual(steps, ["acquire","admit","stage","run","close","absent","release"])
        report, steps = self.transaction({**base,"absent":(1,b"refused not_returned\n")})
        self.assertTrue(report["recovery_required"]); self.assertNotIn("release",steps)


if __name__ == "__main__":
    unittest.main()
