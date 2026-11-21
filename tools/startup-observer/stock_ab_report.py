"""Summarize bounded stock Library release-to-SEND intervals from a fixed, closed A/B schedule."""

import argparse
from datetime import datetime
from fractions import Fraction
import json
from pathlib import Path
import re

from qualified_submit import qualify
from run_daily_render import CLOCK, LABEL, Refused
from stock_ab_transaction import CLAIM_PROTOCOL, valid_stat


PURPOSE = 'ordinary-stock-library-submit-ab'
MATRIX = [(0, 'warmup', lane) for lane in 'AB'] + [
    (pair, 'measured', lane) for pair in range(1, 7) for lane in ('AB' if pair % 2 else 'BA')]


def checked_slots(batch):
    if not isinstance(batch, dict) or batch.get('purpose') != PURPOSE or batch.get('claim_protocol') != CLAIM_PROTOCOL:
        raise Refused('unknown_stock_ab_plan')
    slots = batch.get('schedule')
    if not isinstance(slots, list) or len(slots) != len(MATRIX):
        raise Refused('invalid_stock_ab_schedule')
    labels = set()
    for index, (slot, (pair, role, lane)) in enumerate(zip(slots, MATRIX)):
        if (not isinstance(slot, dict) or type(slot.get('slot')) is not int or slot['slot'] != index or
                type(slot.get('pair')) is not int or slot['pair'] != pair or slot.get('lane') != lane or
                slot.get('role') != role or slot.get('visit') != f'visits/{index:02d}-{lane}' or
                not isinstance(slot.get('label'), str) or not LABEL.fullmatch(slot['label']) or slot['label'] in labels):
            raise Refused('invalid_stock_ab_slot')
        labels.add(slot['label'])
    return slots


def lower_upper_median(values):
    def central(sequence):
        ordered = sorted(sequence)
        half = len(ordered) // 2
        return Fraction(ordered[half]) if len(ordered) % 2 else Fraction(ordered[half-1] + ordered[half], 2)

    def json_number(value):
        if value.denominator == 1:
            return value.numerator
        if abs(value) >= 2**52:
            raise Refused('elapsed_median_out_of_range')
        return float(value)

    return [json_number(central([bounds[side] for bounds in values])) for side in (0, 1)]


def classify(bounds):
    if bounds[0] > 0:
        return 'B_faster'
    return 'B_slower' if bounds[1] < 0 else 'overlap'


def time_value(text):
    try:
        stamp = datetime.fromisoformat(text)
    except (ValueError, TypeError):
        raise Refused('invalid_handback_time') from None
    if stamp.utcoffset() is None:
        raise Refused('invalid_handback_time')
    return stamp


def guarded_identities(receipt):
    checks = receipt.get('guard_checks')
    if (receipt.get('native_guard_verified') is not True or not isinstance(checks, list) or len(checks) != 3 or
            not all(isinstance(row, dict) and type(row.get('status')) is int and row['status'] == 0 and row.get('reason') == ''
                    for row in checks) or [row.get('mode') for row in checks] != ['original','audit_before','audit_after']):
        return None
    original = checks[0].get('receipt')
    end = receipt['operations'][-1].get('receipt')
    fields = ('launch','document','application')
    if (not isinstance(original, dict) or original.keys() != {'state', *fields} or original['state'] != 'original' or
            not all(valid_stat(key, original[key]) for key in fields) or
            not isinstance(end, dict) or end.keys() != original.keys() or
            end['state'] != 'absent' or any(end[key] != original[key] or type(end[key]) is not type(original[key]) for key in fields) or
            any(check.get('receipt') != {'state':'audited'} for check in checks[1:])):
        return None
    return tuple(original[key] for key in fields)


