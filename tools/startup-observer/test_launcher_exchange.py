"""Exercise the exchange-only primitive on a private worker filesystem."""

import json
import os
from pathlib import Path
import shlex
import shutil
import stat
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).resolve().with_name('launcher_exchange.c')
LABEL = '0123456789abcdef'


def stamp(path):
    info = path.stat()
    return info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode, info.st_nlink, info.st_size, info.st_mtime_ns


class ExchangeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.workspace.cleanup)
        filesystem = os.environ.get('WEREAD_EXCHANGE_TEST_FS')
        cls.store = tempfile.TemporaryDirectory(dir=filesystem)
        cls.addClassCleanup(cls.store.cleanup)
        cls.base = Path(cls.store.name).resolve()
        cls.binary = Path(cls.workspace.name) / 'exchange'
        compiler = shlex.split(os.environ.get('CC', 'cc'))
        subprocess.run([*compiler, '-std=c11', '-Wall', '-Wextra', '-Werror', '-O2',
                        '-DEXCHANGE_TEST_USERSTORE=' + json.dumps(str(cls.base)), SOURCE, '-o', cls.binary], check=True)
        cls.frontend = shlex.split(os.environ.get('WEREAD_EXCHANGE_TEST_EXEC', ''))

    def setUp(self):
        self.home = self.base / 'weread'
        self.stage = self.base / ('.weread-render-' + LABEL)
        for path in (self.home, self.stage):
            path.mkdir()
            self.addCleanup(shutil.rmtree, path)
        self.owner = self.stage / 'owner'
        self.owner.write_bytes((LABEL + '\n').encode())
        self.live = self.home / 'launch.sh'
        self.stash = self.stage / 'selector-original-slot'
        self.live.write_bytes(b'original\n')
        self.stash.write_bytes(b'selector\n')
        self.live.chmod(0o755)
        self.stash.chmod(0o700)

    def invoke(self, *args):
        return subprocess.run([*self.frontend, self.binary, *args], capture_output=True, text=True, timeout=8)

    def test_probe_and_round_trip(self):
        old, replacement = stamp(self.live), stamp(self.stash)
        request = self.invoke('probe', LABEL)
        if os.environ.get('WEREAD_EXCHANGE_EXPECT_REFUSAL') == '1':
            self.assertNotEqual(request.returncode, 0)
            self.assertEqual(stamp(self.live), old)
            self.assertEqual(stamp(self.stash), replacement)
        else:
            self.assertEqual((request.returncode, request.stdout), (0, 'exchange_supported\n'), request.stderr)
            self.assertEqual(stamp(self.live), old)
            for active, parked, content in [(replacement, old, b'selector\n'), (old, replacement, b'original\n')]:
                request = self.invoke('swap', LABEL)
                self.assertEqual((request.returncode, request.stdout), (0, 'exchanged\n'), request.stderr)
                self.assertEqual(stamp(self.live), active)
                self.assertEqual(stamp(self.stash), parked)
                self.assertEqual(self.live.read_bytes(), content)
        self.assertEqual(sorted(item.name for item in self.stage.iterdir()), ['owner', 'selector-original-slot'])

    def test_bad_owner_and_unsupported_mode_are_read_only(self):
        old = stamp(self.live)
        self.assertEqual(self.invoke('--policy').stdout, 'selector-exchange-v1\n')
        for args in [('activate', LABEL), ('swap', LABEL.upper()), ('swap', LABEL, str(self.live))]:
            self.assertEqual(self.invoke(*args).returncode, 64)
        for content in [b'', b'123456789abcdef0\n', LABEL.encode(), (LABEL + '\n\n').encode()]:
            self.owner.write_bytes(content)
            for mode in ('probe', 'swap'):
                self.assertNotEqual(self.invoke(mode, LABEL).returncode, 0)
                self.assertEqual(stamp(self.live), old)

    def test_no_fallback_for_a_missing_or_linked_slot(self):
        old = stamp(self.live)
        self.stash.unlink()
        self.assertNotEqual(self.invoke('swap', LABEL).returncode, 0)
        self.assertEqual(stamp(self.live), old)
        self.stash.mkdir()
        self.assertNotEqual(self.invoke('swap', LABEL).returncode, 0)
        self.stash.rmdir()
        try:
            self.stash.symlink_to(self.live)
        except OSError:
            return
        self.assertNotEqual(self.invoke('swap', LABEL).returncode, 0)
        self.assertEqual(stamp(self.live), old)
        self.stash.unlink()
        self.stash.hardlink_to(self.live)
        self.assertNotEqual(self.invoke('swap', LABEL).returncode, 0)
        self.stash.unlink()
        self.assertEqual(stamp(self.live), old)

    def test_probe_preserves_foreign_collisions(self):
        old = stamp(self.live)
        foreign = self.stage / 'exchange-probe-left'
        foreign.write_bytes(b'not ours')
        self.assertNotEqual(self.invoke('probe', LABEL).returncode, 0)
        self.assertEqual(foreign.read_bytes(), b'not ours')
        self.assertFalse((self.stage / 'exchange-probe-parent').exists())
        foreign.unlink()
        nested = self.stage / 'exchange-probe-parent'
        nested.mkdir()
        (nested / 'foreign').write_text('retain')
        self.assertNotEqual(self.invoke('probe', LABEL).returncode, 0)
        self.assertTrue(stat.S_ISREG((nested / 'foreign').stat().st_mode))
        self.assertEqual(stamp(self.live), old)


if __name__ == '__main__':
    unittest.main()
