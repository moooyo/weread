import fcntl
import gzip
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
DRIVER = runpy.run_path(str(HERE / "cpu-batch.py"))
LABEL = "0123456789abcdef"
LINES = [f"round={index} full_ns={100 + index} receipt_ns=2" for index in range(7)] + ["stage_removed"]


class StagingTest(unittest.TestCase):
    def setUp(self):
        area = tempfile.TemporaryDirectory()
        self.addCleanup(area.cleanup)
        self.root = Path(area.name)
        self.volume = self.root / "volume"
        self.volume.mkdir()
        self.stage = self.volume / (".weread-font-" + LABEL)
        (self.volume / "unrelated").write_bytes(b"untouched")
        (self.root / "locks").mkdir()
        self.code = self.root / "model"
        self.code.write_text("G0027J")
        (self.root / "firmware").write_text("Kindle 5.19.5 (fixture)\n")
        (self.root / "mounts").write_text(f"/dev/fixture {self.volume} vfat rw,relatime 0 0\n")
        (self.root / "memory").write_text("MemAvailable: 999999 kB\n")
        body = (HERE / "cpu-stage.sh").read_text()
        for original, name in [("/mnt/us", "volume"), ("/proc/usid", "model"),
                               ("/etc/prettyversion.txt", "firmware"), ("/proc/mounts", "mounts"),
                               ("/proc/meminfo", "memory"), ("/tmp/$app-launcher.lock", "locks/$app-launcher.lock")]:
            body = body.replace(original, str(self.root / name))
        self.script = self.root / "driver"
        self.script.write_text(body)
        for name, stub in {"uname": "echo armv7l", "pidof": '[ "$1" = awesome ] && echo "$FIXTURE_PID"',
                           "status": "echo statusbar start/running",
                           "lipc-get-prop": 'case "$3" in state) echo "${STOCK_POWER:-active}";; activeApp) echo com.lab126.KPPMainApp;; esac'}.items():
            path = self.root / name
            path.write_text("#!/bin/sh\n" + stub + "\n")
            path.chmod(0o700)
        self.binary = ("#!/bin/sh\n" + '[ "$1" = --bench-guarded ] && [ "$2" = "$PPID" ] || exit 2\n'
                       + "".join(f"echo '{line}'\n" for line in LINES[:-1])).encode()
        self.env = dict(os.environ, PATH=str(self.root) + ":" + os.environ["PATH"], FIXTURE_PID=str(os.getpid()))

    def call(self, mode, payload=None):
        result = subprocess.run(["sh", self.script, mode, LABEL, str(len(self.binary))], env=self.env,
                                input=gzip.compress(self.binary) if payload is None else payload, capture_output=True)
        self.assertEqual((self.volume / "unrelated").read_bytes(), b"untouched")
        return result.returncode, result.stdout.decode().splitlines()

    def test_scoped_run(self):
        self.assertEqual(self.call("check"), (0, [DRIVER["IDLE"]]))
        self.assertEqual(self.call("sample"), (0, LINES))
        self.assertEqual(self.call("close"), (0, ["stage_absent"]))
        self.assertFalse(self.stage.exists())

    def test_incomplete_upload(self):
        self.assertEqual(self.call("sample", gzip.compress(self.binary)[:-8]), (1, ["refused transfer", "stage_removed"]))
        self.assertFalse(self.stage.exists())
        self.assertEqual(self.call("sample", gzip.compress(b"partial")), (1, ["refused short_copy", "stage_removed"]))
        self.assertFalse(self.stage.exists())

    def test_refusals_before_writing(self):
        self.code.write_text("G003V0")
        self.assertEqual(self.call("sample"), (1, ["refused model"]))
        self.code.write_text("G0027J")
        self.env["STOCK_POWER"] = "screenSaver"
        self.assertEqual(self.call("sample"), (1, ["refused not_active"]))
        self.assertFalse(self.stage.exists())

    def test_foreign_or_busy_directory(self):
        self.stage.mkdir()
        (self.stage / "owner").write_text("not-this-run\n")
        self.assertEqual(self.call("close"), (1, ["refused cleanup"]))
        (self.stage / "owner").write_text(LABEL + "\n")
        (self.stage / "unexpected").touch()
        self.assertEqual(self.call("close"), (1, ["refused cleanup"]))
        (self.stage / "unexpected").unlink()
        descriptor = os.open(self.stage, os.O_RDONLY)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX)
            self.assertEqual(self.call("close"), (1, ["refused operation_active"]))
        finally:
            os.close(descriptor)
        self.assertEqual(self.call("close"), (0, ["stage_absent"]))


