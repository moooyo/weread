import io
import json
import os
from contextlib import redirect_stdout
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run_stock_ab_range as controller
from test_stock_ab_report import examples


class RangeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name).resolve(); self.root = self.work/'stock-library-ab'; self.root.mkdir()
        self.batch, self.receipts = examples(); self.batch.update(controller.SCOPE)
        self.owner = 'test-series-owner'
        self.batch.update(source_commit='1'*40,bundles={lane:{'members':{'selector-bind':66868}} for lane in 'AB'})
        self.first_label = self.batch['schedule'][0]['label']
        self.verify = self.enterContext(patch.object(controller,'verify_batch',return_value=self.batch))
        self.enterContext(redirect_stdout(io.StringIO()))
        for slot, receipt in zip(self.batch['schedule'],self.receipts,strict=True):
            (self.root/slot['visit']).mkdir(parents=True)
            receipt.update(owner=self.owner,acquisition='acquired')
        (self.root/'batch.json').write_text(json.dumps(self.batch)); controller.seal(self.work,self.first_label)
        self.calls = []; self.connected = True

    def save(self, index):
        attempt = self.root/self.batch['schedule'][index]['visit']/'attempt'; attempt.mkdir()
        (attempt/'operations.json').write_text(json.dumps(self.receipts[index]))
        return attempt

    def qualified(self, work, slot, owner, secret):
        self.assertEqual((work,owner,secret),(self.work,self.owner,b'test-only\n'))
        self.calls.append(slot['slot']); self.save(slot['slot']); return 0

    def run_range(self, first=0, last=3, dispatch=None):
        with patch.object(controller,'single_visit',side_effect=dispatch or self.qualified):
            return controller.deliver(self.work,controller.pinned(self.work,self.first_label),first,last,self.owner,b'test-only\n',lambda:self.connected)

    def test_contiguous_calls_require_original_qualified_handbacks_and_never_replay(self):
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range(first=1,last=1)
        self.assertEqual(self.run_range(),0); self.assertEqual(self.calls,[0,1,2,3])
        with self.assertRaisesRegex(controller.Refused,'already_attempted'): self.run_range(first=0,last=0)
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range(first=5,last=5)
        self.assertEqual(self.calls,[0,1,2,3]); self.assertEqual(self.run_range(first=4,last=4),0)

    def test_busy_or_failure_does_not_wait_retry_or_dispatch_a_second_slot(self):
        for code in (1,2,3,255,-9):
            with self.subTest(code=code):
                calls = []
                def refuse(work, slot, owner, secret):
                    calls.append(slot['slot']); return code
                self.assertEqual(self.run_range(dispatch=refuse),code if code in (1,2,3) else 2)
                self.assertEqual(calls,[0]); self.assertFalse((self.root/'visits/01-B/attempt').exists())

    def test_all_fourteen_fresh_visits_use_the_original_balanced_order(self):
        self.assertEqual(self.run_range(first=0,last=13),0)
        self.assertEqual(self.calls,list(range(14)))
        summary=controller.summarize(self.batch,controller.read_receipts(self.root,self.batch))
        self.assertEqual(summary['status'],'complete')
        self.assertEqual([pair['order'] for pair in summary['pairs']],['AB','BA']*3)

    def test_a_dispatched_b_warmup_with_no_claim_cannot_unlock_formal_pair_one(self):
        self.save(0); receipt=self.receipts[1]; receipt.update(primary='not_timed',refused='claim:unconfirmed')
        for step in receipt['operations']:
            if step['mode']=='claim': step.update(status=1,reason='unconfirmed')
        self.save(1)
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range(first=2,last=2)
        with self.assertRaisesRegex(controller.Refused,'already_attempted'): self.run_range(first=1,last=1)
        self.assertEqual(self.calls,[])

    def test_zero_exit_does_not_replace_a_receipt_or_missing_absence_evidence(self):
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range(dispatch=lambda *_:0)
        self.receipts[0]['native_guard_verified'] = False
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range()
        self.assertEqual(self.calls,[0]); self.assertFalse((self.root/'visits/01-B/attempt').exists())

    def test_different_owner_outstanding_token_or_recovery_are_never_accepted(self):
        self.save(0)
        self.receipts[1]['owner'] = 'wrong'; attempt=self.save(1)
        with self.assertRaisesRegex(controller.Refused,'not_qualified'): self.run_range(first=2)
        self.receipts[1]['owner'] = self.owner; (attempt/'operations.json').write_text(json.dumps(self.receipts[1]))
        (attempt/'private-token').touch()
        with self.assertRaisesRegex(controller.Refused,'outstanding'): self.run_range(first=2)
        (attempt/'private-token').unlink(); (attempt/'recovery.json').write_text('{}')
        with self.assertRaisesRegex(controller.Refused,'recovery'): self.run_range(first=2)
        self.assertEqual(self.calls,[])

    def test_stop_after_active_slot_leaves_the_unvisited_slot_for_explicit_resume(self):
        def handoff(*args):
            controller.stop(self.work); return self.qualified(*args)
        self.assertEqual(self.run_range(dispatch=handoff),4); self.assertEqual(self.calls,[0])
        self.assertEqual(self.run_range(first=1),4); self.assertEqual(self.calls,[0])
        with controller.controller_guard(self.work):
            with self.assertRaises(BlockingIOError), controller.controller_guard(self.work): pass
        controller.clear_stop(self.work); self.assertEqual(self.run_range(first=1,last=1),0)

    def test_caller_disconnect_stops_at_current_handback_and_receipts_cannot_be_relabelled(self):
        def disconnect(*args):
            self.connected=False; return self.qualified(*args)
        self.assertEqual(self.run_range(dispatch=disconnect),4); self.assertEqual(self.calls,[0])
        self.batch['schedule'][0]['label']='000000000000ffff'; (self.root/'batch.json').write_text(json.dumps(self.batch))
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): self.run_range(first=1,last=1)

    def test_manifest_requires_original_worker_labels_protocol_and_bound_helper(self):
        path = self.work/controller.APPROVAL
        approved = controller.private_record(path)
        self.verify.assert_called_once_with(self.work)
        for changed in (dict(worker='/other/frozen-batch'), dict(labels=list(reversed(approved['labels']))),
                        dict(claim_protocol='launcher-ack'), dict(selector_bytes=66044), dict(source_commit='2'*40)):
            with self.subTest(changed=tuple(changed)):
                path.chmod(0o600); path.write_text(json.dumps(approved | changed)); path.chmod(0o400)
                with self.assertRaisesRegex(controller.Refused,'approved_batch'): self.run_range()
        path.chmod(0o600); path.write_text(json.dumps(approved))
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): self.run_range()
        path.chmod(0o400)
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): controller.pinned(self.work,'deadbeef'*2)
        foreign=self.work/'foreign-seal'; path.rename(foreign); path.symlink_to(foreign)
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): self.run_range()
        path.unlink(); path.hardlink_to(foreign)
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): self.run_range()
        self.assertIsNone(controller.WORK_PATH.fullmatch('/root/weread-stock-bind-fixed-ab-20261122.ldhwovHO'))
        self.assertEqual(self.calls,[])

    def test_seal_is_exclusive_and_only_applies_to_a_new_verified_series(self):
        with self.assertRaises(FileExistsError): controller.seal(self.work,self.first_label)
        path = self.work/controller.APPROVAL; path.unlink()
        with self.assertRaisesRegex(controller.Refused,'not_sealed'): controller.pinned(self.work,self.first_label)
        with patch.object(controller,'verify_batch',side_effect=ValueError('invalid generated shell')):
            with self.assertRaisesRegex(ValueError,'invalid generated shell'): controller.seal(self.work,self.first_label)
        self.batch['bundles']['A']['members']['selector-bind']=66044
        with self.assertRaisesRegex(controller.Refused,'approved_batch'): controller.seal(self.work,self.first_label)
        self.batch['bundles']['A']['members']['selector-bind']=66868
        self.save(1)
        with self.assertRaisesRegex(controller.Refused,'already_attempted'): controller.seal(self.work,self.first_label)
        self.assertFalse(path.exists())

    def test_private_line_stays_in_stdin_and_pipe_eof_or_extra_data_stops_delivery(self):
        example = b'test-only\n'
        for trailing in (b'',b'x'):
            reader, writer = os.pipe()
            try:
                os.write(writer,example)
                with controller.live_input(reader) as (secret, attached):
                    self.assertTrue(attached()); self.assertEqual(secret,example)
                    with patch.object(subprocess,'run',return_value=subprocess.CompletedProcess([],3)) as child:
                        self.assertEqual(controller.single_visit(self.work,self.batch['schedule'][1],self.owner,secret),3)
                    call=child.call_args
                    self.assertNotIn('test-only',str(call.args)); self.assertNotIn('env',call.kwargs)
                    self.assertIs(call.kwargs['input'],secret); self.assertTrue(call.kwargs['start_new_session'])
                    self.assertEqual(call.args[0][-2:],['--lease-tool',controller.LEASE_TOOL])
                    if trailing: os.write(writer,trailing)
                    else: os.close(writer); writer=None
                    self.assertFalse(attached())
                self.assertEqual(secret,bytes(len(example)))
            finally:
                os.close(reader)
                if writer is not None: os.close(writer)
        reader,writer=os.pipe()
        try:
            os.write(writer,b'test-only\nextra\n')
            with self.assertRaises(controller.Refused), controller.live_input(reader): pass
        finally:
            os.close(reader); os.close(writer)


if __name__ == '__main__':
    unittest.main()
