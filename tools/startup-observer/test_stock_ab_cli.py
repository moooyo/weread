"""Offline CLI integration with a local transport and a private, simulated exclusive lease."""

from contextlib import ExitStack, contextmanager
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from run_daily_render import Refused, now
from run_stock_ab_visit import execute
from test_stock_ab_visit import ARM, CONFIG, FIXED, MARKER, START


class LocalLeases:
    def __init__(self, lock):
        self.lock = lock
        self.requested = 0
        self.busy = False
        self.held = False

    def LeaseRequest(self, directory, token, owner, name):
        return SimpleNamespace(token_file=token, directory=directory, owner=owner, name=name)

    def acquire(self, request):
        self.requested += 1
        if self.busy:
            error = RuntimeError('simulated race'); error.code = 'lease_exists'; error.details = {}; raise error
        self.lock.mkdir(); request.token_file.write_text('fixture-token'); self.held=True
        return {'acquired_at_utc':now()}

    def status(self, request):
        return {'ownership_verified':self.held and request.token_file.read_text()=='fixture-token'}

    def release(self, request, native_restored_and_connections_closed):
        if not self.status(request)['ownership_verified'] or not native_restored_and_connections_closed:
            raise RuntimeError('fixture ownership or handback missing')
        self.lock.rmdir(); self.held=False
        return dict(owner_and_token_checked=True,only_owned_owner_json_removed_then_rmdir=True,
                    native_restored_and_connections_closed=True,lease_directory_absent_after_release=True,released_at_utc=now())


def encoded(record):
    return json.dumps(record,separators=(',',':'))


