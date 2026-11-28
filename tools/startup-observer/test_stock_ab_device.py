"""Exercise the per-visit device program on an isolated Linux filesystem, never a Kindle."""

import fcntl
import io
import json
import os
from pathlib import Path
import re
import select
import shlex
import subprocess
import tarfile
import time
import unittest

import test_daily_render_device as daily
from stock_ab_device import render_device
from stock_ab_transaction import LOAD_CLOSE_RECEIPTS


MOCK_SQL = r'''
sql() {
    case "$1:$2" in
        "$reg:SELECT value FROM properties"*) printf '%s/aidoku-library-dispatch\n' "$runtime";;
        "$reg:SELECT count(*) FROM associations"*) printf '1\n';;
        "$reg:BEGIN; SELECT "*) cat "$fixture/public-registration";;
        "$db:SELECT count(*)||"*) printf '1:1\n';;
        "$db:SELECT quote(p_location)"*) printf 'public catalog %s\n' "$2";;
        *) return 1;;
    esac
}
'''


def contents(items, *, inner=False):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode='w:gz', format=tarfile.GNU_FORMAT if inner else tarfile.USTAR_FORMAT) as output:
        for name, body, permissions in sorted(items):
            member = tarfile.TarInfo(name)
            member.mode = permissions
            if isinstance(body, bytes):
                member.size = len(body); output.addfile(member, io.BytesIO(body))
            else:
                member.type, member.linkname = tarfile.SYMTYPE, body
                output.addfile(member)
    return buffer.getvalue()


