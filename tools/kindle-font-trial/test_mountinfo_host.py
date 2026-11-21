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

from mount_table_observation import observation
import run_mountinfo_diagnostic as runner


LABEL = '1abcde789012345f'
OBSERVATION = dict(policy='mount-table-read-v1',table='ok',fd='ok',base_rows='zero',base_fd_rows='zero',fd_rows='one',
                   base_shape='none',first_error='base_absent',error_scope='volume',repeat='same',seek='not_observed',propagation='unknown')
RAW = (json.dumps(OBSERVATION)+'\n').encode('ascii')
TIME = '2026-11-22T00:12:00+00:00'
RELEASE = dict(owner_and_token_checked=True,only_owned_owner_json_removed_then_rmdir=True,
               native_restored_and_connections_closed=True,lease_directory_absent_after_release=True,released_at_utc=TIME)


class ProtocolTests(unittest.TestCase):
    def test_only_one_flat_public_classification_is_allowed(self):
        self.assertEqual(observation(RAW),OBSERVATION)
        for fields in (dict(extra='private'),dict(fd=123),dict(base_rows='7'),dict(seek='same'),dict(base_shape=['root'])):
            with self.assertRaises(ValueError): observation((json.dumps(OBSERVATION|fields)+'\n').encode())
        for data in (RAW.rstrip(),RAW+RAW,RAW.replace(b'"policy":',b'"fd":"ok","policy":'),b'wrong\n'):
            with self.assertRaises(ValueError): observation(data)

    def test_read_only_guards_do_not_require_the_operation_they_diagnose(self):
        body=runner.guard().decode('ascii'); self.assertNotIn('mountinfo',body); self.assertNotIn('/proc/mounts',body)
        for item in ('mkdir ', 'chmod ', 'rm ', 'mount ', 'lipc-set-prop', 'flock ', 'mktemp ', 'sh_integration_launcher', 'sqlite3'):
            self.assertNotIn(item,body)
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory); volume=base/'volume'; private=base/'temporary'; volume.mkdir(); private.mkdir()
            body=body.replace('/mnt/us',str(volume)).replace('/var/tmp',str(private))
            body=body.replace('native || fail baseline_native','native() { return 0; }; native || fail baseline_native')
            def invoke(mode):
                return subprocess.run(['sh','-s','--',mode,LABEL],input=body,capture_output=True,text=True,timeout=4)
            self.assertEqual(invoke('verify').stdout,'native_clear verify '+LABEL+'\n')
            old=volume/('.weread-bind-diagnostic-'+runner.PRECEDING); old.mkdir(); self.assertEqual(invoke('admit').stderr,'refused baseline_residue\n')
            self.assertTrue(old.is_dir()); old.rmdir(); sentinel=private/'weread-stock-selector-bind'; sentinel.symlink_to(base/'missing')
            self.assertEqual(invoke('verify').stderr,'refused baseline_residue\n'); self.assertTrue(sentinel.is_symlink()); sentinel.unlink()
            self.assertEqual(invoke('admit').stdout,'native_clear admit '+LABEL+'\n'); self.assertEqual(list(private.iterdir())+list(volume.iterdir()),[])


