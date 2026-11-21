from contextlib import redirect_stdout
import fcntl
import io
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run_pose_probe
from run_pose_probe import transact
from scratch_bind import observation, program


LABEL = 'aaaaaaaaaa111111'
HELPER = b'''#!/bin/sh
set -eu
case "$1" in --policy) echo selector-bind-v1; exit;; esac
[ "$2" = scratch ] && [ "$3" = "$MOCK_LABEL" ]
exec 9<"$MOCK_RUNTIME"
flock -n 9 || { echo 'refused bind_contended' >&2; exit 1; }
printf '%s\\n' "$1" >>"$MOCK_DIR/calls"
case "$1:$MOCK_FAULT" in
prepare:refused) echo 'refused bind_propagation' >&2; exit 1;;
prepare:prepare_record) printf 'prepared\\n' >"$MOCK_RUNTIME/bind-journal";;
bind:syscall) printf 'journal\\n' >"$MOCK_RUNTIME/bind-journal"; echo 'refused bind_mount_call_1' >&2; exit 1;;
restore:busy) echo 'refused bind_unmount_call_16' >&2; exit 1;; esac
case "$1" in
prepare) echo bind_prepared;;
bind) printf 'journal\\n' >"$MOCK_RUNTIME/bind-journal"; printf 'bound\\n' >"$MOCK_RUNTIME/bind-active"; echo bound;;
status) if [ -f "$MOCK_RUNTIME/bind-active" ] && [ "$MOCK_FAULT" = witness ]; then echo 'refused bind_native_view' >&2; exit 1; fi
        if [ -f "$MOCK_RUNTIME/bind-active" ]; then echo bound; else echo unbound; fi;;
restore) rm -f "$MOCK_RUNTIME/bind-active"; echo bind_restored;;
absent) test ! -f "$MOCK_RUNTIME/bind-active"; echo bind_absent;;
*) exit 64;; esac
'''


class ScratchBindTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.tmp, self.volume = self.root/'runtime', self.root/'volume'
        self.tmp.mkdir(); self.volume.mkdir()
        self.runtime = self.tmp/('weread-bind-diagnostic-'+LABEL)
        self.scratch = self.volume/('.weread-bind-diagnostic-'+LABEL)
        self.body = program(len(HELPER)).replace('volume=/mnt/us\n', 'volume='+shlex.quote(str(self.volume))+'\n')
        self.body = self.body.replace('tmp=/var/tmp\n', 'tmp='+shlex.quote(str(self.tmp))+'\n')
        original = 'admission() { directory "$volume" && directory "$tmp" && absent "$stage" && absent "$scratch"; }'
        self.body = self.body.replace('owner_file() {', original+'\nowner_file() {')
        self.env = dict(os.environ, MOCK_LABEL=LABEL, MOCK_DIR=str(self.root), MOCK_RUNTIME=str(self.runtime))

    def invoke(self, mode, *, fault='', success=True, payload=HELPER):
        result = subprocess.run(['sh','-c',self.body,'test',mode,LABEL,'none'],input=payload if mode=='probe' else b'',
                                env=dict(self.env,MOCK_FAULT=fault),capture_output=True,timeout=6)
        self.assertEqual(result.returncode==0,success,result.stderr.decode()); return result.stdout

    def test_phases_have_separate_witness_and_recover_before_release(self):
        self.assertEqual(self.invoke('admit'),b'admitted\n'); raw=self.invoke('probe')
        self.assertTrue((self.runtime/'bind-journal').is_file())
        raw+=self.invoke('witness'); self.invoke('absent',success=False)
        self.assertEqual(self.invoke('close'),b'closed\n'); self.assertEqual(self.invoke('absent'),b'absent\n')
        self.assertEqual((self.root/'calls').read_text().splitlines(),['prepare','bind','status','status','restore','status','absent'])
        result=observation(raw); self.assertEqual(result['result'],'current_views_verified')
        self.assertTrue(result['original_identity_and_bytes_restored']); self.assertFalse(result['on_demand_handler_observed'])
        self.invoke('close'); self.assertFalse(self.runtime.exists() or self.scratch.exists())

    def test_prepare_capability_refusal_is_collected_and_safely_retired(self):
        raw=self.invoke('probe',fault='refused')+self.invoke('witness'); self.invoke('close'); self.invoke('absent')
        self.assertEqual(observation(raw)['reason'],'bind_propagation')
        self.assertIs(observation(raw)['mount_call_not_reached'],True)
        self.assertEqual((self.root/'calls').read_text().splitlines(),['prepare'])

    def test_inherited_setgid_is_cleared_and_shell_and_helper_lock_different_inodes(self):
        self.tmp.chmod(0o2777); raw=self.invoke('probe'); self.assertEqual(self.runtime.stat().st_mode & 0o7777,0o700)
        with (self.runtime/'owner').open() as owner:
            self.assertEqual(os.fstat(owner.fileno()).st_mode & 0o777,0o600)
            fcntl.flock(owner,fcntl.LOCK_EX|fcntl.LOCK_NB); self.invoke('witness',success=False)
        self.assertEqual(observation(raw+self.invoke('witness'))['result'],'current_views_verified'); self.invoke('close')

    def test_prepare_disconnect_with_a_journal_uses_close_only_before_bind_intent(self):
        normal=self.body; self.body=normal.replace('once "$stage/bind-intent" "$label" || fail bind_intent','exit 45')
        self.invoke('probe',fault='prepare_record',success=False); self.body=normal
        self.assertFalse((self.runtime/'bind-intent').exists()); self.assertTrue((self.runtime/'prepare-intent').is_file())
        self.invoke('close'); self.invoke('absent')
        self.assertEqual((self.root/'calls').read_text().splitlines(),['prepare','restore','status','absent'])

    def test_cleanup_can_resume_between_owner_unlink_and_rmdir_but_not_on_a_different_directory(self):
        self.invoke('probe'); normal=self.body; self.body=normal.replace('same_scratch && rmdir "$scratch" || return 1','return 1')
        self.invoke('close',success=False); self.body=normal; self.assertEqual(list(self.scratch.iterdir()),[])
        original=self.volume/'held'; self.scratch.rename(original); self.scratch.mkdir(); self.invoke('close',success=False)
        self.assertTrue(self.scratch.is_dir()); self.scratch.rmdir(); original.rename(self.scratch)
        outsider=self.scratch/'unrelated'; outsider.write_bytes(b'keep'); self.invoke('close',success=False)
        self.assertEqual(outsider.read_bytes(),b'keep'); outsider.unlink(); self.invoke('close'); self.invoke('absent')

    def test_early_refusal_does_not_clear_if_init_has_a_mount_or_pointers_are_changed(self):
        raw=self.invoke('probe',fault='refused')+self.invoke('witness'); self.assertIs(observation(raw)['mount_call_not_reached'],True)
        table=self.root/'mountinfo'; table.write_text(f'44 1 0:29 / {self.scratch}/target rw - tmpfs x rw\n')
        normal=self.body; self.body=normal.replace('/proc/1/mountinfo;',shlex.quote(str(table))+';'); self.invoke('close',success=False)
        self.assertTrue(self.scratch.is_dir()); self.body=normal; target=self.scratch/'target'; saved=self.root/'saved'; target.rename(saved)
        target.write_bytes(saved.read_bytes()); self.invoke('close',success=False); target.unlink(); saved.rename(target)
        self.invoke('close'); self.invoke('absent')

    def test_errno_and_pre_mount_evidence_are_distinct_and_bound_to_their_phases(self):
        raw=self.invoke('probe',fault='syscall')+self.invoke('witness'); self.invoke('close'); self.invoke('absent')
        result=observation(raw); self.assertEqual((result['reason'],result['errno']),('bind_mount_call',1)); self.assertNotIn('mount_call_not_reached',result)
        for phase,reason in (('bind','bind_mount_call_1 before_mount'),('bind','bind_mount_call_0'),('bind','bind_mount_call_4096'),
                             ('status','bind_native_view before_mount'),('prepare','bind_native_service unexpected')):
            with self.assertRaises(ValueError): observation(f'scratch_bind refused {phase} {reason}\nindependent_view not_attempted\n'.encode())

    def test_interrupted_bound_visit_close_only_never_rebinds(self):
        self.invoke('probe'); self.invoke('close',fault='busy',success=False)
        self.assertTrue((self.runtime/'bind-active').is_file()); self.assertTrue((self.scratch/'target').is_file())
        self.invoke('close'); self.invoke('absent')
        self.assertEqual((self.root/'calls').read_text().splitlines(),['prepare','bind','status','restore','restore','status','absent'])

    def test_cross_connection_mismatch_is_not_a_positive(self):
        raw=self.invoke('probe')+self.invoke('witness',fault='witness'); self.invoke('close'); self.invoke('absent')
        self.assertEqual(observation(raw)['reason'],'bind_native_view')

    def test_original_replaced_with_same_bytes_or_external_alias_keeps_stage(self):
        self.invoke('probe'); target=self.scratch/'target'; alternate=self.scratch/'backup'; target.rename(alternate)
        target.write_bytes(alternate.read_bytes()); alternate.rename(self.root/'saved')
        self.invoke('close',success=False); self.assertTrue(target.exists()); target.unlink(); (self.root/'saved').rename(target)
        alias=self.root/'external'; alias.hardlink_to(target); self.invoke('close',success=False); self.assertTrue(alias.exists()); alias.unlink()
        self.invoke('close'); self.invoke('absent')

    def test_foreign_paths_and_incomplete_upload_are_never_unlinked(self):
        self.invoke('probe',payload=b'a',success=False); self.assertFalse(self.scratch.exists())
        self.invoke('close'); self.invoke('probe'); outsider=self.root/'outside'; outsider.write_bytes(b'unrelated')
        extra=self.scratch/'foreign'; extra.symlink_to(outsider); self.invoke('close',success=False)
        self.assertTrue(extra.is_symlink()); extra.unlink(); self.invoke('close'); self.invoke('absent')
        self.assertEqual(outsider.read_bytes(),b'unrelated')

    def test_host_witness_disconnect_releases_only_after_close_and_absence(self):
        steps=[]; report={}
        def call(mode,_):
            steps.append(mode)
            if mode=='witness': raise ValueError('connection_lost')
            return self.invoke(mode)
        def release():
            self.assertFalse(self.scratch.exists() or self.runtime.exists()); steps.append('release')
            return dict(owner_and_token_checked=True,only_owned_owner_json_removed_then_rmdir=True,released_at_utc='fixture',lease_directory_absent_after_release=True)
        with self.assertRaisesRegex(ValueError,'connection_lost'):
            transact(call,release,report,lambda:None,decode=observation,witness=True)
        self.assertEqual(steps,['admit','probe','witness','close','absent','release']); self.assertIn('released',report)

    def test_host_preflight_refusal_exit_and_recovery_identity(self):
        helper=self.root/'arm'; helper.write_bytes(b'\x7fELF\x01\x01\x01'+bytes(11)+b'\x28\x00'+bytes(80))
        args=SimpleNamespace(runs=self.root,helper=helper,lease_tool=helper,owner='test-owner',recover=None,
                             display_trace=False,turn_cue=False,operator_ready=False,preflight=True)
        lease=Mock(); lease.LeaseRequest.side_effect=lambda _,token,*__: SimpleNamespace(token_file=token)
        lease.acquire.side_effect=lambda req: (req.token_file.touch(),{'acquired_at_utc':'fixture'})[1]
        lease.status.return_value={'ownership_verified':True}
        lease.release.return_value=dict(owner_and_token_checked=True,only_owned_owner_json_removed_then_rmdir=True,
                                        released_at_utc='fixture',lease_directory_absent_after_release=True)
        steps=[]
        def wire(command,*,input,**_):
            mode=command[-1].split()[-3]; steps.append(mode)
            if mode=='probe': self.assertTrue(input.endswith(helper.read_bytes()))
            packet={'admit':b'admitted\n','probe':b'scratch_bind refused prepare bind_namespace before_mount\n',
                    'witness':b'independent_view not_attempted\n','close':b'closed\n','absent':b'absent\n'}[mode]
            return subprocess.CompletedProcess(command,0,packet,b'')
        regular=Path.is_file
        with patch.object(Path,'is_file',lambda path: str(path)=='/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
                patch.dict(run_pose_probe.UTIL,ssh_command=lambda *_:['fixture-ssh'],load_lease=lambda *_:lease), \
                patch.object(run_pose_probe.signal,'signal'), patch.object(run_pose_probe.sys,'stdin',io.StringIO('fixture\nfixture\n')), \
                patch.object(run_pose_probe.subprocess,'run',side_effect=wire), redirect_stdout(io.StringIO()):
            self.assertEqual(run_pose_probe.main(args,bind=True),0); lease.acquire.assert_not_called(); self.assertEqual(steps,[])
            args.preflight=False; self.assertEqual(run_pose_probe.main(args,bind=True),1)
            attempt,=self.root.glob('bind-probe-*'); report=json.loads((attempt/'operations.json').read_text())
            self.assertFalse(report['passed']); self.assertTrue(report['transaction_completed'] and report['owner_and_token_checked'])
            self.assertEqual(report['observation']['result'],'not_qualified'); self.assertEqual(steps,['admit','probe','witness','close','absent'])
            self.assertFalse((attempt/'private-token').exists()); (attempt/'private-token').write_text('fixture'); args.recover=attempt; steps.clear()
            for options in ({},{'volume':True}):
                with self.assertRaisesRegex(ValueError,'probe type'): run_pose_probe.main(args,**options)
            self.assertEqual(steps,[]); self.assertEqual(run_pose_probe.main(args,bind=True),0)
            self.assertEqual(steps,['close','absent']); lease.acquire.assert_called_once()
            recovered=json.loads((attempt/'recovery.json').read_text()); self.assertNotIn('observation',recovered)


if __name__ == '__main__':
    unittest.main()
