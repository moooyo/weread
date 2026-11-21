"""Exercise the normal-card device transaction with a private Linux filesystem and fake processes."""

import io
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import sqlite3
import subprocess
import tarfile
import tempfile
import time
import unittest


HERE = Path(__file__).resolve().parent
LABEL = '0123456789abcdef'
NONCE = 'b' * 32
ENTRY, MASTER, RUNNER, GUI, MANAGER = range(871001, 871006)
RELEASE = ('{"ok":true,"mode":"XTest-button-1","clock":"CLOCK_MONOTONIC","hold_requested_ms":50,'
           '"down_confirmed":true,"up_sent":true,"buttons_clear_after":true,"target":"open-card",'
           '"down_before_ns":1000000000,"down_after_ns":1001000000,"up_before_ns":1052000000,"up_after_ns":1053000000}')
SCENE = ('{"clock":"CLOCK_MONOTONIC","scene":"grid","evidence":"memory_only","last_negative_ns":1100000000,'
         '"first_complete_scene_ns":1200000000,"confirmed_scene_ns":1250000000,"submit_upper_bound_ns":0}')
MAIN = 'case "$mode" in\n    admit)'
TILE = r'''#!/bin/sh
set -eu
root=@ROOT@; stage=$root/mnt/us/.weread-render-$2
inspect() {
    [ ! -e "$root/ambiguous-footer" ] || return 1
    printf '%s %s\n' "$(cat "$root/tab")" "$(cat "$root/pointer")"
}
case "$1" in
    inspect) inspect;;
    prepare)
        [ "$(cat "$stage/prepare-started")" = "$2" ] && [ -s "$stage/stock-original" ] || exit 1
        if [ -e "$root/prepare-refused" ]; then printf 'private diagnostic\ntile_refused original\n' >&2; exit 1; fi
        if [ -e "$root/prepare-unknown" ]; then printf 'unknown private diagnostic\n' >&2; exit 1; fi
        if [ -e "$root/change-origin" ]; then printf '8 9\n' >"$root/pointer"; fi
        inspect >"$stage/tile-original"; printf 'library\n' >"$root/tab"; printf '515 760\n' >"$root/pointer"
        if [ -e "$root/prepare-interrupted" ]; then printf 'private diagnostic\ntile_refused library_or_artwork\n' >&2; exit 1; fi
        printf '%s\n' "$2" >"$stage/tile-prepared"; printf 'tile_prepared\n';;
    tap)
        printf '%s\n' "$2" >"$stage/tile-gate"
        while [ -e "$root/hold-tile" ]; do sleep 0.02; done
        [ ! -e "$root/refuse-before-down" ] || exit 1
        printf '%s\n' "$2" >"$stage/tile-dispatch-intent"
        cat "$root/receipt" >"$stage/tile-release"; cat "$stage/tile-release";;
    restore)
        IFS=' ' read -r tab column row <"$stage/tile-original"
        printf '%s\n' "$tab" >"$root/tab"
        if [ ! -e "$root/incomplete-restore" ]; then printf '%s %s\n' "$column" "$row" >"$root/pointer"; fi
        printf 'tile_restored\n';;
    *) exit 1;;
esac
'''
OBSERVER = r'''#!/bin/sh
set -eu
root=@ROOT@; stage=$root/mnt/us/.weread-render-$1
printf 'ARMED\n'
for attempt in $(seq 1 800); do
    if [ -e "$stage/render-cancel" ]; then if [ -e "$root/delay-cancel" ]; then sleep 0.18; fi; exit 1; fi
    if [ -e "$root/end-observer" ] || { [ -e "$stage/session" ] && [ ! -e "$root/hold-observer" ]; }; then
        cat "$root/scene"; [ ! -e "$root/helper-failed" ]; exit
    fi
    sleep 0.02
done
exit 1
'''
MOCKS = r'''
fixture=@ROOT@
if [ -d "$fixture/ssh-bin" ]; then PATH="$fixture/ssh-bin:$PATH"; fi
uname() { [ "$*" = -m ] && printf 'armv7l\n'; }
df() { [ "$*" = "-Pk $fixture/mnt/us" ] && printf 'local 999999 1 999998 1%% /\n'; }
pidof() {
    case "$1" in
        awesome) printf '@MANAGER@\n';;
        weread) [ -f "$proc/@GUI@/role" ] && [ "$(cat "$proc/@GUI@/role")" = weread ] && printf '@GUI@\n';;
        aidoku|bilicomics) [ -f "$fixture/peer-$1" ] && printf '871100\n';;
        render-observer) return 1;;
        *) return 2;;
    esac
}
status() { [ "$*" = statusbar ] && printf 'statusbar start/running, process 17\n'; }
fixture_lipc() {
    case "$*" in
        '-s com.lab126.powerd state') printf 'active\n';;
        '-s com.lab126.appmgrd activeApp') printf 'com.lab126.KPPMainApp\n';;
        '-s com.lab126.appmgrd activeView') cat "$fixture/active-view"; [ ! -e "$fixture/unsupported-view" ];;
        '-i com.lab126.powerd flIntensity'|'-i com.lab126.powerd currentAmberLevel') cat "$fixture/light";;
        *) return 1;;
    esac
}
sqlite3() {
    [ "$#" = 7 ] && [ "$1:$2:$3:$4:$5:$6" = "-readonly:-batch:-noheader:-cmd:.timeout 1500:$fixture/var/local/cc.db" ] &&
        python3 "$fixture/query.py" "$6" "$7"
}
seq() { [ "$#" = 2 ] && [ "$1" = 1 ] && printf '1\n2\n3\n'; }
usleep() { if [ "$1" = 125000 ]; then sleep 0.125; fi; }
kill() {
    [ "$#" = 2 ] && [ "$1:$2" = '-TERM:@MASTER@' ] || { printf 'wrong target\n' >>"$fixture/signals"; return 1; }
    [ ! -e "$fixture/fail-before-signal" ] || return 1
    printf '%s %s\n' "$1" "$2" >>"$fixture/signals"
    if [ ! -e "$fixture/defer-exit" ]; then
        for stub in @ENTRY@ @MASTER@ @RUNNER@ @GUI@; do
            if [ -d "$proc/$stub" ]; then mv "$proc/$stub" "$fixture/retired-$stub" || return 1; fi
        done
        if [ -d "$lock" ]; then mv "$lock" "$fixture/retired-lock" || return 1; fi
    fi
    [ ! -e "$fixture/fail-after-signal" ]
}
if [ "$mode" = arm ]; then
    mkdir "$proc/$$"; printf '%s (fixture arm) S 1 @ZEROS@100 0\n' "$$" >"$proc/$$/stat"
fi
'''


