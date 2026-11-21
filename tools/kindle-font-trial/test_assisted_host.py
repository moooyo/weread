import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import subprocess
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run
import run_assisted_shelf as assisted
from analyze_assisted_shelf import BASE, CASE, OPERATOR_FIELDS, SEQUENCE, analyze_shelf
from test_orientation import records


class AssistedHostTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.plan = self.work / 'plan'
        self.plan.mkdir()
        self.config = assisted.CONTRACT | dict(label='0123456789abcdef')
        (self.plan / 'plan.json').write_text(json.dumps(self.config))
        (self.plan / 'device.sh').write_text('exit 1\n')
        (self.plan / 'inputs.tgz').write_bytes(b'public fixture')
        self.args = SimpleNamespace(work=self.work, plan=self.plan, lease_tool=self.plan/'device.sh', owner='test-owner',
                                    preflight=False, recover=None, operator_ready=True, wait=0, admit_only=False)
        self.calls = []
        self.output = io.StringIO()
        self.start = b'D:1\n'
        self.bad_absence = False
        self.lease = Mock()
        self.lease.LeaseRequest.side_effect = lambda _, token, owner, ident: SimpleNamespace(token_file=token, owner=owner, ident=ident)
        self.lease.acquire.side_effect = self.acquire
        self.lease.status.return_value = dict(ownership_verified=True)
        self.lease.release.return_value = dict(owner_and_token_checked=True, only_owned_owner_json_removed_then_rmdir=True,
                                              released_at_utc='2026-11-21T02:11:12+00:00')

    def acquire(self, request):
        self.assertEqual(request.ident, 'weread-assisted-' + self.config['label'])
        request.token_file.touch(exist_ok=False)
        return dict(acquired_at_utc='2026-11-21T02:10:01+00:00')

    def simulate(self, cmd, **kwargs):
        if cmd[0] == '/opt/kindle-toolchain/ubuntu-run':
            self.assertTrue(self.lease.release.called)
            self.assertIn('/work/src/tools/kindle-font-trial/analyze_assisted_shelf.py', cmd)
            self.calls.append('analyze-after-release')
            return subprocess.CompletedProcess(cmd, 0)
        self.assertEqual(cmd[0], 'fixture-ssh')
        mode, label, name = cmd[-1].split()[-3:]
        self.assertEqual(label, self.config['label'])
        self.calls.append((mode, name))
        replies = dict(admit=b'admitted', reference=self.start, stage=b'indexed', case=b'completed B-library',
                       restore=b'restored', close=b'closed', absent=b'wrong' if self.bad_absence else b'absent')
        if mode == 'stage':
            self.assertIn('OPERATOR_CUE', self.output.getvalue())
        if mode == 'collect':
            kwargs['stdout'].write(b'fixture')
        return subprocess.CompletedProcess(cmd, 0, replies.get(mode, b''), b'')

    def execute(self):
        regular = Path.is_file
        with patch.object(assisted, 'verify_plan', return_value=self.config), patch.object(assisted, 'verify_recovery', return_value=self.config), \
             patch.object(Path, 'is_file', lambda path: str(path) == '/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
             patch.object(run, 'LEASE', self.work/'lock'), patch.dict(run.UTIL, ssh_command=lambda *_: ['fixture-ssh'], load_lease=lambda *_: self.lease), \
             patch.object(run.sys, 'stdin', io.StringIO('test-secret\n')), patch.object(run.signal, 'signal'), \
             patch.object(run.subprocess, 'run', side_effect=self.simulate), redirect_stdout(self.output):
            return assisted.execute(self.args)

    def test_operator_required_and_preflight_never_takes_lease(self):
        self.args.operator_ready = False
        with self.assertRaisesRegex(ValueError, 'present operator'):
            self.execute()
        self.args.preflight = True
        self.assertEqual(self.execute(), 0)
        self.assertFalse(self.calls)
        self.lease.acquire.assert_not_called()

    def test_down_prompts_one_turn_and_only_one_case(self):
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, SEQUENCE + ['analyze-after-release'])
        self.assertIn('OPERATOR_CUE TURN_180', self.output.getvalue())
        attempt, = self.plan.glob('batch-*')
        self.assertEqual(json.loads((attempt/'operations.json').read_text())['initial_native_reference'], 'D:1')
        self.assertFalse((attempt/'private-token').exists())
        with self.assertRaisesRegex(ValueError, 'new label'):
            self.execute()

    def test_up_prompts_no_turn(self):
        self.start = b'U:3\n'
        self.assertEqual(self.execute(), 0)
        self.assertIn('OPERATOR_CUE HOLD_STILL', self.output.getvalue())
        self.assertNotIn('TURN_180', self.output.getvalue())

    def test_unrecognized_reference_never_prompts_or_stages(self):
        self.start = b'FIXED:3\n'
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls, [('admit','none'), ('reference','none'), 'analyze-after-release'])
        self.assertNotIn('OPERATOR_CUE', self.output.getvalue())

    def test_absence_failure_and_close_only_recovery(self):
        self.bad_absence = True
        self.assertEqual(self.execute(), 1)
        self.lease.release.assert_not_called()
        self.args.recover, = self.plan.glob('batch-*')
        self.calls.clear()
        self.args.operator_ready = self.bad_absence = False
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, [('restore','none'), ('close','none'), ('absent','none')])
        self.lease.acquire.assert_called_once()


class AssistedAnalysisTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.archive = Path(temporary.name)/'results.tgz'
        self.plan = assisted.CONTRACT | dict(label='0123456789abcdef')
        self.receipt = dict(label=self.plan['label'], purpose=self.plan['purpose'], scope='transaction_only', passed=True,
                            acquired='2026-11-21T00:00:00+00:00', operator_cue_issued='2026-11-21T00:00:01+00:00',
                            released='2026-11-21T00:01:00+00:00', owner_token_checked_on_release=True,
                            initial_native_reference='D:1', operations=[dict(mode=mode, case=name, status=0) for mode, name in SEQUENCE])
        self.operator = {'label': self.plan['label']} | dict.fromkeys(OPERATOR_FIELDS, True)
        request, markers = records(CASE, 30)
        self.metadata = dict(profile='scribe', backend='kindle', route='settings', window_size=[1860,2480], snapshot_size=[1860,2480],
                             logical_size=[930,1240], design_logical_size=[930,1240], scale_factor=2.0)
        self.entries = dict(portrait=b'U:3\n', sensor=''.join(f'{index} {index*100} {index*100+10} 15\n' for index in range(1,11)).encode(),
                            observation=b'captured candidate_library_exit\n', request=request.encode(), markers=markers.encode(),
                            shelf=b'SHELF 30150000000 30160000000 3 grid\n') | {'portrait-launch': b'U:3\n', 'portrait-return': b'U:3\n',
                            'portrait-source': b'native_idle_calibration\n', 'final.metadata.json': json.dumps(self.metadata).encode()}

    def analyze(self, entries=None, operator=None, label=None):
        members = [(f'out/{CASE}/{name}', raw) for name,raw in (self.entries if entries is None else entries).items()]
        members.append(('out/orientation-batch', ((label or self.plan['label'])+'\n').encode()))
        with tarfile.open(self.archive, 'w:gz') as archive:
            for name, raw in members:
                member = tarfile.TarInfo(name)
                member.size = len(raw)
                archive.addfile(member, io.BytesIO(raw))
        return analyze_shelf(self.plan, self.receipt, self.archive, operator)

    def test_route_and_digital_shelf_do_not_independently_prove_physical_tap(self):
        result = self.analyze()
        self.assertEqual(result['single_profile_tap'], 'settings_observed_operator_unconfirmed')
        self.assertEqual(result['digital_shelf']['status'], 'observed_populated_shelf')
        self.assertEqual(self.analyze(operator=self.operator)['single_profile_tap'], 'operator_and_route_confirmed')
        denied = self.operator | {'tapped_profile_tab_once': False}
        self.assertEqual(self.analyze(operator=denied)['single_profile_tap'], 'operator_observations_do_not_confirm')
        with self.assertRaisesRegex(ValueError, 'bound'):
            self.analyze(operator=self.operator | {'label':'f'*16})

    def test_wrong_final_route_is_preserved_and_missing_shelf_is_not_a_positive(self):
        entries = self.entries | {'final.metadata.json': json.dumps(self.metadata | {'route':'search'}).encode(), 'shelf':b'not_observed\n'}
        report = self.analyze(entries, self.operator)
        self.assertEqual(report['final_application_route'], 'search')
        self.assertEqual(report['single_profile_tap'], 'expected_final_route_not_observed')
        self.assertEqual(report['digital_shelf']['status'], 'not_observed')
        for raw in (b'SHELF 29900000000 29950000000 3 grid\n', b'SHELF 30150000000 30160000000 1 grid\n'):
            self.assertEqual(self.analyze(self.entries | {'shelf': raw})['digital_shelf']['status'], 'insufficient_clock_or_rotation')

    def test_refused_pose_is_retained_and_never_accepted_as_launched(self):
        blocked = {name: raw for name,raw in self.entries.items() if name in BASE}
        blocked.update(portrait=b'D:1\n', observation=b'not_started pose_gate\n')
        self.assertEqual(self.analyze(blocked, self.operator)['application'], 'not_started_pose_gate')
        for changed in ({'portrait':b'D:1\n'}, {'sensor':self.entries['sensor'].replace(b'15\n',b'16\n')}, {'portrait-launch':b'D:1\n'}):
            with self.assertRaisesRegex(ValueError, 'target portrait'):
                self.analyze(self.entries | changed)

    def test_missing_extra_or_unbound_records_are_rejected(self):
        for changed in (self.entries|{'private':b'foreign'}, {k:v for k,v in self.entries.items() if k != 'markers'},
                        self.entries|{'sensor':self.entries['sensor'].replace(b'3 300', b'4 299')}):
            with self.assertRaises(ValueError):
                self.analyze(changed)
        with self.assertRaisesRegex(ValueError, 'label'):
            self.analyze(label='f'*16)
        self.receipt['owner_token_checked_on_release'] = False
        with self.assertRaisesRegex(ValueError, 'released'):
            self.analyze()


if __name__ == '__main__':
    unittest.main()
