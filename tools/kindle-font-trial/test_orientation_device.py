"""Exercise the isolated shell transaction on Linux without a Kindle or lease."""

import io
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tarfile
import tempfile
import unittest

from orientation_device import render_device


LABEL = "0123456789abcdef"
DISPATCH = 'case "$mode" in\n    reference)'
PUBLIC = "request markers portrait portrait-source portrait-launch portrait-return thumb.pgm panel.pgm final.png final.metadata.json".split()


def archive(entries):
    result = io.BytesIO()
    with tarfile.open(fileobj=result, mode="w:gz") as output:
        for name, value in entries:
            member = tarfile.TarInfo(name)
            if isinstance(value, bytes):
                member.size, member.mode = len(value), 0o755
                output.addfile(member, io.BytesIO(value))
            else:
                member.type, member.linkname = tarfile.SYMTYPE, value
                output.addfile(member)
    return result.getvalue()


class DeviceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.volume = self.root / "mnt/us"
        self.stage = self.volume / (".weread-orientation-" + LABEL)
        self.documents = self.volume / "documents"
        self.documents.mkdir(parents=True)
        self.document = self.documents / ("weread-orientation-trial-" + LABEL + ".sh")
        self.daily = self.documents / "weread.sh"
        self.daily.write_text("existing daily document\n")
        for name in ("proc", "tmp", "var/local", "sys/class/input"):
            (self.root / name).mkdir(parents=True)
        (self.root / "proc/mounts").write_text(f"local {self.volume} tmpfs rw 0 0\n")
        self.app = self.volume / "weread/app/weread"
        self.app.parent.mkdir(parents=True)
        self.app.write_text('#!/bin/sh\n[ "$*" = "--kindle --device scribe --probe-native-portrait-calibration" ] || exit 1\nprintf "U:3\\n"\n')
        self.app.chmod(0o755)
        self.source = render_device(5, 6)
        self.assertIn(DISPATCH, self.source)
        self.assertNotIn('sha256sum', self.source)
        self.assertNotIn('absent /mnt/us/weread/launch.sh', self.source)
        self.source = re.sub(r'/mnt/us|/sys/class/input|/proc/|/var/local|/tmp(?=[/)])',
                             lambda match: str(self.root) + match[0], self.source)
        self.definitions, dispatch = self.source.split(DISPATCH)
        self.dispatch = DISPATCH + dispatch
        self.mocks = '''
native() { return 0; }
state() { cat "$daily"; }
count_doc() { if [ -f "$doc" ]; then echo 1; else echo 0; fi; }
wait_catalog() { [ "$(count_doc)" = "$1" ]; }
scan() { return 0; }
'''
        seed = archive([("demo/library.sqlite", b"public"), ("library-covers.json", b"{}")])
        self.entries = [("document", f"#!/bin/sh\nstage={self.stage}\n".encode()), ("measure", b"fixture"), ("seed.tar.gz", seed)]
        for lane, size in (("A", 5), ("B", 6)):
            self.entries.extend((f"{lane}/{name}", b"x" * (size if name == "app/weread" else 3)) for name in
                                ("launch.sh", "app/weread", "app/weread-frame-guard", "app/weread-native-query"))

    def invoke(self, mode, *, name="none", data=b"", program=None, expected=0, mocks=None, reason=None):
        body = self.definitions + (self.mocks if mocks is None else mocks) + (self.dispatch if program is None else program)
        result = subprocess.run(["sh", "-c", body, "orientation-fixture", mode, LABEL, name], input=data,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=12)
        self.assertEqual(result.returncode, expected, result.stderr.decode())
        if reason:
            self.assertEqual(result.stderr, f"refused {reason}\n".encode())
        return result.stdout

    def test_admission_reports_native_protection_or_pose_without_writing(self):
        for check, reason in (("native", "native_not_idle"), ("state", "protected_state"), ("portrait", "portrait_unavailable")):
            with self.subTest(check=check):
                self.invoke("admit", mocks=self.mocks + f"{check}() {{ return 1; }}\n", expected=1, reason=reason)
                self.assertFalse(self.stage.exists())
                self.assertFalse(self.document.exists())

    def test_partial_input_cannot_replace_the_owner_or_escape_its_stage(self):
        for changed in ([*self.entries, ("owner", b"foreign")], [("../escaped", b"bad"), *self.entries[1:]],
                        [("document", "../weread.sh"), *self.entries[1:]],
                        [(key, archive([("demo/../escaped", b"bad")]) if key == "seed.tar.gz" else value) for key, value in self.entries]):
            with self.subTest(member=changed[0][0]):
                self.invoke("stage", data=archive(changed), expected=1)
                self.assertEqual((self.stage / "owner").read_text(), LABEL + "\n")
                self.assertFalse(self.document.exists())
                self.assertFalse((self.volume / "escaped").exists())
                self.invoke("restore")
                self.invoke("close")
                self.assertFalse(self.stage.exists())

    def test_reference_requires_valid_native_pose_and_stage_checks_drift(self):
        self.assertEqual(self.invoke("reference"), b"U:3\n")
        self.invoke("stage", data=archive(self.entries))
        self.assertEqual(self.document.read_bytes(), self.entries[0][1])
        old = self.daily.read_bytes()
        self.daily.write_bytes(b"operator change\n")
        self.invoke("case", name="B-library", expected=1)
        self.assertFalse((self.stage / "out/B-library").exists())
        self.invoke("restore", expected=1)
        self.assertEqual(self.daily.read_bytes(), b"operator change\n")
        self.assertTrue(self.document.exists())
        self.daily.write_bytes(old)
        self.invoke("restore")
        self.invoke("close")
        self.assertFalse(self.document.exists())
        self.assertEqual(self.daily.read_bytes(), old)
        self.app.write_text('#!/bin/sh\nprintf "FIXED:3\\n"\n')
        self.invoke("reference", expected=1)

    def test_restore_cannot_confuse_idle_native_with_an_entered_worker_still_starting(self):
        self.invoke("stage", data=archive(self.entries))
        entered = self.stage / "out/A-library/entered"
        entered.mkdir(parents=True)
        (entered / "arm").write_text("A-library\n")
        self.invoke("restore", mocks=self.mocks + 'sleep() { :; }\n', expected=1)
        self.assertTrue(self.document.exists())
        self.assertFalse((self.stage / "restored").exists())
        self.invoke("restore", mocks=self.mocks + 'sleep() { test -f "$doc" && echo 143 >"$stage/out/A-library/status"; }\n')
        self.assertFalse(self.document.exists())
        (entered.parent / "status").unlink()
        self.invoke("close", expected=1)
        self.assertTrue(self.stage.exists())
        (entered.parent / "status").write_text("143\n")
        self.invoke("close")
        self.assertFalse(self.stage.exists())

    def test_restore_disarms_before_an_entry_has_consumed_the_grant(self):
        self.invoke("stage", data=archive(self.entries))
        entered = self.stage / "out/A-library/entered"
        entered.mkdir(parents=True)
        (self.stage / "armed").write_text("A-library\n")
        self.invoke("restore", mocks=self.mocks + 'sleep() { exit 83; }\n')
        self.assertFalse((entered / "arm").exists())
        self.assertFalse((self.stage / "armed").exists())
        self.invoke("close")
        self.assertFalse(self.stage.exists())

    def test_collect_excludes_raw_diagnostics_private_projection_and_unfinished_cases(self):
        self.invoke("stage", data=archive(self.entries))
        out = self.stage / "out/A-library"
        out.mkdir()
        (self.stage / "out/B-library").mkdir()
        (out / "complete").write_text("A-library\n")
        for leaf in [*PUBLIC, "worker", "entry.log"]:
            (out / leaf).write_text(leaf)
        with tarfile.open(fileobj=io.BytesIO(self.invoke("collect"))) as saved:
            self.assertEqual(set(saved.getnames()), {"out/orientation-batch", *(f"out/A-library/{leaf}" for leaf in PUBLIC)})
            self.assertEqual(saved.extractfile("out/orientation-batch").read(), (LABEL + "\n").encode())
        (out / "portrait").unlink()
        (out / "portrait").symlink_to(self.daily)
        self.invoke("collect", expected=1)
        self.invoke("restore")
        self.invoke("close")

    def test_same_launch_record_is_bound_without_exporting_its_nonce(self):
        self.invoke("stage", data=archive(self.entries))
        out = self.stage / "out/A-library"
        out.mkdir()
        lock = self.root / "tmp/weread-launcher.lock"
        lock.mkdir()
        token = "a" * 32
        (out / "portrait").write_text("D:1\n")
        (lock / "nonce").write_text(token + "\n")
        (lock / "generation").write_text(f"1:{token}\n")
        (lock / "state-open").write_text(f"1:{token}\n")
        record = lock / "native-portrait-calibration"
        actual = f"1:{token}:D:1\n"
        body = 'gui_owned() { return 0; }; out="$stage/out/A-library"; live_pose\n'
        for wrong in (actual.replace(":D:1", ":U:3"), actual.replace("1:", "2:", 1), actual + "extra\n"):
            record.write_text(wrong)
            self.invoke("fixture", program=body, expected=1)
        record.write_text(actual)
        self.invoke("fixture", program=body)
        self.assertEqual((out / "portrait-launch").read_text(), "D:1\n")

    def test_scoped_case_checks_actual_lane_first_send_and_redacts_calibration(self):
        token = "b" * 32
        fixture = f'''#!/bin/sh
set -eu
stage={self.stage}; root={self.root}; lock=$root/tmp/weread-launcher.lock
case "$1" in
request)
    trial=$(cat "$stage/armed"); arm=${{trial%"${{trial#?}}"}}; out=$stage/out/$trial; home=$stage/$arm
    mkdir "$out/entered"; mv "$stage/armed" "$out/entered/arm"
    mkdir -p "$lock" "$home/data/logs" "$root/proc/123"
    printf '123\\n' >"$lock/application-pid"; printf '42\\n' >"$lock/application-birth"
    printf '{token}\\n' >"$lock/nonce"; printf '1:{token}\\n' >"$lock/generation"; cp "$lock/generation" "$lock/state-open"
    printf '1:{token}:U:3\\n' >"$lock/native-portrait-calibration"
    ln -sf "$home/app/weread" "$root/proc/123/exe"
    printf '%s\\n' '123 (weread) S {"0 " * 18}42' >"$root/proc/123/stat"
    printf '%s\\000' "$home/app/weread" --kindle --device scribe --data-dir "$home/data" --demo --offline --screen library --exit-after-ms 12000 --capture "$out/final.png" >"$root/proc/123/cmdline"
    printf '%s\\n' 'not public {token}' 'WEREAD_STARTUP 22.100000000 main_enter' 'WEREAD_STARTUP 23.000000000 first_application_frame_send' >"$home/data/logs/application-bootstrap.log"
    echo fake >"$out/final.png"; echo '{{}}' >"$out/final.metadata.json"; echo 0 >"$out/status"; echo 'REQUEST 22.000000000';;
thumb|panel) printf 'fixture image';;
*) exit 1;;
esac
'''
        self.invoke("stage", data=archive([(key, fixture.encode() if key == "measure" else value) for key, value in self.entries]))
        commands = self.root / "bin"
        commands.mkdir()
        timer = commands / "lipc-set-prop"
        timer.write_text('#!/bin/sh\n[ "$*" = "-i com.lab126.powerd touchScreenSaverTimeout 1" ]\n')
        timer.chmod(0o755)
        mocks = self.mocks + f'PATH="{commands}:$PATH"; sleep() {{ :; }}; usleep() {{ :; }}\n'
        self.assertEqual(self.invoke("case", name="A-library", mocks=mocks), b"completed A-library\n")
        self.invoke("case", name="A-library", mocks=mocks, expected=1)
        with tarfile.open(fileobj=io.BytesIO(self.invoke("collect"))) as saved:
            for entry in saved:
                self.assertNotIn(token.encode(), saved.extractfile(entry).read())
            self.assertEqual(saved.extractfile("out/A-library/portrait-launch").read(), b"U:3\n")
        self.invoke("restore")
        self.invoke("close")
        self.assertFalse(self.stage.exists())

    def test_public_stat_refuses_wrong_size_and_observes_replacement_without_hashing(self):
        self.invoke("fixture", program='file_identity "$daily" 100\n', expected=1)
        first = self.invoke("fixture", program='file_identity "$daily"\n')
        replacement = self.documents / "replacement"
        replacement.write_bytes(self.daily.read_bytes())
        os.replace(replacement, self.daily)
        self.assertNotEqual(first, self.invoke("fixture", program='file_identity "$daily"\n'))

    def test_scoped_catalog_sql_rejects_case_collision_title_drift_and_registration_changes(self):
        shim = self.root / "query.py"
        shim.write_text('import sqlite3,sys\n'
                        'with sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True) as db:\n'
                        ' for statement in sys.argv[2].split(";"):\n'
                        '  for row in db.execute(statement): print("|".join(str(value) for value in row))\n')
        sqlite = f'sql() {{ python3 "{shim}" "$@"; }}\n'
        db_path = self.root / "var/local/cc.db"
        bili = str(self.documents / "bilicomics.sh")
        with sqlite3.connect(db_path) as db:
            db.execute("CREATE TABLE Entries(p_location TEXT COLLATE NOCASE,p_mimeType,p_titles_0_nominal)")
            db.execute("INSERT INTO Entries VALUES(?,?,?)", (bili, "text/x-shellscript", "BiliComics for Kindle"))
        check = f'''catalog_one '{bili}' "AND p_titles_0_nominal='BiliComics for Kindle'"\n'''
        self.assertIn(b"BiliComics for Kindle", self.invoke("fixture", program=check, mocks=sqlite))
        with sqlite3.connect(db_path) as db:
            db.execute("INSERT INTO Entries SELECT replace(p_location,'bilicomics.sh','BiliComics.sh'),p_mimeType,p_titles_0_nominal FROM Entries")
        self.invoke("fixture", program=check, mocks=sqlite, expected=1)
        with sqlite3.connect(db_path) as db:
            db.execute("DELETE FROM Entries WHERE p_location COLLATE BINARY=?", (bili.replace("bilicomics.sh", "BiliComics.sh"),))
            db.execute("UPDATE Entries SET p_titles_0_nominal='other'")
        self.invoke("fixture", program=check, mocks=sqlite, expected=1)
        reg_path = self.root / "var/local/appreg.db"
        handler = "tech.hackerdude.shell_integration.launcher"
        runtime = re.search(r"^runtime=(.+)$", self.definitions, re.M)[1]
        with sqlite3.connect(reg_path) as db:
            db.execute("CREATE TABLE properties(handlerId,name,value)")
            db.execute("CREATE TABLE associations(contentId,handlerId)")
            db.execute("INSERT INTO properties VALUES(?,?,?)", (handler, "command", runtime + "/aidoku-library-dispatch"))
            db.execute("INSERT INTO associations VALUES(?,?)", ("MT:text/x-shellscript", handler))
        self.assertIn(b"aidoku-library-dispatch", self.invoke("fixture", program="registration\n", mocks=sqlite))
        with sqlite3.connect(reg_path) as db:
            db.execute("UPDATE properties SET value='replaced'")
        self.invoke("fixture", program="registration\n", mocks=sqlite, expected=1)


if __name__ == "__main__":
    unittest.main()
