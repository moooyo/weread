"""Build and run the exact/reconcile Linux comparison in the test-env Ubuntu wrapper."""

import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import tomllib


TEST = 'platform::reconcile_host::one_decision'
TAG = 'startup-orientation-'
LANES = ('published_exact', 'exact', 'reconcile')
CASES = {
    'stable_up': ('21011', '21011'), 'stable_down': ('21011', '21011'),
    'down_to_up': ('11000', '31111'), 'up_to_down': ('11000', '31111'),
    'second_changed': ('11000', '21100'), 'second_unknown': ('11000', '21100'),
    'raw_changed_mid': ('11000', '21000'), 'raw_changed_final': ('11000', '31100'),
    'raw_lost_final': ('11000', '31100'), 'raw_unreadable': ('10000', '10000'),
    'first_unknown': ('11000', '11000'), 'request_advanced': ('11000', '31110'),
    'record_changed': ('11000', '31110'), 'request_lost': ('11000', '31110'),
    'stable_advanced': ('21010', '21010'), 'wake_sequence': ('00000', '00000'),
    'no_anchor': ('00000', '00000'), 'unsupported_anchor': ('00000', '00000'),
}


def prepare(lane, stage):
    fixture = (Path(__file__).parent / 'reconcile-host.rs').read_text()
    published = lane == 'published_exact'
    fixture = fixture.replace('cfg(published_exact)', 'cfg(all())' if published else 'cfg(any())')
    fixture = fixture.replace('cfg(not(published_exact))', 'cfg(any())' if published else 'cfg(all())')
    target = stage / 'src/reconcile_host.rs'
    if not target.exists() or target.read_text() != fixture:
        target.write_text(fixture)
    platform = stage / 'src/platform.rs'
    source = platform.read_text()
    if 'mod reconcile_host;' not in source:
        platform.write_text(source + '\n#[cfg(test)]\n#[path = "reconcile_host.rs"]\nmod reconcile_host;\n')


def build(lane, work, log):
    stage = work / ('baseline' if lane == 'published_exact' else 'stage')
    prepare(lane, stage)
    features = TAG + ('reconcile' if lane == 'reconcile' else 'reuse')
    command = ['cargo', 'test', '--offline', '--locked', '--lib', '--no-run', '--no-default-features',
               '--features', features, '--message-format=json-render-diagnostics']
    with log.open('w') as output:
        deps = json.loads(subprocess.check_output(['cargo', 'metadata', '--offline', '--format-version', '1',
                            '--filter-platform', 'x86_64-unknown-linux-gnu'], cwd=stage, stderr=output))
        if any(not any(pkg['name'] == name and not pkg['source'] for pkg in deps['packages'])
               for name in ('i-slint-core', 'swash', 'zeno')):
            raise RuntimeError('Use the application Slint patches')
        def packages(lock):
            return {(row['name'], row['version'], row.get('source')) for row in tomllib.loads(lock.read_text())['package']}
        if not packages(stage / 'Cargo.lock') <= packages(work / 'src/Cargo.lock'):
            raise RuntimeError('Unexpected standalone dependency identity')
        output.write(json.dumps(command) + '\n')
        output.flush()
        run = subprocess.run(command, cwd=stage, stdout=subprocess.PIPE, stderr=output, text=True)
        events = [json.loads(line) for line in run.stdout.splitlines()]
        for event in events:
            if event['reason'] == 'compiler-message':
                output.write(event['message'].get('rendered') or '')
        run.check_returncode()
    package, = [event for event in events if event['reason'] == 'compiler-artifact' and event['target']['name'] == 'slint_backend_kindle'
                and event['profile']['test'] and event['executable']]
    executable = Path(package['executable'])
    expected = {TAG+'reuse', TAG+'reconcile'} if lane == 'reconcile' else {TAG+'reuse'}
    if set(package['features']) != expected or not executable.is_file():
        raise ValueError('Unexpected features or backend test executable')
    private = work / ('matrix-' + lane)
    shutil.copy2(executable, private)
    return private


