import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image

import run
import run_assisted_sleep as sleep
from analyze_assisted_sleep import analyze, initial_diagnostic, mappings, reader_images, sensor_rows
from analyze_pose_visit import SEQUENCE
from sleep_turn_stream import CueGate, permitted, stream_case
from test_orientation import encoded, records
from test_orientation_gate import public_fields
import test_pose_visit_runner as fixture


LABEL = '0123456789abcdef'
PLAN = sleep.CONTRACT | {'label': LABEL}


class StreamTests(unittest.TestCase):
    def setUp(self):
        self.ledger = []
        self.gate = CueGate(LABEL, lambda _: True, lambda *entry: self.ledger.append(entry))

    def test_out_of_order_stale_and_duplicate_stages_do_not_instruct(self):
        self.gate.offer('turn')
        self.gate.permit = lambda _: False
        self.gate.offer('sleep')
        self.gate.offer('wake')
        self.assertFalse(self.gate.issued)
        with self.assertRaises(ValueError):
            self.gate.offer('sleep')
        with self.assertRaises(ValueError):
            self.gate.line(b'PHASE ffffffffffffffff wake')

    def test_expired_or_foreign_permits_are_refused(self):
        for status, output, elapsed in [(0,b'permitted 0000000000000000 sleep\n', 0.02),
                                        (0,f'permitted {LABEL} sleep\n'.encode(), 3.1), (1,b'', 0.02)]:
            self.assertFalse(permitted(LABEL, 'sleep', lambda *_:subprocess.CompletedProcess([],status,output),
                                       iter((400,400+elapsed)).__next__))

    def test_recovery_and_ordinary_wake_share_one_notification(self):
        for phase in ('sleep','turn','wake','recover-wake'):
            self.gate.offer(phase)
        self.assertEqual(self.ledger[-1], ('recover-wake', 'suppressed_order'))
        self.assertEqual(len(self.gate.issued), 3)

    def test_real_pipe_delivers_fragments_and_fewer_than_three_is_allowed(self):
        encoded_line = 'PHASE ' + LABEL + ' sleep'
        code = f'import os; os.write(1,{encoded_line[:8].encode()!r}); os.write(1,{encoded_line[8:].encode()!r}+b"\\ncompleted B-reader\\n")'
        with tempfile.TemporaryFile() as source:
            result = stream_case([sys.executable,'-c',code],source,os.environ,self.gate,3)
        self.assertEqual((result.returncode,self.ledger), (0,[('sleep','issued')]))

    def test_real_transport_timeout_and_bad_stream_are_bounded(self):
        with tempfile.TemporaryFile() as source:
            with self.assertRaises(subprocess.TimeoutExpired):
                stream_case([sys.executable,'-c','import time; time.sleep(1)'],source,os.environ,self.gate,0.05)
            invalid = stream_case([sys.executable,'-c',"print('untrusted')"],source,os.environ,self.gate,3)
        self.assertEqual(invalid.returncode,65)
        self.assertFalse(self.ledger)