class StockDeviceFixture(unittest.TestCase):
    def setUp(self):
        if (os.geteuid() != 0 or not Path('/').samefile('/proc/1/root') or
                not Path('/proc/self/ns/mnt').samefile('/proc/1/ns/mnt') or Path('/proc/1/comm').read_text().strip() in ('init','systemd') or
                subprocess.check_output(['findmnt','-n','-o','FSTYPE,PROPAGATION','-T','/tmp'],text=True).split() != ['tmpfs','private']):
            self.skipTest('Requires root in a disposable PID/mount namespace with a private tmpfs /tmp')
        self.device_shell = ['busybox','sh'] if os.getenv('WEREAD_TEST_BUSYBOX') else ['sh']
        self.base = daily.DeviceTests(methodName='runTest')
        self.base.setUp()
        self.addCleanup(self.base.doCleanups)
        for key in ('root', 'volume', 'home', 'daily', 'stage', 'proc', 'lock', 'catalog', 'program'):
            setattr(self, key, getattr(self.base, key))
        self.live = self.home / 'launch.sh'
        subprocess.run(['mount','--bind',self.volume,self.volume],check=True)
        self.addCleanup(self.clear_fixture_mounts)
        self.fixed = self.root / 'var/tmp/weread-library-frame'
        self.fixed.parent.mkdir(mode=0o700)
        self.authority = self.fixed.parent / 'weread-stock-selector-bind'
        self.selector_slot = self.stage / 'selector-original-slot'
        self.live.write_text('#!/bin/sh\nset -eu\n[ "$#" = 1 ] && [ "$1" = --library ] && [ "${WEREAD_API_KEY-unset}:${WEREAD_STARTUP_TIMING-unset}" = unset:unset ] || exit 1\nprintf "%s\\n" "$WEREAD_HOME" >"$WEREAD_HOME/../fixture-staged-entry"\n')
        self.live.chmod(0o755)
        stamp = self.live.stat()
        self.base.baseline[self.live] = (stamp.st_ino, stamp.st_size, stamp.st_mtime_ns)
        self.reference = self.daily.read_bytes() + self.live.read_bytes()
        self.boot = self.root / 'public-registration'
        self.boot.write_text('public shared launch registration\n')
        self.peer = self.volume / 'documents/Aidoku.sh'
        orientation = (daily.HERE / '../kindle-font-trial/orientation-device.sh').read_text()
        self.runtime = re.search(r'^runtime=(\S+)', orientation, re.M)[1]
        declared = orientation.split('for file in "$daily" ', 1)[1].split('; do', 1)[0].replace('\\\n', ' ')
        for named in shlex.split(declared) + ['/var/local/appreg.db']:
            path = self.root / named.replace('$runtime', self.runtime).lstrip('/')
            path.parent.mkdir(exist_ok=True, parents=True)
            if not path.exists(): path.write_bytes(b'public fixture\n')
        for match in re.finditer(r'file_identity (/mnt/us/bilicomics/\S+) (\d+)', orientation):
            path = self.root / match[1].lstrip('/')
            path.parent.mkdir(exist_ok=True, parents=True)
            with path.open('wb') as output: output.truncate(int(match[2]))
        helper = self.root / 'selector-bind'
        subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-O2','-DBIND_TEST_USERSTORE='+json.dumps(str(self.volume)),
                        '-DBIND_TEST_TMP='+json.dumps(str(self.fixed.parent)),daily.HERE/'launcher_file_bind.c','-o',helper], check=True)
        observer = self.root / 'render-observer'
        rendered = (daily.HERE / 'render-observer.c').read_text().replace('/var/tmp/', str(self.fixed.parent) + '/')
        native = self.root / 'observer.c'; native.write_text(rendered)
        subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-O2','-I',daily.HERE,native,'-o',observer], check=True)
        self.seed = {'device-profile':b'scribe\n','device-allowlist':b'scribe 5.19.5\n','library.sqlite':b'public shelf\n',
                     'images/cache-index.sqlite3':b'public image cache','covers/cache-index.sqlite3':b'public cover cache',
                     f'images/{"a"*64}-{"b"*64}.png':b'public image'} | {
                     f'covers/{index:064x}-{"b"*64}.png':f'public cover {index}'.encode() for index in range(15)}
        self.items = {name:body for name, body, _ in self.base.entries()} | {
            'arm/app/weread':b'isolated measurement application', 'arm/launch.sh':self.live.read_bytes(),
            'document-reference':self.daily.read_bytes(), 'launch-reference':self.live.read_bytes(),
            'selector-bind':helper.read_bytes(), 'render-observer':observer.read_bytes(),
            'arm/app/weread-frame-guard':(self.home/'app/weread-frame-guard').read_bytes(),
            'arm/app/weread-native-query':(self.home/'app/weread-native-query').read_bytes()}
        self.rebuild()

    def clear_fixture_mounts(self):
        for path in (self.live,self.volume):
            while any(row.split()[4] == str(path) for row in Path('/proc/self/mountinfo').read_text().splitlines()):
                subprocess.run(['umount',path],check=True)

    def rebuild(self, *, tile_script=None, entries=None, seed_entries=None, lane='A'):
        if tile_script is not None:
            self.items['tile/native_tile.sh'] = tile_script.encode('ascii') if isinstance(tile_script, str) else tile_script
        seed = seed_entries if seed_entries is not None else [(name,body,0o644) for name,body in self.seed.items()]
        self.items['arm-seed.tar.gz'] = contents(seed, inner=True)
        rows = [(name,body,0o644 if name in ('arm-seed.tar.gz','launch-reference','document-reference') or name.startswith('tile/models/') else 0o755)
                for name,body in self.items.items()]
        self.payload = contents(entries if entries is not None else rows)
        config = dict(label=daily.LABEL, lane=lane, selector_strategy='single-file-bind-v1', claim_protocol='independent-stock-app-v1', members={name:len(body) for name,body in self.items.items()},
                      seed_members={name:len(body) for name,body in self.seed.items()}, archive_bytes=len(self.payload))
        self.config = config
        script = render_device(config)
        overrides = r'/mnt/us|/sys/class/input|/proc(?=\n|/usid\b)|/tmp(?=\))|/var/(?:local|tmp)|/etc/prettyversion\.txt'
        script = re.sub(overrides, lambda match:str(self.root) + match[0],script)
        mocks = daily.MOCKS.replace('@ROOT@', shlex.quote(str(self.root))).replace('@ZEROS@','0 '*17)
        mocks = mocks.replace('for stub in @ENTRY@ ', 'for stub in "$(cat "$fixture/mock-entry-pid")" ')
        for name in ('ENTRY','MASTER','RUNNER','GUI','MANAGER'):
            mocks = mocks.replace('@'+name+'@',str(getattr(daily,name)))
        script = script.replace('\nretired() {', '\nfixture_retired() {')
        mocks += '\nretired() ( case "$1" in "$(cat "$fixture/mock-entry-pid" 2>/dev/null || :)"|' + '|'.join(map(str,(daily.MASTER,daily.RUNNER,daily.GUI))) + ') ;; *) proc=/proc;; esac; fixture_retired "$@"; )\n'
        self.program.write_text(script.replace('lipc-get-prop','fixture_lipc').replace(daily.MAIN,mocks+'\n'+MOCK_SQL+'\n'+daily.MAIN))

    def run_mode(self, mode, data=b''):
        return subprocess.run([*self.device_shell, self.program, mode, daily.LABEL],input=data,capture_output=True,timeout=30)

    def call(self, mode, state=None, *, data=b'', refused=None):
        result = self.run_mode(mode,data)
        terminal = LOAD_CLOSE_RECEIPTS[mode]+daily.LABEL.encode('ascii')+b'\n' if mode in LOAD_CLOSE_RECEIPTS else b''
        if refused:
            self.assertEqual((result.returncode,result.stdout,result.stderr),(1,b'',f'refused {refused}\n'.encode()+terminal),mode); return
        self.assertEqual((result.returncode,result.stderr),(0,terminal),(mode,result.stdout,result.stderr))
        receipt = json.loads(result.stdout)
        if state: self.assertEqual(receipt.get('state'),state,mode)
        return receipt

    def ready(self, *, activate=True):
        self.call('admit','admitted'); self.call('stage','staged'); self.call('install','installed',data=self.payload)
        original = self.call('original','original'); self.call('audit','audited',data=self.reference); self.call('prepare','prepared')
        if activate:
            self.call('activate','activated'); self.call('arm','armed')
        return original

    def enter(self, *args):
        return subprocess.run([*self.device_shell,self.live,*(args or ('--library',))],capture_output=True,timeout=5,env=os.environ | {
            'WEREAD_HOME':str(self.home),'WEREAD_API_KEY':'synthetic','WEREAD_STARTUP_TIMING':'synthetic'})

    def chain(self, **kwargs):
        original = self.base.home,self.base.app,self.base.process
        owner,entry,birth=(self.stage/'selector-callback').read_text().strip().split(':')
        self.assertEqual(owner,daily.LABEL); entry=int(entry); offset=int(birth)-110
        (self.root/'mock-entry-pid').write_text(f'{entry}\n')
        def process(pid,parent,started,*args,**options):
            original[2](entry if pid==daily.ENTRY else pid,entry if parent==daily.ENTRY else parent,started+offset,*args,**options)
        try:
            self.base.home,self.base.app,self.base.process = self.stage/'arm',self.stage/'arm/app/weread',process
            self.base.chain(**kwargs)
            if kwargs.get('generation',True):
                (self.lock/'application-birth').write_text(f'{125+offset}\n')
                (self.lock/'application-owned').write_text(f'1:{daily.NONCE}:{daily.GUI}:{125+offset}\n')
        finally:
            self.base.home,self.base.app,self.base.process = original

    def handback(self, previous=None):
        self.call('stop','stopped'); self.call('restore','restored'); self.call('close','closed')
        observed = self.call('absent','absent'); self.call('audit','audited',data=self.reference)
        if previous is not None: self.assertEqual(observed,previous|{'state':'absent'})
        self.assertFalse(self.stage.exists()); self.assertFalse(self.fixed.exists()); self.assertFalse(self.authority.exists())

    def tag(self, name):
        (self.stage/name).write_text(daily.LABEL+'\n')

    def prepare_authority(self):
        self.tag('bind-intent'); self.authority.mkdir(mode=0o700); self.authority.chmod(0o700)
        owner=self.authority/'owner'; owner.write_text(daily.LABEL+'\n'); owner.chmod(0o600)
        for record,path in [('bind-directory',self.authority),('bind-owner',owner)]:
            identity=path.stat(); (self.stage/record).write_text(f'{identity.st_dev}:{identity.st_ino}\n')

    def helper(self, mode):
        return subprocess.run([self.stage/'selector-bind',mode,'stock',daily.LABEL],check=True,capture_output=True).stdout

    def open_channel(self):
        child = subprocess.Popen([*self.device_shell,self.program,'request',daily.LABEL],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        def finish():
            if child.poll() is None: child.communicate(b'cancel\n',timeout=4)
        self.addCleanup(finish)
        self.assertTrue(select.select([child.stdout],[],[],6)[0]); line = child.stdout.readline()
        self.assertTrue(line,child.communicate() if not line else None)
        return child,json.loads(line)


class DeviceTests(StockDeviceFixture):
    def test_preinput_cross_tab_refusal_returns_without_navigation(self):
        tab, pointer, view = (self.root/name for name in ('tab','pointer','active-view'))
        tab.write_text('library\n'); view.write_text('KPP_LIBRARY\n')
        self.call('stage','staged'); self.call('install','installed',data=self.payload)
        original=self.call('original','original'); self.call('audit','audited',data=self.reference)
        tab.write_text('home\n'); view.write_text('KPP_HOME\n'); pointer.write_text('75 91\n')
        self.call('prepare',refused='native_view_changed')
        self.assertEqual((self.stage/'prepare-view-refused').read_text(),daily.LABEL+'\n')
        self.assertFalse((self.stage/'stock-original').exists()); self.assertFalse((self.stage/'prepare-started').exists())
        self.call('prepare',refused='preparation')
        self.handback(original)
        self.assertEqual((tab.read_text(), pointer.read_text(), view.read_text()),('home\n','75 91\n','KPP_HOME\n'))
        self.assertFalse(self.base.signals())

    def test_preinput_view_changes_during_read_only_scene_probe(self):
        script = self.items['tile/native_tile.sh'].decode('ascii')
        script = script.replace('inspect) inspect;;', '''inspect)
        inspect
        printf 'KPP_LIBRARY\\n' >"$root/active-view"
        printf 'library\\n' >"$root/tab";;''')
        self.assertNotEqual(script.encode('ascii'),self.items['tile/native_tile.sh'])
        self.rebuild(tile_script=script)
        self.call('stage','staged'); self.call('install','installed',data=self.payload)
        self.call('prepare',refused='native_view_changed')
        self.assertFalse((self.stage/'stock-original').exists())
        self.handback()
        self.assertEqual((self.root/'tab').read_text(),'library\n')

    def test_preinput_return_needs_its_own_tag_stable_scene_and_pristine_scope(self):
        active, tab, pointer = (self.root/name for name in ('active-view','tab','pointer'))
        self.call('stage','staged'); self.call('install','installed',data=self.payload)
        active.write_text('KPP_LIBRARY\n')
        self.call('prepare',refused='native_view_changed'); self.call('stop','stopped')
        self.call('restore',refused='native_restore')  # Live firmware view contradicts OCR.
        tab.write_text('library\n')
        for leaf in ('prepare-view-refused','stock-original','tile-gate','bind-intent','observer-active','input.tgz'):
            with self.subTest(leaf=leaf):
                file = self.stage/leaf
                if leaf == 'prepare-view-refused':
                    file.write_text('ffffffffffffffff\n')
                else:
                    file.touch()
                self.call('restore',refused='native_restore')
                self.assertFalse((self.stage/'restored').exists())
                file.unlink()
                if leaf == 'prepare-view-refused': self.tag(leaf)
        self.call('restore','restored')
        initial = pointer.read_bytes(); pointer.write_text('19 31\n')
        self.call('close',refused='closeout'); self.assertTrue(self.stage.is_dir())
        pointer.write_bytes(initial)
        self.handback()
        self.assertEqual(tab.read_text(),'library\n')

    def test_complete_original_entry_and_same_session_request(self):
        with self.live.open('rb') as underlying:
            previous = self.ready()
            self.assertEqual(os.fstat(underlying.fileno()).st_ino,int(previous['launch'].split(':')[1]))
            self.assertEqual(underlying.read(),self.items['launch-reference'])
        for proc_view in ('self','1'):
            table=Path('/proc',proc_view,'mountinfo').read_text().splitlines()
            self.assertEqual(sum(row.split()[4]==str(self.live) for row in table),1)
            self.assertTrue(self.selector_slot.samefile(Path('/proc',proc_view,'root')/self.live.relative_to('/')))
        self.assertNotEqual(self.enter().returncode,0)
        self.call('tap',refused='uncertain_or_refused_tap')
        child, report = self.open_channel()
        self.assertEqual(report['state'],'armed'); self.assertFalse((self.stage/'tile-dispatch-intent').exists())
        self.call('stop',refused='unproven_close'); receipt = self.call('tap')
        self.assertNotEqual(self.enter('--offline').returncode,0)
        self.assertEqual(self.enter().returncode,0)
        self.assertEqual((self.stage/'selector-service-view').read_text(),daily.LABEL+'\n')
        self.assertEqual((self.stage/'fixture-staged-entry').read_text(),str(self.stage/'arm')+'\n')
        self.chain(); self.call('claim','claimed'); self.call('witness','witnessed')
        marker = f'WEREAD_LIBRARY_SUBMITTED {daily.LABEL} {time.monotonic_ns()} entries\n'
        (self.fixed/'request').write_text(marker)
        stdout,stderr = child.communicate(b'collect\n',timeout=6)
        self.assertEqual((child.returncode,stderr),(0,b'')); result = json.loads(stdout)
        self.assertEqual((result['marker'],result['tile_receipt']),(marker,receipt))
        self.handback(previous); self.assertEqual(self.base.signals(),[f'-TERM {daily.MASTER}'])

    def test_interrupt_after_bind_before_activation_record(self):
        previous = self.ready(activate=False)
        self.prepare_authority(); self.assertEqual(self.helper('bind'),b'bound\n')
        self.assertFalse((self.stage/'selector-live').exists()); self.handback(previous)

    def test_interrupt_before_authority_never_binds(self):
        previous = self.ready(activate=False)
        self.tag('bind-intent')
        self.handback(previous)

    def test_lost_restore_ack_and_repeated_restore_never_bind(self):
        previous = self.ready(); self.call('stop','stopped')
        self.assertEqual(self.helper('restore'),b'bind_restored\n'); self.assertFalse(self.live.samefile(self.selector_slot))
        self.assertTrue((self.authority/'bind-active').is_file()); self.call('restore','restored')
        self.assertFalse(self.authority.exists()); self.assertFalse(self.live.samefile(self.selector_slot))
        self.handback(previous)

    def test_foreign_stacked_mount_and_authority_object_are_never_removed(self):
        previous = self.ready(); self.call('stop','stopped'); foreign=self.root/'foreign-file'; foreign.write_text('retain\n')
        subprocess.run(['mount','--bind',foreign,self.live],check=True)
        self.call('restore',refused='stage_owner_or_busy'); self.assertTrue(foreign.samefile(self.live)); self.assertTrue(self.authority.exists())
        subprocess.run(['umount',self.live],check=True)
        unknown=self.authority/'unknown'; unknown.symlink_to(foreign)
        self.call('restore',refused='native_restore'); self.assertTrue(unknown.is_symlink()); self.assertEqual(foreign.read_text(),'retain\n')
        unknown.unlink(); draft=self.authority/'bind-journal.next'; draft.write_text('bind-v1 stock '+daily.LABEL+' invalid\n'); draft.chmod(0o600)
        self.call('restore',refused='native_restore'); self.assertTrue(draft.exists()); draft.unlink(); self.handback(previous)

    def test_unchanged_metadata_with_a_foreign_mount_is_not_an_independent_absence(self):
        self.call('absent','absent'); subprocess.run(['mount','--bind',self.live,self.live],check=True)
        self.call('absent',refused='absent'); self.call('audit',data=self.reference,refused='production_byte_audit')
        subprocess.run(['umount',self.live],check=True); self.call('absent','absent')

    def test_missing_or_old_strategy_and_exchange_members_are_refused(self):
        for protocol in (None, 'shell-marker-only'):
            with self.subTest(protocol=protocol), self.assertRaises(ValueError):
                render_device(self.config | {'claim_protocol':protocol})
        for strategy in (None,'atomic-exchange'):
            with self.assertRaises(ValueError): render_device(self.config|{'selector_strategy':strategy})
        with self.assertRaises(ValueError): render_device(self.config|{'members':self.config['members']|{'selector-exchange':42}})

    def test_another_service_view_declines_then_quarantines_without_sample_or_signal(self):
        previous = self.ready(); child,_ = self.open_channel(); self.call('tap')
        result=subprocess.run(['unshare','--mount','--propagation','private',*self.device_shell,self.live,'--library'],capture_output=True,timeout=8)
        self.assertNotEqual(result.returncode,0); self.assertFalse((self.stage/'selector-service-view').exists())
        self.assertFalse((self.stage/'selector-entered').exists()); self.assertTrue((self.stage/'selector-declined').is_file())
        self.call('witness',refused='scene_identity'); self.assertEqual(child.communicate(b'cancel\n',timeout=6),(b'',b''))
        self.call('stop','stopped'); self.assertTrue((self.stage/'quarantined').is_file()); self.assertFalse(self.base.signals())
        self.handback(previous)

    def test_lookalike_group_or_missing_service_witness_never_claims_a_gui(self):
        previous = self.ready(); child,_ = self.open_channel(); self.call('tap'); self.assertEqual(self.enter().returncode,0)
        self.chain(); callback=self.stage/'selector-callback'; accepted=callback.read_text()
        for name,value in [('selector-callback',daily.LABEL+':871100:9\n'),('selector-service-view','foreign\n')]:
            record=self.stage/name; saved=record.read_text(); record.write_text(value)
            self.call('claim',refused='launch_identity'); self.call('witness',refused='scene_identity')
            self.assertFalse((self.stage/'group').exists()); self.assertFalse((self.stage/'session').exists()); record.write_text(saved)
        self.assertEqual(callback.read_text(),accepted); self.call('claim','claimed')
        self.assertEqual(child.communicate(b'cancel\n',timeout=5),(b'',b'')); self.handback(previous)

    def test_malformed_seed_and_linked_archive_stop_before_navigation(self):
        rows = [(name,body,0o644) for name,body in self.seed.items()] + [('account-private',b'no',0o644)]
        self.rebuild(seed_entries=rows); self.call('stage','staged'); self.call('install',data=self.payload,refused='public_seed')
        self.assertFalse((self.stage/'tile-original').exists()); self.handback()
        self.rebuild(entries=[('tile/card-input','/etc/passwd',0o755)])
        self.call('stage','staged'); self.call('install',data=self.payload,refused='install_outer_archive'); self.handback()

    def test_receive_requires_exact_eof_and_a_working_remote_deadline(self):
        previous = self.call('absent','absent')
        for extra, reason in ((b'', 'install_archive_short'), (b'\x00', 'install_archive_long')):
            with self.subTest(reason=reason):
                self.call('stage','staged')
                self.call('install',data=self.payload+extra if extra else self.payload[:-1],refused=reason)
                self.assertFalse((self.stage/'unpack').exists())
                self.assertFalse((self.stage/'installed').exists())
                self.handback(previous)
        body = self.program.read_text('ascii')
        self.assertEqual(body.count('exec timeout -s KILL 150 sh -c'),1)
        self.program.write_text(body.replace('exec timeout -s KILL 150 sh -c','exec timeout -s KILL 2 sh -c'),encoding='ascii')
        self.call('stage','staged')
        terminal=LOAD_CLOSE_RECEIPTS['install']+daily.LABEL.encode('ascii')+b'\n'
        with subprocess.Popen([*self.device_shell,self.program,'install',daily.LABEL],stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE,stderr=subprocess.PIPE,bufsize=0) as child:
            try:
                child.stdin.write(self.payload)
                child.stdin.flush()
                # The sender stays open through the real receive watchdog exit.
                self.assertEqual(child.wait(timeout=10),1)
                self.assertFalse(child.stdin.closed)
                self.assertEqual(child.stdout.read(),b'')
                self.assertEqual(child.stderr.read(),b'refused install_receive_timeout\n'+terminal)
            finally:
                if child.poll() is None:
                    child.kill(); child.wait(timeout=4)
        for leaf in ('outer-manifest','seed-manifest','unpack','installed','bind-intent','tile-dispatch-intent'):
            self.assertFalse((self.stage/leaf).exists(),leaf)
        self.assertFalse((self.stage/'input.tgz').exists(), 'Timed-out input pathname was not revoked')
        self.handback(previous)

    def test_supervisor_exit_is_not_retirement_while_the_reader_owns_its_lock(self):
        previous = self.call('absent','absent')
        self.call('stage','staged')
        body = self.program.read_text('ascii')
        supervisor = self.root/'fixture-supervisor'
        supervisor.write_text(f'''#!/bin/sh
limit=60
while [ ! -f {shlex.quote(str(self.root/'fake-supervisor-return'))} ]; do
    [ "$limit" -gt 0 ] || exit 77
    limit=$((limit-1)); sleep 0.05
done
exit 124
''',encoding='ascii')
        supervisor.chmod(0o700)
        anchor='exec timeout -s KILL 150 sh -c'
        self.assertEqual(body.count(anchor),1)
        self.program.write_text(body.replace(anchor,'exec '+shlex.quote(str(supervisor))+' sh -c'),encoding='ascii')
        with subprocess.Popen([*self.device_shell,self.program,'install',daily.LABEL],stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE,stderr=subprocess.PIPE) as child:
            archive=self.stage/'input.tgz'
            end=time.monotonic()+6
            while not archive.is_file() and time.monotonic()<end and child.poll() is None:
                time.sleep(0.02)
            self.assertTrue(archive.is_file())
            with archive.open('rb') as simulated_reader:
                fcntl.flock(simulated_reader,fcntl.LOCK_EX|fcntl.LOCK_NB)
                (self.root/'fake-supervisor-return').touch()
                out,err=child.communicate(timeout=6)
                self.assertEqual((child.returncode,out,err),(1,b'',b'refused install_receiver_unconfirmed\n'))
                self.assertFalse((self.stage/'unpack').exists())
                self.assertFalse((self.stage/'installed').exists())
        self.handback(previous)

    def test_delayed_reader_cannot_recreate_or_write_revoked_archive(self):
        previous = self.call('absent','absent')
        self.call('stage','staged')
        body = self.program.read_text('ascii')
        runner = self.root/'fixture-abandoned-reader'
        runner.write_text(f'''#!/bin/sh
"$@" </dev/null >/dev/null 2>/dev/null &
for attempt in $(seq 1 60); do
    [ ! -f {shlex.quote(str(self.root/'reader-ready'))} ] || exit 124
    sleep 0.05
done
exit 77
''',encoding='ascii')
        runner.chmod(0o700)
        fifo = self.root/'release-reader'
        os.mkfifo(fifo)
        # An RDWR handle makes release in cleanup bounded even on failure.
        with os.fdopen(os.open(fifo,os.O_RDWR | os.O_NONBLOCK),'wb',buffering=0) as control:
            anchor = 'exec timeout -s KILL 150 sh -c'
            open_reader = '        exec 6<"$1" || exit 76\n'
            self.assertEqual(body.count(anchor),1)
            self.assertEqual(body.count(open_reader),1)
            fixture = body.replace(anchor,'exec '+shlex.quote(str(runner))+' sh -c').replace(open_reader, open_reader +
                f'        printf ready >"{self.root}/reader-ready"\n'
                f'        read -r go <"{fifo}" || exit 79\n'
                f'        trap "stat -Lc %s /proc/self/fd/6 >{self.root}/reader-size" EXIT\n')
            self.program.write_text(fixture,encoding='ascii')
            archive=self.stage/'input.tgz'
            try:
                self.call('install',refused='install_receive_timeout')
                self.assertFalse(archive.exists())
            finally:
                control.write(b'go\n')
            deadline=time.monotonic()+5
            measured=self.root/'reader-size'
            while (not measured.exists() or not measured.stat().st_size) and time.monotonic()<deadline:
                time.sleep(0.02)
            self.assertEqual(measured.read_bytes(),b'0\n')
            self.assertFalse(archive.exists())
        self.handback(previous)

    def test_audit_rejects_truncation_trailing_bytes_mutation_and_foreign_request(self):
        for wrong in (self.reference[:-1],self.reference+b'\x00',self.reference.replace(b'public wrapper',b'public swapped')):
            self.call('audit',data=wrong,refused='production_byte_audit')
        self.call('audit','audited',data=self.reference); self.fixed.mkdir(); foreign=self.fixed/'foreign'; foreign.write_text('retain')
        self.call('admit',refused='stock_install_or_request'); self.call('absent',refused='absent'); self.assertEqual(foreign.read_text(),'retain')

    def test_peer_snapshot_and_unexpected_mounts_are_not_repaired(self):
        previous = self.ready(activate=False)
        mount = self.root/'proc/mounts'; before = mount.read_text()
        mount.write_text(before+f'foreign {self.live} none rw,bind 0 0\n')
        self.call('activate',refused='stage_owner_or_busy'); mount.write_text(before)
        registered = self.boot.read_bytes(); self.boot.write_text('foreign registration\n')
        self.call('activate',refused='selector_activation'); self.call('stop','stopped'); self.call('restore',refused='native_restore')
        self.assertEqual(self.boot.read_bytes(),b'foreign registration\n'); self.boot.write_bytes(registered); self.handback(previous)

    def test_only_native_document_time_changes_are_accepted_without_rewriting_them(self):
        previous = self.ready(activate=False)
        stamp = self.daily.stat()
        os.utime(self.daily,ns=(stamp.st_atime_ns,stamp.st_mtime_ns+3_000_000_000))
        changed = self.daily.stat()
        self.base.baseline[self.daily] = (changed.st_ino,changed.st_size,changed.st_mtime_ns)
        self.assertEqual(len(previous['document'].split(':')),7)
        self.assertEqual([len(previous[key].split(':')) for key in ('launch','application')],[8,8])
        self.handback(previous)
        self.assertEqual(self.daily.stat().st_mtime_ns,changed.st_mtime_ns)

    def test_same_size_edits_replacement_inodes_and_ownership_changes_are_refused(self):
        previous = self.ready(activate=False); original = self.daily.read_bytes(); stamp = self.daily.stat()
        self.call('stop','stopped')
        edited = original.replace(b'public wrapper fixture',b'public wrapper changed')
        self.assertEqual(len(edited),len(original)); self.assertNotEqual(edited,original)
        self.daily.write_bytes(edited); os.utime(self.daily,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
        self.call('restore',refused='native_restore'); self.assertEqual(self.daily.read_bytes(),edited)
        self.daily.write_bytes(original); os.utime(self.daily,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
        previous_file = self.volume/'original-document'; self.daily.rename(previous_file); self.daily.write_bytes(original)
        self.daily.chmod(stamp.st_mode & 0o777); os.utime(self.daily,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
        self.call('restore',refused='native_restore'); self.daily.unlink(); previous_file.rename(self.daily)
        os.chown(self.daily,10,10)
        self.call('restore',refused='native_restore'); self.assertEqual(self.daily.stat().st_uid,10)
        os.chown(self.daily,stamp.st_uid,stamp.st_gid); self.handback(previous)

    def test_cancel_then_restore_refuses_orphan_intent(self):
        self.ready(); child, _ = self.open_channel()
        self.assertEqual(child.communicate(b'cancel\n',timeout=5),(b'',b'')); self.assertFalse(self.fixed.exists())
        (self.stage/'tile-dispatch-intent').write_text(daily.LABEL+'\n'); self.call('stop',refused='unproven_close')
        self.assertFalse((self.stage/'stopped').exists()); self.assertFalse(self.base.signals())
        (self.stage/'tile-dispatch-intent').unlink(); self.handback()


if __name__ == '__main__':
    unittest.main()
