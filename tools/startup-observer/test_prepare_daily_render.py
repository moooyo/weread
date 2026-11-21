"""Exercise the daily-render archive contract without a Kindle or a system install."""

import gzip
import io
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import prepare_daily_render as bundle


def arm_elf():
    header = bytearray(84)
    header[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<HHI", header, 16, 2, 40, 1)
    struct.pack_into("<I", header, 28, 52)
    struct.pack_into("<I", header, 36, 0x05000400)
    struct.pack_into("<HHH", header, 40, 52, 32, 1)
    struct.pack_into("<I", header, 52, 1)
    return bytes(header)


def archive(path, entries, *, format=tarfile.USTAR_FORMAT):
    with tarfile.open(path, "w:gz", format=format) as output:
        for name, data, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.mode = 0o700 if name not in bundle.MODELS else 0o600
            member.size = len(data)
            if kind == tarfile.SYMTYPE:
                member.linkname = "/unused"
            output.addfile(member, io.BytesIO(data))


class PrepareDailyRenderTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.kit = self.root / "tile.tgz"
        self.shell = self.root / "native_tile.sh"
        self.observer = self.root / "render-observer"
        self.output = self.root / "install.tgz"
        self.shell.write_bytes(b"#!/bin/sh\nexit 0\n")
        self.observer.write_bytes(arm_elf())
        entries = []
        for name in sorted(bundle.REQUIRED - {bundle.OBSERVER} | {"tile/lib/libstdc++.so.6"}):
            data = self.shell.read_bytes() if name == bundle.SHELL else b"fixture" if name in bundle.MODELS else arm_elf()
            entries.append((name, data, tarfile.REGTYPE))
        self.entries = entries
        archive(self.kit, self.entries)

    def prepare(self):
        return bundle.prepare(self.kit, self.shell, self.observer, self.output)

    def test_round_trip_has_only_regular_normalized_files(self):
        result = self.prepare()
        self.assertEqual(result, bundle.validate(self.output))
        self.assertEqual(result["members"], len(self.entries) + 1)
        self.assertEqual(result["archive_bytes"], self.output.stat().st_size)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o600)
        self.assertEqual(result["payload_bytes"], sum(len(data) for _, data, _ in self.entries) + len(arm_elf()))
        with tarfile.open(self.output) as prepared:
            for member in prepared:
                self.assertEqual((member.type, member.uid, member.gid), (tarfile.REGTYPE, 0, 0))
                self.assertEqual(member.mode, 0o755 if member.name in bundle.EXECUTABLES else 0o644)
            self.assertEqual(prepared.extractfile(bundle.SHELL).read(), self.shell.read_bytes())
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertEqual(result, bundle.validate(self.output))

    def test_requires_matching_qualified_script(self):
        self.shell.write_bytes(b"#!/bin/sh\nexit 1\n")
        with self.assertRaisesRegex(ValueError, "current source"):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_rejects_missing_or_foreign_members_and_links(self):
        for entries in (self.entries[1:], self.entries + [self.entries[0]],
                        self.entries + [("tile/lib/../native-target", arm_elf(), tarfile.REGTYPE)],
                        self.entries + [(bundle.OBSERVER, arm_elf(), tarfile.REGTYPE)],
                        self.entries + [("tile/lib/libextra.so.1", b"", tarfile.SYMTYPE)],
                        self.entries + [("tile/lib", b"", tarfile.DIRTYPE)]):
            with self.subTest(last=entries[-1][0]):
                archive(self.kit, entries)
                with self.assertRaises(ValueError):
                    self.prepare()
                self.assertFalse(self.output.exists())

    def test_rejects_wrong_observer_abi_and_symlink(self):
        for offset, value in ((4, 2), (5, 2), (18, 62), (37, 2), (39, 3)):
            invalid = bytearray(arm_elf())
            invalid[offset] = value
            self.observer.write_bytes(invalid)
            with self.subTest(offset=offset), self.assertRaisesRegex(ValueError, "ARM32"):
                self.prepare()
        self.observer.unlink()
        self.observer.symlink_to(self.shell)
        with self.assertRaisesRegex(ValueError, "regular input"):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_rejects_archive_bounds_and_trailing_content(self):
        self.prepare()
        for limit in ("MAX_FILE", "MAX_PAYLOAD", "MAX_ARCHIVE", "MAX_MEMBERS"):
            with self.subTest(limit=limit), patch.object(bundle, limit, 2), self.assertRaises(ValueError):
                bundle.validate(self.output)
        self.output.write_bytes(self.output.read_bytes() + gzip.compress(b"hidden data"))
        with self.assertRaisesRegex(ValueError, "trailing archive content"):
            bundle.validate(self.output)

    def test_direct_observer_must_not_depend_on_a_userspace_interpreter(self):
        dynamic = bytearray(arm_elf() + bytes(32))
        struct.pack_into("<H", dynamic, 44, 2)
        struct.pack_into("<I", dynamic, 84, 3)
        self.observer.write_bytes(dynamic)
        with self.assertRaisesRegex(ValueError, "ELF interpreter"):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_rejects_non_ustar_input(self):
        archive(self.kit, self.entries, format=tarfile.GNU_FORMAT)
        with self.assertRaisesRegex(ValueError, "USTAR"):
            self.prepare()
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
