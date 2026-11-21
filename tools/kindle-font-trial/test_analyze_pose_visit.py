import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

from PIL import Image, ImageDraw

from analyze_pose_visit import SEQUENCE, analyze_visit, splash_evidence, wake_evidence
from analyze_orientation import orientation_sample
from pose_visit import CONTRACT
from test_orientation import encoded, records


class VisitAnalysisTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        image = Image.new('L', (1860, 2480), 255)
        ImageDraw.Draw(image).rectangle((130, 450, 1220, 1360), fill=25)
        cls.panel = encoded(image, 'PPM')
        cls.inverted = encoded(image.transpose(Image.Transpose.ROTATE_180), 'PPM')
        request, trace = records('B-reader', 30)
        trace = trace.replace('WEREAD_STARTUP 30.050000000 window_created',
                              'WEREAD_STARTUP 30.049000000 startup_feedback_finished\nWEREAD_STARTUP 30.050000000 window_created')
        cls.sample = orientation_sample(request, trace)
        cls.fields = {'request': request.encode(), 'markers': trace.encode(), 'portrait-source': b'native_idle_calibration\n',
                      'portrait': b'U:3\n', 'portrait-launch': b'U:3\n', 'portrait-return': b'U:3\n',
                      'splash-status': b'0\n', 'splash': b'SPLASH 30042000000 30046000000 3 2 210\n',
                      'wake': b'observed screenSaver stopped same_process wake_ready active cover unobserved\n',
                      'final.metadata.json': json.dumps(dict(route='reader', profile='scribe', backend='kindle',
                                                window_size=image.size, snapshot_size=image.size)).encode(),
                      'panel.pgm': cls.panel, 'resumed.pgm': cls.panel, 'final.png': encoded(image, 'PNG')}

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.archive = Path(temporary.name) / 'results.tgz'
        self.plan = CONTRACT | dict(label='0123456789abcdef')
        self.receipt = dict(purpose=self.plan['purpose'], label=self.plan['label'], scope='transaction_only',
                            passed=True, owner_token_checked_on_release=True, acquired='2026-11-21T02:10:01+00:00',
                            released='2026-11-21T02:11:12+00:00', operations=[dict(mode=mode, case=case, status=0)
                                                                            for mode, case in SEQUENCE])

    def archive_with(self, fields=None, label=None, extra=()):
        fields = self.fields if fields is None else fields
        lines = [('out/orientation-batch', ((label or self.plan['label']) + '\n').encode())]
        lines.extend(('out/B-reader/' + name, data) for name, data in fields.items())
        with tarfile.open(self.archive, 'w:gz', compresslevel=1) as archive:
            for name, data in [*lines, *extra]:
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_strictly_bound_splash_and_same_digital_reader_after_native_cycle(self):
        self.archive_with()
        result = analyze_visit(self.plan, self.receipt, self.archive)
        self.assertEqual(result['splash']['status'], 'matches_expected_0_degrees')
        self.assertEqual(result['wake']['status'], 'observed_screensaver_wake')
        self.assertEqual(result['reader_comparison']['whole_different_pixels'], 0)
        self.assertEqual(result['reader_resumed']['status'], 'matches_expected_0_degrees')
        self.assertEqual(result['native_pose']['relative_to_historical_down_1'], 'opposite_native_reference')
        self.assertIn('physical touch mapping', result['not_measured'])

    def test_opposite_splash_and_reversed_wake_frame_are_not_normalized(self):
        self.archive_with(self.fields | {'portrait-return': b'D:1\n', 'resumed.pgm': self.inverted, 'splash-status': b'20\n',
                                         'splash': b'SPLASH 30042000000 30046000000 3 210 2\n'})
        result = analyze_visit(self.plan, self.receipt, self.archive)
        self.assertEqual(result['splash']['status'], 'opposite_180_degrees')
        self.assertEqual(result['reader_before']['status'], 'matches_expected_0_degrees')
        self.assertEqual(result['reader_resumed']['status'], 'opposite_180_degrees')
        self.assertNotIn('reader_comparison', result)
        self.assertEqual(result['native_pose']['return'], 'D:1')

    def test_bad_clock_rotation_status_or_weak_score_cannot_qualify_splash(self):
        for raw, status in [(b'SPLASH 30040000000 30045000000 3 0 220\n', b'0\n'),
                            (b'SPLASH 30042000000 30049000000 3 0 220\n', b'0\n'),
                            (b'SPLASH 30042000000 30045000000 1 0 220\n', b'0\n'),
                            (b'SPLASH 30042000000 30045000000 3 2 3\n', b'0\n'),
                            (b'SPLASH 30042000000 30045000000 3 210 0\n', b'0\n')]:
            self.assertEqual(splash_evidence(raw, status, self.sample, 'U:3')['status'], 'insufficient')

    def test_skipped_wake_keeps_reader_and_does_not_consume_resumed_picture(self):
        self.archive_with(self.fields | {'wake': b'not_run alarm_unavailable\n', 'splash-status': b'not_observed\n', 'splash': b'not_observed\n'})
        result = analyze_visit(self.plan, self.receipt, self.archive)
        self.assertEqual(result['wake'], {'status': 'not_run', 'reason': 'alarm_unavailable'})
        self.assertEqual(result['reader_resumed']['status'], 'not_assessed')
        self.assertEqual(result['reader_before']['status'], 'matches_expected_0_degrees')
        self.assertEqual(result['splash']['status'], 'not_observed')
        self.assertEqual(wake_evidence(b'observed steady\n')['status'], 'insufficient')

    def test_failed_or_unreleased_operations_do_not_use_the_archive(self):
        with self.assertRaises(ValueError):
            analyze_visit(self.plan, self.receipt | {'owner_token_checked_on_release': False}, self.archive)
        self.assertEqual(analyze_visit(self.plan, self.receipt | {'passed': False}, self.archive)['collection'], 'not_qualified')
        self.assertEqual(analyze_visit(self.plan, self.receipt | {'operations': self.receipt['operations'][:-1]}, self.archive)['wake']['status'], 'not_assessed')

    def test_cross_visit_or_extra_records_are_rejected(self):
        self.archive_with({}, label='ffffffffffffffff')
        with self.assertRaisesRegex(ValueError, 'label'):
            analyze_visit(self.plan, self.receipt, self.archive)
        self.archive_with({}, extra=[('out/B-reader/unknown', b'data')])
        with self.assertRaisesRegex(ValueError, 'member'):
            analyze_visit(self.plan, self.receipt, self.archive)


if __name__ == '__main__':
    unittest.main()
