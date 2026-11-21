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
import run_pose_flip
from analyze_pose_flip import CASE, CHECKPOINT, SEQUENCE, analyze
from prepare_pose_flip import CONTRACT
from test_orientation import records


LABEL = '0123456789abcdef'


class FlipEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.plan = CONTRACT | dict(label=LABEL)
        self.receipt = dict(label=LABEL, purpose=CONTRACT['purpose'], scope='transaction_only', passed=True,
                            initial_native_reference='D:1', owner_token_checked_on_release=True,
                            acquired='2026-11-21T02:10:00Z', operator_cue_issued='2026-11-21T02:10:02Z',
                            released='2026-11-21T02:12:00Z', operations=[dict(mode=mode,case=name,status=0) for mode,name in SEQUENCE])
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.path = Path(temp.name) / 'results.tgz'
        self.entries = self.make_stream()

    def make_stream(self, switch=10, direction=16, base=100_000_000_000):
        request, lines = records(CASE,100)
        names = [line.split()[2] for line in lines.splitlines()]
        markers = ''.join(f'WEREAD_STARTUP 100.{index * 100_000_000:09d} {name}\n' if index < 10 else
                          f'WEREAD_STARTUP 101.{(index-10)*100_000_000:09d} {name}\n' for index,name in enumerate(names,1))
        rows = [[index+1, base-16_000_000 + index*50_000_001, base-15_999_999 + index*50_000_001,
                 direction if index < switch else 15] for index in range(CONTRACT['sensor_samples'])]
        binding = f'LAUNCH {LABEL} B-library 11111111-1111-1111-1111-111111111111 343 454 CLOCK_MONOTONIC\n'
        render = dict(profile='scribe',backend='kindle',route='library',design_logical_size=[930,1240],logical_size=[930,1240],
                      window_size=[1860,2480],snapshot_size=[1860,2480],scale_factor=2)
        return dict(request=request, markers=markers, sensor=''.join('%d %d %d %d\n' % tuple(row) for row in rows),
                    shelf='SHELF 112500000005 112500000999 3 grid\n', portrait='D:1\n', observation='captured startup_turn\n') | {
                        'app-binding':binding,'sensor-binding':binding,'sensor-ready':'READY %d %d %d\n'%tuple(rows[0][1:]),
                        'portrait-source':'native_idle_calibration\n','portrait-launch':'D:1\n','portrait-return':'U:3\n',
                        'final.metadata.json':json.dumps(render)}

    def analyze(self):
        with tarfile.open(self.path, 'w:gz') as output:
            for name, value in [('out/orientation-batch', LABEL+'\n'), *((f'out/{CASE}/{name}',value) for name,value in self.entries.items())]:
                data = value.encode(); info = tarfile.TarInfo(name); info.size = len(data); output.addfile(info, io.BytesIO(data))
        return analyze(self.plan, self.receipt, self.path)

    def test_proper_release_bound_sensor_turn_does_not_imply_touch_or_upright_frame(self):
        report = self.analyze()
        self.assertEqual(report['startup_turn']['coverage'], 'hit')
        self.assertEqual(report['digital_shelf']['status'], 'observed_populated_shelf')
        self.assertEqual((report['touch'], report['visual_uprightness'], report['first_frame_orientation']),
                         ('not_tested','not_established','not_established'))
        self.assertNotIn('boot_id', str(report))

    def test_unobserved_display_does_not_discard_real_pose_change(self):
        self.entries['shelf'] = 'not_observed\n'
        self.assertEqual(self.analyze()['startup_turn']['coverage'], 'hit')

    def test_static_outside_and_edge_are_not_hits(self):
        for switch, expected in [(4,'missed'), (9,'insufficient'), (40,'missed'), (1000,'insufficient')]:
            with self.subTest(switch=switch):
                self.entries = self.make_stream(switch=switch)
                self.assertEqual(self.analyze()['startup_turn']['coverage'], expected)

    def test_distinct_binding_wrong_ready_and_identity_replay_rejected(self):
        for field,replacement in [('sensor-binding',self.entries['sensor-binding'].replace(' 343 ', ' 344 ')),
                                  ('app-binding',self.entries['app-binding'].replace(LABEL, 'f'*16)),
                                  ('sensor-ready', 'READY 123 124 16\n'), ('sensor', self.entries['sensor'].split('\n',1)[1])]:
            with self.subTest(field=field), patch.dict(self.entries, {field:replacement}), self.assertRaises(ValueError):
                self.analyze()

    def test_short_coverage_or_unclosed_transaction_not_credited(self):
        self.entries = self.make_stream(base=100_500_000_000)
        self.assertEqual(self.analyze()['startup_turn']['reason'],'sensor_does_not_span_launch')
        self.entries = {}; self.receipt['passed'] = False
        self.receipt['operations'][3].update(status=1, diagnostic='refused flip_sampler_inactive')
        partial = self.analyze()
        self.assertEqual((partial['collection'], partial['case_refusal']), ('not_completed','flip_sampler_inactive'))
        self.receipt['owner_token_checked_on_release'] = False
        with self.assertRaises(ValueError): self.analyze()

    def test_live_calibration_is_not_evidence_for_or_against_a_time_bound_change(self):
        for value,status in [('D:1','observed'),('U:3','observed'),('FIXED:1','fixed'),('missing','missing'),
                             ('malformed','malformed'),('foreign_generation','foreign_generation')]:
            for switch, expected in [(10,'hit'), (40,'missed'), (1000,'insufficient')]:
                with self.subTest(value=value, switch=switch):
                    self.entries = self.make_stream(switch=switch)
                    self.entries['portrait-launch'] = value+'\n'
                    result = self.analyze()
                    self.assertEqual(result['startup_turn']['coverage'], expected)
                    self.assertEqual(result['launch_calibration']['status'], status)
                    self.assertEqual(result['sensor_direction']['directional_changes'],int(switch < 1000))
                    self.assertEqual(result['native_launch'],value if status in ('fixed','observed') else None)
        self.entries['portrait-launch'] = '1:private:U:3\n'
        with self.assertRaises(ValueError): self.analyze()

    def test_closed_exact_checkpoint_can_classify_without_visual_end_state(self):
        self.receipt['passed'] = False
        self.receipt['operations'][3].update(status=1,diagnostic='refused capture_metadata')
        self.entries = {key:value for key,value in self.entries.items() if key in CHECKPOINT}
        self.entries['observation'] = 'partial capture_metadata\n'
        self.entries['portrait-launch'] = 'U:3\n'
        report = self.analyze()
        self.assertEqual(report['collection'],'pose_checkpoint_only_released')
        self.assertEqual(report['startup_turn']['coverage'],'hit')
        self.assertEqual(report['launch_calibration']['comparison_to_before'],'different')
        self.assertIsNone(report['native_return']); self.assertIsNone(report['final_route'])
        self.assertEqual(report['digital_shelf']['status'],'not_assessed')
        with patch.dict(self.entries, {'observation':'partial secret_content\n'}), self.assertRaises(ValueError): self.analyze()
        with patch.dict(self.entries, {'final.metadata.json':'{}'}), self.assertRaises(ValueError): self.analyze()
        self.receipt['operations'][4]['status'] = 1
        self.assertEqual(self.analyze()['collection'],'not_completed')

    def test_absent_optional_marker_is_not_manufactured_or_required(self):
        lines = self.entries['markers'].splitlines(keepends=True)
        self.entries['markers'] = ''.join(line for line in lines if not line.endswith('fonts_registered\n'))
        result=self.analyze(); self.assertEqual(result['startup_turn']['coverage'],'hit'); self.assertIsNone(result['orientation_queries'])
        self.entries['markers'] = ''.join(line for line in lines if not line.endswith('portrait_opened\n'))
        result=self.analyze(); self.assertEqual(result['startup_turn']['coverage'],'insufficient')
        self.assertEqual(result['startup_turn']['phase_relation'],'after_splash_before_submit_log_predraw_unknown')
        self.assertEqual(result['sensor_direction']['directional_changes'],1)
        self.entries['markers'] = ''.join(lines + [lines[-1]])
        self.assertEqual(self.analyze()['startup_turn']['reason'],'missing_or_invalid_window_markers')

    def test_prior_reference_or_image_mismatch_does_not_create_a_sensor_turn(self):
        self.receipt['initial_native_reference']='U:3'
        self.entries['final.metadata.json']='{}'; self.entries['shelf']='not_observed\n'
        result=self.analyze(); self.assertEqual(result['startup_turn']['coverage'],'hit')
        self.assertFalse(result['initial_reference_matches_before']); self.assertEqual(result['final_route_status'],'unusable_metadata')


class FlipRunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name); self.plan = self.work / 'plan'; self.plan.mkdir()
        self.config = CONTRACT | dict(label=LABEL)
        (self.plan/'plan.json').write_text(json.dumps(self.config)); (self.plan/'device.sh').write_text('exit 1\n')
        (self.plan/'inputs.tgz').write_bytes(b'fixture')
        self.args = SimpleNamespace(work=self.work,plan=self.plan,lease_tool=self.plan/'device.sh',owner='test-owner',
                                    preflight=False,recover=None,operator_ready=True,wait=0,admit_only=False)
        self.lease=Mock(); self.calls=[]; self.reference = 'D:1'; self.bad = None
        self.output = io.StringIO()
        self.lease.LeaseRequest.side_effect = lambda _,token,owner,ident: SimpleNamespace(token_file=token,owner=owner,ident=ident)
        def acquire(req):
            self.assertEqual(req.ident, 'weread-startup-flip-' + LABEL); req.token_file.touch(exist_ok=False)
            return dict(acquired_at_utc='2026-11-21T02:10:01+00:00')
        self.lease.acquire.side_effect=acquire; self.lease.status.return_value=dict(ownership_verified=True)
        self.lease.release.return_value=dict(owner_and_token_checked=True,only_owned_owner_json_removed_then_rmdir=True,
                                             released_at_utc='2026-11-21T02:11:12+00:00')

    def command(self, cmd, **kwargs):
        if cmd[0]=='/opt/kindle-toolchain/ubuntu-run':
            self.assertIn('/work/src/tools/kindle-font-trial/analyze_pose_flip.py',cmd); self.assertTrue(self.lease.release.called)
            self.calls.append('analyze'); return subprocess.CompletedProcess(cmd,0)
        mode,label,case=cmd[-1].split()[-3:]; self.assertEqual(label,LABEL); self.calls.append(mode)
        if mode == self.bad: return subprocess.CompletedProcess(cmd,1,b'',b'refused fixture\n')
        if mode=='case': self.assertEqual(case,CASE)
        replies=dict(reference=self.reference,admit='admitted',stage='indexed',case='completed B-library',restore='restored',close='closed',absent='absent')
        if mode=='collect': kwargs['stdout'].write(b'fixture')
        return subprocess.CompletedProcess(cmd,0,(replies.get(mode,'')+'\n').encode(),b'')

    def execute(self):
        isfile=Path.is_file
        with patch.object(run_pose_flip,'verify_plan',return_value=self.config), patch.object(run_pose_flip,'verify_recovery',return_value=self.config), \
             patch.object(Path,'is_file',lambda path: str(path)=='/opt/kindle-toolchain/ubuntu-run' or isfile(path)), \
             patch.object(run,'LEASE',self.work/'lock'), patch.dict(run.UTIL,ssh_command=lambda *_:['fixture-ssh'],load_lease=lambda *_:self.lease), \
             patch.object(run.sys,'stdin',io.StringIO('test-secret\n')), patch.object(run.signal,'signal'), \
             patch.object(run.subprocess,'run',side_effect=self.command), redirect_stdout(self.output):
            return run_pose_flip.execute(self.args)

    def test_no_lease_for_preflight_or_absent_operator(self):
        self.args.operator_ready=False
        with self.assertRaisesRegex(ValueError,'operator'): self.execute()
        self.args.preflight=True; self.assertEqual(self.execute(),0); self.lease.acquire.assert_not_called()

    def test_each_valid_initial_pose_may_start_exactly_once(self):
        self.reference='U:3'; self.assertEqual(self.execute(),0)
        self.assertEqual(self.calls, [mode for mode,_ in SEQUENCE]+['analyze']); self.assertIn('OPERATOR_CUE STARTUP_TURN',self.output.getvalue())
        self.assertNotIn('HOLD_STILL',self.output.getvalue())
        with self.assertRaisesRegex(ValueError,'new label'): self.execute()

    def test_unconfirmed_absence_only_allows_recovery(self):
        self.bad='absent'; self.assertEqual(self.execute(),1); self.lease.release.assert_not_called()
        self.args.recover, = self.plan.glob('batch-*'); self.calls.clear(); self.bad=None
        self.assertEqual(self.execute(),0); self.assertEqual(self.calls,['restore','close','absent']); self.lease.acquire.assert_called_once()

    def test_unknown_native_no_stage(self):
        self.reference='D:3'; self.assertEqual(self.execute(),1)
        self.assertEqual(self.calls,['admit','reference','analyze']); self.lease.release.assert_called_once()


if __name__ == '__main__': unittest.main()
