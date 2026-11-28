"""Freeze the ordinary-card transaction for one exact arm, payload and public seed."""

from pathlib import Path
import re

from prepare_daily_render import LIBRARY, REQUIRED


HERE = Path(__file__).resolve().parent
MIB = 1024 * 1024
ARM = {'arm/launch.sh', 'arm/app/weread', 'arm/app/weread-native-query', 'arm/app/weread-frame-guard'}
INPUT = ARM | REQUIRED | {'selector-bind', 'document-reference', 'launch-reference', 'arm-seed.tar.gz'}
SEED = {'library.sqlite', 'images/cache-index.sqlite3', 'covers/cache-index.sqlite3', 'device-profile', 'device-allowlist'}


def source(name):
    return (HERE / name).read_text(encoding='ascii')


def section(text, first, last):
    if text.count('\n' + first) != 1 or text.count('\n' + last) != 1:
        raise ValueError('Unrecognized shell library boundary')
    return (first + text.split('\n' + first, 1)[1]).split('\n' + last, 1)[0] + '\n'


def change(text, old, new):
    if text.count(old) != 1:
        raise ValueError('Unrecognized shell contract')
    return text.replace(old, new)


def manifest(values, seed=False):
    if type(values) is not dict or not 1 <= len(values) <= 110:
        raise ValueError('Unexpected archive member count')
    leaves = values.keys()
    if seed:
        if not SEED <= leaves or len(leaves) != 21 or sum(name.startswith('covers/') for name in leaves) != 16:
            raise ValueError('Incomplete public seed')
    elif not INPUT <= leaves:
        raise ValueError('Incomplete arm archive')
    result = []
    for name, size in sorted(values.items()):
        if type(name) is not str or type(size) is not int or not 0 < size <= (2 * MIB if seed else 112 * MIB):
            raise ValueError('Invalid member declaration')
        if seed:
            if name not in SEED and not re.fullmatch(r'(covers|images)/[A-Za-z0-9_-]{1,160}\.(png|jpg|jpeg|webp)', name):
                raise ValueError('Unexpected public seed path')
        elif name not in INPUT and (not LIBRARY.fullmatch(name) or '..' in name):
            raise ValueError('Unexpected arm archive path')
        regular = seed or name in {'arm-seed.tar.gz', 'document-reference', 'launch-reference'} or name.startswith('tile/models/')
        result.append(f'{name} {size} {"-rw-r--r--" if regular else "-rwxr-xr-x"}')
    return '\n'.join(result)