class WorkerTests(unittest.TestCase):
    setUp = fixture.VisitRunnerTests.setUp

    def acquire(self, request):
        self.assertEqual(request.ident, 'weread-sleep-turn-' + LABEL)
        request.token_file.touch()
        return {'acquired_at_utc': '2026-11-21T02:10:01+00:00'}

    def test_stream_is_scoped_and_analysis_follows_verified_absence(self):
        self.config = PLAN.copy()
        (self.plan / 'plan.json').write_text(json.dumps(self.config))
        self.args.operator_ready = True

        def transport(command, **kwargs):
            if command[0] == '/opt/kindle-toolchain/ubuntu-run':
                self.assertTrue(self.lease.release.called)
                self.assertIn('/work/src/tools/kindle-font-trial/analyze_assisted_sleep.py',command)
                self.calls.append('analysis'); return subprocess.CompletedProcess(command,0)
            mode, label, name = command[-1].split()[-3:]
            self.calls.append(mode)
            replies = dict(admit='admitted', stage='indexed', permit='permitted '+label+' '+name, restore='restored', close='closed', absent='absent')
            if mode == 'collect': kwargs['stdout'].write(b'fixture')
            return subprocess.CompletedProcess(command,0,(replies.get(mode,'')+'\n').encode(),b'')

        def stream(command, _data, _env, gate):
            self.calls.append('case')
            for phase in ('sleep','turn','wake'): gate.line(f'PHASE {LABEL} {phase}'.encode())
            gate.line(b'completed B-reader')
            return subprocess.CompletedProcess(command,0,b'completed B-reader\n',b'')

        regular = Path.is_file
        with patch.object(sleep,'verify_plan',return_value=self.config), patch.object(Path,'is_file',lambda path: str(path)=='/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
             patch.object(run,'LEASE',self.work/'lock'), patch.dict(run.UTIL,ssh_command=lambda *_:['fake'],load_lease=lambda *_:self.lease), \
             patch.object(run.sys,'stdin',io.StringIO('fixture\n')), patch.object(run.signal,'signal'), patch.object(run.subprocess,'run',side_effect=transport), \
             patch('sleep_turn_stream.stream_case',side_effect=stream), patch('sys.stdout',new_callable=io.StringIO) as output:
            self.assertEqual(sleep.execute(self.args),0)
            self.assertEqual(output.getvalue().count('SLEEP_CUE '),3)
        self.assertEqual(self.calls,['admit','stage','case','permit','permit','permit','restore','collect','close','absent','analysis'])


def captured():
    request, markers = records('B-reader',30)
    entries = public_fields('reader')
    entries.pop('thumb.pgm')
    entries.update(request=request.encode(), markers=markers.encode(), portrait=b'D:1\n', **{'portrait-launch':b'D:1\n', 'portrait-return':b'U:3\n',
                   'observation':b'observed changed_sleep_cycle\n','image-mapping':b'IMAGE pre keep 16 1\nIMAGE post turn 15 1\n'})
    with Image.open(io.BytesIO(entries['final.png'])) as initial:
        final = initial.transpose(Image.Transpose.ROTATE_180)
        entries.update({'panel.pgm': encoded(initial, 'PPM'), 'resumed.pgm': encoded(final, 'PPM'),
                        'final.png': encoded(final, 'PNG')})
    start = 31_000_000_000
    for phase, pressures, cal in [('pre',(16,), 'D:1'), ('sleep-pre',(16,18),'none'), ('sleep-post',(15,17),'none'), ('post',(15,),'none')]:
        lines = []
        for value in pressures:
            lines.append(f'WAKE_OBS {phase} {start} {start+100} {value} 1 {cal}\n')
            start += 200_000_000
        entries[phase+'.row'] = ''.join(lines).encode()
    return entries


class EvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.entries = captured()

    def report(self, entries, cues=('sleep','turn','wake'), plan=PLAN, operator=None):
        import tarfile
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'results.tgz'
            with tarfile.open(path,'w:gz',compresslevel=1) as archive:
                for name, raw in {'out/orientation-batch':(LABEL+'\n').encode(), **{'out/B-reader/'+key:val for key,val in entries.items()}}.items():
                    item=tarfile.TarInfo(name); item.size=len(raw); archive.addfile(item,io.BytesIO(raw))
            receipt = dict(label=LABEL, purpose=PLAN['purpose'], scope='transaction_only', passed=True, owner_token_checked_on_release=True,
                           acquired='2026-11-21T02:09:00+00:00', released='2026-11-21T02:15:00+00:00',
                           operations=[dict(mode=mode,case=name,status=0) for mode,name in SEQUENCE],
                           operator_phases=[dict(phase=name,status='issued',at=f'2026-11-21T02:1{index}:00+00:00') for index,name in enumerate(cues)])
            return analyze(plan,receipt,path,operator)

    def test_expected_half_turn_from_pose_not_post_hoc_panel_normalization(self):
        result = self.report(self.entries)
        images = result['reader_images']
        self.assertEqual((images['method'], images['absolute_orientation']), ('relative_scanout_v2', 'not_measured'))
        self.assertEqual((images['pre']['degrees'], images['post']['degrees']), (0, 180))
        self.assertEqual(images['comparison']['expected_delta_degrees'], 180)
        self.assertEqual(images['comparison']['status'], 'expected_relative_alignment')
        self.assertEqual(images['comparison']['different_body_pixels'], 0)
        self.assertEqual(images['software_content'], 'consistent')
        self.assertEqual(result['reader'],'identical_aligned_body')
        self.assertEqual(result['qualification'],'confirmed_same_reader_and_changed_pose_in_scope')
        self.assertIn('physical touch mapping',result['not_measured'])

    def test_one_symmetric_crop_excludes_both_margins_before_relative_comparison(self):
        with Image.open(io.BytesIO(self.entries['resumed.pgm'])) as post:
            width, height = post.size
            post.paste(124, (0, 0, width, 72))
            post.paste(69, (0, height - 72, width, height))
            result = self.report(self.entries | {'resumed.pgm': encoded(post, 'PPM')})
        comparison = result['reader_images']['comparison']
        self.assertEqual(comparison['body_pixels'], width * (height - 144))
        self.assertEqual((comparison['different_body_pixels'], comparison['expected_mae']), (0, 0))
        self.assertEqual(comparison['status'], 'expected_relative_alignment')
        self.assertEqual(result['reader'], 'identical_aligned_body')

    def test_body_difference_remains_nonidentical_even_with_correct_relative_orientation(self):
        with Image.open(io.BytesIO(self.entries['resumed.pgm'])) as post:
            point = (0, 72)
            prior = post.getpixel(point)
            post.putpixel(point, 255 - prior)
            result = self.report(self.entries | {'resumed.pgm': encoded(post, 'PPM')})
        comparison = result['reader_images']['comparison']
        self.assertEqual(comparison['status'], 'expected_relative_alignment')
        self.assertEqual(comparison['different_body_pixels'], 1)
        self.assertAlmostEqual(comparison['expected_mae'], abs(255 - 2 * prior) / comparison['body_pixels'])
        self.assertEqual(result['reader'], 'approximately_matching_aligned_body')
        self.assertEqual(result['qualification'], 'not_established')

    def test_symmetric_but_nonblank_pages_cannot_identify_a_half_turn(self):
        scene = Image.new('L', (1860, 2480), 255)
        scene.paste(0, (430, 500, 1430, 1980))
        panel = encoded(scene, 'PPM')
        result = self.report(self.entries | {'panel.pgm': panel, 'resumed.pgm': panel, 'final.png': encoded(scene, 'PNG')})
        comparison = result['reader_images']['comparison']
        self.assertEqual((comparison['expected_mae'], comparison['opposite_mae']), (0, 0))
        self.assertEqual(comparison['status'], 'inconclusive')
        self.assertEqual((result['reader'], result['qualification']), ('not_established', 'not_established'))

    def test_matching_relative_scans_cannot_substitute_for_same_reader_content(self):
        with Image.open(io.BytesIO(self.entries['final.png'])) as reference:
            unrelated = reference.point(lambda gray: 255 - gray)
            resolved = self.report(self.entries | {'final.png': encoded(unrelated, 'PNG')})
        self.assertEqual(resolved['reader_images']['comparison']['status'], 'expected_relative_alignment')
        self.assertEqual(resolved['reader_images']['software_content'], 'not_established')
        self.assertEqual((resolved['reader'], resolved['qualification']), ('not_established', 'not_established'))

    def test_relative_scan_respects_existing_mae_and_direction_margin_limits(self):
        size = (1860, 2480)
        for contrast, bias, margin, status in [
            (140, 8, 132, 'expected_relative_alignment'),
            (140, 9, 131, 'inconclusive'),
            (3, 0, 3, 'expected_relative_alignment'),
            (2, 0, 2, 'inconclusive'),
        ]:
            with self.subTest(bias=bias, margin=margin):
                before = Image.new('L', size, 35 + contrast)
                before.paste(35, (0, 0, size[0] // 2, size[1]))
                if not bias:
                    before.paste(200, (0, size[1] // 2, size[0] // 2, size[1]))
                    before.paste(200 + contrast, (size[0] // 2, 0, size[0], size[1] // 2))
                after = before.transpose(Image.Transpose.ROTATE_180).point(lambda gray: gray + bias)
                raws = self.entries | {'panel.pgm': encoded(before, 'PPM'), 'resumed.pgm': encoded(after, 'PPM'),
                                       'final.png': encoded(after, 'PNG')}
                comparison = reader_images(raws, {'pre': 0, 'post': 180})['comparison']
                self.assertEqual(comparison['expected_mae'], bias)
                self.assertEqual(comparison['opposite_mae'] - comparison['expected_mae'], margin)
                self.assertEqual(comparison['status'], status)

    def test_invalid_reader_reference_never_qualifies_matching_raw_frames(self):
        wrong_route = json.loads(self.entries['final.metadata.json']) | {'route': 'library'}
        for change in ({'final.metadata.json': json.dumps(wrong_route).encode()}, {'final.png': b'broken'}):
            with self.subTest(replaced=next(iter(change))):
                report = self.report(self.entries | change)
                self.assertTrue(report['reader_images']['reference_or_frame_error'])
                self.assertEqual((report['reader'], report['qualification']), ('not_established', 'not_established'))

    def test_post_only_changed_pose_does_not_replace_missing_sleep_window(self):
        entries = dict(self.entries); del entries['sleep-post.row']
        result = self.report(entries)
        self.assertEqual((result['sleep_turn'], result['qualification']),('not_established','not_established'))

    def test_inverted_or_missing_wake_picture_does_not_erase_observed_cycle(self):
        entries = self.entries | {'resumed.pgm': self.entries['panel.pgm']}
        result = self.report(entries)
        self.assertEqual(result['reader_images']['comparison']['status'], 'opposite_relative_alignment')
        self.assertNotEqual(result['reader_images']['software_content'], 'consistent')
        self.assertNotEqual(result['same_resident_cycle'],'not_established')
        self.assertEqual(result['qualification'],'not_established')

    def test_fallback_keeps_the_sleep_observation_but_does_not_qualify_reuse(self):
        entries = self.entries | {'markers':self.entries['markers'].replace(b'splash_orientation_reused', b'splash_orientation_timeout')}
        result = self.report(entries)
        self.assertFalse(result['both_orientation_sites_reused'])
        self.assertNotEqual(result['sleep_turn'], 'not_established')
        self.assertEqual(result['qualification'],'not_established')

    def test_bad_phase_spacing_and_contradictory_alignment_fail_closed(self):
        rows = sensor_rows(self.entries,'D:1',30_000_000_000)
        with self.assertRaisesRegex(ValueError,'contradicts'):
            mappings(self.entries|{'image-mapping':b'IMAGE post keep 15 1\n'}, rows,'D:1')
        old = self.entries['sleep-post.row'].splitlines()[0]
        with self.assertRaisesRegex(ValueError,'order'):
            sensor_rows(self.entries|{'sleep-post.row':old+b'\n'+old+b'\n'},'D:1',30_000_000_000)

    def test_released_first_stage_guard_refusal_remains_an_untouched_sleep_case(self):
        entries = {key: self.entries[key] for key in ('portrait','portrait-launch','portrait-return','portrait-source','request',
                                                      'markers','final.png','final.metadata.json')}
        entries.update(observation=b'not_qualified initial_reader\n')
        entries['initial-diagnostic'] = b'INITIAL observe_before task_busy 12\n'
        diagnostic_plan = PLAN | {'initial_diagnostic':'bounded_v1'}
        result = self.report(entries, cues=(), plan=diagnostic_plan)
        self.assertEqual(result['initial_reader_diagnostic'], {'status':'refused','site':'observe_before','reason':'task_busy','attempts':12})
        self.assertEqual((result['issued_notices'], result['samples'],result['qualification']),([],{},'not_established'))
        del entries['initial-diagnostic']
        self.assertEqual(self.report(entries,cues=())['initial_reader_diagnostic'], {'status':'not_recorded'})
        with self.assertRaisesRegex(ValueError, 'required'):
            self.report(entries,cues=(),plan=diagnostic_plan)

    def test_initial_reason_rejects_private_text_cross_stage_reasons_and_overclaimed_photos(self):
        for data in [b'INITIAL observe_before unexpected_token 1\n',b'INITIAL observe_after private/path 1\n',
                     b'INITIAL image_read generation 1\n',b'INITIAL observe_before task_busy 13\n', b'INITIAL ready ready 2\n']:
            with self.assertRaises(ValueError): initial_diagnostic(data,True,False,[])
        with self.assertRaises(ValueError): initial_diagnostic(b'INITIAL observe_before managed_available 1\n',True,False,[{}])
        self.assertEqual(initial_diagnostic(b'INITIAL ready ready 2\n',False,True,[])['status'],'captured')

    def test_device_first_guard_refusal_reaches_offline_report_without_notice_or_capture(self):
        tools = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            out = root / 'out'; out.mkdir()
            script = r'''
set -eu
. "$1/assisted-wake-observe.sh"
. "$1/assisted-sleep-change.sh"
out=$2; label=$3
as_now() { printf '100000\n'; }
wake_observe() { return 24; }
as_run_cycle
printf '%s\n' "$as_result" >"$out/observation"
printf 'completed B-reader\n'
'''
            events = []
            gate = CueGate(LABEL, lambda _: self.fail('This refused case must not solicit a permit'), lambda *item: events.append(item))
            with tempfile.TemporaryFile() as source:
                transaction = stream_case(['sh','-c',script,'fixture',str(tools),str(out),LABEL],source,os.environ,gate,3)
            self.assertEqual(transaction.returncode,0)
            self.assertTrue(gate.completed)
            self.assertFalse(events)
            self.assertEqual(sorted(path.name for path in out.iterdir()),['initial-diagnostic','observation'])
            entries = {key:self.entries[key] for key in ('portrait','portrait-launch','portrait-return','portrait-source',
                                                          'request','markers','final.png','final.metadata.json')}
            entries.update({path.name:path.read_bytes() for path in out.iterdir()})
            result = self.report(entries,cues=(),plan=PLAN|{'initial_diagnostic':'bounded_v1'})
            self.assertEqual(result['initial_reader_diagnostic'],
                             {'status':'refused','site':'observe_before','reason':'application_identity','attempts':1})
            self.assertEqual(result['qualification'],'not_established')


class WitnessEvidenceTests(unittest.TestCase):
    report = EvidenceTests.report

    @classmethod
    def setUpClass(cls): cls.entries = captured()

    def witness(self, after=16, during=(), post_sleep_rows=True, post_opposite=False, end='elapsed'):
        entries = self.entries.copy()
        plan = PLAN | {'sleep_turn_protocol':'witness_window_v1','operator_action_window_seconds':16,'initial_diagnostic':'bounded_v1'}
        entries.update({'observation':b'observed resident_wake\n','initial-diagnostic':b'INITIAL ready ready 1\n'})
        start = 32_000_000_000
        def readings(phase, values):
            nonlocal start
            lines = []
            for raw in values:
                lines.append(f'WAKE_OBS {phase} {start} {start+100} {raw} 1 none\n')
                start += 350_000_000
            return ''.join(lines).encode()
        entries['sleep-trace'] = readings('sleep-post', during)
        if post_sleep_rows:
            entries['sleep-post.row'] = readings('sleep-post',[15,17] if post_opposite else [16,18])
        else: del entries['sleep-post.row']
        entries['post-status.row'] = readings('post', [after])
        entries['post.row'] = readings('post', [after])
        entries['image-mapping'] = f"IMAGE pre keep 16 1\nIMAGE post {'turn' if after in (15,17) else 'keep'} {after} 1\n".encode()
        if after in (16,18):
            entries['resumed.pgm'] = entries['panel.pgm']
            with Image.open(io.BytesIO(entries['panel.pgm'])) as final:
                entries['final.png'] = encoded(final, 'PNG')
        entries['turn-window'] = f'WINDOW 100 2420 {len(during)+2} 2 {end}\n'.encode()
        return entries, plan

    def test_same_direction_after_fixed_window_still_qualifies_same_reader_not_turn(self):
        entries, plan = self.witness(during=(16,18))
        result = self.report(entries, plan=plan)
        self.assertEqual(result['sleep_turn'],'same_direction_in_recorded_samples')
        self.assertEqual(result['reader_images']['comparison']['expected_delta_degrees'], 0)
        self.assertEqual(result['reader_images']['comparison']['status'], 'expected_relative_alignment')
        self.assertEqual(result['reader_images']['software_content'], 'consistent')
        self.assertEqual(result['resident_wake_qualification'],'same_cycle_and_same_aligned_reader_body')
        self.assertEqual(result['final_application_route'],'reader')
        self.assertEqual(result['qualification'],'not_established')
        self.assertEqual(result['turn_window']['skipped'],2)

    def test_after_wake_opposite_is_separate_from_human_account_and_sleep_turn(self):
        entries, plan = self.witness(after=15,during=(16,18))
        account = {key:True for key in ('saw_screensaver','turned_while_screensaver','held_pose_through_wake','same_readable_page_after_wake')} | {'label':LABEL}
        result = self.report(entries,plan=plan,operator=account)
        self.assertEqual(result['sleep_turn'],'awake_endpoints_differ_timing_unresolved')
        self.assertEqual(result['operator_report'],account)
        self.assertEqual(result['resident_wake_qualification'],'same_cycle_and_same_aligned_reader_body')
        self.assertEqual(result['qualification'],'not_established')
        with self.assertRaisesRegex(ValueError,'label-bound'):
            self.report(entries, plan=plan, operator=account|{'label':'ffffffffffffffff'})

    def test_window_trace_can_prove_change_even_if_late_sleep_rows_missing(self):
        entries, plan = self.witness(after=15,during=(16,17),post_sleep_rows=False)
        result = self.report(entries, plan=plan)
        self.assertEqual(result['sleep_turn'],'opposite_directions_observed_while_stopped')
        self.assertEqual(len(result['pose_evidence']['stopped_change_brackets_ns']),1)
        self.assertEqual(result['qualification'],'confirmed_same_reader_and_changed_pose_in_scope')

    def test_early_wake_without_window_end_still_observes_same_cycle_and_does_not_issue_wake(self):
        entries, plan = self.witness(after=15, during=(), post_sleep_rows=False, end='early_wake')
        result = self.report(entries,plan=plan,cues=('sleep','turn'))
        self.assertEqual(result['same_resident_cycle'],'stopped_then_active_same_reader_identity_and_cycle_records')
        with self.assertRaisesRegex(ValueError,'action window'):
            self.report(entries,plan=plan)

    def test_bound_post_status_remains_real_even_when_no_post_picture_could_be_taken(self):
        entries, plan = self.witness(after=15, during=())
        del entries['post.row'], entries['resumed.pgm']
        entries['image-mapping'] = b'IMAGE pre keep 16 1\n'
        result = self.report(entries,plan=plan)
        self.assertEqual(result['sleep_turn'],'awake_endpoints_differ_timing_unresolved')
        self.assertNotEqual(result['same_resident_cycle'],'not_established')
        self.assertNotIn('post',result['reader_images'])
        self.assertEqual(result['resident_wake_qualification'],'not_established')

    def test_window_counts_and_guarded_clock_order_are_checked_without_mixing_clock_domains(self):
        entries, plan = self.witness(after=15,during=(15,))
        with self.assertRaisesRegex(ValueError,'counts'):
            self.report(entries|{'turn-window':b'WINDOW 100 2450 4 2 elapsed\n'},plan=plan)
        with self.assertRaisesRegex(ValueError,'counts'):
            self.report(entries|{'turn-window':b'WINDOW 100 500 3 2 elapsed\n'},plan=plan)
        with self.assertRaisesRegex(ValueError,'order'):
            self.report(entries|{'sleep-trace':entries['sleep-post.row']},plan=plan)


if __name__ == '__main__':
    unittest.main()