def summarize(batch, receipts):
    slots = checked_slots(batch)
    if not isinstance(receipts, list) or len(receipts) != len(slots):
        raise Refused('unbound_slot_receipts')
    samples = []
    previous_end = None
    previous_handback = None
    identity = None
    for slot, receipt in zip(slots, receipts):
        sample = {key: slot[key] for key in ('slot', 'label', 'pair', 'lane', 'role')}
        sample['result'] = 'not_timed'
        samples.append(sample)
        if receipt is None:
            sample['reason'] = 'no_attempt'
            continue
        if not isinstance(receipt, dict) or any(type(receipt.get(key)) is not type(slot[key]) or receipt[key] != slot[key]
                                                for key in ('label','lane','slot','pair','role')):
            raise Refused('foreign_or_reordered_attempt')
        if (receipt.get('protocol') != 'isolated_stock_launch' or receipt.get('claim_protocol') != batch['claim_protocol'] or
                receipt.get('fixture') != 'disposable_public' or
                receipt.get('measurement_build') != 'temporary_measurement_build' or receipt.get('purpose') != PURPOSE):
            raise Refused('unknown_stock_ab_scope')
        if (receipt.get('device_accessed') is not True or receipt.get('released') is not True or receipt.get('owner_and_token_checked') is not True or
                receipt.get('recovery_required') is not False):
            raise Refused('unclosed_stock_ab_attempt')
        start, acquired, handback = (time_value(receipt.get(key)) for key in ('started_at_utc','acquired_at_utc','released_at_utc'))
        if not start <= acquired < handback or (previous_handback and acquired < previous_handback):
            raise Refused('out_of_order_handback')
        previous_handback = handback
        try:
            event = qualify(receipt)
        except (Refused, KeyError, TypeError, ValueError):
            sample['reason'] = 'no_qualified_same_launch_submission'
            continue
        if event['content'] != 'entries':
            sample['reason'] = 'unexpected_shelf'
            continue
        if (receipt.get('primary') != 'qualified' or receipt.get('qualified') != event or receipt.get('qualification_reason')):
            sample['reason'] = 'unbound_qualified_result'
            continue
        observed_identity = guarded_identities(receipt)
        if observed_identity is None:
            sample['reason'] = 'unverified_original_installation'
            continue
        if identity is not None and observed_identity != identity:
            raise Refused('different_original_installation')
        identity = observed_identity
        if previous_end is not None and event['synthetic_release_ns'][0] <= previous_end:
            raise Refused('overlapping_or_reset_device_clock')
        previous_end = event['submitted_ns']
        sample.update(result='timed', content=event['content'], release_ns=event['synthetic_release_ns'],
                      submitted_ns=event['submitted_ns'], elapsed_ns=event['elapsed_interval_ns'])

    pairs = []
    counts = dict(B_faster=0, B_slower=0, overlap=0, not_timed=0)
    order_counts = dict(first_faster=0, second_faster=0, overlap=0, not_timed=0)
    for pair in range(1, 7):
        group = [sample for sample in samples if sample['pair'] == pair]
        lanes = {sample['lane']:sample for sample in group}
        row = {'pair': pair, 'order': ''.join(sample['lane'] for sample in group)}
        if any(sample['result'] != 'timed' for sample in group):
            row['result'] = 'not_timed'
        else:
            base, candidate = lanes['A']['elapsed_ns'], lanes['B']['elapsed_ns']
            saving = [base[0] - candidate[1], base[1] - candidate[0]]
            row.update(a_ns=base, b_ns=candidate, a_minus_b_ns=saving, result=classify(saving))
        counts[row['result']] += 1
        position = row['result']
        if position in ('B_faster', 'B_slower'):
            faster_arm = 'B' if position == 'B_faster' else 'A'
            position = 'first_faster' if row['order'][0] == faster_arm else 'second_faster'
        order_counts[position] += 1
        pairs.append(row)
    complete = all(sample['result'] == 'timed' for sample in samples)
    result = {'purpose': PURPOSE, 'status': 'complete' if complete else 'incomplete', 'clock': CLOCK,
              'measurement_build': 'temporary_measurement_build', 'start': 'synthetic_stock_library_card_release',
              'end': 'library_frame_send_accepted_application_marker', 'optical_completion_measured': False,
              'all_covers_required': False, 'cross_visit_same_boot_verified': False,
              'samples': samples, 'pairs': pairs, 'pair_counts': counts, 'order_counts': order_counts,
              'paired_median_result':'not_timed', 'median_ns': None}
    if complete:
        medians = {target:lower_upper_median([row[key] for row in pairs])
                   for key,target in [('a_ns','A'),('b_ns','B'),('a_minus_b_ns','A_minus_B')]}
        result.update(median_ns=medians, paired_median_result=classify(medians['A_minus_B']))
    return result