class HostCLITests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup); self.work=Path(temporary.name)
        self.plan=self.work/'plan'; self.plan.mkdir(); self.lease=LocalLeases(self.work/'lease'); self.lease_tool=self.work/'lease.py'
        self.lease_tool.touch(); self.gate=self.work/'fail-restore'; self.traffic=self.work/'calls'
        self.args=SimpleNamespace(work=self.work,plan=self.plan,owner='original',lease_tool=self.lease_tool,preflight=False,recover=None)
        self.attempt=self.plan/'attempt'; self.marker_path=self.work/'marker'; self.marker_path.write_text(encoded(MARKER)+'\n')
        shell=f'''set -eu
printf '%s\\n' "$1" >>'{self.traffic}'
case "$1" in
admit) state=admitted;; stage) printf 'WEREAD_STOCK_STAGE_RETIRED %s\\n' "$2" >&2; state=staged;;
install) cat >/dev/null; printf 'WEREAD_STOCK_INSTALL_RETIRED %s\\n' "$2" >&2; state=installed;; prepare) state=prepared;;
activate) state=activated;; arm) state=armed;; claim) state=claimed;; witness) state=witnessed;; stop) state=stopped;;
restore) [ ! -e '{self.gate}' ] || {{ echo 'refused native_restore' >&2; exit 1; }}; state=restored;; close) state=closed;;
original|absent) printf '{{"state":"%s",{encoded(FIXED)[1:]}\\n' "$1"; exit;;
audit) test "$(cat)" = "DOC-LAUNCH"; state=audited;;
tap) printf '%s\\n' '{encoded(START)}'; exit;;
request) printf '%s\\n' '{encoded(ARM)}'; IFS= read -r reply; [ "$reply" = collect ] || exit 0; if IFS= read -r extra; then exit 1; fi
cat '{self.marker_path}'; exit;; *) exit 64;;
esac
printf '{{"state":"%s"}}\\n' "$state"
'''
        (self.plan/'device.sh').write_text(shell)
        with tarfile.open(self.plan/'inputs.tgz','w:gz') as archive:
            for name,data in [('launch-reference',b'LAUNCH'),('document-reference',b'DOC-')]:
                entry=tarfile.TarInfo(name); entry.size=len(data); archive.addfile(entry,io.BytesIO(data))
        self.verifies=[]
        def verify(work,plan,*,recovery):
            self.verifies.append(recovery); return CONFIG
        self.stack=ExitStack(); self.addCleanup(self.stack.close)
        self.stack.enter_context(patch.dict(sys.modules,stock_ab_freeze=SimpleNamespace(verify_visit=verify)))
        self.stack.enter_context(patch('run_stock_ab_visit.WORKER_RUNNER',self.lease_tool))
        shared={'ssh_command':lambda *unused:['sh','-c'],'load_lease':lambda unused:self.lease,'LEASE':self.lease.lock}
        self.stack.enter_context(patch('run_stock_ab_visit.runpy.run_path',return_value=shared))
        self.lock_held = False

        @contextmanager
        def operation():
            self.assertFalse(self.lock_held)
            self.lock_held = True
            try:
                yield
            finally:
                self.lock_held = False

        self.stack.enter_context(patch('run_stock_ab_visit.physical_operation', new=operation))
        self.stack.enter_context(patch('sys.stdout',new_callable=io.StringIO)); self.stack.enter_context(patch('signal.signal'))

    def run_cli(self):
        with patch('sys.stdin',SimpleNamespace(isatty=lambda:False,buffer=io.BytesIO(b'only-private-fixture\n'))):
            return execute(self.args)

    def test_actual_cli_with_local_stream_archive_and_credentials_never_records_private_input(self):
        self.assertEqual(self.run_cli(),0)
        operations=json.loads((self.attempt/'operations.json').read_text())
        self.assertEqual(operations['primary'],'qualified'); self.assertTrue(operations['native_guard_verified']); self.assertFalse(self.lease.held)
        self.assertEqual(self.traffic.read_text().splitlines().count('tap'),1)
        self.assertFalse((self.attempt/'private-token').exists())
        for path in self.attempt.iterdir(): self.assertNotIn(b'only-private-fixture',path.read_bytes())
        with self.assertRaises(Refused): self.run_cli()
        self.assertEqual(self.lease.requested,1)

    def test_held_lock_and_atomic_race_do_not_start_any_ssh(self):
        self.lease.lock.mkdir(); self.assertEqual(self.run_cli(),3); self.assertFalse(self.attempt.exists()); self.lease.lock.rmdir()
        self.lease.busy=True; self.assertEqual(self.run_cli(),3)
        prior=json.loads((self.attempt/'operations.json').read_text()); self.assertEqual(prior['acquisition'],'busy')
        self.assertFalse(prior['device_accessed']); self.assertFalse(self.traffic.exists()); self.lease.busy=False
        self.assertEqual(self.run_cli(),0); self.assertEqual(self.traffic.read_text().splitlines().count('tap'),1)

    def test_exact_close_only_recovery_never_needs_upload_and_never_produces_a_sample(self):
        self.gate.touch(); self.assertEqual(self.run_cli(),2); self.assertTrue(self.lease.held)
        self.assertFalse(self.lock_held)
        original=(self.attempt/'operations.json').read_bytes(); failed=json.loads(original); self.assertTrue(failed['recovery_required'])
        self.assertFalse(failed['released']); self.assertNotIn('qualified',failed)
        self.args.recover=self.attempt; self.args.owner='wrong'
        with self.assertRaises(Refused): self.run_cli()
        self.args.owner='original'; self.gate.unlink(); (self.plan/'inputs.tgz').unlink()
        prior_count=len(self.traffic.read_text().splitlines()); self.assertEqual(self.run_cli(),0)
        self.assertEqual(self.traffic.read_text().splitlines()[prior_count:],['stop','restore','close','absent','audit'])
        self.assertEqual((self.attempt/'operations.json').read_bytes(),original); recovery=json.loads((self.attempt/'recovery.json').read_text())
        self.assertTrue(recovery['recovery_only']); self.assertTrue(recovery['released']); self.assertNotIn('qualified',recovery)
        self.assertFalse((self.attempt/'private-token').exists()); self.assertEqual(self.lease.requested,1)

    def test_released_but_empty_seed_or_unbound_submit_is_never_a_timed_visit(self):
        self.marker_path.write_text(encoded(dict(MARKER,marker=MARKER['marker'].replace('entries','empty')))+'\n')
        self.assertEqual(self.run_cli(),1); self.assertFalse(self.lease.held)
        report=json.loads((self.attempt/'operations.json').read_text()); self.assertTrue(report['released'])
        self.assertEqual(report['primary'],'not_timed'); self.assertEqual(report['qualification_reason'],'unexpected_public_fixture_content')


if __name__ == '__main__':
    unittest.main()
