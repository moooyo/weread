from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

from qualified_submit import ISOLATED_SEQUENCE, Refused, qualify
from stock_ab_report import PURPOSE, classify, lower_upper_median, private_record, read_receipts, summarize
from stock_ab_transaction import STATES, drive, fresh_journal, initial
from test_qualified_submit import receipt


def examples():
    pattern = ['AB', 'AB', 'BA', 'AB', 'BA', 'AB', 'BA']
    schedule, visits = [], []
    for pair, order in enumerate(pattern):
        for lane in order:
            index = len(schedule)
            label = f'{index + 10:016x}'
            slot = dict(slot=index, role='warmup' if pair == 0 else 'measured', lane=lane, pair=pair,
                        label=label, visit=f'visits/{index:02d}-{lane}')
            schedule.append(slot)
            visit = receipt(scope='temporary_measurement_build')
            visit.update(**{key:slot[key] for key in ('label','lane','slot','role','pair')}, device_accessed=True,
                         purpose=PURPOSE, fixture='disposable_public', protocol='isolated_stock_launch',
                         started_at_utc=f'2026-11-22T01:{index:02d}:01+00:00', acquired_at_utc=f'2026-11-22T01:{index:02d}:02+00:00',
                         released_at_utc=f'2026-11-22T01:{index:02d}:30+00:00')
            ops = {step['mode']:step for step in visit['operations']}
            ops['request_armed'] = dict(ops['arm'], mode='request_armed')
            ops['activate'] = dict(mode='activate', status=0, reason='')
            ops['arm'] = dict(ops['arm'], receipt={'state':'armed'})
            identities = dict(launch='1:2:3:4:5:6:7:8', document='1:3:3:4:5:6:8', application='1:4:3:4:5:6:7:8')
            visit['native_guard_verified'] = True
            visit['guard_checks'] = [dict(mode=mode, status=0, reason='', receipt=dict(state='original', **identities)
                                          if mode == 'original' else {'state':'audited'}) for mode in ('original','audit_before','audit_after')]
            ops['absent']['receipt'] = dict(state='absent', **identities)
            visit['operations'] = [ops[mode] for mode in ISOLATED_SEQUENCE]
            down = 70_000_000_000 * (index + 1)
            before = down + 51_000_000
            after = before + (3 if lane == 'A' else 7)
            lower = 1_000_000_000 + (10_000 if pair == 0 else 101 - pair if lane == 'A' else 11 * pair)
            submitted = after + lower
            ops['request_armed']['receipt'].update(label=label, request_armed_ns=down-100, request_deadline_ns=down+59_000_000_000)
            tap = ops['tap']['receipt']
            tap.update(down_before_ns=down, down_after_ns=down+100, up_before_ns=before, up_after_ns=after)
            ops['library_submit']['receipt'].update(label=label, tile_receipt=tap.copy(), observed_ns=submitted+10,
                                                    marker=f'WEREAD_LIBRARY_SUBMITTED {label} {submitted} entries\n')
            visit['synthetic_release_ns'] = [before, after]
            visit['primary'] = 'qualified'
            visit['qualified'] = qualify(visit)
            visits.append(visit)
    return dict(purpose=PURPOSE, schedule=schedule), visits


