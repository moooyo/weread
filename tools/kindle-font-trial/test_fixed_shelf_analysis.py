import json
import unittest

import assisted_shelf
import test_assisted_host
from analyze_assisted_shelf import BASE, CASE, OPERATOR_FIELDS
from test_orientation import records


class FixedShelfAnalysisTests(unittest.TestCase):
    def setUp(self):
        self.visit = test_assisted_host.AssistedAnalysisTests()
        self.visit.setUp()
        self.addCleanup(self.visit.doCleanups)
        self.visit.plan = assisted_shelf.DOWN_CONTRACT | {'label': self.visit.plan['label']}
        self.visit.receipt.update(purpose=self.visit.plan['purpose'], initial_native_reference='D:1')
        self.operator = self.visit.operator
        self.visit.entries.update(portrait=b'D:1\n', sensor=b''.join(
            f'{index} {index * 100} {index * 100 + 10} {16 if index % 2 else 18}\n'.encode() for index in range(1, 11)),
            shelf=b'SHELF 30150000000 30160000000 1 grid\n')
        self.visit.entries.update({'portrait-launch': b'D:1\n', 'portrait-return': b'D:1\n'})

    def test_fixed_visit_only_confirms_its_own_scope(self):
        report = self.visit.analyze(operator=self.operator)
        self.assertEqual((report['purpose'], report['qualification_scope'], report['expected_target']),
                         ('assisted-fixed-down-shelf-touch', 'fixed_native_down_single_profile_tap', 'settings'))
        self.assertTrue(all(report[key] == 'D:1' for key in (
            'expected_calibration', 'initial_native_reference', 'native_before', 'native_launch', 'native_return')))
        self.assertIn('startup and touch in the opposite portrait', report['not_measured'])
        self.assertEqual(report['prelaunch_snapshot_raw'], [16, 18] * 5)
        self.assertEqual(report['digital_shelf']['status'], 'observed_populated_shelf')
        self.assertEqual(report['orientation_queries'], {'splash': 'reused', 'event_loop': 'reused'})
        self.assertEqual(report['single_profile_tap'], 'operator_and_route_confirmed')
        self.assertEqual(report['candidate_pose_qualification'], 'confirmed_in_scope')

    def test_machine_route_never_supplies_a_missing_human_observation(self):
        report = self.visit.analyze()
        self.assertIsNone(report['operator_report'])
        self.assertEqual(report['single_profile_tap'], 'settings_observed_operator_unconfirmed')
        self.assertEqual(report['candidate_pose_qualification'], 'not_established')
        for field in OPERATOR_FIELDS:
            with self.subTest(field=field):
                negative = self.operator | {field: False}
                report = self.visit.analyze(operator=negative)
                self.assertEqual(report['operator_report'], negative)
                self.assertEqual(report['single_profile_tap'], 'operator_observations_do_not_confirm')
                self.assertEqual(report['candidate_pose_qualification'], 'not_established')
        for invalid in (self.operator | {'label': 'f' * 16}, self.operator | {'saw_profile_page': 1},
                        {key: value for key, value in self.operator.items() if key != 'saw_profile_page'}):
            with self.assertRaisesRegex(ValueError, 'bound'):
                self.visit.analyze(operator=invalid)

    def test_mixed_pressure_or_foreign_launch_anchor_is_not_a_down_visit(self):
        for replacement in ({'sensor': self.visit.entries['sensor'].replace(b' 16\n', b' 15\n', 1)},
                            {'portrait': b'U:3\n'}, {'portrait-launch': b'D:3\n'}):
            with self.subTest(replacement=replacement), self.assertRaisesRegex(ValueError, 'target portrait'):
                self.visit.analyze(self.visit.entries | replacement, self.operator)
        self.visit.receipt['initial_native_reference'] = 'U:3'
        with self.assertRaisesRegex(ValueError, 'pre-instruction'):
            self.visit.analyze(operator=self.operator)

    def test_digital_shelf_return_and_route_remain_independent_of_the_human(self):
        for raw, status in ((b'not_observed\n', 'not_observed'), (b'SHELF 30150000000 30160000000 3 grid\n', 'insufficient_clock_or_rotation'),
                            (b'SHELF 30150000000 30160000000 1 empty\n', 'public_entries_not_observed')):
            report = self.visit.analyze(self.visit.entries | {'shelf': raw}, self.operator)
            self.assertEqual(report['digital_shelf']['status'], status)
            self.assertEqual(report['candidate_pose_qualification'], 'not_established')
        for changed, status in (({'portrait-return': b'U:3\n'}, 'return_pose_different'),
                                ({'final.metadata.json': json.dumps(self.visit.metadata | {'route': 'search'}).encode()},
                                 'expected_final_route_not_observed')):
            report = self.visit.analyze(self.visit.entries | changed, self.operator)
            self.assertEqual(report['single_profile_tap'], status)
            self.assertEqual(report['candidate_pose_qualification'], 'not_established')

    def test_candidate_fallback_is_reported_not_credited_as_reuse(self):
        _, markers = records(CASE, 30, fallback=True)
        report = self.visit.analyze(self.visit.entries | {'markers': markers.encode(), 'shelf':
                                    b'SHELF 31000000000 31010000000 1 grid\n'}, self.operator)
        self.assertEqual(report['digital_shelf']['status'], 'observed_populated_shelf')
        self.assertEqual(report['orientation_queries'], {'splash': 'timeout', 'event_loop': 'reused'})
        self.assertFalse(report['both_orientation_sites_reused'])
        self.assertEqual(report['single_profile_tap'], 'operator_and_route_confirmed')
        self.assertEqual(report['candidate_pose_qualification'], 'not_established')

    def test_false_pose_gate_or_wrong_plan_never_credits_the_candidate(self):
        blocked = {key: self.visit.entries[key] for key in BASE} | {'observation': b'not_started pose_gate\n', 'portrait': b'U:1\n'}
        report = self.visit.analyze(blocked, self.operator)
        self.assertEqual(report['application'], 'not_started_pose_gate')
        self.assertEqual(report['native_before'], 'U:1')
        self.assertNotIn('orientation_queries', report)
        self.assertEqual(report['candidate_pose_qualification'], 'not_established')
        self.visit.plan['expected_calibration'] = 'U:3'
        with self.assertRaises(ValueError):
            self.visit.analyze(operator=self.operator)


if __name__ == '__main__':
    unittest.main()
