import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import run_pose_probe
from pose_probe import render_device
from pose_turn_cue import MARKER, ROW, cue_program, first_group, streaming_probe
from test_pose_probe import LABEL


FIRST = b'read 1 100 101 16 1 D 200 201 16 1'
NOTE = MARKER + LABEL.encode() + b' ' + FIRST
RESULT = b'pose_display_trace ' + LABEL.encode() + b'\n' + FIRST + b'\ntrace_end budget\nprotected_state unchanged\n'
PREFIX = 'import os,sys,time\nsys.stdin.buffer.read()\n'


class CueTests(unittest.TestCase):
    def stream(self, source, events, timeout=2):
        return streaming_probe([sys.executable, '-c', PREFIX + source], b'public fixture', os.environ,
                               LABEL, events.append, timeout=timeout)

    def test_only_exact_first_measured_down_and_no_inference_from_equal_directions(self):
        self.assertEqual(first_group(NOTE, LABEL), (FIRST, 'TURN_NOW'))
        for changed in [NOTE.replace(b' D ', b' R '), NOTE.replace(b'16 1', b'18 1'), NOTE.replace(b' D ', b' unknown '),
                        NOTE.replace(b'16 1', b'15 3'), NOTE.replace(b' D ', b' U '), NOTE.replace(b'16 1', b'16 0')]:
            self.assertEqual(first_group(changed, LABEL)[1], 'HOLD_STILL')
        for invalid in [NOTE.replace(LABEL.encode(), b'0' * 16), NOTE.replace(b'read 1 ', b'read 2 '),
                        NOTE.replace(b'100 101', b'102 101')]:
            with self.assertRaises(ValueError):
                first_group(invalid, LABEL)

    def test_real_partial_pipe_line_yields_live_action_before_completion(self):
        source = f'os.write(2, {NOTE[:15]!r}); time.sleep(.02); os.write(2, {NOTE[15:]!r} + b"\\n"); time.sleep(.15); os.write(1, {RESULT!r})'
        events = []
        result, receipt = self.stream(source, events)
        self.assertEqual((events, result.returncode, result.stdout, result.stderr), (['TURN_NOW'], 0, RESULT, b''))
        self.assertEqual(receipt, {'first_group_wire': FIRST.decode(), 'action': 'TURN_NOW'})

    def test_old_buffered_or_final_output_never_issues_turn_retroactively(self):
        wire = f'os.write(2, {NOTE!r} + b"\\n"); time.sleep(.05)'
        events = []
        with patch('pose_turn_cue.MAX_CUE_AGE', 0.02):
            _, late = self.stream('time.sleep(.08); ' + wire + f'; os.write(1, {RESULT!r})', events)
        _, finished = self.stream(f'os.write(1, {RESULT!r}); time.sleep(.05); ' + wire, events)
        self.assertEqual((late['action'], finished['action'], events), ('not_issued', 'not_issued', []))

    def test_missing_wrong_duplicate_and_non_matching_first_group_refuse(self):
        def put(descriptor, data):
            return f'os.write({descriptor}, {data!r}); '
        for stderr, stdout in [(b'', RESULT), (NOTE.replace(LABEL.encode(), b'0' * 16) + b'\n', RESULT),
                               ((NOTE + b'\n') * 2, RESULT), (NOTE + b'\n', RESULT.replace(FIRST, FIRST.replace(b' D ', b' R ')))]:
            with self.assertRaises(ValueError):
                self.stream(put(2, stderr) + put(1, stdout), [])
        result, receipt = self.stream(put(2, b'refused protected_drift\n') + 'sys.exit(1)', [])
        self.assertEqual((result.returncode, result.stderr, receipt), (1, b'refused protected_drift\n', None))

    def test_timeout_reaps_its_own_local_process(self):
        with tempfile.TemporaryDirectory() as root:
            name = Path(root) / 'pid'
            with self.assertRaises(subprocess.TimeoutExpired):
                self.stream(f'from pathlib import Path; Path({str(name)!r}).write_text(str(os.getpid())); time.sleep(4)', [], .3)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(name.read_text()), 0)

    def test_cued_shell_changes_only_one_live_progress_line_preserving_capture_and_cleanup(self):
        plain = render_device(26848, display_trace=True)
        cued = cue_program(plain)
        self.assertEqual(cued[:cued.index(ROW)], plain[:plain.index(ROW)])
        self.assertEqual(cued[cued.index('        sequence=$((sequence + 1))'):], plain[plain.index('        sequence=$((sequence + 1))'):])
        for other in (cued, render_device(26032), plain.replace(ROW, '')):
            with self.assertRaises(ValueError):
                cue_program(other)
        source = cued[cued.index('display_rows() {'):cued.index('\ncase "$mode" in')]
        source += f'''\nlabel={LABEL}
trace_clock() {{ echo 4; }}
trace_sample() {{ echo '100 101 16 1'; }}
trace_winmgr() {{ echo D; }}
trace_pause() {{ :; }}
raw=$(display_rows) && printf '%s\\n' "$raw"
'''
        result = subprocess.run(['sh', '-ec', source], capture_output=True, timeout=3, check=True)
        self.assertEqual(len(result.stderr.splitlines()), 1)
        self.assertEqual(result.stderr.splitlines()[0], MARKER + LABEL.encode() + b' ' + result.stdout.splitlines()[0])

    def test_no_new_readiness_flag_no_lease(self):
        class Arguments:
            display_trace = True
            turn_cue = True
            recover = None
            preflight = False
            operator_ready = False
        with patch.object(Path, 'is_file', return_value=True), self.assertRaisesRegex(ValueError, 'present operator'):
            run_pose_probe.main(Arguments())


if __name__ == '__main__':
    unittest.main()
