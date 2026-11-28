"""Qualify a closed, instrumented stock-card launch; never access a device or join separate runs."""

import json
import re

from run_daily_render import CLOCK, LABEL, Refused, release_bounds


SEQUENCE = ('admit', 'stage', 'install', 'prepare', 'arm', 'observe_armed', 'tap', 'claim', 'observe',
            'witness', 'library_submit', 'stop', 'restore', 'close', 'absent')
ISOLATED_SUBMISSION = ('admit', 'stage', 'install', 'prepare', 'activate', 'arm', 'request_armed', 'tap', 'claim',
                       'witness', 'library_submit')
ISOLATED_SEQUENCE = ISOLATED_SUBMISSION + ('stop', 'restore', 'close', 'absent')
SUBMITTED = re.compile(r'WEREAD_LIBRARY_SUBMITTED ([0-9a-f]{16}) ([1-9][0-9]{0,18}) (entries|empty)\n')
REQUEST_LIFETIME_NS = 60_000_000_000
PORTRAIT_DIAGNOSTIC_PURPOSE = 'portrait-notification-diagnostic'


def qualify(report):
    """Use the live arm and collect receipts, including the original stage's tile release."""
    if not isinstance(report, dict) or not isinstance(report.get('label'), str) or not LABEL.fullmatch(report['label']):
        raise Refused('unknown_batch')
    if report.get('purpose') == PORTRAIT_DIAGNOSTIC_PURPOSE:
        raise Refused('diagnostic_only')
    if (report.get('released') is not True or report.get('owner_and_token_checked') is not True or
            report.get('recovery_required') is not False or not isinstance(report.get('released_at_utc'), str) or
            not report['released_at_utc'].strip() or 'error' in report or 'refused' in report):
        raise Refused('unconfirmed_handback')
    if report.get('primary_reason') == 'no_same_frame_completion_marker':
        raise Refused('missing_same_frame_marker')
    if report.get('measurement_build') not in ('ordinary_installation', 'temporary_measurement_build'):
        raise Refused('unknown_build_scope')
    isolated = report.get('protocol') == 'isolated_stock_launch'
    if ('protocol' in report and not isolated or isolated and (report['measurement_build'] != 'temporary_measurement_build' or
            report.get('fixture') != 'disposable_public' or 'observation' in report)):
        raise Refused('unknown_stock_protocol')
    sequence = ISOLATED_SEQUENCE if isolated else SEQUENCE
    event = checked_submission(report, sequence, isolated=isolated)
    submitted = event['submitted_ns']
    before, after = event['synthetic_release_ns']
    return {'label': event['label'], 'measurement_build': report['measurement_build'], 'clock': CLOCK,
            'start': 'synthetic_stock_library_card_release', 'end': 'library_frame_send_accepted_application_marker',
            'content': event['content'], 'synthetic_release_ns': [before, after], 'submitted_ns': submitted,
            'elapsed_interval_ns': [submitted - after, submitted - before], 'optical_completion_measured': False}


def checked_submission(report, sequence, *, isolated):
    """Check the live endpoint without treating it as a closed or timed visit."""
    steps = report.get('operations')
    if not isinstance(steps, list) or len(steps) != len(sequence):
        raise Refused('incomplete_transaction')
    for mode, step in zip(sequence, steps):
        if (not isinstance(step, dict) or step.get('mode') != mode or type(step.get('status')) is not int or
                step['status'] != 0 or step.get('reason') != ''):
            raise Refused('unexpected_operation')
    by_mode = {step['mode']: step for step in steps}
    if isolated and by_mode['arm'].get('receipt') != {'state': 'armed'}:
        raise Refused('unbound_arm')
    tap = by_mode['tap'].get('receipt')
    if not isinstance(tap, dict):
        raise Refused('missing_tile_receipt')
    try:
        before, after = release_bounds(json.dumps(tap, separators=(',', ':'), allow_nan=False).encode('ascii'))
    except (TypeError, ValueError):
        raise Refused('invalid_tile_receipt') from None
    stored = report.get('synthetic_release_ns')
    if stored != [before, after] or any(type(value) is not int for value in stored):
        raise Refused('unbound_release')
    witness = by_mode['library_submit'].get('receipt')
    if (not isinstance(witness, dict) or set(witness) != {'label', 'marker', 'tile_receipt', 'observed_ns'} or
            witness['label'] != report['label'] or not isinstance(witness['tile_receipt'], dict) or
            type(witness['observed_ns']) is not int or not 0 < witness['observed_ns'] < 2**63):
        raise Refused('unbound_library_witness')
    stage_tap = witness['tile_receipt']
    if (stage_tap.keys() != tap.keys() or
            any(type(stage_tap[key]) is not type(value) or stage_tap[key] != value for key, value in tap.items())):
        raise Refused('unbound_stage_release')
    marker = witness['marker']
    match = SUBMITTED.fullmatch(marker) if isinstance(marker, str) and len(marker) < 100 else None
    if not match or match[1] != report['label']:
        raise Refused('unbound_library_submit')
    submitted = int(match[2])
    request = by_mode['request_armed' if isolated else 'arm'].get('receipt')
    if (not isinstance(request, dict) or set(request) != {'state', 'label', 'request_clock', 'request_armed_ns', 'request_deadline_ns'} or
            request['state'] != 'armed' or request['label'] != report['label']):
        raise Refused('unbound_arm')
    armed, deadline = request.get('request_armed_ns'), request.get('request_deadline_ns')
    if (request.get('request_clock') != CLOCK or type(armed) is not int or type(deadline) is not int or
            not 0 < armed <= tap['down_before_ns'] <= before < deadline < 2**63):
        raise Refused('unbound_request_window')
    if deadline - armed > REQUEST_LIFETIME_NS or not after <= submitted < deadline or submitted > witness['observed_ns']:
        raise Refused('unresolved_endpoint_order')
    scene = report.get('observation')
    if scene is not None and (not isinstance(scene, dict) or scene.get('clock') != CLOCK or
                              scene.get('scene') not in (('grid', 'list') if match[3] == 'entries' else ('empty',))):
        raise Refused('conflicting_scene')
    return {'label': report['label'], 'content': match[3], 'synthetic_release_ns': [before, after],
            'submitted_ns': submitted, 'observed_ns': witness['observed_ns']}
