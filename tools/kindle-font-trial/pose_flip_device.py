"""Render the single candidate launch with a live, independently clocked posture trace."""

from pathlib import Path

from assisted_shelf import MEMBERS, device_program as shelf_program
from orientation_device import section
from pose_visit import once


HERE = Path(__file__).resolve().parent
CASE = 'B-library'
SAMPLES = 401
LEAVES = ('request', 'markers', 'sensor', 'sensor-ready', 'app-binding', 'sensor-binding', 'portrait', 'portrait-source',
          'portrait-launch', 'portrait-return', 'shelf', 'final.metadata.json', 'observation')
BASE_LEAVES = tuple(leaf for leaf in LEAVES if leaf not in ('portrait-return', 'shelf', 'final.metadata.json'))


def device_program():
    body = shelf_program(fixed_down=True)
    old = section(body, '        if absent "$entry/complete";', '        for leaf in $leaves;')
    new = '''        if absent "$entry/pose-checkpoint"; then absent "$entry/complete" || fail public_output; continue; fi
        plain "$stage/$entry/pose-checkpoint" && printf 'bound pose capture\\n' | cmp -s - "$entry/pose-checkpoint" &&
            plain "$stage/$entry/observation" || fail public_output
        if absent "$entry/complete"; then
            awk 'NR!=1 || $0 !~ /^partial (incomplete_after_pose|scene_cleanup|exit|capture_metadata|native_return|final_publish|presentation_(gui_args|gui_identity|generation|owner|log_file|log_writer|log_inode|boot|record|binding))$/ {bad=1} END {exit bad || NR!=1}' "$entry/observation" || fail public_output
            leaves='%s'
        else
            plain "$stage/$entry/complete" && printf '%%s\\n' "$trial" | cmp -s - "$entry/complete" && printf 'captured startup_turn\\n' | cmp -s - "$entry/observation" || fail public_output
            leaves='%s'
        fi''' % (' '.join(BASE_LEAVES), ' '.join(LEAVES))
    body = once(body, old, new.rstrip())
    body = once(body, section(body, 'assisted_case() {', 'no_trial() {'), (HERE / 'pose-flip-case.sh').read_text(encoding='ascii').strip())
    body = once(body, 'case) assisted_case;;', 'case) flip_case;;')
    if len(body.encode('ascii')) > 32768:
        raise ValueError('Startup flip device program exceeds the established transport')
    return body
