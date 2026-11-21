"""Freeze fourteen stock-card visits with two shared, independently verified ARM payloads."""

import argparse
from contextlib import contextmanager
import gzip
import io
import json
import os
from pathlib import Path
import re
import runpy
import secrets
import sqlite3
import subprocess
import tarfile
import tempfile

from prepare_daily_render import scan


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RELEASE_WORK = Path('/root/weread-font-ab-rebuild-20261120.uHKbMu4A')
RELEASE = Path('indexed-trial.qNFrmeWH/release1')
SEED_WORK = Path('/root/weread-orientation-20261121.WR7ofkGL')
TILE_WORK = Path('/root/weread-native-tile-20261121.JcUBADYA')
ABI = 'armv7-unknown-linux-musleabihf'
TARGET = f'target/{ABI}/release/weread'
WITNESS = 'trial-library-frame'
REUSE = 'startup-orientation-reuse'
PURPOSE = 'ordinary-stock-library-submit-ab'
BATCH = 'stock-library-ab'
PROFILE = {'device-profile': b'scribe\n', 'device-allowlist': b'scribe 5.19.5\n'}
MATRIX = [(0, 'warmup', arm) for arm in 'AB'] + [(pair, 'measured', arm) for pair in range(1,7) for arm in ('AB' if pair % 2 else 'BA')]
SCOPE = dict(purpose=PURPOSE, device='Kindle Scribe 2022', firmware='5.19.5', clock='CLOCK_MONOTONIC',
             entry='ordinary_stock_weread_tile', fixture='public_normal_15')
CODE = ('weread-frame-guard', 'weread-native-query')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def local(work, path, *, directory=False):
    file = Path(path).absolute()
    require(file.resolve(strict=True) == file and file.is_relative_to(work) and (file.is_dir() if directory else file.is_file()),
            f'Not an ordinary in-scope path: {file.name}')
    return file


def record(work, path):
    file = local(work, work / path)
    require(0 < file.stat().st_size <= 256 * 1024, 'Unbounded receipt')
    return json.loads(file.read_bytes())


def publish(path, value):
    with path.open('x', encoding='utf-8', newline='\n') as out:
        json.dump(value, out, indent=2); out.write('\n')


def check_database(data, bindings):
    with tempfile.TemporaryDirectory() as scratch:
        database = Path(scratch) / 'library.sqlite'
        database.write_bytes(data)
        connection = sqlite3.connect(database.as_uri() + '?mode=ro&immutable=1', uri=True)
        try:
            connection.execute('PRAGMA query_only=ON')
            require(connection.execute('PRAGMA quick_check').fetchall() == [('ok',)], 'Invalid public Library database')
            targets = {(row['book_id'],row['cover_url']) for row in bindings['targets']}
            require(len(targets) == 15 and set(connection.execute('SELECT id,cover_url FROM books WHERE on_shelf=1')) == targets
                    and connection.execute('SELECT count(*) FROM download_tasks').fetchone() == (0,), 'Wrong public Library fixture')
        finally:
            connection.close()


def seed(work):
    path = local(work, work/'inputs/public-seed.tgz')
    bindings = record(ROOT, 'docs/review/catalog-entry-adapter-20261118/library-cover-bindings.json')
    expected = {row['thumbnail'] for row in bindings['targets']} | {'demo/library.sqlite','demo/covers/cache-index.sqlite3',
                'demo/images/cache-index.sqlite3','library-covers.json'}
    require(path.stat().st_size < 16*1024**2, 'Oversized public fixture')
    seen, regular, values = set(), set(), dict(PROFILE)
    with tarfile.open(path, 'r:gz') as source:
        for item in source:
            name = item.name
            require(name not in seen and len(seen) < 32, 'Unexpected duplicate public seed entry'); seen.add(name)
            if item.isdir():
                require(name in ('demo','demo/covers','demo/images'), 'Unexpected seed directory'); continue
            require(item.isfile() and 0 < item.size <= 2*1024**2 and (name in expected or
                    re.fullmatch(r'demo/images/[0-9a-f]{64}-[0-9a-f]{64}\.png', name)), 'Private or unexpected seed member')
            value = source.extractfile(item).read(); regular.add(name)
            if name == 'library-covers.json':
                require(json.loads(value) == bindings, 'Wrong public cover set')
            else:
                values[name.removeprefix('demo/')] = value
    require(expected <= regular and len(regular - expected) == 1, 'Incomplete public shelf')
    check_database(values['library.sqlite'], bindings)
    return values