class HostTests(unittest.TestCase):
    def setUp(self):
        temp=tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup); self.base=Path(temp.name); self.runs=self.base/'runs'; self.runs.mkdir(mode=0o700)
        self.tool=self.base/'lease-helper'; self.tool.touch(); self.args=SimpleNamespace(runs=self.runs,lease_tool=self.tool,owner='test-owner',preflight=False,recover=False)
        self.lease=Mock(); self.lease.LeaseRequest.side_effect=lambda path,token,owner,tag: SimpleNamespace(token_file=token,tag=tag,owner=owner)
        self.lease.acquire.side_effect=self.acquire; self.lease.status.return_value={'ownership_verified':True}; self.lease.release.side_effect=self.release
        self.order=[]; self.baseline=runner.guard(); self.diagnostic=b'printf \'{"only":"fixture"}\\n\'\n'; self.failure={}; self.release_result=RELEASE
        self.lock_path=self.base/'shared-lease'; self.label=LABEL; self.secret='local-test-only'; self.stop=False

    def acquire(self,request):
        self.assertEqual(request.tag,'weread-mount-read-'+LABEL); self.assertEqual(request.owner,self.args.owner)
        self.order.append('acquire'); request.token_file.write_text('fixture'); return {'acquired_at_utc':TIME}

    def release(self,_,*,native_restored_and_connections_closed):
        self.assertIs(native_restored_and_connections_closed,True); self.order.append('release'); return self.release_result

    def wire(self,command,*,input,capture_output,env,timeout):
        self.assertEqual(command[0],'fixture-ssh'); self.assertNotIn(self.secret,' '.join(command)); self.assertNotIn(self.secret.encode(),input)
        self.assertTrue(capture_output); self.assertGreater(timeout,0); self.assertTrue(Path(env['SSH_ASKPASS']).is_file())
        self.assertEqual(env['WEREAD_TRANSIENT_PASSWORD'],self.secret)
        if input==self.diagnostic:
            self.assertEqual(command[-1],'export LC_ALL=C; exec sh -s'); mode='diagnose'; reply=RAW
        else:
            self.assertEqual(input,self.baseline); parts=shlex.split(command[-1]); self.assertEqual(parts[:6],['export','LC_ALL=C;','exec','sh','-s','--'])
            mode,label=parts[-2:]; self.assertEqual(label,self.label); reply=f'native_clear {mode} {label}\n'.encode()
            if mode=='verify' and self.stop: self.assertIn('observation',self.report())
        self.order.append(mode); response=self.failure.get(mode,(0,reply,b''))
        if isinstance(response,BaseException): raise response
        return subprocess.CompletedProcess(command,*response)

    def execute(self):
        real=Path.is_file
        with patch.object(Path,'is_file',lambda path:str(path)=='/opt/kindle-toolchain/ubuntu-run' or real(path)), \
                patch.object(runner,'programs',return_value=(self.baseline,self.diagnostic)), \
                patch.dict(runner.UTIL,load_lease=lambda *_:self.lease,ssh_command=lambda *_:['fixture-ssh']), \
                patch.object(runner,'LEASE',self.lock_path), patch.object(runner.secrets,'token_hex',return_value=self.label), \
                patch.object(runner.signal,'signal'), patch.object(runner.sys,'stdin',io.StringIO(self.secret+'\n')), \
                patch.object(runner.subprocess,'run',side_effect=self.wire), redirect_stdout(io.StringIO()):
            return runner.run(self.args)

    def report(self,mode='operations'):
        return json.loads((self.runs/'attempt'/(mode+'.json')).read_text())

    def test_completed_negative_classification_is_not_a_bind_gate_and_cannot_rerun(self):
        self.args.preflight=True; self.assertEqual(self.execute(),0); self.lease.LeaseRequest.assert_not_called(); self.assertEqual(list(self.runs.iterdir()),[])
        self.args.preflight=False; self.stop=True; self.assertEqual(self.execute(),0)
        self.assertEqual(self.order,['acquire','admit','diagnose','verify','release']); report=self.report()
        self.assertEqual(report['observation'],OBSERVATION); self.assertTrue(report['completed'] and report['native_and_absence_verified'])
        self.assertFalse(report['bind_tested']); self.assertIsNone(report['supports_bind']); self.assertEqual(report['formal_pairs'],0)
        self.assertFalse((self.runs/'attempt'/'private-token').exists()); self.assertNotIn(self.secret,json.dumps(report))
        self.assertEqual(set(path.name for path in (self.runs/'attempt').iterdir()),{'plan.json','operations.json'})
        with self.assertRaisesRegex(ValueError,'one-shot'): self.execute()

    def test_valid_observation_on_nonzero_exit_is_retained_before_verify_and_release(self):
        self.failure={'diagnose':(1,RAW,b'other private input')}; self.stop=True; self.assertEqual(self.execute(),1)
        report=self.report(); self.assertEqual(report['observation'],OBSERVATION); self.assertFalse(report['completed'])
        self.assertTrue(report['owner_and_token_checked']); self.assertNotIn('other private',json.dumps(report)); self.lease.release.assert_called_once()

    def test_lost_diagnostic_reply_verifies_and_releases_without_repeating(self):
        self.failure={'diagnose':subprocess.TimeoutExpired(['fixture-ssh'],13)}; self.assertEqual(self.execute(),1)
        report=self.report(); self.assertNotIn('observation',report); self.assertEqual(report['operations'][1]['reason'],'transport_timeout')
        self.assertEqual(self.order,['acquire','admit','diagnose','verify','release']); self.assertTrue(report['owner_and_token_checked'])

    def test_failed_independent_verify_leaves_own_token_and_recovery_cannot_redo_diagnosis(self):
        self.failure={'verify':(1,b'',b'refused baseline_native\n')}; self.assertEqual(self.execute(),1)
        self.assertEqual(self.order,['acquire','admit','diagnose','verify']); self.assertTrue(self.report()['recovery_required']); self.assertEqual(self.report()['observation'],OBSERVATION)
        self.args.recover=True; self.args.owner='changed'; self.order.clear()
        with self.assertRaisesRegex(ValueError,'original owner'): self.execute()
        self.args.owner='test-owner'; self.failure.clear(); self.assertEqual(self.execute(),0); self.assertEqual(self.order,['verify','release'])
        recovery=self.report('recovery'); self.assertFalse(recovery['bind_tested']); self.assertIsNone(recovery['supports_bind']); self.assertNotIn('observation',recovery)
        self.assertFalse((self.runs/'attempt'/'private-token').exists()); self.lease.acquire.assert_called_once()

    def test_busy_does_not_contact_device_and_readonly_admission_refusal_does_not_sample(self):
        self.lock_path.mkdir(); self.assertEqual(self.execute(),3); self.assertEqual(self.order,[]); self.lock_path.rmdir()
        self.failure={'admit':(1,b'',b'refused baseline_residue\n')}; self.assertEqual(self.execute(),1)
        self.assertEqual(self.order,['acquire','admit','verify','release']); self.assertEqual(self.report()['operations'][0]['reason'],'baseline_residue')

    def test_wrong_release_or_malformed_public_output_cannot_mark_complete(self):
        self.release_result=RELEASE|{'owner_and_token_checked':1}
        self.failure={'diagnose':(0,b'cannot share arbitrary private output\n',b'')}; self.assertEqual(self.execute(),1)
        report=self.report(); self.assertTrue(report['recovery_required']); self.assertNotIn('released_at_utc',report)
        self.assertNotIn('observation',report); self.assertNotIn('cannot share',json.dumps(report))


if __name__ == '__main__':
    unittest.main()
