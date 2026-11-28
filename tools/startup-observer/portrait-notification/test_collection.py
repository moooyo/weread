"""Exercise the diagnostic visit across the real host stream and an isolated device fixture."""

import argparse
import fcntl
import io
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
FROZEN_DEVICE = None
sys.path[:0] = [str(HERE), str(HERE.parent)]
import diagnostic_visit as diagnostic
from qualified_submit import PORTRAIT_DIAGNOSTIC_PURPOSE, qualify
from run_daily_render import Refused
import run_stock_ab_visit as runner
import test_stock_ab_device as stock
from test_daily_render_device import LABEL, MASTER, RELEASE
from test_stock_ab_cli import LocalLeases
from test_stock_ab_visit import CONFIG, line


READ = runner.PORTRAIT_READ
TRACE = 'portrait-notification.diag'


def trace(*rows):
    return ''.join(f'WEREAD_PORTRAIT_DIAG {row}\n' for row in rows).encode('ascii')


class CollectionTests(stock.StockDeviceFixture):
    def setUp(self):
        super().setUp()
        base = self.program.read_bytes()
        # Stall before the reader finishes checking the stage. All writes
        # are fixtures; the detached reader must not hold any stage resource.
        guard = b'entry_guard() {\n'
        self.assertEqual(base.count(guard), 1)
        self.base_program = base.replace(guard, guard + b'''    if [ "$mode" = portrait_diagnostic ] && [ -e "$fixture/delay-read" ]; then
        : >"$fixture/read-started"
        sleep 10
        : >"$fixture/read-finished"
    fi
''')
        # A same-size replacement after the fd is open must not be joined
        # to the launch, even when both files have the same valid records.
        anchor = b'\nstock_request_serve() {'
        self.assertEqual(self.base_program.count(anchor), 1)
        self.base_program = self.base_program.replace(anchor, b'''
dd() {
    if [ "$mode:$1" = portrait_diagnostic:bs=8193 ] && [ -e "$fixture/swap-trace" ]; then
        cp -p "$lock/portrait-notification.diag" "$lock/replacement" &&
            mv "$lock/replacement" "$lock/portrait-notification.diag" || return 1
    fi
    command dd "$@"
}
stock_request_serve() {''', 1)
        self.plan = self.root/'visit'
        self.plan.mkdir(mode=0o700)
        (self.plan/'device.sh').write_bytes(diagnostic.device_program(self.base_program))
        (self.plan/'inputs.tgz').write_bytes(self.payload)
        self.attempt = self.plan/'attempt'
        self.receipt = self.attempt/'portrait-notification.json'
        self.token = self.attempt/'private-token'
        self.lease = LocalLeases(self.root/'lease')
        self.lockfile = self.root/'operation'/'device.lock'
        self.lockfile.parent.mkdir(mode=0o700)
        self.lockfile.parent.chmod(0o700)
        self.lockfile.touch(mode=0o600)
        self.lockfile.chmod(0o600)
        locks = runpy.run_path(str(HERE.parents[1]/'kindle-physical/device_lock.py'))
        locks['operation_lock'].__wrapped__.__globals__['_LOCK_PATH'] = self.lockfile
        self.args = SimpleNamespace(work=self.root, plan=self.plan, owner='fixture', lease_tool=self.program,
                                    preflight=False, recover=None)
        self.config = CONFIG | {'label': LABEL, 'lane':'A', 'purpose':PORTRAIT_DIAGNOSTIC_PURPOSE}
        self.calls = []
        self.trace_kind = 'valid'
        self.corrupt_gui = self.fail_anchor = self.fail_write = self.detached_reader = False
        self.content = 'entries'
        self.stdout = io.StringIO()
        shared = {'ssh_command':lambda *_:[*self.device_shell, '-c'], 'load_lease':lambda _:self.lease, 'LEASE':self.lease.lock}
        self.enterContext(patch.object(runner.runpy, 'run_path', side_effect=lambda path:
                                locks if path == str(runner.OPERATION_LOCK_MODULE) else shared))
        self.enterContext(patch.object(runner, 'WORKER_RUNNER', self.program))
        self.enterContext(patch.object(runner.signal, 'signal'))
        self.enterContext(patch('sys.stdout', self.stdout))
        raw_call, raw_read, checkpoint = runner.Transport.call, runner.Transport.read_portrait_diagnostic, runner.checkpoint

        def call(transport, mode):
            self.assert_lock(held=True)
            self.calls.append(mode)
            if mode == 'tap':
                now = time.monotonic_ns()
                receipt = json.loads(RELEASE) | dict(down_before_ns=now, down_after_ns=now+1,
                                                    up_before_ns=now+50_000_001, up_after_ns=now+50_000_002)
                (self.root/'receipt').write_bytes(line(receipt))
                time.sleep(0.055)
            result = raw_call(transport, mode)
            if mode == 'tap':
                self.assertEqual(self.enter().returncode, 0)
                self.chain()
                self.write_trace()
            if mode == 'witness':
                if self.fail_anchor:
                    (self.stage/'portrait-diagnostic-submit').mkdir()
                (self.fixed/'request').write_text(f'WEREAD_LIBRARY_SUBMITTED {LABEL} {time.monotonic_ns()} {self.content}\n')
            return result

        def read(transport, limit):
            self.assert_lock(held=True)
            self.assertTrue(self.lease.held)
            self.assertFalse(self.fixed.exists())
            self.assertFalse((self.stage/'stopped').exists())
            original = self.lock/'application-birth'
            birth = original.read_bytes()
            if self.corrupt_gui:
                original.write_text('777\n')
            self.calls.append(READ)
            ssh = transport.ssh
            if self.detached_reader:
                # Simulate a remote watchdog that is not in the SSH client's
                # process group, as on a physical SSH connection.
                transport.ssh = ['setsid', '--fork', '--wait', *ssh]
            try:
                return raw_read(transport, limit)
            finally:
                transport.ssh = ssh
                if self.corrupt_gui:
                    original.write_bytes(birth)

        def save(path, value):
            if path == self.receipt:
                self.assert_lock(held=False)
                if self.fail_write:
                    raise OSError('fixture diagnostic storage failure')
            checkpoint(path, value)

        self.enterContext(patch.object(runner.Transport, 'call', new=call))
        self.enterContext(patch.object(runner.Transport, 'read_portrait_diagnostic', new=read))
        self.enterContext(patch.object(runner, 'checkpoint', new=save))

    def assert_lock(self, *, held):
        with self.lockfile.open('rb') as guard:
            if held:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            else:
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def write_trace(self):
        if self.trace_kind == 'missing':
            return
        now = time.monotonic_ns() - 10 if self.trace_kind != 'stale' else 10
        rows = [f'publish_begin {now}', f'publish_end {now+1} ok']
        if self.trace_kind == 'duplicate':
            rows.append(rows[0])
        data = trace(*rows)
        if self.trace_kind == 'foreign':
            data += b'unrelated private output\n'
        path = self.lock/TRACE
        path.write_bytes(data)
        path.chmod(0o600)

    def run_visit(self):
        def verify(work, plan, *, recovery):
            self.assertEqual((work, plan, recovery), (self.root, self.plan, bool(self.args.recover)))
            return self.config, self.base_program

        private_stdin = SimpleNamespace(isatty=lambda:False, buffer=io.BytesIO(b'fixture-only\n'))
        with patch('sys.stdin', private_stdin):
            return diagnostic.execute(self.args, verify)

    def diagnostics(self):
        result = json.loads(self.receipt.read_bytes())
        self.assertEqual((result['purpose'], result['label']), (PORTRAIT_DIAGNOSTIC_PURPOSE, LABEL))
        self.assertFalse(result['startup_timing_eligible'])
        self.assertEqual(self.receipt.stat().st_mode & 0o777, 0o600)
        return result

    def returned(self):
        report = json.loads((self.attempt/'operations.json').read_bytes())
        self.assertTrue(report['released']); self.assertTrue(report['native_guard_verified'])
        self.assertFalse(report['recovery_required']); self.assertFalse(self.lease.held)
        self.assertEqual(report['primary'], 'not_timed'); self.assertNotIn('qualified', report)
        with self.assertRaisesRegex(Refused, 'diagnostic_only'):
            qualify(report)
        self.assertNotIn('startup_trace', report); self.assertNotIn('portrait_diagnostic', report)
        self.assertFalse(self.stage.exists()); self.assertFalse(self.fixed.exists()); self.assertFalse(self.authority.exists())
        self.assertFalse(self.token.exists()); self.assert_lock(held=False)
        self.assertEqual(self.calls[-5:], ['stop','restore','close','absent','audit_after'])
        self.assertEqual(self.base.signals(), [f'-TERM {MASTER}'])
        return report

    def test_exact_snapshot_after_library_submit_and_before_safe_return(self):
        self.assertEqual(self.run_visit(), 0)
        report = self.returned()
        self.assertEqual(self.calls[-7:-5], ['witness',READ])
        self.assertEqual(self.calls.count(READ), 1)
        result = self.diagnostics()
        self.assertEqual((result['status'],result['released']), ('collected',True))
        submit = next(row['receipt'] for row in report['operations'] if row['mode']=='library_submit')
        self.assertEqual(result['submission']['submitted_ns'], int(submit['marker'].split()[2]))
        self.assertEqual(result['submission']['observed_ns'], submit['observed_ns'])
        self.assertEqual(result['analysis']['publication']['result'], 'ok')
        self.assertEqual(len(result['records']), 2)
        self.assertIsNone(result['analysis']['missing_wait_slots'])

    def unavailable(self, reason):
        self.assertEqual(self.run_visit(), 0)
        self.returned()
        receipt = self.diagnostics()
        self.assertEqual((receipt['status'],receipt['reason']), ('unavailable',reason))
        self.assertTrue(receipt['released']); self.assertNotIn('analysis', receipt)
        self.assertNotIn('unrelated private output', str(receipt))
        self.assertEqual(self.calls.count(READ), 1)

    def test_missing_trace_returns_without_evidence(self):
        self.trace_kind = 'missing'
        self.unavailable('diagnostic_read_refused')

    def test_foreign_line_is_rejected_by_the_device(self):
        self.trace_kind = 'foreign'
        self.unavailable('diagnostic_read_refused')

    def test_semantically_invalid_trace_is_rejected_by_the_host(self):
        self.trace_kind = 'duplicate'
        self.unavailable('invalid_diagnostic_snapshot')

    def test_trace_from_before_this_launch_cannot_be_relabelled(self):
        self.trace_kind = 'stale'
        self.unavailable('invalid_diagnostic_snapshot')

    def test_changed_gui_cannot_export_snapshot(self):
        self.corrupt_gui = True
        self.unavailable('diagnostic_read_refused')

    def test_same_size_trace_replacement_after_open_is_rejected(self):
        (self.root/'swap-trace').touch()
        self.unavailable('diagnostic_read_refused')

    def test_missing_submission_anchor_does_not_change_send_or_prevent_close(self):
        self.fail_anchor = True
        self.unavailable('diagnostic_read_refused')

    def test_read_timeout_does_not_hold_the_stage_lock_or_block_close(self):
        (self.root/'delay-read').touch()
        self.detached_reader = True
        with patch.object(runner, 'PORTRAIT_HOST_SECONDS', 2), patch.object(runner, 'PORTRAIT_DEVICE_SECONDS', 6):
            self.unavailable('diagnostic_read_timeout')
        self.assertTrue((self.root/'read-started').exists())
        self.assertFalse((self.root/'read-finished').exists())

    def test_unqualified_submission_does_not_collect(self):
        self.content = 'empty'
        self.assertEqual(self.run_visit(), 1)
        report = self.returned()
        self.assertEqual(report['refused'], 'unexpected_public_fixture_content')
        self.assertEqual(self.diagnostics()['status'], 'not_attempted')
        self.assertNotIn(READ, self.calls)

    def test_diagnostic_storage_failure_is_after_handback(self):
        self.fail_write = True
        self.assertEqual(self.run_visit(), 0)
        self.returned()
        self.assertFalse(self.receipt.exists())
        self.assertEqual(json.loads(self.stdout.getvalue())['portrait_diagnostic'], 'receipt_write_failed')

    def test_recovery_only_uses_original_token_and_never_resnapshots(self):
        gate = self.root/'incomplete-restore'
        gate.touch()
        self.assertEqual(self.run_visit(), 2)
        original = self.receipt.read_bytes()
        self.assertEqual(self.diagnostics()['status'], 'collected')
        self.assertFalse(self.diagnostics()['released']); self.assertTrue(self.token.exists())
        self.assert_lock(held=False)
        self.args.recover = self.attempt
        self.calls.clear(); gate.unlink()
        self.assertEqual(self.run_visit(), 0)
        self.assertEqual(self.calls, ['stop','restore','close','absent','audit_after'])
        self.assertEqual(self.receipt.read_bytes(), original)
        self.assertFalse(self.token.exists()); self.assertFalse(self.lease.held)
        self.assertFalse(self.stage.exists()); self.assert_lock(held=False)

    def test_ordinary_or_changed_program_cannot_use_diagnostic_entry(self):
        diagnostic_config = self.config
        self.config = CONFIG
        with self.assertRaisesRegex(Refused, 'invalid_diagnostic_scope'):
            self.run_visit()
        self.config = diagnostic_config
        (self.plan/'device.sh').write_bytes(self.base_program)
        with self.assertRaisesRegex(Refused, 'invalid_diagnostic_device_program'):
            self.run_visit()
        self.assertEqual(self.calls, []); self.assertFalse(self.attempt.exists())
        self.assertEqual(self.lease.requested, 0)


