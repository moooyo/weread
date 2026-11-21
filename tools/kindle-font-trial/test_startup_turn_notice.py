from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import time
import unittest
from unittest.mock import patch

import run
import test_pose_flip_host as fixture


LABEL = '0123456789abcdef'
CHALLENGE = '1234567890abcdef' * 2
ACK = f'STARTUP_ACK {LABEL} {CHALLENGE}\n'.encode('ascii')
CANCEL = f'STARTUP_CANCEL {LABEL} {CHALLENGE}\n'.encode('ascii')


class StartupReplyTests(unittest.TestCase):
    def pipe(self):
        read_fd, write_fd = os.pipe()
        reader = os.fdopen(read_fd, 'r', encoding='utf-8')
        writer = os.fdopen(write_fd, 'wb', buffering=0)
        self.addCleanup(reader.close)
        self.addCleanup(writer.close)
        return reader, writer

    def reply(self, data, *, elapsed=6, close=True):
        reader, writer = self.pipe()
        writer.write(data)
        if close:
            writer.close()
        return run.read_startup_reply(reader.fileno(), LABEL, CHALLENGE, time.monotonic() - elapsed)

    def test_exact_single_line_and_end_of_stream(self):
        self.assertIs(self.reply(ACK), True)
        with patch.object(run, 'STARTUP_NOTICE_SECONDS', 0.02), patch.object(run, 'STARTUP_READING_SECONDS', 0):
            with self.assertRaisesRegex(run.StartupNoticeRefused, 'timed_out'):
                self.reply(ACK, elapsed=0, close=False)

    def test_early_or_late_confirmation_cannot_start(self):
        for elapsed, status in [(0, 'too_early'), (26, 'timed_out')]:
            with self.subTest(elapsed=elapsed), self.assertRaisesRegex(run.StartupNoticeRefused, status):
                self.reply(ACK, elapsed=elapsed)

    def test_cancel_aborts_even_if_peer_keeps_input_open(self):
        with self.assertRaisesRegex(run.StartupNoticeRefused, 'canceled'):
            self.reply(CANCEL, close=False)

    def test_partial_or_missing_reply_and_disconnected_bridge(self):
        for data in (b'', ACK[:-1], b'STARTUP_'):
            with self.subTest(size=len(data)), self.assertRaisesRegex(run.StartupNoticeRefused, 'disconnected'):
                self.reply(data)

    def test_bare_wrong_identity_or_malformed_replies(self):
        for data in (b'ack\n', b'OK\n', ACK.replace(LABEL.encode(), b'f' * 16), ACK.replace(CHALLENGE.encode(), b'f' * 32),
                     ACK + b'\n', ACK + ACK, ACK + b'trailing bytes', ACK.replace(b'\n', b'\r\n'), b'X' * 512):
            with self.subTest(data=data[:12]), self.assertRaisesRegex(run.StartupNoticeRefused, 'invalid_reply'):
                self.reply(data)

    def test_queued_data_after_password_is_not_hidden_by_text_buffer(self):
        for data in (ACK, b'ack\n'):
            with self.subTest(data=data[:8]):
                reader, writer = self.pipe()
                writer.write(b'fixture-pass\r\n' + data)
                self.assertEqual(run.read_startup_password(reader), 'fixture-pass')
                with patch.object(run.sys, 'stdin', reader), self.assertRaisesRegex(run.StartupNoticeRefused, 'unexpected_input'):
                    run.startup_turn_ack(LABEL, on_issued=lambda: self.fail('queued reply issued a cue'))

    def test_eof_before_challenge_does_not_issue(self):
        reader, writer = self.pipe()
        writer.close()
        with patch.object(run.sys, 'stdin', reader), self.assertRaisesRegex(run.StartupNoticeRefused, 'unexpected_input'):
            run.startup_turn_ack(LABEL, on_issued=lambda: self.fail('disconnected peer issued a cue'))

    def test_new_challenge_is_sent_only_after_issue_receipt(self):
        reader, writer = self.pipe()
        events = []

        def deliver(line, *, flush):
            self.assertTrue(flush)
            self.assertEqual(events, ['persisted'])
            self.assertRegex(line, rf'^STARTUP_CUE {LABEL} {CHALLENGE} [0-9]{{13}}$')
            events.append('delivered')
            writer.write(ACK)
            writer.close()

        with patch.object(run.sys, 'stdin', reader), patch.object(run, 'STARTUP_READING_SECONDS', 0), \
             patch.object(run.secrets, 'token_hex', return_value=CHALLENGE) as generate, patch('builtins.print', side_effect=deliver):
            self.assertIs(run.startup_turn_ack(LABEL, on_issued=lambda: events.append('persisted')), True)
        generate.assert_called_once_with(16)
        self.assertEqual(events, ['persisted', 'delivered'])

    def test_raw_password_requires_terminator_within_limit(self):
        for data in (b'incomplete', b'p' * 4096 + b'\n'):
            reader, writer = self.pipe()
            writer.write(data)
            writer.close()
            with self.assertRaisesRegex(ValueError, 'Incomplete private authentication'):
                run.read_startup_password(reader)