def render_device(config):
    label, lane, size = config['label'], config['lane'], config['archive_bytes']
    if (config.get('selector_strategy') != 'single-file-bind-v1' or config.get('claim_protocol') != 'independent-stock-app-v1'
            or not re.fullmatch('[0-9a-f]{16}', label)
            or lane not in ('A', 'B') or type(size) is not int or not 0 < size <= 180 * MIB):
        raise ValueError('Invalid frozen visit')
    members, seed = config['members'], config['seed_members']
    daily = source('daily_render_device.sh')
    indexed = source('../kindle-font-trial/device.sh')
    orientation = source('../kindle-font-trial/orientation-device.sh')
    prefix = daily.split('\nstate() {', 1)[0] + '\n'
    prefix = change(prefix, 'stage=/mnt/us/.weread-render-$label', '[ "$label" = @LABEL@ ] || exit 64\nstage=/mnt/us/.weread-render-$label\ntrial=$stage/arm')
    prefix = change(prefix, 'sh "$home/launch.sh" "$@" || argv "$shell_pid" "$stock_shell" "$home/launch.sh"',
                    'sh "$trial/launch.sh" "$@" || argv "$shell_pid" "$stock_shell" "$trial/launch.sh"')
    peer_data = section(orientation, 'db=', '@FONT_COMMON@') + section(indexed, 'sql()', 'count_doc()')
    peer_data += section(indexed, 'registration()', 'peer_proof()') + section(orientation, 'catalog_one()', 'state()')
    peer = section(orientation, 'state()', 'new_stage()')
    peer = change(peer, 'state()', 'peer_state()')
    peer = peer[:peer.index('    cat /proc/')] + peer[peer.index('    for file in'):]
    peer = change(peer, '"$daily" /mnt/us/weread/launch.sh ', '')
    state = section(daily, 'state()', 'new_stage()')
    state = change(state, 'for file in "$home/launch.sh"', 'peer_state || return 12\n    for file in')
    state = change(state, '    ready_volume ||', '    stock_clear && absent "$authority" && fresh_production && stock_request_absent || { admit_reason=stock_install_or_request; return 1; }\n    ready_volume ||')
    state = change(state, '$4+0>=245760', '$4+0>=@FREE_KIB@')
    navigation = section(daily, 'tile()', 'observation_output()')
    navigation = change(navigation, 'stock-original prepare-started prepare-no-input tile-original',
                        'stock-original prepare-started prepare-no-input prepare-view-refused preinput-return tile-original')
    navigation = change(navigation, '    matches "$stage/view" "$(view)" || { prepare_reason=native_view_changed; return 1; }',
                        '    preinput_view_unchanged || return 1')
    navigation = change(navigation, '    record_once stock-original "$original"',
                        '    preinput_view_unchanged || return 1\n    record_once stock-original "$original"')
    navigation = change(navigation, 'tag installed && tag tile-prepared && origin_agrees && absent', 'selector_active && tag installed && tag tile-prepared && origin_agrees && absent')
    navigation = change(navigation, 'native_idle && tag observer-active', 'stock_request_active && selector_active && native_idle && tag observer-active')
    navigation = change(navigation, '    printf \'%s\\n\' "$receipt"\n}', '    stock_note_release "$receipt" && printf \'%s\\n\' "$receipt"\n}')
    group = section(daily, 'supervision()', 'witness()').replace('$home/', '$trial/')
    group = change(group, 'armed && tag tile-dispatch-intent && no_peer && account=$(lock_owner) &&',
                   'armed && tag tile-dispatch-intent && tag selector-service-view && tag selector-entered && no_peer && account=$(lock_owner) && [ "$account" = 0 ] &&')
    group = change(group, '    entry_birth=$birth\n', '    entry_birth=$birth\n    matches "$stage/selector-callback" "$label:$entry:$entry_birth" || return 1\n')
    group = change(group, section(group, 'gui()', 'bind_group()'), 'gui() { stock_prove_gui timed; }\n')
    close = section(daily, 'retired()', 'restore()')
    close = change(close, '    observer_stopped || return 1', '    stock_request_cleanup && channel_idle || return 1')
    close = change(close, 'drained() {\n', 'drained() {\n    stock_close_chain_retired || return 1\n')
    close = change(close, 'if bind_gui; then current=$(gui) && matches "$stage/session" "$current" && ready=1',
                   'if stock_close_session; then ready=1')
    close = change(close, 'elif absent "$stage/session" && absent "$lock/application-pid"',
                   'elif absent "$stage/session" && absent "$stage/stock-stop-session" && stock_no_trial_gui && absent "$lock/application-pid"')
    close = change(close, 'write_tag term-intent && current=$(supervision)', 'stock_preterm_identity && write_tag term-intent && current=$(supervision)')
    close = change(close, 'absent "$stage/session" && absent "$stage/group"', 'absent "$stage/selector-entered" && absent "$stage/session" && absent "$stage/group"')
    close = change(close, '    armed && tag tile-dispatch-intent && start=$(seconds)', '''    if ! absent "$stage/quarantined"; then stock_quarantine_settled; return; fi
    if absent "$stage/selector-entered" && absent "$stage/group" && absent "$stage/launch-owner"; then
        stock_quarantine_no_group && return 0
        tag selector-entered || return 1
    fi
    armed && tag tile-dispatch-intent && start=$(seconds)''')
    stock = change(source('stock-ab-functions.sh'), '    pair_original && write_tag installed',
                   '    trial_nodes=$(stock_trial_nodes) && record_once stock-trial-nodes "$trial_nodes" && pair_original && write_tag installed')
    text = '\n'.join((prefix, peer_data, peer, state, navigation, group, close, source('stock-app-proof.sh'),
                      source('library-submit.sh'), source('stock-library-request.sh'), source('stock-unsurfaced-close.sh'),
                      stock))
    select = change(change(source('stock_ab_selector.sh'), 'label=@LABEL@', 'label=' + label), 'arm-name @LANE@', 'arm-name ' + lane)
    select = change(select, '@SELECTOR_GATE@', source('stock-selector-gate.sh').rstrip('\n'))
    substitutions = {'@SELECTOR@':select.rstrip('\n'), '@LABEL@':label, '@LANE@':lane, '@ARCHIVE_BYTES@':str(size),
                     '@FREE_KIB@':str((size + sum(members.values()) + sum(seed.values())) // 1024 + 102401),
                     '@DOCUMENT_BYTES@':str(members['document-reference']), '@LAUNCH_BYTES@':str(members['launch-reference']),
                     '@OUTER_MANIFEST@':manifest(members), '@SEED_MANIFEST@':manifest(seed, True)}
    for name, value in substitutions.items():
        text = text.replace(name, value)
    if re.search(r'@[A-Z_]+@', text) or len(text.encode('ascii')) > 96 * 1024:
        raise ValueError('Unresolved or oversized device transaction')
    return text