def archive(entries):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode='w:gz', format=tarfile.USTAR_FORMAT) as bundle:
        for name, body, mode in entries:
            member = tarfile.TarInfo(name)
            member.mode = mode
            if isinstance(body, bytes):
                member.size = len(body)
                bundle.addfile(member, io.BytesIO(body))
            else:
                member.type, member.linkname = tarfile.SYMTYPE, body
                bundle.addfile(member)
    return buffer.getvalue()


class DeviceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.volume = self.root / 'mnt/us'
        self.home = self.volume / 'weread'
        self.app = self.home / 'app/weread'
        self.daily = self.volume / 'documents/weread.sh'
        self.stage = self.volume / ('.weread-render-' + LABEL)
        self.proc = self.root / 'proc'
        self.lock = self.root / 'tmp/weread-launcher.lock'
        self.catalog = self.root / 'var/local/cc.db'
        for directory in ('etc', 'proc', 'sys/class/input', 'var/local', 'tmp', 'mnt/us/documents',
                          'mnt/us/weread/app', 'mnt/us/weread/data/logs'):
            (self.root / directory).mkdir(parents=True, exist_ok=True)
        for name, data in {'proc/stat': 'btime 1700000000\n', 'proc/uptime': '12345.25 1234.00\n',
                           'proc/mounts': f'local {self.volume} tmpfs rw 0 0\n', 'proc/usid': 'G0027J',
                           'etc/prettyversion.txt': 'Kindle 5.19.5 (fixture)\n', 'sys/class/input/event1': '',
                           'tab': 'home\n', 'pointer': '45 63\n', 'active-view': 'KPP_HOME\n',
                           'light': '10\n', 'scene': SCENE + '\n', 'receipt': RELEASE + '\n'}.items():
            (self.root / name).write_text(data)
        guards = [self.daily, self.home / 'launch.sh', self.home / 'app/weread-frame-guard',
                  self.home / 'app/weread-native-query', self.home / 'data/library.sqlite',
                  self.home / 'data/logs/private', self.volume / 'documents/other-document']
        for path in guards:
            path.write_bytes(b'protected fixture\n')
        self.daily.write_text('#!/bin/sh\n# Name: WeRead for Kindle\n# public wrapper fixture\n'
                              'WEREAD_HOME=${WEREAD_HOME:-/mnt/us/weread}\nexec sh "$WEREAD_HOME/launch.sh" "$@"\n')
        with self.app.open('wb') as handle:
            handle.truncate(95137176)
        self.app.chmod(0o755)
        self.baseline = {path: (path.stat().st_ino, path.stat().st_size, path.stat().st_mtime_ns) for path in [*guards, self.app]}
        self.addCleanup(self.assert_protected)
        with sqlite3.connect(self.catalog) as database:
            database.execute('CREATE TABLE Entries(p_location TEXT COLLATE NOCASE,p_mimeType TEXT)')
            database.execute('INSERT INTO Entries VALUES(?, ?)', (str(self.daily), 'text/x-shellscript'))
        (self.root / 'query.py').write_text('import sqlite3,sys\n'
                                            'with sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True) as db:\n'
                                            ' for row in db.execute(sys.argv[2]): print(*row)\n')
        original = (HERE / 'daily_render_device.sh').read_text(encoding='ascii')
        source = re.sub(r'/mnt/us|/sys/class/input|/proc(?=[/\n])|/tmp(?=\))|/var/local/cc\.db|/etc/prettyversion\.txt',
                        lambda match: str(self.root) + match[0], original)
        self.assertEqual(source.count(MAIN), 1)
        self.program = self.root / 'device.sh'
        mocks = MOCKS.replace('@ROOT@', shlex.quote(str(self.root))).replace('@ZEROS@', '0 ' * 17)
        for name, process in [('ENTRY', ENTRY), ('MASTER', MASTER), ('RUNNER', RUNNER), ('GUI', GUI), ('MANAGER', MANAGER)]:
            mocks = mocks.replace('@' + name + '@', str(process))
        self.program.write_text(source.replace('lipc-get-prop', 'fixture_lipc').replace(MAIN, mocks + '\n' + MAIN))
        self.shell = Path(shutil.which('sh')).resolve()
        self.process(MANAGER, 1, 25, [str(self.shell)], self.shell)

    def assert_protected(self):
        for path, state in self.baseline.items():
            self.assertEqual((path.stat().st_ino, path.stat().st_size, path.stat().st_mtime_ns), state, path)

    def process(self, process, parent, birth, args, executable, state='S', role='shell', owner=0):
        folder = self.proc / str(process)
        folder.mkdir(exist_ok=True)
        (folder / 'stat').write_text(f'{process} (fixture ) process) {state} {parent} ' + '0 ' * 17 + f'{birth} 0\n')
        (folder / 'cmdline').write_bytes(b'\0'.join(os.fsencode(value) for value in args) + b'\0')
        (folder / 'role').write_text(role)
        (folder / 'status').write_text(f'Name:\tfixture\nUid:\t{owner}\t{owner}\t{owner}\t{owner}\n')
        image = folder / 'exe'
        image.unlink(missing_ok=True)
        image.symlink_to(executable)

    def chain(self, *, generation=True, worker='--detached --managed-logs', parent=MASTER, gui_extra=(), entry_birth=110,
              owner=0, directory_mode=0o700, shell_name='sh'):
        launch = str(self.home / 'launch.sh')
        self.process(ENTRY, MANAGER, entry_birth, [shell_name, launch, '--library'], self.shell, owner=owner)
        self.process(MASTER, ENTRY, 115, [shell_name, launch, *worker.split()], self.shell, owner=owner)
        self.lock.mkdir(mode=0o700, exist_ok=True)
        os.chown(self.lock, owner, owner); self.lock.chmod(directory_mode)
        contents = dict(pid=MASTER, nonce=NONCE)
        if generation:
            self.process(RUNNER, parent, 120, [shell_name, launch, *worker.split()], self.shell, owner=owner)
            self.process(GUI, RUNNER, 125, [self.app, '--kindle', '--device', 'scribe', '--data-dir', self.home / 'data', *gui_extra],
                         self.app, role='weread', owner=owner)
            contents.update({'application-pid': GUI, 'application-birth': 125, 'generation': '1:' + NONCE,
                             'state-open': '1:' + NONCE, 'application-owned': f'1:{NONCE}:{GUI}:125'})
        for name, value in contents.items():
            (self.lock / name).write_text(str(value) + '\n')
            os.chown(self.lock / name, owner, owner); (self.lock / name).chmod(0o600)

    def entries(self):
        shell = shlex.quote(str(self.root))
        return [('render-observer', OBSERVER.replace('@ROOT@', shell).encode(), 0o755),
                ('tile/native_tile.sh', TILE.replace('@ROOT@', shell).encode(), 0o755),
                *((name, b'fixture', 0o755) for name in ('tile/native-target', 'tile/card-input', 'tile/lib/ld-linux-armhf.so.3')),
                *((name, b'fixture', 0o644) for name in ('tile/models/chi_sim.traineddata', 'tile/models/eng.traineddata',
                                                          'tile/lib/libX11.so.6', 'tile/lib/libXtst.so.6', 'tile/lib/libc.so.6', 'tile/lib/libtesseract.so.5'))]

    def invoke(self, mode, state=None, *, refused=None, payload=b'', label=LABEL, status=None):
        result = subprocess.run(['sh', self.program, mode, label], input=payload, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, status if status is not None else (1 if refused else 0), (mode, result.stderr))
        self.assertEqual(result.stderr, f'refused {refused}\n'.encode() if refused else b'')
        if state:
            self.assertEqual(json.loads(result.stdout), {'state': state})
        if refused or status:
            self.assertEqual(result.stdout, b'')
        return result.stdout

    def setup_stage(self, *, prepare=True):
        self.invoke('stage', 'staged')
        self.invoke('install', 'installed', payload=archive(self.entries()))
        if prepare:
            self.invoke('prepare', 'prepared')
            self.invoke('arm', 'armed')

    def cleanup_stage(self):
        for mode, state in [('stop', 'stopped'), ('restore', 'restored'), ('close', 'closed'), ('absent', 'absent')]:
            self.invoke(mode, state)
        self.assertFalse(self.stage.exists())
        self.assertEqual((self.root / 'tab').read_text(), 'home\n')
        self.assertEqual((self.root / 'pointer').read_text(), '45 63\n')

    def child(self, mode):
        child = subprocess.Popen(['sh', self.program, mode, LABEL], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        def finish():
            (self.root / 'hold-tile').unlink(missing_ok=True)
            (self.root / 'end-observer').touch()
            try:
                child.communicate(timeout=4)
            except subprocess.TimeoutExpired:
                child.kill(); child.communicate(timeout=4)
        self.addCleanup(finish)
        return child

    def observe(self):
        child = self.child('observe')
        self.assertTrue(select.select([child.stdout], [], [], 5)[0], 'ARMED must flush before card input')
        self.assertEqual(child.stdout.readline(), b'ARMED\n')
        return child

    def ended(self, child, stdout=b'', stderr=b'', code=0):
        self.assertEqual(child.communicate(timeout=7), (stdout, stderr))
        self.assertEqual(child.returncode, code)

    def intent(self):
        (self.stage / 'tile-dispatch-intent').write_text(LABEL + '\n')

    def signals(self):
        path = self.root / 'signals'
        return path.read_text().splitlines() if path.exists() else []

    def test_idle_admission_writes_nothing_and_missing_stage_is_explicit(self):
        before = list(self.volume.iterdir())
        self.invoke('admit', 'admitted')
        for mode in ('stop', 'restore', 'close'):
            self.invoke(mode, 'already_absent')
        self.invoke('absent', 'absent')
        for invalid in ('', '1' * 15, 'a' * 17, 'F' * 16):
            self.invoke('stage', label=invalid, status=64)
        self.assertEqual(before, list(self.volume.iterdir()))
        self.invoke('stage', 'staged')
        self.cleanup_stage()
        (self.root / 'active-view').write_text('\n')
        self.invoke('admit', 'admitted'); self.invoke('absent', 'absent'); self.invoke('restore', 'already_absent')

    def test_native_manager_peer_catalog_and_wrong_owner_refused(self):
        self.process(MANAGER, 1, 25, ['sh'], self.shell, state='D')
        self.invoke('admit', refused='not_native_idle')
        self.process(MANAGER, 1, 25, ['sh'], self.shell)
        (self.root / 'peer-aidoku').touch()
        self.invoke('admit', refused='not_native_idle')
        (self.root / 'peer-aidoku').unlink()
        with sqlite3.connect(self.catalog) as database:
            database.execute('INSERT INTO Entries VALUES(?,?)', (str(self.daily).upper(), 'text/x-shellscript'))
        self.invoke('admit', refused='daily_install_guard')
        self.stage.mkdir(); (self.stage / 'owner').write_text('foreign\n')
        self.invoke('admit', refused='stage_occupied')
        self.invoke('close', refused='stage_owner_or_busy')
        self.assertTrue(self.stage.is_dir())

    def test_readonly_admission_reports_only_fixed_environment_categories(self):
        for path, replacement, reason in [('proc/mounts', 'unrelated\n', 'volume'),
                                          ('etc/prettyversion.txt', 'Kindle 0.0 (fixture)\n', 'model_or_firmware'),
                                          ('light', 'private invalid value\n', 'native_projection')]:
            with self.subTest(reason=reason):
                target = self.root / path
                before = target.read_bytes(); target.write_text(replacement)
                self.invoke('admit', refused=reason); self.invoke('stage', refused=reason)
                self.assertFalse(self.stage.exists()); target.write_bytes(before)

    def test_rejects_bad_archive_members_without_leaving_the_stage(self):
        entries = self.entries()
        for member in [('../escaped', b'bad', 0o644), ('tile/lib/bad', str(self.daily), 0o755), entries[0],
                       ('tile/extra', b'bad', 0o644), ('tile/lib/libtest', b'bad', 0o4777)]:
            with self.subTest(member=member[0]):
                self.invoke('stage', 'staged')
                self.invoke('install', refused='install', payload=archive([*entries, member]))
                self.assertFalse((self.volume / 'escaped').exists())
                self.assertFalse((self.stage / 'installed').exists())
                self.cleanup_stage()

    def test_preexisting_stage_links_never_redirect_writes_to_the_install(self):
        self.invoke('stage', 'staged')
        (self.stage / 'members').symlink_to(self.daily)
        self.invoke('install', refused='install', payload=archive(self.entries()))
        self.assertFalse((self.stage / 'input.tgz').exists())
        (self.stage / 'after.next').symlink_to(self.daily)
        self.cleanup_stage()

    def test_ordinary_tap_claim_observation_witness_and_supervised_restore(self):
        self.setup_stage()
        observer = self.observe()
        self.assertEqual(self.invoke('tap'), (RELEASE + '\n').encode())
        self.invoke('tap', refused='uncertain_or_refused_tap')
        self.chain()
        self.invoke('claim', 'claimed')
        self.ended(observer, (SCENE + '\n').encode())
        self.invoke('witness', 'witnessed')
        self.assertEqual((self.stage / 'session').read_bytes(), (self.stage / 'observed-session').read_bytes())
        self.cleanup_stage()
        self.assertEqual(self.signals(), [f'-TERM {MASTER}'])

    def test_claim_rejects_foreign_arguments_ancestry_fence_or_launch_record(self):
        self.setup_stage(); self.intent()
        for arguments in [{'entry_birth': 99}, {'parent': MANAGER}, {'gui_extra': ('--demo',)}]:
            with self.subTest(arguments=arguments):
                self.chain(**arguments)
                self.invoke('claim', refused='launch_identity')
                self.invoke('stop', refused='unproven_close')
        self.chain()
        (self.lock / 'application-owned').write_text(f'1:{"c" * 32}:{GUI}:125\n')
        self.invoke('claim', refused='launch_identity'); self.invoke('stop', refused='unproven_close')
        self.assertFalse((self.stage / 'session').exists()); self.assertFalse(self.signals())
        self.chain(); self.invoke('claim', 'claimed')
        (self.lock / 'nonce').write_text('c' * 32 + '\n')
        self.invoke('stop', refused='unproven_close')
        self.assertFalse(self.signals())

    def test_exact_firmware_shell_and_private_setgid_group_ignore_unrelated_ssh_path(self):
        ssh_bin = self.root / 'ssh-bin'; ssh_bin.mkdir()
        alternate = Path('/bin/bash').resolve()
        self.assertNotEqual(alternate.stat().st_ino, self.shell.stat().st_ino)
        (ssh_bin / 'sh').symlink_to(alternate)
        for owner, directory_mode, first in [(0, 0o2700, 'sh'), (32123, 0o2700, '/bin/sh'), (32124, 0o700, 'sh')]:
            with self.subTest(owner=owner, mode=oct(directory_mode), first=first):
                self.setup_stage(); self.intent(); self.chain(owner=owner, directory_mode=directory_mode, shell_name=first)
                self.invoke('claim', 'claimed'); self.assertEqual((self.stage / 'launch-owner').read_text(), f'{owner}\n')
                self.cleanup_stage()
                shutil.rmtree(self.root / 'retired-lock')
                for process in [ENTRY, MASTER, RUNNER, GUI]:
                    shutil.rmtree(self.root / f'retired-{process}')
        self.assertEqual(self.signals(), [f'-TERM {MASTER}'] * 3)

    def test_group_access_loose_records_or_mixed_effective_uids_cannot_be_signalled(self):
        self.setup_stage(); self.intent()
        for directory_mode in (0o755, 0o2770, 0o1700, 0o4700):
            self.chain(directory_mode=directory_mode); self.invoke('claim', refused='launch_identity')
        self.chain(owner=32123, directory_mode=0o2700)
        for role in [ENTRY, MASTER, RUNNER, GUI]:
            status = self.proc / str(role) / 'status'; initial = status.read_text()
            status.write_text('Uid:\t32123\t0\t32123\t32123\n'); self.invoke('claim', refused='launch_identity')
            self.invoke('stop', refused='unproven_close'); status.write_text(initial)
        for record in ['pid', 'nonce', 'application-owned', 'state-open']:
            path = self.lock / record
            path.chmod(0o640); self.invoke('claim', refused='launch_identity'); path.chmod(0o600)
            os.chown(path, 0, 0); self.invoke('claim', refused='launch_identity'); os.chown(path, 32123, 32123)
        self.assertFalse(self.signals()); self.assertFalse((self.stage / 'session').exists())

    def test_foreign_shell_inode_or_unclassified_first_argument_remains_refused(self):
        self.setup_stage(); self.intent(); self.chain(shell_name='/usr/bin/sh')
        self.invoke('claim', refused='launch_identity'); self.chain()
        for role in [ENTRY, MASTER, RUNNER]:
            path = self.proc / str(role) / 'exe'; path.unlink(); path.symlink_to('/bin/bash')
            self.invoke('claim', refused='launch_identity'); self.invoke('stop', refused='unproven_close')
            path.unlink(); path.symlink_to(self.shell)
        self.assertFalse(self.signals())

    def test_witness_rejects_missing_result_and_changed_live_session(self):
        self.setup_stage(); self.intent(); self.chain(worker='--library-catalog-worker')
        self.invoke('claim', 'claimed'); self.invoke('witness', refused='scene_identity')
        (self.stage / 'observer-active').write_text(LABEL + '\n')
        (self.stage / 'observed-session').write_bytes((self.stage / 'session').read_bytes().replace(b'125\n', b'126\n'))
        self.invoke('witness', refused='scene_identity')
        (self.stage / 'observed-session').write_bytes((self.stage / 'session').read_bytes())
        self.invoke('witness', 'witnessed')
        self.process(GUI, RUNNER, 126, [self.app, '--kindle', '--device', 'scribe', '--data-dir', self.home / 'data'], self.app, role='weread')
        self.invoke('witness', refused='scene_identity'); self.invoke('stop', refused='unproven_close')
        self.assertFalse(self.signals())

    def test_unbound_dispatch_is_not_a_proven_idle_close(self):
        self.setup_stage(); self.intent()
        self.invoke('claim', refused='launch_identity'); self.invoke('stop', refused='unproven_close')
        self.invoke('restore', refused='native_restore'); self.invoke('close', refused='closeout')
        self.assertTrue(self.stage.is_dir()); self.assertFalse(self.signals())

    def test_early_bound_supervisor_only_and_incomplete_generation(self):
        self.setup_stage(); self.intent(); self.chain(generation=False)
        (self.lock / 'generation').write_text('1:' + NONCE + '\n')
        self.invoke('stop', refused='unproven_close'); self.assertFalse(self.signals())
        (self.lock / 'generation').unlink()
        self.cleanup_stage(); self.assertEqual(self.signals(), [f'-TERM {MASTER}'])

    def test_term_failure_before_signal_retries_only_the_still_bound_supervisor(self):
        self.setup_stage(); self.intent(); self.chain(); self.invoke('claim', 'claimed')
        (self.root / 'fail-before-signal').touch()
        self.invoke('stop', refused='unproven_close')
        self.assertTrue((self.stage / 'term-intent').is_file()); self.assertFalse((self.stage / 'term-sent').exists())
        self.assertFalse(self.signals()); (self.root / 'fail-before-signal').unlink()
        self.cleanup_stage(); self.assertEqual(self.signals(), [f'-TERM {MASTER}'])

    def test_failed_ack_after_retirement_and_reused_pid_never_signal_twice(self):
        self.setup_stage(); self.intent(); self.chain(); self.invoke('claim', 'claimed')
        (self.root / 'fail-after-signal').touch()
        self.invoke('stop', refused='unproven_close')
        self.process(MASTER, 1, 777, ['foreign'], self.shell, role='foreign')
        self.cleanup_stage(); self.assertEqual(self.signals(), [f'-TERM {MASTER}'])

    def test_confirmed_signal_is_not_repeated_while_waiting_for_real_retirement(self):
        self.setup_stage(); self.intent(); self.chain(); (self.root / 'defer-exit').touch()
        for unused in range(2):
            self.invoke('stop', refused='unproven_close')
        self.assertEqual(self.signals(), [f'-TERM {MASTER}'])
        self.invoke('restore', refused='native_restore')

    def test_dead_or_cancelled_observer_blocks_card_and_active_one_blocks_cleanup(self):
        self.setup_stage()
        self.invoke('tap', refused='uncertain_or_refused_tap')
        child = self.observe()
        self.invoke('stop', refused='unproven_close'); self.invoke('cancel', 'cancelled')
        self.invoke('tap', refused='uncertain_or_refused_tap'); self.ended(child, stderr=b'refused observer\n', code=1)
        self.assertFalse((self.stage / 'tile-dispatch-intent').exists()); self.cleanup_stage()

    def test_inflight_prepress_refusal_cannot_be_mistaken_for_quiescence(self):
        self.setup_stage(); child = self.observe()
        (self.root / 'hold-tile').touch(); (self.root / 'refuse-before-down').touch()
        tapping = self.child('tap')
        deadline = time.monotonic() + 5
        while not (self.stage / 'tile-gate').exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue((self.stage / 'tile-gate').exists())
        self.invoke('stop', refused='stage_owner_or_busy')
        (self.root / 'hold-tile').unlink(); self.ended(tapping, stderr=b'refused uncertain_or_refused_tap\n', code=1)
        self.assertFalse((self.stage / 'tile-dispatch-intent').exists())
        self.invoke('cancel', 'cancelled'); self.ended(child, stderr=b'refused observer\n', code=1)
        self.cleanup_stage(); self.assertFalse(self.signals())

    def test_recovery_waits_for_a_cancelled_observer_to_drop_its_lock(self):
        self.setup_stage(); (self.root / 'delay-cancel').touch(); child = self.observe()
        self.invoke('cancel', 'cancelled'); self.invoke('stop', 'stopped')
        self.ended(child, stderr=b'refused observer\n', code=1)
        self.cleanup_stage(); self.assertFalse(self.signals())

    def test_bad_output_cannot_escape_and_failed_observer_never_creates_a_witness(self):
        for line, succeeded in [(SCENE + '\nprivate extra\n', True), (SCENE + '\n', False), ('private value\n', True)]:
            with self.subTest(line_valid=line == SCENE + '\n', helper=succeeded):
                self.setup_stage(); child = self.observe()
                if not succeeded:
                    (self.root / 'helper-failed').touch()
                (self.root / 'scene').write_text(line); (self.root / 'end-observer').touch()
                self.ended(child, stderr=b'refused observer\n', code=1)
                self.assertFalse((self.stage / 'observed-session').exists())
                self.invoke('witness', refused='scene_identity'); self.cleanup_stage()
                (self.root / 'end-observer').unlink(); (self.root / 'helper-failed').unlink(missing_ok=True)

    def test_invalid_tap_remains_uncertain_but_does_not_export_unknown_fields(self):
        self.setup_stage(); observer = self.observe()
        (self.root / 'receipt').write_text(RELEASE[:-1] + ',"private":"not public"}\n')
        self.invoke('tap', refused='uncertain_or_refused_tap'); self.assertTrue((self.stage / 'tile-dispatch-intent').is_file())
        self.invoke('cancel', 'cancelled'); self.ended(observer, stderr=b'refused observer\n', code=1)
        self.invoke('stop', refused='unproven_close')

    def test_failed_prepare_restores_an_actual_navigation_and_detects_baseline_drift(self):
        self.setup_stage(prepare=False)
        (self.root / 'prepare-refused').touch(); self.invoke('prepare', refused='stock_tabs_unconfirmed')
        self.assertFalse((self.stage / 'tile-original').exists()); self.cleanup_stage()
        (self.root / 'prepare-refused').unlink(); self.setup_stage(prepare=False)
        (self.root / 'prepare-interrupted').touch(); self.invoke('prepare', refused='library_or_artwork')
        self.assertEqual((self.root / 'tab').read_text(), 'library\n'); self.invoke('stop', 'stopped')
        (self.root / 'light').write_text('12\n'); self.invoke('restore', refused='native_restore')
        self.assertFalse((self.stage / 'restored').exists()); (self.root / 'light').write_text('10\n')
        self.cleanup_stage(); self.assertFalse(self.signals())

    def test_lost_original_after_preparation_does_not_confirm_restoration(self):
        self.setup_stage(); (self.stage / 'tile-original').unlink()
        self.invoke('stop', 'stopped'); self.invoke('restore', refused='native_restore')
        self.invoke('close', refused='closeout'); self.assertTrue(self.stage.is_dir())

    def test_unavailable_view_never_exports_raw_values_and_needs_stock_tab_and_pointer(self):
        for value, unsupported in [('\n', False), ('KPP_HOME\n', True), ('private.view:123\n', False)]:
            with self.subTest(unsupported=unsupported, empty=value == '\n'):
                (self.root / 'active-view').write_text(value)
                if unsupported:
                    (self.root / 'unsupported-view').touch()
                self.invoke('admit', 'admitted'); self.setup_stage()
                self.assertEqual((self.stage / 'view').read_text(), 'unavailable\n')
                self.assertEqual((self.stage / 'stock-original').read_text(), 'home 45 63\n')
                self.assertEqual((self.stage / 'stock-original').read_bytes(), (self.stage / 'tile-original').read_bytes())
                self.cleanup_stage(); (self.root / 'unsupported-view').unlink(missing_ok=True)

    def test_unknown_ambiguous_or_out_of_bounds_start_never_authorizes_input(self):
        (self.root / 'active-view').write_text('private\n')
        for tab, pointer, ambiguous in [('home', '45 63', True), ('reader', '45 63', False),
                                        ('library', '1860 32', False), ('home', '0 2480', False), ('home', '12 2 extra', False)]:
            with self.subTest(tab=tab, pointer=pointer, ambiguous=ambiguous):
                self.setup_stage(prepare=False)
                (self.root / 'tab').write_text(tab + '\n'); (self.root / 'pointer').write_text(pointer + '\n')
                if ambiguous:
                    (self.root / 'ambiguous-footer').touch()
                self.invoke('prepare', refused='stock_tabs_unconfirmed')
                for leaf in ('stock-original', 'prepare-started', 'tile-original', 'tile-prepared', 'tile-dispatch-intent'):
                    self.assertFalse((self.stage / leaf).exists())
                self.invoke('stop', 'stopped'); self.invoke('restore', 'restored'); self.invoke('close', 'closed')
                self.assertEqual((self.root / 'tab').read_text(), tab + '\n')
                self.assertEqual((self.root / 'pointer').read_text(), pointer + '\n')
                (self.root / 'ambiguous-footer').unlink(missing_ok=True)

    def test_missing_optional_view_does_not_bless_unrestored_or_changed_pointer(self):
        (self.root / 'unsupported-view').touch(); self.setup_stage(); self.invoke('stop', 'stopped')
        (self.root / 'incomplete-restore').touch(); self.invoke('restore', refused='native_restore')
        self.assertFalse((self.stage / 'restored').exists()); (self.root / 'incomplete-restore').unlink()
        self.invoke('restore', 'restored'); (self.root / 'pointer').write_text('89 17\n')
        self.invoke('close', refused='closeout'); self.assertTrue(self.stage.is_dir())
        self.cleanup_stage()

    def test_optional_classified_view_needs_same_class_on_return_and_before_navigation(self):
        self.setup_stage(prepare=False)
        self.assertEqual((self.stage / 'view').read_text(), 'kpp_home\n')
        (self.root / 'active-view').write_text('KPP_LIBRARY\n'); self.invoke('prepare', refused='native_view_changed')
        self.assertFalse((self.stage / 'prepare-started').exists()); (self.root / 'active-view').write_text('KPP_HOME\n')
        self.invoke('prepare', 'prepared'); self.invoke('stop', 'stopped')
        for changed in ['KPP_LIBRARY\n', '\n']:
            (self.root / 'active-view').write_text(changed); self.invoke('restore', refused='native_restore')
            self.assertFalse((self.stage / 'restored').exists())
        (self.root / 'active-view').write_text('KPP_HOME\n'); self.cleanup_stage()

    def test_uncertain_start_or_mismatched_memos_cannot_be_restored_by_guessing(self):
        (self.root / 'unsupported-view').touch(); self.setup_stage(prepare=False)
        (self.root / 'prepare-unknown').touch(); self.invoke('prepare', refused='preparation')
        self.invoke('stop', 'stopped'); self.invoke('restore', refused='native_restore')
        (self.stage / 'stock-original').unlink(); self.invoke('restore', refused='native_restore')
        (self.stage / 'prepare-started').unlink(); self.invoke('restore', refused='native_restore')
        self.invoke('close', refused='closeout')

    def test_prepared_helper_origin_conflict_refuses_arm_and_untrusted_restore(self):
        self.setup_stage(prepare=False); (self.root / 'change-origin').touch()
        self.invoke('prepare', refused='stock_origin_changed'); self.invoke('arm', refused='arm')
        self.invoke('stop', 'stopped'); self.invoke('restore', refused='native_restore')
        self.assertFalse((self.stage / 'restored').exists()); self.assertFalse(self.signals())

    def test_normal_catalog_touch_of_unchanged_wrapper_can_close(self):
        self.setup_stage(); self.intent(); self.chain(); self.invoke('claim', 'claimed')
        original = self.daily.read_bytes()
        self.assertEqual((self.stage / 'document-original').read_bytes(), original)
        stamp = self.daily.stat()
        os.utime(self.daily, ns=(stamp.st_atime_ns, stamp.st_mtime_ns + 2_000_000_000))
        self.baseline[self.daily] = (stamp.st_ino, stamp.st_size, self.daily.stat().st_mtime_ns)
        self.cleanup_stage(); self.assertEqual(self.daily.read_bytes(), original)
        self.assertEqual(self.signals(), [f'-TERM {MASTER}'])

    def test_equal_size_in_place_wrapper_change_is_not_a_catalog_touch(self):
        self.setup_stage(); self.invoke('stop', 'stopped')
        original, stamp = self.daily.read_bytes(), self.daily.stat()
        changed = original.replace(b'public wrapper fixture', b'public wrapper changed')
        self.assertNotEqual(original, changed); self.assertEqual(len(original), len(changed))
        self.daily.write_bytes(changed); os.utime(self.daily, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
        self.invoke('restore', refused='native_restore'); self.invoke('close', refused='closeout')
        self.assertEqual(self.daily.read_bytes(), changed); self.assertEqual((self.stage / 'document-original').read_bytes(), original)
        self.daily.write_bytes(original); os.utime(self.daily, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
        self.cleanup_stage()


if __name__ == '__main__':
    unittest.main()
