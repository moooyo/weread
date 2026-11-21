from copy import deepcopy
import unittest

from qualified_submit import Refused, qualify
from test_run_daily_render import START


LABEL = '0123456789abcdef'
SENT = 1_000_000_010
MODES = 'admit stage install prepare arm observe_armed tap claim observe witness library_submit stop restore close absent'.split()


def receipt(content='entries', scope='ordinary_installation'):
    steps = [{'mode': mode, 'status': 0, 'reason': '', 'at': '2026-11-21T00:00:00.000+00:00'} for mode in MODES]
    rows = {step['mode']: step for step in steps}
    rows['arm']['receipt'] = dict(state='armed', label=LABEL, request_clock='CLOCK_MONOTONIC',
                                  request_armed_ns=1, request_deadline_ns=60_000_000_001)
    rows['tap']['receipt'] = START.copy()
    rows['library_submit']['receipt'] = dict(label=LABEL, tile_receipt=START.copy(), observed_ns=SENT,
                                             marker=f'WEREAD_LIBRARY_SUBMITTED {LABEL} {SENT} {content}\n')
    return {'label': LABEL, 'operations': steps, 'measurement_build': scope, 'released': True,
            'released_at_utc': '2026-11-21T00:01:00.000+00:00', 'owner_and_token_checked': True,
            'recovery_required': False, 'synthetic_release_ns': [START['up_before_ns'], START['up_after_ns']]}


class QualifiedSubmitTests(unittest.TestCase):
    def test_synthetic_release_brackets_a_same_launch_marker(self):
        for content in ('entries', 'empty'):
            for scope in ('ordinary_installation', 'temporary_measurement_build'):
                result = qualify(receipt(content, scope))
                self.assertEqual(result['content'], content)
                self.assertEqual(result['measurement_build'], scope)
                self.assertEqual(result['elapsed_interval_ns'], [SENT - START['up_after_ns'], SENT - START['up_before_ns']])
                self.assertEqual(result['clock'], 'CLOCK_MONOTONIC')
                self.assertIs(result['optical_completion_measured'], False)

    def test_release_native_close_and_claim_are_required(self):
        for key in ('released', 'owner_and_token_checked', 'released_at_utc', 'recovery_required', 'measurement_build'):
            trial = receipt()
            del trial[key]
            with self.subTest(key=key), self.assertRaises(Refused):
                qualify(trial)
        for mode in MODES:
            for change in ('failed', 'missing', 'repeated'):
                trial = receipt()
                index = MODES.index(mode)
                if change == 'failed':
                    trial['operations'][index]['status'] = 1
                elif change == 'missing':
                    del trial['operations'][index]
                else:
                    trial['operations'].insert(index, deepcopy(trial['operations'][index]))
                with self.subTest(mode=mode, change=change), self.assertRaises(Refused):
                    qualify(trial)

    def test_no_historical_or_proxy_endpoint(self):
        for marker in ('memory_only', 'first_application_frame_send', 'REQUEST 100', '', None,
                       f'WEREAD_LIBRARY_SUBMITTED {LABEL} {SENT} loading\n',
                       f'WEREAD_LIBRARY_SUBMITTED 0000000000000000 {SENT} entries\n',
                       f'WEREAD_LIBRARY_SUBMITTED {LABEL} {START["up_after_ns"] - 1} entries\n'):
            trial = receipt()
            trial['operations'][MODES.index('library_submit')]['receipt']['marker'] = marker
            trial['observation'] = {'clock': 'CLOCK_MONOTONIC', 'scene': 'grid', 'evidence': 'memory_only'}
            with self.subTest(marker=marker), self.assertRaises(Refused):
                qualify(trial)

    def test_request_must_have_been_armed_before_release_and_still_be_live(self):
        for changes in ({'request_clock': 'CLOCK_BOOTTIME'}, {'request_armed_ns': True},
                        {'request_armed_ns': START['down_before_ns'] + 1}, {'request_armed_ns': START['up_after_ns'] + 1},
                        {'request_deadline_ns': 60_000_000_002}, {'request_deadline_ns': SENT},
                        {'request_deadline_ns': SENT - 1}):
            trial = receipt()
            trial['operations'][MODES.index('arm')]['receipt'].update(changes)
            with self.subTest(changes=changes), self.assertRaises(Refused):
                qualify(trial)
        trial = receipt()
        trial['primary_reason'] = 'no_same_frame_completion_marker'
        with self.assertRaises(Refused):
            qualify(trial)

    def test_live_collector_must_witness_same_release_and_actual_device_time(self):
        for mode, updates in [('library_submit', {'tile_receipt': dict(START, up_after_ns=START['up_after_ns'] + 1)}),
                              ('library_submit', {'tile_receipt': dict(START, up_after_ns=float(START['up_after_ns']))}),
                              ('library_submit', {'label': '0'*16}), ('arm', {'label': '0'*16}),
                              ('library_submit', {'observed_ns': SENT-1}), ('library_submit', {'observed_ns': True})]:
            trial = receipt()
            trial['operations'][MODES.index(mode)]['receipt'].update(updates)
            with self.subTest(mode=mode, updates=updates), self.assertRaises(Refused):
                qualify(trial)

    def test_unrelated_release_clocks_and_scene_are_rejected(self):
        for updates in ({'clock': 'CLOCK_BOOTTIME'}, {'target': 'file-open'}, {'mode': 'keyevent'}, {'up_before_ns': 50_000_004}):
            trial = receipt()
            trial['operations'][MODES.index('tap')]['receipt'] = dict(START, **updates)
            with self.subTest(updates=updates), self.assertRaises(Refused):
                qualify(trial)
        trial = receipt('empty')
        trial['observation'] = {'clock': 'CLOCK_MONOTONIC', 'scene': 'grid'}
        with self.assertRaises(Refused):
            qualify(trial)


if __name__ == '__main__':
    unittest.main()
