from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run_pose_probe
from volume_diagnostic import observation, program


LABEL = '1a2b3c4d5e6f7890'
RESULT = dict(policy='selector-diagnostic-v1', filesystem='vfat', admission='verified', admission_errno=0,
              exchange='exchange_out', exchange_errno=95, hardlink='link_create', hardlink_errno=1, clean=True)


class VolumeDiagnosticTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.tmp = self.root/'runtime'; self.volume = self.root/'volume'
        self.tmp.mkdir(); self.volume.mkdir()
        self.runtime = self.tmp/('weread-exchange-diagnostic-'+LABEL)
        self.stage = self.volume/('.weread-exchange-diagnostic-'+LABEL)
        self.raw = (json.dumps(RESULT)+'\n').encode()
        self.helper = b'#!/bin/sh\ncase "$1" in --policy) echo selector-diagnostic-v1;; diagnose) echo \'' + self.raw.rstrip() + b"';; *) exit 64;; esac\n"
        source = program(len(self.helper)).replace('volume=/mnt/us\n', 'volume='+shlex.quote(str(self.volume))+'\n')
        source = source.replace('tmp=/var/tmp\n', 'tmp='+shlex.quote(str(self.tmp))+'\n')
        admission = 'admission() { directory "$volume" && directory "$tmp" && absent "$stage" && absent "$scratch"; }'
        self.source = source.replace('owner_file() {', admission+'\nowner_file() {')

    def call(self, mode, payload=b'', *, ok=True):
        result = subprocess.run(['sh','-c',self.source,'probe',mode,LABEL,'none'], input=payload, capture_output=True, timeout=5)
        self.assertEqual(result.returncode == 0, ok, result.stderr.decode())
        return result.stdout

    def test_two_bounded_stages_and_independent_absent(self):
        sentinel = self.volume/'unrelated'; sentinel.write_bytes(b'do not touch')
        self.assertEqual(self.call('admit'), b'admitted\n')
        self.assertEqual(observation(self.call('probe', self.helper)), RESULT)
        self.assertEqual(list(self.stage.iterdir()), [self.stage/'owner'])
        self.assertEqual(set(self.runtime.iterdir()), {self.runtime/'owner', self.runtime/'helper'})
        self.call('absent', ok=False)
        self.assertEqual(self.call('close'), b'closed\n')
        self.assertEqual(self.call('close'), b'closed\n')
        self.assertEqual(self.call('absent'), b'absent\n')
        self.assertEqual(sentinel.read_bytes(), b'do not touch')

    def test_failed_transfer_still_closes_without_userstore_stage(self):
        self.call('probe', self.helper[:10], ok=False)
        self.assertFalse(self.stage.exists())
        self.call('close'); self.call('absent')

    def test_owned_marker_pairs_and_fully_internal_hardlinks_can_be_retired(self):
        self.call('probe', self.helper)
        (self.stage/'sub').mkdir(); (self.stage/'left').write_text(LABEL+':A\n')
        (self.stage/'sub/right').write_text(LABEL+':B\n')
        (self.stage/'sub/held').hardlink_to(self.stage/'left')
        self.call('close'); self.call('absent')

    def test_unknown_marker_or_external_alias_is_never_removed(self):
        self.call('probe', self.helper)
        foreign = self.volume/'unrelated'; foreign.write_text(LABEL+':A\n')
        target = self.stage/'left'
        target.hardlink_to(foreign); self.call('close', ok=False); self.assertTrue(target.exists()); target.unlink()
        target.symlink_to(foreign); self.call('close', ok=False); self.assertTrue(target.is_symlink()); target.unlink()
        target.write_text('wrong'); self.call('close', ok=False); self.assertTrue(target.exists()); target.unlink()
        self.assertTrue(foreign.is_file()); self.call('close'); self.call('absent')

    def test_strict_fixed_receipt_disallows_extra_data_and_false_success(self):
        for value in (RESULT | {'id': 'secret'}, RESULT | {'clean': 1}, RESULT | {'exchange': 'private'},
                      RESULT | {'admission_errno': 1}, RESULT | {'hardlink_errno': True}):
            with self.assertRaises(ValueError): observation((json.dumps(value)+'\n').encode())
        with self.assertRaises(ValueError): observation(self.raw.replace(b'"policy":', b'"clean":true,"policy":'))

    def test_private_host_preflight_and_unsupported_result_after_exact_release(self):
        helper = self.root/'helper'; helper.write_bytes(b'\x7fELF\x01\x01\x01'+bytes(11)+b'\x28\x00'+bytes(80))
        tool = self.root/'lease'; tool.touch()
        args = SimpleNamespace(runs=self.root, helper=helper, lease_tool=tool, owner='test-owner', recover=None,
                               display_trace=False, turn_cue=False, operator_ready=False, preflight=True)
        lease=Mock(); lease.LeaseRequest.side_effect=lambda path, token, *_: SimpleNamespace(token_file=token)
        lease.acquire.side_effect=lambda req: (req.token_file.touch(), {'acquired_at_utc':'fixture'})[1]
        lease.status.return_value={'ownership_verified':True}
        lease.release.return_value={'owner_and_token_checked':True,'only_owned_owner_json_removed_then_rmdir':True,
                                    'lease_directory_absent_after_release':True,'released_at_utc':'fixture'}
        operations=[]
        def invoke(command, *, input, **_):
            mode=command[-1].split()[-3]; operations.append(mode)
            if mode=='probe': self.assertTrue(input.endswith(helper.read_bytes()))
            answer={'probe':self.raw,'admit':b'admitted\n','close':b'closed\n','absent':b'absent\n'}[mode]
            return subprocess.CompletedProcess(command,0,answer,b'')
        original=Path.is_file
        with patch.object(Path,'is_file',lambda path: str(path)=='/opt/kindle-toolchain/ubuntu-run' or original(path)), \
                patch.dict(run_pose_probe.UTIL,ssh_command=lambda *_:['mock-ssh'],load_lease=lambda *_:lease), \
                patch.object(run_pose_probe.signal,'signal'), patch.object(run_pose_probe.sys,'stdin',io.StringIO('fixture\n')), \
                patch.object(run_pose_probe.subprocess,'run',side_effect=invoke), redirect_stdout(io.StringIO()):
            self.assertEqual(run_pose_probe.main(args,volume=True),0); lease.acquire.assert_not_called()
            self.assertEqual(operations,[]); args.preflight=False
            self.assertEqual(run_pose_probe.main(args,volume=True),0)
        self.assertEqual(operations,['admit','probe','close','absent']); lease.release.assert_called_once()
        attempt,=self.root.glob('fs-probe-*'); stored=json.loads((attempt/'operations.json').read_text())
        self.assertEqual(stored['observation'],RESULT); self.assertEqual(stored['scope'],'scratch_filesystem_capabilities_only')
        self.assertTrue(stored['passed'] and stored['owner_and_token_checked']); self.assertFalse((attempt/'private-token').exists())


if __name__ == '__main__':
    unittest.main()