def observe(binary, case, silent):
    lane = binary.name.removeprefix('matrix-')
    env = dict(os.environ, WEREAD_RECONCILE_CASE=case, WEREAD_RECONCILE_CHILD='silent' if silent else 'reply',
               WEREAD_STARTUP_TIMING='1')
    result = subprocess.run([str(binary), TEST, '--exact', '--ignored', '--nocapture', '--test-threads=1'], env=env,
                            check=True, capture_output=True, text=True, timeout=10)
    line, = re.findall(r'\bRECON_HOST ([^\r\n]+)', result.stdout)
    path, queries, timeouts, raw, first, confirm, current, render, touch, kept, duration = line.split()
    markers = re.findall(r'^WEREAD_STARTUP (\d+)\.(\d{9}) event_loop_orientation_(\w+)$', result.stderr, re.MULTILINE)
    if [item[2] for item in markers] != ['begin', path] or 'test result: ok. 1 passed; 0 failed' not in result.stdout:
        raise ValueError('Actual production marker disagrees with the branch')
    desired = ('reused' if case in ('stable_up', 'stable_down') else 'reconciled'
               if lane == 'reconcile' and case in ('down_to_up', 'up_to_down') else 'timeout' if silent else 'success')
    count = int(desired in ('timeout', 'success'))
    reference = CASES[case][lane == 'reconcile']
    raw_count, first_count, confirm_count, current_count = [int(item) for item in (raw, first, confirm, current)]
    if (path != desired or int(queries) != count or int(timeouts) != int(silent and count)
            or kept != ('false' if count else 'true')
            or [raw_count, first_count, confirm_count, current_count, int(not count)] != [int(digit) for digit in reference]):
        raise ValueError(f'{lane}/{case} returned {line}; expected {desired}, counts {reference}')
    times = [int(sec) * 10**9 + int(ns) for sec, ns, _ in markers]
    elapsed = int(duration)
    if not 0 < times[1] - times[0] <= elapsed or (desired == 'timeout' and elapsed < 500_000_000):
        raise ValueError('Missing or invalid monotonic duration')
    return dict(arm=lane, case=case, outcome=path, legacy_queries=count, timeouts=int(timeouts),
                observations=[raw_count, first_count, confirm_count, current_count], render_offset=int(render),
                touch_offset=int(touch), returned_observer=(kept == 'true'), elapsed_ns=elapsed, marker_ns=times[1]-times[0])


def main():
    files = Path('/work')
    if not (files / 'baseline/src/platform.rs').is_file() or not (files / 'stage/src/platform.rs').is_file():
        raise ValueError('Supply private baseline and candidate backend sources in /work')
    output = files / 'evidence'
    output.mkdir(exist_ok=True)
    os.environ.update(CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_TARGET_DIR='/work/target',
                      CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    binaries = {lane: build(lane, files, output / (lane+'.log')) for lane in LANES}
    matrix = [observe(binaries[lane], case, False) for lane in LANES for case in CASES]
    for case in CASES:
        old, current = [next(row for row in matrix if row['case'] == case and row['arm'] == lane)
                        for lane in ('published_exact', 'exact')]
        if any(old[key] != current[key] for key in old if key not in ('arm','marker_ns','elapsed_ns')):
            raise ValueError('The exact lane diverged from the published baseline')
    cases = ('down_to_up','up_to_down','up_to_down','down_to_up','down_to_up','up_to_down')
    def timed(case, reverse, silent, control):
        lanes = ('reconcile', control) if reverse else (control, 'reconcile')
        runs = [observe(binaries[lane], case, silent) for lane in lanes]
        elapsed = {row['arm']:row['elapsed_ns'] for row in runs}
        return dict(input=case, order='BA' if reverse else 'AB', child='silent' if silent else 'responsive',
                    runs=runs, a_minus_b_ns=elapsed[control]-elapsed['reconcile'])
    warmup = timed('up_to_down', True, True, 'published_exact')
    groups = {}
    for name, inputs, silent, control in (
        ('published_silent', cases, True, 'published_exact'), ('matched_silent', cases, True, 'exact'),
        ('matched_responsive', cases, False, 'exact'),
        ('stable', ('stable_up','stable_down','stable_down','stable_up'), False, 'exact'),
        ('confirmation_refused', ('second_unknown','request_advanced','request_advanced','second_unknown'), False, 'exact'),
    ):
        pairs = [timed(case, index % 2 != 0, silent, control) for index,case in enumerate(inputs)]
        groups[name] = dict(control=control, pairs=pairs, median_paired_saving_ns=statistics.median([pair['a_minus_b_ns'] for pair in pairs]))
    result = {'scope':'Linux event-loop orientation decision only; synthetic metadata, actual child polling and retirement',
              'production_control':'074a4d2 exact reuse', 'feature_arms':['startup-orientation-reuse','startup-orientation-reconcile'],
              'matrix':matrix, 'warmup_excluded':warmup, 'groups':groups}
    (output/'measurements.json').write_text(json.dumps(result, indent=2)+'\n')
    print(f'{len(matrix)} branch observations.')
    for name,group in groups.items():
        print(f"{name}: {len(group['pairs'])} pairs, {group['median_paired_saving_ns']/1e6:.4f} ms median paired saving")


if __name__ == '__main__':
    main()
