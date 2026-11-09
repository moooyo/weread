"""Verify passive helper-death notification on the designated Linux environment."""

import ctypes
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time
import unittest


TOKEN = "19c084ace627f09231a004167d914bee"


def birth(pid):
    return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]


def owner_fixture(binary, context, descriptor, helper, helper_birth):
    context = Path(context)
    (context / "pid").write_text(f"{os.getpid()}\n")
    (context / "nonce").write_text(TOKEN + "\n")
    child = subprocess.Popen([binary, "--context", str(context), "--token", TOKEN,
        "--owner", str(os.getpid()), "--owner-birth", birth(os.getpid()),
        "--watch-helper", helper, "--helper-birth", helper_birth,
        "--lifetime-fd", descriptor], pass_fds=(int(descriptor),))
    print(child.pid, flush=True)
    while True:
        signal.pause()


class FrameGuardWatchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux":
            raise RuntimeError("Run frame guard verification only on the designated Linux environment.")
        cls.temporary = tempfile.TemporaryDirectory(prefix="weread-frame-watch-test-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.suite = Path(cls.temporary.name)
        cls.binary = cls.suite / "frame-guard"
        source = Path(__file__).resolve().parents[1] / "kindle/frame_guard.c"
        subprocess.run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
            str(source), "-ldl", "-o", str(cls.binary)],
            check=True, capture_output=True, timeout=30)

    def setUp(self):
        self.root = self.suite / self._testMethodName
        self.root.mkdir(mode=0o700)
        (self.root / "pid").write_text(f"{os.getpid()}\n")
        (self.root / "nonce").write_text(TOKEN + "\n")
        os.mkfifo(self.root / "power-events", 0o600)
        self.events = os.open(self.root / "power-events", os.O_RDWR | os.O_NONBLOCK)
        self.addCleanup(os.close, self.events)
        self.reader, self.writer = os.pipe()
        self.addCleanup(self.close_pipe)
        self.helper = subprocess.Popen(["sleep", "60"], close_fds=True)
        self.addCleanup(self.retire, self.helper)
        self.helper_birth = birth(self.helper.pid)

    @staticmethod
    def retire(process):
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()

    def close_pipe(self):
        for descriptor in (self.reader, self.writer):
            if descriptor is not None:
                os.close(descriptor)
        self.reader = self.writer = None

    def start(self, token=TOKEN, descriptor=None):
        descriptor = self.reader if descriptor is None else descriptor
        child = subprocess.Popen([str(self.binary), "--context", str(self.root),
            "--token", token, "--owner", str(os.getpid()),
            "--owner-birth", birth(os.getpid()), "--watch-helper", str(self.helper.pid),
            "--helper-birth", self.helper_birth, "--lifetime-fd", str(descriptor)],
            pass_fds=(descriptor,), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.addCleanup(self.retire, child)
        return child

    def close_writer(self):
        os.close(self.writer)
        self.writer = None

    def assert_no_event(self):
        self.assertEqual(select.select([self.events], [], [], 0.1)[0], [])

    def test_eof_reports_the_original_helper_identity_without_polling(self):
        watcher = self.start()
        self.assert_no_event()
        self.assertIsNone(watcher.poll())
        self.close_writer()
        self.assertEqual(watcher.wait(timeout=3), 0)
        self.assertEqual(os.read(self.events, 256).decode(),
            f"_weread_frame_guard_exit {TOKEN} {self.helper.pid} {self.helper_birth} 1\n")

    def test_full_power_event_channel_cannot_block_the_watcher(self):
        while True:
            try:
                os.write(self.events, b"x" * 4096)
            except BlockingIOError:
                break
        watcher = self.start()
        self.close_writer()
        self.assertEqual(watcher.wait(timeout=3), 0)

    def test_a_readwrite_lifetime_descriptor_is_rejected(self):
        os.mkfifo(self.root / "wrong-lifetime", 0o600)
        descriptor = os.open(self.root / "wrong-lifetime", os.O_RDWR | os.O_NONBLOCK)
        self.addCleanup(os.close, descriptor)
        watcher = self.start(descriptor=descriptor)
        self.assertEqual(watcher.wait(timeout=3), 1)
        self.assert_no_event()

    def test_a_foreign_nonce_prevents_any_death_notification(self):
        watcher = self.start(token="0" * 32)
        self.close_writer()
        self.assertEqual(watcher.wait(timeout=3), 1)
        self.assert_no_event()

    def test_parent_death_retires_the_watcher_with_a_live_writer(self):
        # Adopt the orphan only to reap it in the Linux test runner. The open
        # writer proves that EOF cannot be responsible for watcher retirement.
        libc = ctypes.CDLL(None, use_errno=True)
        original = ctypes.c_int()
        self.assertEqual(libc.prctl(37, ctypes.byref(original), 0, 0, 0), 0)
        self.assertEqual(libc.prctl(36, 1, 0, 0, 0), 0)
        owner = None
        watcher_pid = None
        reaped = False
        try:
            owner = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                "--fixture-owner", str(self.binary), str(self.root), str(self.reader),
                str(self.helper.pid), self.helper_birth], pass_fds=(self.reader,),
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            readable, _, _ = select.select([owner.stdout], [], [], 3)
            self.assertTrue(readable)
            watcher_pid = int(owner.stdout.readline())
            # The watcher must first enter its read, after owner protection.
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                try:
                    state = Path(f"/proc/{watcher_pid}/wchan").read_text().strip()
                except FileNotFoundError:
                    state = ""
                if "pipe_read" in state:
                    break
                time.sleep(0.01)
            else:
                self.fail("The watcher did not reach its passive lifetime read.")
            owner.kill()
            owner.wait(timeout=3)
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                process, status = os.waitpid(watcher_pid, os.WNOHANG)
                if process:
                    reaped = True
                    self.assertTrue(os.WIFEXITED(status) or os.WIFSIGNALED(status))
                    break
                time.sleep(0.01)
            self.assertTrue(reaped, "Parent death did not retire the passive watcher.")
            self.assertEqual(os.fstat(self.writer).st_mode & 0o170000, 0o010000)
            self.assert_no_event()
        finally:
            if owner is not None:
                self.retire(owner)
            if watcher_pid is not None and not reaped:
                try:
                    os.kill(watcher_pid, signal.SIGKILL)
                    os.waitpid(watcher_pid, 0)
                except (ProcessLookupError, ChildProcessError):
                    pass
            libc.prctl(36, original.value, 0, 0, 0)


if __name__ == "__main__":
    if len(sys.argv) == 7 and sys.argv[1] == "--fixture-owner":
        owner_fixture(*sys.argv[2:])
    else:
        unittest.main(verbosity=2)
