import json
import os
import time

import test_stock_ab_device as stock
from test_daily_render_device import GUI, LABEL, MASTER, NONCE


class StockAppProofTests(stock.StockDeviceFixture):
    def launched(self):
        original = self.ready()
        child, arm = self.open_channel()
        tap = self.call('tap')
        self.assertEqual(self.enter().returncode, 0)
        self.chain()
        (self.lock / 'application-owned').unlink()
        return original, child, arm, tap

    def evidence(self):
        self.call('claim', 'claimed')
        self.call('witness', 'witnessed')

    def test_missing_supervisor_ack_still_needs_original_app_submission(self):
        original, child, arm, tap = self.launched()
        self.evidence()
        self.assertFalse((self.lock / 'application-owned').exists())
        self.assertIn('WEREAD_LIBRARY_REQUEST', (self.fixed / 'request').read_text())
        with (self.fixed / 'request').open('r+b') as descriptor:
            descriptor.truncate(0)
            sent = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
            marker = f'WEREAD_LIBRARY_SUBMITTED {LABEL} {sent} entries\n'
            os.pwrite(descriptor.fileno(), marker.encode(), 0)
        output, error = child.communicate(b'collect\n', timeout=7)
        self.assertEqual((child.returncode, error), (0, b''))
        result = json.loads(output)
        self.assertEqual((result['marker'], result['tile_receipt']), (marker, tap))
        self.assertLess(sent, arm['request_deadline_ns'])
        self.handback(original)
        self.assertEqual(self.base.signals(), [f'-TERM {MASTER}'])

    def test_missing_state_open_is_close_only(self):
        original, child, _, _ = self.launched()
        (self.lock / 'state-open').unlink()
        self.call('claim', refused='launch_identity')
        self.assertFalse((self.stage / 'session').exists())
        self.assertEqual(child.communicate(b'cancel\n', timeout=6), (b'', b''))
        self.call('stop', 'stopped')
        self.assertFalse((self.stage / 'session').exists())
        self.assertTrue((self.stage / 'stock-stop-session').is_file())
        self.handback(original)
        self.assertEqual(self.base.signals(), [f'-TERM {MASTER}'])

    def test_wrong_partial_symlink_or_hardlink_ack_rejects_even_after_claim(self):
        original, child, _, _ = self.launched()
        self.evidence()
        path = self.lock / 'application-owned'
        value = f'1:{NONCE}:{GUI}:{(self.lock / "application-birth").read_text().strip()}\n'
        for wrong in (b'', b'incomplete', value.replace('1:', '2:', 1).encode(), (value+'extra\n').encode()):
            path.write_bytes(wrong); path.chmod(0o600)
            self.call('witness', refused='scene_identity'); path.unlink()
        outside = self.root / 'unrelated-ack'; outside.write_text(value); outside.chmod(0o600)
        for link in (lambda: path.symlink_to(outside), lambda: os.link(outside, path)):
            link(); self.call('witness', refused='scene_identity'); path.unlink()
        self.assertEqual(child.communicate(b'cancel\n', timeout=6), (b'', b''))
        self.handback(original)

    def test_second_gui_changed_seed_or_replaced_binary_cannot_claim(self):
        original, child, _, _ = self.launched()
        app = self.stage / 'arm/app/weread'
        second = self.proc / '991234'
        second.mkdir(); (second/'exe').symlink_to(app)
        self.call('claim', refused='launch_identity'); (second/'exe').unlink(); second.rmdir()
        for artifact in (app, self.stage/'arm/data/library.sqlite'):
            backup = artifact.with_name(artifact.name+'.original')
            artifact.rename(backup); artifact.write_bytes(backup.read_bytes()); artifact.chmod(backup.stat().st_mode & 0o777)
            self.call('claim', refused='launch_identity'); artifact.unlink(); backup.rename(artifact)
        (self.lock/'generation').write_text('2:'+NONCE+'\n')
        self.call('claim', refused='launch_identity')
        (self.lock/'generation').write_text('1:'+NONCE+'\n')
        self.evidence()
        self.assertEqual(child.communicate(b'cancel\n', timeout=6), (b'', b''))
        self.handback(original)

    def test_observed_supervisor_ack_cannot_later_disappear(self):
        original, child, _, _ = self.launched()
        birth=(self.lock/'application-birth').read_text().strip()
        ack=self.lock/'application-owned'; proper=f'1:{NONCE}:{GUI}:{birth}\n'
        ack.write_text(proper); ack.chmod(0o600); self.evidence(); ack.unlink()
        self.call('witness', refused='scene_identity')
        self.assertEqual(child.communicate(b'cancel\n', timeout=6), (b'', b''))
        self.call('stop', refused='unproven_close'); self.assertFalse(self.base.signals())
        ack.write_text(proper); ack.chmod(0o600); self.handback(original)

    def test_changed_request_identity_or_incomplete_close_refuses_term(self):
        original, child, _, _ = self.launched()
        leaf=self.stage/'request-file'; prior=leaf.read_bytes(); leaf.write_text('123:456\n')
        self.call('claim', refused='launch_identity'); leaf.write_bytes(prior)
        self.assertEqual(child.communicate(b'cancel\n', timeout=6), (b'', b''))
        latest=self.lock/'application-birth'; prior=latest.read_bytes(); latest.write_text('777\n')
        self.call('stop', refused='unproven_close')
        self.assertFalse(self.base.signals()); latest.write_bytes(prior)
        self.handback(original)