class BatchTest(unittest.TestCase):
    def test_requires_full_schedule(self):
        result = DRIVER["samples"](LINES)
        self.assertEqual(result["median_paired_saving_ns"], 101.5)
        self.assertEqual([row["full_first"] for row in result["measured"]], [True, False] * 3)
        for invalid in [LINES[:-1], [LINES[1]] + LINES[1:], [LINES[0]] + LINES, LINES[:-2] + ["round=6 full_ns=0 receipt_ns=2", "stage_removed"]]:
            with self.assertRaises(ValueError):
                DRIVER["samples"](invalid)

    def test_release_and_recovery(self):
        idle = (0, [DRIVER["IDLE"]])
        for check, cleanup, expected in [(idle, (0, ["stage_absent"]), ["check", "sample", "close", "check", "release"]),
                                         (idle, (1, []), ["check", "sample", "close"]),
                                         ((1, ["refused not_active"]), (1, []), ["check", "release"])]:
            actions = []
            report = {"passed": False}

            def call(mode):
                actions.append(mode)
                return {"check": check, "close": cleanup, "sample": (1, ["refused transfer"])}[mode]

            DRIVER["execute"](call, lambda: actions.append("release"), report)
            self.assertEqual(actions, expected)
            self.assertFalse(report["passed"])
            self.assertEqual(bool(report.get("recovery_required")), "release" not in expected)


class EntryTest(unittest.TestCase):
    def test_exact_document_and_no_catalog_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "documents").mkdir()
            document = root / "documents/weread.sh"
            document.write_bytes(b"fixture only\n")
            catalog = root / "cc.db"
            connection = sqlite3.connect(catalog)
            with connection:
                connection.execute("CREATE TABLE Entries(p_location TEXT)")
                connection.executemany("INSERT INTO Entries VALUES (?)", [(str(document),), ("unrelated",)])
            connection.close()
            client = root / "sqlite3"
            client.write_text(
                "#!/usr/bin/env python3\nimport sqlite3,sys\n"
                "if sys.argv[1:] == ['-version']: print(sqlite3.sqlite_version)\n"
                "else:\n"
                " assert sys.argv[1:4] == ['-batch','-noheader','-readonly'] and sys.argv[4].endswith('?mode=ro&immutable=1')\n"
                " print(sqlite3.connect(sys.argv[4],uri=True).execute(sys.argv[5]).fetchone()[0])\n"
            )
            client.chmod(0o700)
            script = (HERE / "entry-audit.sh").read_text().replace("/mnt/us", str(root)).replace("/var/local/cc.db", str(catalog))

            def invoke(reference):
                result = subprocess.run(["sh", "-c", script], input=reference, check=True, capture_output=True,
                                        env=dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"]))
                return result.stdout.decode().splitlines()

            before = catalog.read_bytes()
            self.assertEqual(invoke(b"fixture only\n"), ["document=regular", "reference=equal", "supervisor=absent",
                                                       "application=absent", "selected_rows=1"])
            self.assertIn("reference=different", invoke(b"different\n"))
            catalog.with_name("cc.db-wal").write_bytes(b"active")
            self.assertIn("selected_rows=not_sampled", invoke(b"fixture only\n"))
            document.unlink()
            self.assertIn("reference=not_compared", invoke(b"fixture only\n"))
            self.assertEqual(catalog.read_bytes(), before)

    def test_release_even_if_audit_is_inconclusive(self):
        report = {"passed": False}
        calls = []

        def call(mode):
            calls.append(mode)
            return (0, [DRIVER["IDLE"]]) if mode == "check" else (0, ["unexpected_output"])

        DRIVER["audit_entry"](call, lambda: calls.append("release"), report)
        self.assertEqual(calls, ["check", "entry", "check", "release"])
        self.assertFalse(report["passed"])

    def test_release_failure_does_not_pass_an_audit(self):
        def call(mode):
            return (0, [DRIVER["IDLE"]]) if mode == "check" else (
                0, ["document=regular", "reference=equal", "supervisor=absent", "application=regular", "selected_rows=1"])

        def refuse_release():
            raise RuntimeError("release refused")

        report = {"passed": False}
        with self.assertRaises(RuntimeError):
            DRIVER["audit_entry"](call, refuse_release, report)
        self.assertFalse(report["passed"])


if __name__ == "__main__":
    unittest.main()