class StartupNoticeTransactionTests(unittest.TestCase):
    def setUp(self):
        self.visit = fixture.FlipRunnerTests()
        self.visit.setUp()
        self.addCleanup(self.visit.doCleanups)

    def issue(self, label, *, on_issued):
        self.assertEqual(label, LABEL)
        self.assertEqual(self.visit.calls, ['admit', 'reference', 'stage'])
        on_issued()
        report = self.receipt()
        self.assertEqual(report['operator_notice'], 'issued')
        self.assertNotIn('operator_cue_acknowledged', report)

    def execute(self, response):
        visit = self.visit
        regular = Path.is_file

        def notice(label, *, on_issued):
            self.issue(label, on_issued=on_issued)
            if isinstance(response, Exception):
                raise response
            return response

        with patch.object(run, 'LEASE', visit.work/'lock'), patch.object(run.sys, 'stdin', io.StringIO('fixture\n')), \
             patch.object(Path, 'is_file', lambda path: str(path) == '/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
             patch.dict(run.UTIL, ssh_command=lambda *_: ['fixture-ssh'], load_lease=lambda *_: visit.lease), \
             patch.object(run.signal, 'signal'), patch.object(run.subprocess, 'run', side_effect=visit.command), \
             patch.object(run, 'startup_turn_ack', side_effect=notice) as exchange, redirect_stdout(visit.output):
            result = run.run(visit.args, startup_flip_schedule=lambda config: config['schedule'])
        return result, exchange.call_count

    def receipt(self):
        attempt, = self.visit.plan.glob('batch-*')
        return json.loads((attempt/'operations.json').read_text())

    def test_ack_is_between_stage_and_case(self):
        self.assertEqual(self.execute(True), (0, 1))
        report = self.receipt()
        self.assertEqual(report['operator_notice'], 'acknowledged')
        self.assertLessEqual(report['operations'][2]['at'], report['operator_cue_issued'])
        self.assertLessEqual(report['operator_cue_issued'], report['operator_cue_acknowledged'])
        self.assertLessEqual(report['operator_cue_acknowledged'], report['operations'][3]['at'])
        self.assertTrue(report['owner_token_checked_on_release'])

    def test_cancel_timeout_and_false_never_run_case_and_release(self):
        for answer, status in [(run.StartupNoticeRefused('canceled'), 'canceled'),
                               (run.StartupNoticeRefused('timed_out'), 'timed_out'), (False, 'not_confirmed')]:
            with self.subTest(status=status):
                self.assertEqual(self.execute(answer), (1, 1))
                self.assertEqual(self.visit.calls, ['admit', 'reference', 'stage', 'restore', 'collect', 'close', 'absent', 'analyze'])
                report = self.receipt()
                self.assertEqual(report['operator_notice'], status)
                self.assertNotIn('operator_cue_acknowledged', report)
                self.assertTrue(report['owner_token_checked_on_release'])
                self.visit.calls.clear()
                attempt, = self.visit.plan.glob('batch-*')
                attempt.rename(self.visit.plan/('fixture-' + status))

    def test_failed_admit_reference_or_stage_never_issues(self):
        for mode in ('admit', 'reference', 'stage'):
            with self.subTest(mode=mode):
                self.visit.bad = mode
                self.assertEqual(self.execute(True), (1, 0))
                self.assertNotIn('case', self.visit.calls)
                self.assertNotIn('operator_cue_issued', self.receipt())
                self.visit.calls.clear()
                attempt, = self.visit.plan.glob('batch-*')
                attempt.rename(self.visit.plan/('fixture-' + mode))

    def test_recovery_never_requests_confirmation(self):
        self.visit.bad = 'absent'
        self.assertEqual(self.execute(run.StartupNoticeRefused('disconnected')), (1, 1))
        self.visit.lease.release.assert_not_called()
        self.visit.args.recover, = self.visit.plan.glob('batch-*')
        self.visit.calls.clear()
        self.visit.bad = None
        self.assertEqual(self.execute(True), (0, 0))
        self.assertEqual(self.visit.calls, ['restore', 'close', 'absent'])
        self.visit.lease.release.assert_called_once()


if __name__ == '__main__':
    unittest.main()