def release_references():
    plan = record(RELEASE_WORK, RELEASE / 'plan.json')
    ops = record(RELEASE_WORK, RELEASE / 'batch-qscph1bl/operations.json')
    require(plan.get('purpose') == 'installed-retest' and plan.get('label') == '161f1defed4b2c8a' and ops.get('passed') is True
            and ops.get('released') and [(row['mode'],row['status']) for row in ops['operations']] ==
            [(mode,0) for mode in ('admit','stage','case','restore','collect','close')], 'The reference installation lacks a closed rollout')
    source = RELEASE_WORK / 'src/packaging/kindle'
    document = local(RELEASE_WORK, source/'document-launcher.sh'); launcher = local(RELEASE_WORK, source/'launch.sh')
    require(document.stat().st_size == 43102 and launcher.stat().st_size == 137861 and
            document.read_bytes().endswith(b'exec sh "$WEREAD_HOME/launch.sh" "$@"\n'), 'Unexpected retained production entry')
    return {'document-reference':document,'launch-reference':launcher,'arm/launch.sh':launcher}


@contextmanager
def input_stream(value):
    with (value.open('rb') if isinstance(value, Path) else io.BytesIO(value)) as stream:
        yield stream


def length(value):
    return value.stat().st_size if isinstance(value, Path) else len(value)


def same(left, right):
    if length(left) != length(right): return False
    with input_stream(left) as first, input_stream(right) as second:
        while chunk := first.read(1024*1024):
            if second.read(len(chunk)) != chunk: return False
        return not second.read(1)


def tar_mode(name):
    return 0o644 if name in ('document-reference','launch-reference','arm-seed.tar.gz') or name.startswith('tile/models/') else 0o755


def write_bundle(stream, items, *, inner=False):
    with gzip.GzipFile(filename='', mode='wb', compresslevel=1, fileobj=stream, mtime=0) as compressed, \
            tarfile.open(fileobj=compressed, mode='w|', format=tarfile.GNU_FORMAT if inner else tarfile.USTAR_FORMAT) as archive:
        for name, value in sorted(items.items()):
            header = tarfile.TarInfo(name); header.size = length(value); header.mode = 0o644 if inner else tar_mode(name)
            with input_stream(value) as source: archive.addfile(header, source)


def verify_bundle(path, items):
    require(0 < path.stat().st_size <= 180 * 1024**2, 'Unbounded arm bundle')
    with tarfile.open(path,'r|gz') as archive:
        seen = set()
        for item in archive:
            name = item.name
            require(name in items and name not in seen and item.type == tarfile.REGTYPE and not item.pax_headers and
                    not item.uid and not item.gid and not item.linkname and item.mode == tar_mode(name) and
                    item.size == length(items[name]), 'Foreign or changed stock A/B archive member')
            with archive.extractfile(item) as actual, input_stream(items[name]) as source:
                while chunk := actual.read(1024*1024):
                    require(chunk == source.read(len(chunk)), f'Archive bytes differ from the frozen input: {name}')
                require(not source.read(1), 'Short archive member')
            seen.add(name)
        require(seen == items.keys(), 'Missing stock A/B archive member')


def cargo_lane(work, lane, pair):
    code = 'baseline' if lane == 'A' else 'candidate'
    given = pair['lanes'][code]
    result = record(work, f'{code}/result.json')
    selected = ['kindle','bundled-font-preflight',WITNESS] + ([REUSE] if lane == 'B' else [])
    features, backend = sorted(selected), [REUSE] if lane == 'B' else []
    command = ['cargo','zigbuild','--locked','--offline','--release','--target',ABI,'--no-default-features','--features',
               ','.join(selected),'--bin','weread','--message-format=json-render-diagnostics']
    source = record(work, 'logs/frozen-source.json')
    require(given == dict(arm=lane,program=f'{code}/weread',build=f'{code}/result.json',cargo=f'logs/{code}/events.jsonl',
                          bytes=result['elf']['bytes'],application_features=features,backend_features=backend), 'Unexpected pair member')
    require(result.get('lane') == code and result.get('command') == command and result.get('source') == '/work/src'
            and result.get('abi') == ABI and result.get('both_recompiled') is True and result.get('cargo_target') == TARGET
            and result.get('source_commit') == source.get('base') == pair['source_commit'] and source.get('feature') == WITNESS
            and result.get('source_provenance') == source and result.get('source_proof') == 'logs/frozen-source.json' and
            result.get('application_features') == features and result.get('backend_features') == backend and
            result.get('cargo_artifacts') == given['cargo'], 'Source or compiled features do not match the frozen pair')
    path = local(work, work/given['cargo']); require(0 < path.stat().st_size < 16*1024**2, 'Unbounded Cargo artifact log')
    targets = {'weread':('/work/src/Cargo.toml',['bin'],features,'/work/'+TARGET),
               'slint_backend_kindle':('/work/src/vendor/slint-backend-kindle/Cargo.toml',['lib'],backend,None)}
    seen = set(); last = None
    with path.open() as stream:
        for line in stream:
            last = json.loads(line)
            if last.get('reason') != 'compiler-artifact': continue
            name, kind = last['target']['name'], last['target']['kind']
            if name not in targets or kind != targets[name][1]: continue
            require(name not in seen and last['fresh'] is False and last['profile']['test'] is False and
                    (last['manifest_path'],kind,last['features'],last['executable']) == targets[name], 'Wrong or stale ARM artifact')
            seen.add(name)
    program = local(work, work/given['program'])
    inspect = runpy.run_path(str(ROOT/'docs/review/library-startup-latency-20261119/rebuild-arm.py'))['inspect_elf']
    require(seen == targets.keys() and last == {'reason':'build-finished','success':True} and inspect(program) == result['elf'] and
            0 < length(program) <= 110*1024**2 and (lane == 'A' or same(program, local(work,work/TARGET))), 'Not the reviewed ARM output')
    return program


