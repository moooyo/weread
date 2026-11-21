"""Real Linux mount tests. Run as PID 1 inside a new private mount/PID namespace on the worker."""

import fcntl
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest


LABEL = '0123456789abcdef'
SOURCE = Path(__file__).resolve().with_name('launcher_file_bind.c')


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, timeout=12)


def stamp(path):
    stat = path.stat()
    return f'{stat.st_dev}:{stat.st_ino}:{stat.st_size}:{stat.st_mode & 0o7777:o}:{stat.st_uid}:{stat.st_gid}:{int(stat.st_mtime)}:{stat.st_nlink}\n'


@unittest.skipUnless(os.environ.get('WEREAD_BIND_FIXTURE') == 'isolated' and os.getpid() == 1 and os.geteuid() == 0,
                     'Requires isolated root mount/PID namespaces; never run against a host or device mount tree')
class FileBindTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        cls.base = Path(cls.temp.name).resolve()
        cls.volume = cls.base / 'userstore'
        cls.private = cls.base / 'authority'
        cls.volume.mkdir(); cls.private.mkdir()
        cls.binary = cls.base / 'selector-bind'
        compiler = shlex.split(os.environ.get('CC', 'cc'))
        cls.front = shlex.split(os.environ.get('WEREAD_BIND_EXEC', ''))
        run(*compiler, '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', f'-DBIND_TEST_USERSTORE="{cls.volume}"',
            f'-DBIND_TEST_TMP="{cls.private}"', SOURCE, '-o', cls.binary)

    def setUp(self):
        self.profile = 'scratch'
        self.label = LABEL
        self.stage = self.volume / ('.weread-bind-diagnostic-' + LABEL)
        self.authority = self.private / ('weread-bind-diagnostic-' + LABEL)
        self.source = self.stage / 'source'
        self.target = self.stage / 'target'
        run('mount', '-t', 'tmpfs', '-o', 'size=2m', 'tmpfs', str(self.volume))
        self.addCleanup(self.discard)
        self.init()

    def discard(self):
        run('umount', '-R', str(self.volume))
        for entry in self.private.iterdir():
            shutil.rmtree(entry)

    def init(self):
        self.stage.mkdir()
        self.authority.mkdir(mode=0o700); self.authority.chmod(0o700)
        for parent in (self.stage, self.authority):
            record = parent / 'owner'; record.write_text(LABEL + '\n'); record.chmod(0o600)
        self.source.write_text(LABEL + ':S\n'); self.target.write_text(LABEL + ':T\n')

    def helper(self, mode, response, *, fail=False):
        observed = subprocess.run([*self.front, self.binary, mode, self.profile, self.label], capture_output=True, text=True, timeout=12)
        self.assertEqual((observed.returncode, observed.stdout, observed.stderr),
                         (1, '', f'refused bind_{response}\n') if fail else (0, response + '\n', ''))

    def check_uncovered(self, original):
        self.helper('absent', 'bind_absent')
        self.assertEqual(stamp(self.target), original)
        self.assertFalse(any(line.split()[4] == str(self.target) for line in Path('/proc/self/mountinfo').read_text().splitlines()))

    def test_scratch_roundtrip_is_visible_in_an_independent_process(self):
        original = stamp(self.target)
        self.helper('prepare', 'bind_prepared')
        self.assertEqual(list(self.authority.iterdir()), [self.authority / 'owner'])
        self.helper('bind', 'bound'); self.helper('status','bound'); self.helper('current','bound')
        self.assertEqual(run('cat', self.target).stdout, (LABEL + ':S\n').encode())
        self.assertTrue(self.source.samefile(self.target))
        self.assertEqual((self.authority / 'bind-journal').stat().st_mode & 0o7777, 0o600)
        (self.authority / 'bind-active').unlink()
        self.helper('current','unknown_state',fail=True)
        self.helper('restore','bind_restored'); self.helper('restore','bind_restored')
        self.check_uncovered(original)
        self.helper('bind','journal',fail=True)

    def test_refuse_foreign_exact_and_stacked_mounts_without_detaching_them(self):
        original = stamp(self.target)
        run('mount','--bind',self.source,self.target)
        self.helper('restore','foreign_mount',fail=True)
        self.assertTrue(self.source.samefile(self.target)); run('umount', self.target)
        self.helper('bind','bound')
        foreign = self.volume/'foreign'; foreign.write_text('not ours')
        run('mount', '--bind', foreign, self.target)
        self.helper('restore','foreign_mount',fail=True)
        self.assertTrue(foreign.samefile(self.target)); run('umount', self.target)
        self.helper('restore','bind_restored'); self.check_uncovered(original)

    def test_wrong_source_replacement_and_foreign_single_layer_are_not_retired(self):
        original = stamp(self.target)
        self.helper('bind','bound')
        saved = self.stage/'moved'; self.source.rename(saved); self.source.write_text(LABEL+':S\n')
        self.helper('restore','provenance',fail=True)
        self.source.unlink(); saved.rename(self.source)
        self.helper('restore','bind_restored'); self.check_uncovered(original)
        foreign = self.volume/'unrelated'; foreign.write_text(LABEL+':T\n')
        run('mount','--bind',foreign,self.target)
        self.helper('restore','foreign_mount',fail=True)
        self.assertTrue(foreign.samefile(self.target)); run('umount', self.target)
        self.check_uncovered(original)

    def test_same_source_replaced_mount_and_new_propagation_are_not_detached(self):
        original=stamp(self.target); self.helper('bind','bound')
        _, ordinary_id, unique_id=(self.authority/'bind-active').read_text().split()
        run('mount','--make-shared',self.target)
        self.helper('restore','propagation',fail=True)
        self.assertTrue(self.source.samefile(self.target));run('mount','--make-private',self.target)
        run('umount',self.target);run('mount','--bind',self.source,self.target)
        mounted_id=next(line.split()[0] for line in Path('/proc/self/mountinfo').read_text().splitlines() if line.split()[4]==str(self.target))
        if int(unique_id) or mounted_id != ordinary_id:
            self.helper('restore','foreign_mount',fail=True)
            self.assertTrue(self.source.samefile(self.target));run('umount',self.target)
        else:
            self.helper('status','bound');run('umount',self.target)
        self.helper('restore','bind_restored');self.check_uncovered(original)

    def test_independent_owner_file_lock_does_not_conflict_but_authority_dir_lock_does(self):
        original=stamp(self.target)
        with (self.authority/'owner').open('rb') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.helper('prepare','bind_prepared');self.helper('bind','bound')
            handle=os.open(self.authority,os.O_DIRECTORY)
            try:
                fcntl.flock(handle,fcntl.LOCK_EX|fcntl.LOCK_NB)
                self.helper('restore','contended',fail=True)
                self.assertTrue(self.target.samefile(self.source))
            finally:
                os.close(handle)
            self.helper('restore','bind_restored');self.check_uncovered(original)

    def test_incomplete_intent_does_not_consent_to_an_existing_mount(self):
        original=stamp(self.target); (self.authority/'bind-journal.next').write_text('partial')
        self.helper('bind','journal',fail=True); self.helper('restore','bind_restored')
        self.check_uncovered(original)
        run('mount','--bind',self.source,self.target)
        self.helper('restore','foreign_mount',fail=True);run('umount',self.target)
        self.check_uncovered(original)

    def test_service_view_difference_refuses_before_any_mount(self):
        reader, writer = os.pipe(); stopped, block = os.pipe()
        pid = os.fork()
        if pid == 0:
            try:
                os.close(reader); os.close(block)
                os.chroot(str(self.private)); os.chdir('/'); os.write(writer,b'1'); os.read(stopped,1)
                os._exit(0)
            except BaseException:
                os._exit(2)
        os.close(writer); os.close(stopped)
        try:
            self.assertEqual(os.read(reader,1),b'1')
            self.helper('prepare','native_view',fail=True)
            self.helper('bind','native_view',fail=True)
            self.assertFalse((self.authority/'bind-journal').exists())
            self.helper('restore','bind_restored'); self.helper('absent','bind_absent')
        finally:
            os.close(reader); os.close(block); os.waitpid(pid,0)

    def test_other_mount_namespace_and_shared_parent_are_not_modified(self):
        with subprocess.Popen(['unshare','--mount','sh','-c','printf ready; read -r unused'],stdin=subprocess.PIPE,stdout=subprocess.PIPE) as service:
            self.assertEqual(service.stdout.read(5),b'ready')
            self.helper('bind','namespace',fail=True)
            self.assertFalse((self.authority/'bind-journal').exists())
            service.communicate(b'bye\n',timeout=5)
        run('mount','--make-shared',self.volume)
        self.helper('prepare','propagation',fail=True)
        self.helper('bind','propagation',fail=True)
        self.assertFalse((self.authority/'bind-journal').exists())
        run('mount','--make-private',self.volume)
        self.helper('absent','bind_absent')

    def test_stock_original_and_selector_match_the_frozen_wrappers(self):
        self.profile='stock'; self.stage=self.volume/('.weread-render-'+LABEL); self.authority=self.private/'weread-stock-selector-bind'
        home=self.volume/'weread'; home.mkdir(); self.target=home/'launch.sh'; self.source=self.stage/'selector-original-slot'
        self.init(); self.source.chmod(0o755); self.target.chmod(0o755)
        original=stamp(self.target); (self.stage/'selector-stat').write_text(stamp(self.source))
        reference=self.stage/'launch-reference'; reference.write_text('wrong')
        self.helper('prepare','files',fail=True); self.assertFalse((self.authority/'bind-journal').exists())
        reference.write_bytes(self.target.read_bytes());self.helper('prepare','bind_prepared');self.helper('bind','bound')
        self.helper('current','bound');self.helper('restore','bind_restored');self.check_uncovered(original)
        shutil.rmtree(self.stage);shutil.rmtree(self.authority);self.check_uncovered(original)


if __name__ == '__main__':
    unittest.main()
