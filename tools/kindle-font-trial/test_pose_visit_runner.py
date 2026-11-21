import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run
import run_pose_visit as visit


class VisitRunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.plan = self.work / 'visit'
        self.plan.mkdir()
        self.config = visit.CONTRACT | dict(label='0123456789abcdef')
        (self.plan / 'plan.json').write_text(json.dumps(self.config))
        (self.plan / 'device.sh').write_text('exit 1\n')
        (self.plan / 'inputs.tgz').write_bytes(b'public fixture')
        self.args = SimpleNamespace(work=self.work, plan=self.plan, lease_tool=self.plan / 'device.sh', owner='test-owner',
                                    preflight=False, recover=None, wait=0, admit_only=False)
        self.lease = Mock()
        self.lease.LeaseRequest.side_effect = lambda _lease, path, owner, ident: SimpleNamespace(token_file=path, owner=owner, ident=ident)
        self.lease.acquire.side_effect = self.acquire
        self.lease.status.return_value = dict(ownership_verified=True)
        self.lease.release.return_value = dict(owner_and_token_checked=True, only_owned_owner_json_removed_then_rmdir=True,
                                              released_at_utc='2026-11-21T02:11:12+00:00')
        self.calls = []
        self.reject = None

    def acquire(self, request):
        self.assertEqual(request.ident, 'weread-pose-wake-' + self.config['label'])
        request.token_file.touch(exist_ok=False)
        return dict(acquired_at_utc='2026-11-21T02:10:01+00:00')

    def simulate(self, cmd, **kwargs):
        if cmd[0] == '/opt/kindle-toolchain/ubuntu-run':
            self.assertTrue(self.lease.release.called)
            self.assertIn('/work/src/tools/kindle-font-trial/analyze_pose_visit.py', cmd)
            self.calls.append('analyze-after-release')
            return subprocess.CompletedProcess(cmd, 0)
        self.assertEqual(cmd[0], 'fixture-ssh')
        mode, label, name = cmd[-1].split()[-3:]
        self.assertEqual(label, self.config['label'])
        self.calls.append(mode if mode != 'case' else name)
        if mode == 'collect':
            kwargs['stdout'].write(b'fixture')
        replies = dict(admit='admitted', stage='indexed', case='completed B-reader', restore='restored', close='closed', absent='absent')
        if mode == self.reject:
            return subprocess.CompletedProcess(cmd, 1, b'', b'private diagnostic should not be retained')
        return subprocess.CompletedProcess(cmd, 0, replies.get(mode, '').encode(), b'')

    def execute(self):
        regular = Path.is_file
        with patch.object(visit, 'verify_plan', return_value=self.config), patch.object(visit, 'verify_recovery', return_value=self.config), \
             patch.object(Path, 'is_file', lambda path: str(path) == '/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
             patch.object(run, 'LEASE', self.work / 'lock'), patch.dict(run.UTIL, ssh_command=lambda *_: ['fixture-ssh'], load_lease=lambda *_: self.lease), \
             patch.object(run.sys, 'stdin', io.StringIO('test-secret\n')), patch.object(run.signal, 'signal'), \
             patch.object(run.subprocess, 'run', side_effect=self.simulate), redirect_stdout(io.StringIO()):
            return visit.execute(self.args)

    def test_preflight_validates_without_lease_or_transport(self):
        self.args.preflight = True
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, [])
        self.lease.acquire.assert_not_called()
        self.config['expected_panel_alignment_degrees'] = 180
        with self.assertRaisesRegex(ValueError, 'Reader visit'):
            self.execute()

    def test_only_one_visit_then_absent_and_release_before_analysis(self):
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, ['admit','stage','B-reader','restore','collect','close','absent','analyze-after-release'])
        attempt, = self.plan.glob('batch-*')
        receipt = json.loads((attempt / 'operations.json').read_text())
        self.assertTrue(receipt['passed'] and receipt['owner_token_checked_on_release'])
        self.assertEqual(receipt['scope'], 'transaction_only')
        self.assertFalse((attempt / 'private-token').exists())
        self.assertEqual(self.lease.status.call_count, 7)
        with self.assertRaisesRegex(ValueError, 'new label'):
            self.execute()
        self.lease.acquire.assert_called_once()

    def test_unconfirmed_absence_retains_lease_and_recovery_only_closes(self):
        self.reject = 'absent'
        self.assertEqual(self.execute(), 1)
        self.lease.release.assert_not_called()
        attempt, = self.plan.glob('batch-*')
        first = (attempt / 'operations.json').read_bytes()
        self.assertNotIn(b'test-secret', first)
        self.assertNotIn(b'private diagnostic', first)
        self.assertFalse(list(attempt.glob('*.stderr')))
        self.calls.clear()
        self.reject = None
        self.args.recover = attempt
        (self.plan / 'inputs.tgz').unlink()
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, ['restore','close','absent'])
        self.assertEqual(first, (attempt / 'operations.json').read_bytes())
        self.lease.acquire.assert_called_once()

    def test_case_refusal_is_closed_before_incomplete_analysis(self):
        self.reject = 'case'
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls[-4:], ['collect', 'close', 'absent', 'analyze-after-release'])
        attempt, = self.plan.glob('batch-*')
        receipt = json.loads((attempt / 'operations.json').read_text())
        self.assertFalse(receipt['passed'])
        self.assertTrue(receipt['owner_token_checked_on_release'])

    def test_partial_acquire_without_a_token_is_not_treated_as_an_idle_device(self):
        fault = RuntimeError('fixture partial acquire')
        fault.code = 'incomplete_create'
        fault.details = {'lease_directory_created': True, 'private_token_created': False, 'owner_record_created': False}
        self.lease.acquire.side_effect = fault
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls, [])
        self.lease.release.assert_not_called()
        attempt, = self.plan.glob('batch-*')
        receipt = json.loads((attempt / 'operations.json').read_text())
        self.assertTrue(receipt['recovery_required'])
        with self.assertRaisesRegex(ValueError, 'new label'):
            self.execute()

    def test_read_only_admission_refusal_does_not_stage_or_claim_native_restoration(self):
        self.reject = 'admit'
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls, ['admit','analyze-after-release'])
        attempt, = self.plan.glob('batch-*')
        receipt = json.loads((attempt / 'operations.json').read_text())
        self.assertFalse(receipt['passed'])
        self.assertNotIn('private diagnostic', (attempt / 'operations.json').read_text())


if __name__ == '__main__':
    unittest.main()
