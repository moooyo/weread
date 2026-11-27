"""Exercise the shared lock on private fixture files, without device access."""

import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import device_lock


LOAD = '''
import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import device_lock
device_lock._LOCK_PATH = Path(sys.argv[2])
'''


class OperationLockTests(unittest.TestCase):
    def setUp(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.path = root / 'authority' / 'device.lock'
        self.path.parent.mkdir(mode=0o700)
        self.path.touch(mode=0o600)
        self.enterContext(patch.object(device_lock, '_LOCK_PATH', self.path))

    def start_operator(self, body):
        process = subprocess.Popen([sys.executable, '-I', '-B', '-c', LOAD + body,
                                    str(Path(device_lock.__file__).resolve().parent), str(self.path)],
                                   cwd='/', stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)

        @self.addCleanup
        def stop_operator():
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
            process.stdin.close()
            process.stdout.close()

        return process

    def read_event(self, process):
        self.assertTrue(select.select([process.stdout], [], [], 5)[0], 'Silent operator fixture')
        return process.stdout.readline()

    def test_independent_operator_can_hold_two_nonoverlapping_visits(self):
        operator = self.start_operator('''
for slot in range(2):
    sys.stdin.buffer.read(1)
    with device_lock.operation_lock():
        print('acquired', flush=True)
        sys.stdin.buffer.read(1)
        print('cleanup', flush=True)
        sys.stdin.buffer.read(1)
    print('returned', flush=True)
''')
        for _ in range(2):
            with device_lock.operation_lock():
                pass
            for event in (b'acquired\n', b'cleanup\n', b'returned\n'):
                operator.stdin.write(b'x')
                self.assertEqual(self.read_event(operator), event)
                if event != b'returned\n':
                    with self.assertRaises(device_lock.DeviceBusy), device_lock.operation_lock():
                        self.fail('Another operator admitted before handback')
        self.assertEqual(operator.wait(timeout=5), 0)

    def test_body_errors_release_without_changing_their_type(self):
        with self.assertRaisesRegex(OSError, 'device fixture'), device_lock.operation_lock():
            raise OSError('device fixture')
        with device_lock.operation_lock():
            with self.assertRaises(device_lock.DeviceBusy), device_lock.operation_lock():
                self.fail('A nested acquisition must not join its own lock')

    def test_killed_operator_does_not_leave_the_lock_with_its_python_fork(self):
        self.assert_kill_releases_with_live_child('''
import os
import signal
with device_lock.operation_lock():
    if os.fork() == 0:
        print('child-ready', flush=True)
        sys.stdin.buffer.read(1)
        print('child-finished', flush=True)
        os._exit(0)
    signal.pause()
''')

    def test_killed_operator_does_not_leave_the_lock_with_its_exec(self):
        self.assert_kill_releases_with_live_child('''
import signal
import subprocess
child = "print('child-ready', flush=True); sys.stdin.buffer.read(1); print('child-finished', flush=True)"
with device_lock.operation_lock():
    subprocess.Popen([sys.executable, '-I', '-B', '-c', 'import sys; ' + child], close_fds=False)
    signal.pause()
''')

    def assert_kill_releases_with_live_child(self, body):
        operator = self.start_operator(body)
        self.assertEqual(self.read_event(operator), b'child-ready\n')
        with self.assertRaises(device_lock.DeviceBusy), device_lock.operation_lock():
            self.fail('Live operator did not hold the lock')
        operator.kill()
        self.assertEqual(operator.wait(timeout=5), -signal.SIGKILL)
        with device_lock.operation_lock():
            operator.stdin.write(b'x')
            self.assertEqual(self.read_event(operator), b'child-finished\n')

    def test_refuses_missing_symlink_hard_link_fifo_and_wrong_mode(self):
        other = self.path.with_name('other')
        self.path.rename(other)
        with self.assertRaises(device_lock.DeviceLockError), device_lock.operation_lock():
            self.fail('Implicitly created a missing lock')
        def wrong_mode():
            self.path.touch()
            self.path.chmod(0o666)

        def wrong_owner():
            self.path.touch()
            os.chown(self.path, 65534, -1)

        variants = {'symlink': lambda: self.path.symlink_to(other), 'hard link': lambda: os.link(other, self.path),
                    'FIFO': lambda: os.mkfifo(self.path, 0o600), 'mode': wrong_mode, 'owner': wrong_owner}
        try:
            for name, make in variants.items():
                with self.subTest(name=name):
                    make()
                    try:
                        with self.assertRaises(device_lock.DeviceLockError), device_lock.operation_lock():
                            self.fail('Admitted an unexpected lock file')
                    finally:
                        self.path.unlink()
        finally:
            other.rename(self.path)

    def test_refuses_symlinked_authority_or_wrong_directory_mode(self):
        renamed = self.path.parent.with_name('renamed')
        self.path.parent.rename(renamed)
        self.path.parent.symlink_to(renamed)
        with self.assertRaises(device_lock.DeviceLockError), device_lock.operation_lock():
            self.fail('Followed an authority symlink')
        self.path.parent.unlink()
        renamed.rename(self.path.parent)
        self.path.parent.chmod(0o755)
        with self.assertRaises(device_lock.DeviceLockError), device_lock.operation_lock():
            self.fail('Used an unexpected authority directory mode')

    def test_checks_lock_inode_again_after_acquire(self):
        flock = device_lock.fcntl.flock

        def replace_then_acquire(fd, flags):
            self.path.rename(self.path.with_name('displaced'))
            self.path.touch(mode=0o600)
            flock(fd, flags)

        with patch.object(device_lock.fcntl, 'flock', side_effect=replace_then_acquire):
            with self.assertRaisesRegex(device_lock.DeviceLockError, 'identity changed'), device_lock.operation_lock():
                self.fail('Admitted a lock whose name moved to a different inode')
        with device_lock.operation_lock():
            pass


if __name__ == '__main__':
    unittest.main()
