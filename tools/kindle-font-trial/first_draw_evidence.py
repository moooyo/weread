"""Read the software draw boundary from a bound startup-flip GUI log, not the physical panel."""

import re

from analyze import TRACE
from pose_window import HOLDING, MAX_NS


DRAW_FAMILY = 'first_application_frame_draw_begin'
DRAW_STAGE = re.compile(DRAW_FAMILY + r'_rot(0|90|180|270)')
SEND = 'first_application_frame_send'
MAX_DRAW_BRACKET_NS = 100_000_000


def split_draw_records(text):
    ordinary, draws = [], []
    for line in text.splitlines(keepends=True):
        if line.startswith('WEREAD_STARTUP ') and DRAW_FAMILY in line:
            draws.append((len(ordinary), line))
        else:
            ordinary.append(line)
    return ''.join(ordinary), draws


def sample_pair(rows, index):
    return HOLDING[rows[index][3]], rows[index][4]


def draw_mapping(rows, edge, draw, angle, initial, native, launch):
    unverified = {'status':'not_qualified'}
    if not isinstance(launch,str) or not re.fullmatch(r'[UD]:[0-3]', launch):
        return unverified | {'reason':'no_current_portrait_scanout_anchor'}
    if launch not in ('U:3','D:1') or not initial == native == launch:
        return unverified | {'reason':'unverified_or_changed_native_launch_reference'}
    first_holding, first_scan = sample_pair(rows, 0)
    anchor_holding = 'up' if launch.startswith('U:') else 'down'
    anchor_scan = int(launch[-1])
    if (first_holding, first_scan) != (anchor_holding, anchor_scan):
        return unverified | {'reason':'initial_sample_disagrees_with_launch_anchor'}
    before_scan = sample_pair(rows, edge-1)[1]
    if (before_scan-anchor_scan) % 4 not in (0,2):
        return unverified | {'reason':'no_old_portrait_scanout_mapping'}
    enclosing = next(((left,right) for left,right in zip(rows,rows[1:]) if left[2] < draw < right[1]),None)
    if enclosing is None:
        return unverified | {'reason':'draw_not_strictly_between_sample_intervals'}
    left, right = enclosing
    if right[2] - left[1] > MAX_DRAW_BRACKET_NS:
        return unverified | {'reason':'draw_sample_bracket_over_100ms'}
    if not (edge < left[0] and (HOLDING[left[3]],left[4]) == (HOLDING[right[3]],right[4])):
        return unverified | {'reason':'draw_not_bracketed_by_equal_new_holding_and_scanout'}
    after_holding, after_scan = HOLDING[left[3]], left[4]
    if (after_scan-anchor_scan) % 4 not in (0,2):
        return unverified | {'reason':'no_new_portrait_scanout_mapping'}
    old_angle = 180 * ((before_scan-anchor_scan) % 4 == 2)
    new_angle = 180 * ((after_holding != anchor_holding) != ((after_scan-anchor_scan) % 4 == 2))
    details = {'source':'launch_calibration_and_adjacent_independent_pressure_getvar_pairs',
               'draw_bracket_sample_numbers':[left[0],right[0]], 'draw_bracket_outer_ms':(right[2]-left[1])/1e6,
               'sampled_holding':after_holding, 'sampled_scanout_raw':after_scan,
               'prior_rotation_degrees':old_angle, 'expected_new_rotation_degrees':new_angle}
    if angle not in (0,180):
        return details | {'status':'not_qualified','reason':'renderer_angle_outside_tested_portraits'}
    if old_angle == new_angle:
        return details | {'status':'not_qualified','reason':'prior_and_new_mappings_have_same_angle'}
    return details | {'status':'matches_new_mapping' if new_angle==angle else 'matches_prior_not_new_mapping'}


def assess_draw(*, draws, ordinary, times, rows, request, initial, native, launch):
    report = {'status':'insufficient', 'turn_relative_to_draw':'not_assessed',
              'orientation_correctness':'not_verified', 'pixels':'not_observed_for_this_draw'}
    if len(draws) != 1:
        return report | {'reason':'draw_marker_not_recorded' if not draws else 'multiple_draw_markers'}
    index, line = draws[0]
    match = TRACE.fullmatch(line[:-1]) if line.endswith('\n') else None
    angle = DRAW_STAGE.fullmatch(match[3]) if match else None
    if not angle:
        return report | {'reason':'invalid_draw_marker'}
    draw = int(match[1]) * 10**9 + int(match[2])
    main, splash, predraw, sent = (times.get(stage) for stage in ('main_enter','startup_feedback_ready','portrait_opened',SEND))
    ordinary_lines = ordinary.splitlines()
    if (not all((main,splash,predraw,sent)) or not 0 < request <= main < splash < predraw < draw < sent <= MAX_NS
            or index >= len(ordinary_lines) or not ordinary_lines[index].endswith(' ' + SEND)):
        return report | {'reason':'unpaired_or_unordered_draw_and_submit'}
    report.update(status='observed', source='same_gui_software_renderer', application_rotation_guard_degrees=0, clock='CLOCK_MONOTONIC',
                  renderer_rotation_degrees=int(angle[1]), draw_begin_ns=draw, submitted_log_ns=sent,
                  draw_after_splash_ready_ms=(draw-splash)/1e6, draw_to_submit_log_ms=(sent-draw)/1e6)
    if not rows[0][2] < request or not sent < rows[-1][1]:
        return report | {'turn_relative_to_draw':'insufficient_sensor_coverage'}
    edges = [pos for pos in range(1,len(rows)) if HOLDING[rows[pos-1][3]] != HOLDING[rows[pos][3]]]
    if len(edges) != 1:
        return report | {'turn_relative_to_draw':'no_single_observed_change'}
    edge = edges[0]
    earlier, later = rows[edge-1], rows[edge]
    earliest, latest, before, after = earlier[1], later[2], HOLDING[earlier[3]], HOLDING[later[3]]
    relation = 'before_draw' if latest < draw else 'after_draw' if earliest > draw else 'overlaps_draw_boundary'
    in_window = ('after_splash_before_draw' if splash < earliest and latest < draw else
                 'before_splash' if latest < splash else 'after_draw' if earliest > draw else 'boundary_uncertain')
    report.update(turn_relative_to_draw=relation, turn_in_splash_to_draw_window=in_window,
                  change_after_draw_begin_ms=[(earliest-draw)/1e6, (latest-draw)/1e6])
    if relation in ('before_draw','after_draw'):
        report['holding_from_observed_change'] = after if relation == 'before_draw' else before
    if in_window == 'after_splash_before_draw':
        report['software_mapping'] = draw_mapping(rows,edge,draw,int(angle[1]),initial,native,launch)
    return report