def materials(work):
    pair = record(work, 'pair.json')
    require(pair.get('status') == 'ready_for_offline_freeze' and pair.get('diagnostic_feature') == WITNESS and
            pair.get('only_feature_difference') == REUSE and pair.get('reconcile_enabled') is False, 'Not the ordinary Library build pair')
    arms = {lane:cargo_lane(work,lane,pair) for lane in 'AB'}
    require(not same(arms['A'],arms['B']), 'A and B must be distinct executable builds')
    require(same(local(work,work/'inputs/public-seed.tgz'),local(SEED_WORK,SEED_WORK/'orientation-inputs/seed.tar.gz')),
            'The fixture is not the original public shelf')
    fixture = record(work,'inputs/normal-public-fixture.json')['result']
    require(fixture['public-default-scale']['observed_route'] == 'library' and fixture['public-default-scale']['shelf_entries'] == 15
            and fixture['public-default-scale']['local_account_file'] is False and fixture['empty-default-scale']['observed_route'] == 'login',
            'Missing ordinary-mode public Library and empty Login control evidence')
    data = seed(work); packed = io.BytesIO(); write_bundle(packed,data,inner=True)
    common = release_references() | {'arm-seed.tar.gz':packed.getvalue()}
    for name in (*CODE,'selector-exchange','render-observer'):
        path = local(work,work/'inputs'/name)
        require(os.access(path,os.X_OK) and 0 < length(path) <= 1024**2, 'Unqualified executable helper')
        common['arm/app/'+name if name in CODE else name] = path
    observer = record(work,'logs/observer.json'); publication = record(work,'logs/observer-publish.json')
    require(observer['elf']['bytes'] == length(common['render-observer']) and observer['status'] == 'passed' and
            observer['clock'] == 'CLOCK_MONOTONIC' and '--publish-frame-request LABEL' in observer['modes'] and
            publication['existing_destination_unchanged'] is True and publication['destination_matches_pinned_fd6'] is True,
            'Unqualified stock request observer')
    helpers = record(work, 'logs/production-helpers.json')
    for name in CODE:
        require(helpers[name]['byte_compared_to_original'] is True and helpers[name]['size'] == length(common['arm/app/'+name]),
                'Unexpected production helper')
    policy = subprocess.run(['/opt/kindle-toolchain/ubuntu-run',str(work),'qemu-arm','-cpu','cortex-a8',
                             '/work/inputs/selector-exchange','--policy'], capture_output=True, timeout=15)
    require(policy.returncode == 0 and not policy.stderr and policy.stdout == b'selector-exchange-v1\n', 'Unexpected selector exchange policy')
    tile_path = local(work,work/'inputs/stock-tile-ocr.tgz')
    require(same(tile_path,local(TILE_WORK,TILE_WORK/'native-tile-qualified.tar.gz')), 'Not the qualified WeRead artwork selector kit')
    scan(tile_path, kit=True, accept=lambda name, value: common.update({name:value}))
    require(common['tile/native_tile.sh'] == (HERE/'native_tile.sh').read_bytes(), 'Unreviewed stock-card selector')
    return pair, {lane:common | {'arm/app/weread':arms[lane]} for lane in arms}, {name:length(value) for name,value in data.items()}


def schedule(labels):
    require(isinstance(labels, list) and len(labels) == len(MATRIX) and len(set(labels)) == len(labels) and
            all(isinstance(label,str) and re.fullmatch('[a-f0-9]{16}',label) for label in labels), 'Need fourteen distinct visit labels')
    return [dict(slot=index, pair=pair, role=role, lane=lane, label=label, visit=f'visits/{index:02d}-{lane}')
            for index, ((pair,role,lane), label) in enumerate(zip(MATRIX,labels,strict=True))]


def configuration(batch, slot):
    bundle = batch['bundles'][slot['lane']]
    return dict(label=slot['label'], lane=slot['lane'], members=bundle['members'], archive_bytes=bundle['archive_bytes'],
                seed_members=batch['seed_members'])