class StockAbReportTests(unittest.TestCase):
    def test_all_slots_accept_actual_host_transaction_output_with_fixture_transport(self):
        batch, templates = examples()
        real_reports = []
        for slot, template in zip(batch['schedule'], templates):
            report = initial(slot | {'purpose': PURPOSE}, 'fixture-owner')
            report.update(device_accessed=True, started_at_utc=template['started_at_utc'], acquired_at_utc=template['acquired_at_utc'])
            records = {step['mode']:step['receipt'] for step in [*template['operations'], *template['guard_checks']] if 'receipt' in step}
            def line(mode):
                item = records[mode] if mode in records else {'state':STATES[mode]}
                return (json.dumps(item, separators=(',', ':')) + '\n').encode('ascii')
            transport = SimpleNamespace(call=line, request=lambda:SimpleNamespace(armed=lambda:line('request_armed'),
                                        collect=lambda:line('library_submit'), running=lambda:True, close=lambda:None))
            self.assertEqual(drive(transport, lambda:template['released_at_utc'], report, fresh_journal(slot['label']),
                                   lambda:None, lambda:None), 0)
            real_reports.append(report)
        result = summarize(batch, real_reports)
        self.assertEqual((result['status'], result['median_ns']['A_minus_B']), ('complete', [52,62]))

    def test_six_paired_intervals_in_frozen_order_with_fractional_median(self):
        batch, visits = examples()
        report = summarize(batch, visits)
        self.assertEqual(report['status'], 'complete')
        self.assertEqual([pair['order'] for pair in report['pairs']], ['AB','BA','AB','BA','AB','BA'])
        self.assertEqual(report['pair_counts'], dict(B_faster=6, B_slower=0, overlap=0, not_timed=0))
        for pair, row in enumerate(report['pairs'], 1):
            self.assertEqual(row['a_minus_b_ns'], [101-12*pair-7, 101-12*pair+3])
        self.assertEqual(report['median_ns']['A_minus_B'], [52,62])
        self.assertEqual(report['median_ns']['A'], [1_000_000_097.5,1_000_000_100.5])
        self.assertEqual(report['median_ns']['B'], [1_000_000_038.5,1_000_000_045.5])
        self.assertEqual(report['paired_median_result'], 'B_faster')
        self.assertIs(report['optical_completion_measured'], False)
        self.assertEqual(len(report['samples']), 14)

    def test_boundaries_touching_zero_and_reversed_sign(self):
        for interval, result in (([-9, -1], 'B_slower'), ([-1, 0], 'overlap'), ([0, 0], 'overlap'),
                                 ([0, 1], 'overlap'), ([1, 9], 'B_faster')):
            self.assertEqual(classify(interval), result)
        self.assertEqual(lower_upper_median([[10,11], [-4,12], [-5,-5], [0,0], [-1,10], [-10,20]]), [-2.5,10.5])

    def test_cannot_subtract_arm_medians_instead_of_paired_differences(self):
        batch, visits = examples()
        base = [100, 200, 300, 400, 500, 1000]
        candidate = [350, 50, 750, 20, 600, 400]
        for slot, visit in zip(batch['schedule'][2:], visits[2:]):
            fixed = (base if slot['lane']=='A' else candidate)[slot['pair']-1]
            submitted = visit['synthetic_release_ns'][1] + 1_000_000_000 + fixed
            marker = next(step['receipt'] for step in visit['operations'] if step['mode']=='library_submit')
            marker.update(marker=f'WEREAD_LIBRARY_SUBMITTED {visit["label"]} {submitted} entries\n',observed_ns=submitted+1)
            visit['qualified'] = qualify(visit)
        result = summarize(batch, visits)
        self.assertEqual(result['median_ns']['A_minus_B'], [18,28])
        self.assertEqual(result['pair_counts'], dict(B_faster=3, B_slower=3, overlap=0, not_timed=0))

    def test_missing_failed_or_wrong_shelf_is_never_an_improvement(self):
        for bad_index, failure in [(0,'missing'), (5,'missing'), (8,'empty'), (11,'late'), (13,'close')]:
            batch, visits = examples()
            with self.subTest(slot=bad_index, failure=failure):
                if failure == 'missing':
                    visits[bad_index] = None
                else:
                    ops = {step['mode']:step for step in visits[bad_index]['operations']}
                    if failure == 'empty':
                        row = ops['library_submit']['receipt']
                        row['marker'] = row['marker'].replace(' entries\n', ' empty\n')
                    if failure == 'late':
                        ops['library_submit']['receipt']['observed_ns'] = 1
                    if failure == 'close':
                        ops['close']['status'] = 1
                result = summarize(batch, visits)
                self.assertIsNone(result['median_ns'])
                self.assertEqual(result['paired_median_result'], 'not_timed')
                self.assertEqual(result['samples'][bad_index]['result'], 'not_timed')
                if bad_index > 1:
                    self.assertEqual(result['pairs'][(bad_index-2)//2]['result'], 'not_timed')

    def test_forged_handback_cross_clock_reorder_and_duplicate_slots_refuse(self):
        for issue in ('not_released', 'unverified_owner', 'unsafe_recovery', 'clock', 'label', 'order', 'repeated', 'overtime'):
            batch, visits = examples()
            if issue in ('not_released', 'unverified_owner', 'unsafe_recovery'):
                key = {'not_released':'released', 'unverified_owner':'owner_and_token_checked','unsafe_recovery':'recovery_required'}[issue]
                visits[5][key] = not visits[5][key]
            if issue == 'clock':
                old = deepcopy(visits[3]); old = json.loads(json.dumps(old).replace(old['label'], visits[5]['label']))
                visits[5]['operations'] = old['operations']; visits[5]['synthetic_release_ns'] = old['synthetic_release_ns']
                visits[5]['qualified'] = qualify(visits[5])
            if issue == 'label':
                visits[3]['label'] = visits[6]['label']
            if issue == 'order':
                batch['schedule'][4], batch['schedule'][5] = batch['schedule'][5], batch['schedule'][4]
            if issue == 'repeated':
                batch['schedule'][3]['label'] = batch['schedule'][2]['label']
            if issue == 'overtime':
                visits[7]['released_at_utc'] = visits[6]['released_at_utc']
            with self.subTest(issue=issue), self.assertRaises(Refused):
                summarize(batch, visits)

    def test_private_guard_objects_are_checked_but_never_exported(self):
        for failure in ('missing_after', 'unrestored', 'different_daily_file'):
            batch, visits = examples()
            visit = visits[5]
            last = visit['operations'][-1]['receipt']
            with self.subTest(failure=failure):
                if failure == 'missing_after':
                    visit['guard_checks'].pop()
                else:
                    last['application'] = '1:5:9:4:3:6:7:8'
                if failure == 'different_daily_file':
                    visit['guard_checks'][0]['receipt']['application'] = last['application']
                    with self.assertRaisesRegex(Refused, 'different_original_installation'):
                        summarize(batch, visits)
                else:
                    result = summarize(batch, visits)
                    self.assertEqual(result['samples'][5]['reason'], 'unverified_original_installation')
                    self.assertIsNone(result['median_ns'])
                    self.assertNotIn(last['application'], json.dumps(result))

    def test_only_document_omits_mtime_even_if_both_observations_agree(self):
        for key, bad in [('document','1:3:3:4:5:6:0:8'), ('launch','1:2:3:4:5:6:8'), ('application','1:4:3:4:5:6:8')]:
            with self.subTest(key=key):
                batch, visits = examples()
                for visit in visits:
                    visit['guard_checks'][0]['receipt'][key] = bad
                    visit['operations'][-1]['receipt'][key] = bad
                summary = summarize(batch, visits)
                self.assertEqual(summary['samples'][0]['reason'], 'unverified_original_installation')
                self.assertIsNone(summary['median_ns'])

    def test_no_fill_after_missing_slot_and_read_only_recovery(self):
        batch, visits = examples()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for slot, visit in zip(batch['schedule'], visits):
                folder = root / slot['visit']
                folder.mkdir(parents=True)
                if slot['slot'] == 2:
                    continue
                attempt = folder / 'attempt'
                attempt.mkdir()
                (attempt / 'operations.json').write_text(json.dumps(visit))
                if slot['slot'] == 5:
                    (attempt / 'recovery.json').write_text(json.dumps(visit))
            result = summarize(batch, read_receipts(root, batch))
            self.assertEqual(result['samples'][2]['reason'], 'no_attempt')
            self.assertEqual(result['samples'][5]['result'], 'not_timed')
            self.assertIsNone(result['median_ns'])
            token = root / batch['schedule'][3]['visit'] / 'attempt' / 'private-token'
            token.write_text('synthetic')
            with self.assertRaisesRegex(Refused, 'outstanding_stock_ab_token'):
                read_receipts(root, batch)

    def test_foreign_protocol_and_overlapping_leases_cannot_be_paired(self):
        for altered in ({'purpose':'indexed'}, {'protocol':'previous_trial'}, {'slot':True}, {'role':'warmup'},
                        {'measurement_build':'ordinary_installation'}, {'device_accessed':False},
                        {'started_at_utc':'2026-11-22T01:01:00Z','acquired_at_utc':'2026-11-22T01:01:10Z'}):
            batch, visits = examples()
            visits[2].update(altered)
            with self.subTest(altered=altered), self.assertRaises(Refused):
                summarize(batch, visits)

    def test_substitute_summary_is_not_trusted_instead_of_original_marker(self):
        batch, visits = examples()
        visits[3]['qualified']['elapsed_interval_ns'][0] -= 1
        result = summarize(batch, visits)
        self.assertEqual(result['samples'][3]['reason'], 'unbound_qualified_result')
        self.assertIsNone(result['median_ns'])

    def test_ambiguous_raw_receipts_rejected_before_loading(self):
        with tempfile.TemporaryDirectory() as temporary:
            record = Path(temporary) / 'operations.json'
            record.write_text('{"label":0,"label":1}')
            with self.assertRaisesRegex(Refused, 'duplicate_private_record_field'):
                private_record(record)


if __name__ == '__main__':
    unittest.main()
