import gzip
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import software_visit as trial


class OffscreenVisitTests(unittest.TestCase):
    def public_output(self, lane):
        rows = []
        for chapters in (24, 1863):
            work = (1,2,3,7,4,4*chapters,4*chapters) if lane=="A" else (0,1,1,2,2,2*chapters,2*chapters)
            times = dict(zip(trial.TIMES,(37,69,54,123)))
            sample = dict(**dict(zip(trial.COUNTS,work)), **times, page=3, anchor=2548)
            row = dict(chapters=chapters,warmups=3,samples=[sample]*24,p50_ns=times,p95_ns=times)
            rows.append(b"test name ... READER_TURN_JSON "+json.dumps(row).encode())
        return b"\n".join(rows)+b"\n"

    def test_public_samples_refuse_swapped_arm_or_truncated_time(self):
        raw = self.public_output("B")
        self.assertEqual(len(trial.observations(raw,"B")),2)
        with self.assertRaisesRegex(ValueError,"wrong_lane"):
            trial.observations(raw,"A")
        with self.assertRaisesRegex(ValueError,"bad_timing"):
            trial.observations(raw.replace(b'"release_to_draw_ns": 123', b'"release_to_draw_ns": 19',1),"B")

    def test_isolated_run_and_state_drift_without_display_access(self):
        with tempfile.TemporaryDirectory() as scratch:
            root=Path(scratch); label="0123456789abcdef"; stage=root/(".weread-turn-cpu-"+label)
            script=root/"fixture.sh"; contents=(trial.HERE/"software_device.sh").read_text().replace("volume=/mnt/us","volume="+str(root))
            start=contents.index("snapshot() {"); end=contents.index('case "$mode" in',start)
            script.write_text(contents[:start]+'''snapshot() { power=$(cat "$volume/power"); printf 'power=%s\\n' "$power"; }
admit() { absent "$stage" && snapshot >/dev/null; }
'''+contents[end:])
            (root/"power").write_text("screenSaver\n")
            probe = b'''#!/bin/sh
set -eu
[ -z "${DISPLAY:-}" ] && [ "$TMPDIR" = "$HOME/../tmp" -o "$(dirname "$TMPDIR")" = "$(dirname "$HOME")" ] || exit 1
test -d "$TMPDIR" && test -d "$HOME" && test "$#" = 5 || exit 1
printf 'READER_TURN_JSON {"public":true}\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\\n'
'''
            def run(mode, payload=b""):
                return subprocess.run(["sh",str(script),mode,label,str(len(probe))],input=payload,capture_output=True,timeout=12)
            self.assertEqual(run("admit").stdout,b"admitted screenSaver\n")
            self.assertEqual(run("stage",gzip.compress(probe)).stdout,b"staged\n")
            entry=run("run"); self.assertEqual((entry.returncode,entry.stdout),(0,b'READER_TURN_JSON {"public":true}\n'),entry.stderr)
            (root/"power").write_text("active\n")
            self.assertEqual(run("close").stdout,b"closed_state_changed\n")
            self.assertEqual(run("absent").stdout,b"absent\n")
            self.assertFalse(stage.exists())


if __name__=="__main__":
    unittest.main()