class WireTests(unittest.TestCase):
    def test_retired_v6_shape_accepts_adapter_without_executing_a_visit(self):
        if FROZEN_DEVICE is None:
            self.skipTest('Pass the retained v6 device source to this test script')
        base = diagnostic.source(FROZEN_DEVICE).encode('ascii')
        program = diagnostic.device_program(base)
        self.assertIn(b'trace) exit 64;;', program)
        self.assertNotIn(b'portrait-diagnostic-submit', base)
        shell = ['busybox', 'sh'] if os.getenv('WEREAD_TEST_BUSYBOX') else ['sh']
        subprocess.run([*shell, '-n'], input=program, check=True, timeout=5)

    def test_foreign_label_or_submit_identity_never_parses(self):
        submission = dict(label=LABEL, submitted_ns=400, observed_ns=410, synthetic_release_ns=[100, 110])
        body = trace('publish_begin 300', 'publish_end 350 ok')
        self.assertEqual(diagnostic.snapshot(f'WEREAD_PORTRAIT_SNAPSHOT {LABEL} 400 410\n'.encode() + body,
                                             submission)['status'], 'collected')
        for label, sent, observed in [('f'*16,400,410),(LABEL,401,410),(LABEL,400,411)]:
            with self.subTest(label=label, sent=sent, observed=observed), self.assertRaises(ValueError):
                diagnostic.snapshot(f'WEREAD_PORTRAIT_SNAPSHOT {label} {sent} {observed}\n'.encode() + body, submission)

    def test_transport_bounds_untrusted_output_and_discards_failed_partial_reply(self):
        for program, reason in [(b'head -c 100000 /dev/zero', 'diagnostic_output_limit'),
                                (b'printf "WEREAD_PORTRAIT_SNAPSHOT partial\\n"; exit 1', 'diagnostic_read_refused')]:
            with self.subTest(reason=reason), self.assertRaisesRegex(Refused, reason):
                transport = runner.Transport(['sh','-c'], program, LABEL, None, b'', os.environ, lambda:None)
                transport.read_portrait_diagnostic(diagnostic.WIRE_LIMIT)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('frozen_device', type=Path, help='Retired v6 device program, used only for source and syntax checks')
    FROZEN_DEVICE = parser.parse_args().frozen_device
    if (sys.platform != 'linux' or os.geteuid() != 0 or
            {row.split(':',1)[0].strip() for row in Path('/proc/net/dev').read_text('ascii').splitlines()[2:]} != {'lo'} or
            Path('/proc/1/comm').read_text().strip() in ('init', 'systemd')):
        parser.error('Use a disposable, networkless Linux PID/mount namespace with a private /tmp')
    suite = unittest.main(argv=[sys.argv[0], '-v'], exit=False)
    raise SystemExit(not suite.result.wasSuccessful() or bool(suite.result.skipped))