def visit_record(batch, slot):
    return SCOPE | slot | {'source_commit':batch['source_commit'], 'configuration':configuration(batch,slot)}


def program(batch, slot):
    from stock_ab_device import render_device

    content = render_device(configuration(batch,slot))
    return content.encode('ascii')


def layout(work, root):
    root = local(work,root,directory=True)
    batch = record(root,'batch.json')
    require(all(batch.get(key) == value for key,value in SCOPE.items()) and isinstance(batch.get('schedule'),list)
            and batch['schedule'] == schedule([row['label'] for row in batch['schedule']]) and set(batch['bundles']) == {'A','B'} and
            re.fullmatch('[0-9a-f]{40}',batch['source_commit']), 'Invalid frozen stock A/B schedule')
    return batch


def verify_slot(work, root, batch, slot, *, recovery=False):
    visit = local(root,root/slot['visit'],directory=True)
    data = record(root,visit/'plan.json')
    require(data == visit_record(batch,slot), 'Crossed or changed visit plan')
    script = local(root,visit/'device.sh')
    require(0 < length(script) <= 128*1024 and script.read_bytes() == program(batch,slot), 'Wrong frozen device program')
    if not recovery:
        payload = local(root,visit/'inputs.tgz')
        bundle = local(root,root/(slot['lane']+'.tgz'))
        require(payload.samefile(bundle) and length(payload) == batch['bundles'][slot['lane']]['archive_bytes'], 'Wrong shared arm payload')
    return data


def check_materials(work, root, batch, lanes):
    pair, per_arm, seed_members = materials(work)
    require(batch['source_commit'] == pair['source_commit'] and batch['lanes'] == pair['lanes'] and
            batch['seed_members'] == seed_members, 'Frozen pair or normal Library seed changed')
    for lane in lanes:
        path = local(root,root/(lane+'.tgz'))
        require(batch['bundles'][lane] == {'members':{name:length(value) for name,value in per_arm[lane].items()},
                                          'archive_bytes':length(path)}, 'Frozen arm manifest changed')
        verify_bundle(path,per_arm[lane])


def verify_visit(work, plan, *, recovery=False):
    work = Path(work).resolve(strict=True); root = work/BATCH
    plan = local(root,Path(plan),directory=True)
    batch = layout(work,root)
    slot, = [slot for slot in batch['schedule'] if root/slot['visit'] == plan]
    result = verify_slot(work,root,batch,slot,recovery=recovery)
    if not recovery: check_materials(work, root, batch, slot['lane'])
    return result


def verify_set(work, root):
    batch = layout(work,root)
    check_materials(work,root,batch,'AB')
    for slot in batch['schedule']: verify_slot(work,root,batch,slot)
    return batch


def verify_batch(work):
    work = Path(work).resolve(strict=True)
    return verify_set(work,work/BATCH)


def freeze(work):
    work = Path(work).resolve(strict=True); target = work/BATCH
    require(not target.exists() and not target.is_symlink(), 'Refusing to overwrite a frozen batch')
    pair, per_arm, seed_members = materials(work)
    with tempfile.TemporaryDirectory(dir=work,prefix='.prepare-stock-ab-') as scratch:
        root = Path(scratch)/BATCH; root.mkdir(mode=0o700); (root/'visits').mkdir(mode=0o700)
        batch = SCOPE | dict(source_commit=pair['source_commit'], lanes=pair['lanes'], seed_members=seed_members,
                            schedule=schedule([secrets.token_hex(8) for _ in MATRIX]), bundles={})
        for lane, items in per_arm.items():
            package = root/(lane+'.tgz')
            with package.open('xb') as out: write_bundle(out,items)
            package.chmod(0o400)
            batch['bundles'][lane] = dict(archive_bytes=length(package), members={name:length(value) for name,value in items.items()})
        publish(root/'batch.json',batch)
        for slot in batch['schedule']:
            visit = root/slot['visit']; visit.mkdir(mode=0o700)
            publish(visit/'plan.json',visit_record(batch,slot))
            script = program(batch,slot); subprocess.run(['sh','-n'],input=script,check=True)
            (visit/'device.sh').write_bytes(script)
            os.link(root/(slot['lane']+'.tgz'),visit/'inputs.tgz')
        verify_set(work,root)
        root.rename(target)
    return batch


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('freeze','verify'))
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--plan', type=Path)
    args = parser.parse_args()
    if args.operation == 'freeze' and args.plan: parser.error('Freeze the entire prespecified batch, not one visit')
    report = freeze(args.work) if args.operation == 'freeze' else verify_visit(args.work,args.plan) if args.plan else verify_batch(args.work)
    print(json.dumps(dict(validated=True, labels=[report['label']] if args.plan else [row['label'] for row in report['schedule']],
                          device_accessed=False, lease_requested=False)))