def private_record(path):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 128 * 1024:
        raise Refused('unusable_private_record')

    def unique(pairs):
        result = dict(pairs)
        if len(result) != len(pairs):
            raise Refused('duplicate_private_record_field')
        return result

    parsed = json.loads(path.read_bytes(), object_pairs_hook=unique)
    if not isinstance(parsed, dict):
        raise Refused('unusable_private_record')
    return parsed


def read_receipts(directory, batch):
    receipts = []
    for slot in checked_slots(batch):
        visit = directory / slot['visit']
        attempt = visit / 'attempt'
        if visit.resolve(strict=True) != visit or attempt.resolve(strict=False) != attempt:
            raise Refused('ambiguous_stock_ab_visit')
        if not attempt.exists():
            receipts.append(None)
            continue
        if (attempt / 'private-token').exists() or (attempt / 'private-token').is_symlink():
            raise Refused('outstanding_stock_ab_token')
        initial = private_record(attempt / 'operations.json')
        if initial.get('label') != slot['label']:
            raise Refused('foreign_or_reordered_attempt')
        recovery = attempt / 'recovery.json'
        if recovery.exists() or recovery.is_symlink():
            restored = private_record(recovery)
            if (restored.get('label') != slot['label'] or restored.get('released') is not True or restored.get('owner_and_token_checked') is not True or
                    restored.get('recovery_required') is not False):
                raise Refused('unconfirmed_stock_ab_recovery')
            initial = dict(initial, released=True, owner_and_token_checked=True, recovery_required=False,
                           released_at_utc=restored['released_at_utc'], refused='close_only_recovery')
        elif initial.get('device_accessed') is False and not initial.get('acquired_at_utc') and initial.get('recovery_required') is False:
            if initial.get('operations') or initial.get('guard_checks'):
                raise Refused('ambiguous_access_history')
            receipts.append(None)
            continue
        receipts.append(initial)
    return receipts


def verified_report(work):
    from stock_ab_freeze import verify_batch

    work = work.resolve(strict=True)
    batch = verify_batch(work)
    root = work / 'stock-library-ab'
    if root.resolve(strict=True) != root:
        raise Refused('ambiguous_stock_ab_root')
    result = summarize(batch, read_receipts(root, batch))
    source = batch.get('source_commit')
    if (not isinstance(source, str) or not re.fullmatch('[0-9a-f]{40}', source) or batch.get('device') != 'Kindle Scribe 2022'
            or batch.get('firmware') != '5.19.5' or batch.get('clock') != CLOCK or batch.get('entry') != 'ordinary_stock_weread_tile'
            or batch.get('fixture') != 'public_normal_15' or batch.get('selector_strategy') != 'single-file-bind-v1'
            or batch.get('claim_protocol') != CLAIM_PROTOCOL):
        raise Refused('unverified_stock_ab_source_or_scope')
    result.update(device='Kindle Scribe 2022', firmware='5.19.5', entry='ordinary_stock_weread_tile',
                  fixture='public_normal_15', selector_strategy=batch['selector_strategy'], claim_protocol=batch['claim_protocol'],
                  frozen_inputs_verified=True, shared_source_base_commit=source,
                  shared_application_features=['bundled-font-preflight','kindle','trial-library-frame'],
                  only_candidate_feature='startup-orientation-reuse', reconciliation_enabled=False)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    public = verified_report(args.work)
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(public, output, indent=2, allow_nan=False)
        output.write('\n')
    print(f"Stock Library A/B: {public['status']}; paired median: {public['paired_median_result']}")
