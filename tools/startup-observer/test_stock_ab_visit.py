"""Exercise the single-visit lifecycle, bounded SSH channel, and original-token-only recovery offline."""

import copy
from contextlib import contextmanager
from io import BytesIO, StringIO
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from qualified_submit import ISOLATED_SEQUENCE
from run_daily_render import Refused
from run_stock_ab_visit import HEADER_RECEIVED, RequestChannel, Transport, audit_reference, execute, invocation, original_attempt, physical_operation, references, staged_wire, transport_failure
from stock_ab_transaction import GUARDS, LOAD_CLOSE_RECEIPTS, STATES, WireRefusal, checkpoint, checked, drive, fresh_journal, initial, valid_journal
from test_qualified_submit import LABEL, SENT, START, receipt


CONFIG = dict(label=LABEL, lane='B', slot=3, role='measured', pair=1, purpose='ordinary-stock-library-submit-ab',
              claim_protocol='independent-stock-app-v1')
FIXED = dict(launch='1:2:300:755:0:0:1755:1', document='1:3:43:755:0:0:1', application='1:4:400:755:0:0:1752:1')
ARM = next(row['receipt'] for row in receipt()['operations'] if row['mode']=='arm')
MARKER = next(row['receipt'] for row in receipt()['operations'] if row['mode']=='library_submit')


def line(value):
    return (json.dumps(value,separators=(',',':'))+'\n').encode('ascii')


class Fixture:
    def __init__(self, fail='', change=None):
        self.fail, self.change = fail, change or {}
        self.calls = []; self.channel_open = False; self.journals = []; self.saved = []
        self.report = initial(CONFIG, 'owner')
        self.journal = fresh_journal(LABEL)

    def call(self, mode):
        self.calls.append(mode)
        if mode == self.fail:
            raise WireRefusal('guard_refused', 1, load_retired=True) if mode in LOAD_CLOSE_RECEIPTS else Refused('guard_refused')
        values = dict(original={'state':'original',**FIXED}, absent={'state':'absent',**FIXED}, tap=START,
                      audit_before={'state':'audited'}, audit_after={'state':'audited'})
        values.update({key:{'state':value} for key,value in STATES.items()})
        return line(self.change.get(mode,values[mode]))

    def request(self):
        self.calls.append('open_request'); self.channel_open = True
        return self

    def armed(self):
        self.calls.append('request_armed')
        return line(self.change.get('request_armed',ARM))

    def running(self):
        return self.channel_open and self.fail != 'dead_before_tap'

    def collect(self):
        self.calls.append('library_submit')
        if self.fail == 'library_submit': raise Refused('missing_marker')
        return line(self.change.get('library_submit',MARKER))

    def close(self):
        self.channel_open = False; self.calls.append('request_closed')

    def release(self):
        self.calls.append('lease_release')
        if self.channel_open: raise AssertionError('Live channel at handback')
        if self.fail == 'lease_release': raise Refused('unconfirmed_release')
        return '2026-11-21T03:44:50.100+00:00'

    def run(self, recovering=False):
        return drive(self,self.release,self.report,self.journal,
                     lambda:self.saved.append(copy.deepcopy(self.report)),lambda:self.journals.append(copy.deepcopy(self.journal)),
                     recovering=recovering)


class VisitStateTests(unittest.TestCase):
    def test_full_visit_qualifies_real_marker_only_after_guarded_handback(self):
        fixture=Fixture(); self.assertEqual(fixture.run(),0)
        result=fixture.report; self.assertTrue(result['native_guard_verified'])
        self.assertEqual([step['mode'] for step in result['operations']],list(ISOLATED_SEQUENCE))
        self.assertEqual([step['mode'] for step in result['operations'] if step.get('load_retired') is True],['stage','install'])
        self.assertEqual([step['mode'] for step in result['guard_checks']],list(GUARDS))
        self.assertEqual(result['qualified']['elapsed_interval_ns'],[SENT-START['up_after_ns'],SENT-START['up_before_ns']])
        self.assertNotIn('observation',result)
        self.assertTrue(all(row['released'] for row in fixture.saved if row.get('qualified')))
        self.assertLess(fixture.calls.index('request_closed'),fixture.calls.index('stop'))
        self.assertEqual(fixture.calls[-2:],['audit_after','lease_release'])
        self.assertIn('activate_requested',[state['phase'] for state in fixture.journals])
        self.assertTrue(all(valid_journal(state,LABEL) for state in fixture.journals))

    def test_recovery_refuses_reordered_or_foreign_persistent_state(self):
        fixture=Fixture(); fixture.run(); snapshots={state['phase']:state for state in fixture.journals}
        for phase,changes in [('not_started',{'restored':True}),('activate_requested',{'original':None}),
                              ('activate_requested',{'audit_before':False}),('absent',{'guard_complete':False}),
                              ('restored',{'stage_possible':False}),('close_requested',{'label':'f'*16})]:
            record=dict(snapshots.get(phase,fresh_journal(LABEL)),**changes)
            with self.subTest(phase=phase,changes=changes): self.assertFalse(valid_journal(record,LABEL))

    def test_switch_or_tap_error_closes_restores_and_never_repeats(self):
        for mode in ('stage','install','original','audit_before','prepare','activate','arm','tap','claim','witness','library_submit'):
            with self.subTest(mode=mode):
                fixture=Fixture(fail=mode); self.assertEqual(fixture.run(),1)
                self.assertTrue(fixture.report['released']); self.assertFalse(fixture.report['recovery_required'])
                self.assertNotIn('qualified',fixture.report); self.assertLessEqual(fixture.calls.count('tap'),1)
                self.assertIn('absent',fixture.calls)
        fixture=Fixture(fail='admit'); self.assertEqual(fixture.run(),1); self.assertEqual(fixture.calls,['admit','lease_release'])

    def test_preinput_view_refusal_returns_without_becoming_a_sample(self):
        fixture = Fixture()
        original_call = fixture.call

        def changed_view(mode):
            if mode == 'prepare':
                fixture.calls.append(mode)
                raise WireRefusal('native_view_changed',1)
            return original_call(mode)

        fixture.call = changed_view
        self.assertEqual(fixture.run(),1)
        self.assertEqual(fixture.calls,['admit','stage','install','original','audit_before','prepare',
                                        'stop','restore','close','absent','audit_after','lease_release'])
        self.assertEqual(fixture.report['refused'],'prepare:native_view_changed')
        self.assertEqual(fixture.report['primary'],'not_timed')
        self.assertTrue(fixture.report['released']); self.assertFalse(fixture.report['recovery_required'])
        self.assertNotIn('qualified',fixture.report)

    def test_unanswered_load_stays_pending_and_recovery_cannot_use_absence(self):
        for mode in LOAD_CLOSE_RECEIPTS:
            with self.subTest(mode=mode):
                fixture=Fixture()
                original=fixture.call

                def disconnect(call):
                    if call == mode:
                        self.assertEqual(fixture.journals[-1]['load_pending'],mode)
                        raise WireRefusal('ssh_transport_exit',255)
                    return original(call)

                fixture.call=disconnect
                self.assertEqual(fixture.run(),2)
                self.assertEqual(fixture.calls,['admit'] if mode=='stage' else ['admit','stage'])
                self.assertEqual(fixture.journal['load_pending'],mode)
                self.assertTrue(valid_journal(fixture.journal,LABEL))
                self.assertEqual(fixture.report['close_reason'],'remote_load_retirement_unconfirmed')
                self.assertTrue(fixture.report['recovery_required'])
                fixture.calls.clear()
                self.assertEqual(fixture.run(True),2)
                self.assertEqual(fixture.calls,[])

    def test_confirmed_early_refusal_still_needs_retained_original_byte_audit(self):
        fixture=Fixture(fail='install')
        original=fixture.call

        def changed_originals(mode):
            if mode=='audit_after':
                fixture.calls.append(mode)
                raise Refused('production_byte_audit')
            return original(mode)

        fixture.call=changed_originals
        self.assertEqual(fixture.run(),2)
        self.assertEqual(fixture.report['refused'],'install:guard_refused')
        self.assertEqual(fixture.report['close_reason'],'audit_after:production_byte_audit')
        self.assertIn('absent',fixture.calls)
        self.assertNotIn('lease_release',fixture.calls)

    def test_failed_pending_checkpoint_cannot_dispatch_or_release(self):
        for after_return in (False,True):
            with self.subTest(after_return=after_return):
                fixture=Fixture()

                def persist():
                    if fixture.journal['phase']=='stage_requested' and (fixture.journal['load_pending'] is None)==after_return:
                        raise OSError('synthetic checkpoint failure')
                    fixture.journals.append(copy.deepcopy(fixture.journal))

                rc=drive(fixture,fixture.release,fixture.report,fixture.journal,lambda:None,persist)
                self.assertEqual(rc,2)
                self.assertEqual(fixture.calls,['admit','stage'] if after_return else ['admit'])
                self.assertEqual(fixture.journal['load_pending'],'stage')
                self.assertTrue(fixture.report['recovery_required'])

    def test_dead_or_foreign_reader_never_taps(self):
        for fixture in (Fixture(fail='dead_before_tap'),Fixture(change={'request_armed':{**ARM,'label':'f'*16}}),
                        Fixture(change={'request_armed':{**ARM,'request_clock':'CLOCK_REALTIME'}})):
            self.assertEqual(fixture.run(),1); self.assertNotIn('tap',fixture.calls)
            self.assertTrue(fixture.report['released'])

    def test_same_length_wrong_release_and_missing_marker_never_get_measured(self):
        for change in (dict(marker='private raw log'), dict(label='f'*16),
                       dict(tile_receipt={**START,'up_after_ns':START['up_after_ns']+1}), dict(observed_ns=SENT-1)):
            with self.subTest(change=change):
                fixture=Fixture(change={'library_submit':{**MARKER,**change}})
                self.assertEqual(fixture.run(),1); self.assertTrue(fixture.report['released'])
                self.assertNotIn('qualified',fixture.report); self.assertNotIn('private raw log',str(fixture.report))

    def test_unconfirmed_restoration_handback_or_changed_original_keeps_token(self):
        for mode in ('stop','restore','close','absent','audit_after','lease_release'):
            with self.subTest(mode=mode):
                fixture=Fixture(fail=mode); self.assertEqual(fixture.run(),2); self.assertFalse(fixture.report['released'])
                self.assertTrue(fixture.report['recovery_required']); self.assertNotIn('qualified',fixture.report)
        for field in FIXED:
            changed=FIXED[field].split(':'); changed[1]='8'; bad={**FIXED,field:':'.join(changed)}
            fixture=Fixture(change={'absent':dict(state='absent',**bad)}); self.assertEqual(fixture.run(),2)
            self.assertNotIn('lease_release',fixture.calls); self.assertEqual(fixture.report['close_reason'],'original_identity_changed')

    def test_catalog_touchable_document_omits_times_but_not_identity_or_content_guards(self):
        fixture=Fixture(); self.assertEqual(fixture.run(),0)
        guards=fixture.report['guard_checks']; self.assertEqual([row['mode'] for row in guards],list(GUARDS))
        self.assertEqual(len(guards[0]['receipt']['document'].split(':')),7)
        for phase in ('original','absent'):
            for key,invalid in [('document','1:3:43:755:0:0:0:1'),('document','1:3:43:755:0:0:1751:1'),
                                ('launch',FIXED['document']),('application',FIXED['document'])]:
                with self.subTest(phase=phase,key=key,invalid=invalid), self.assertRaises(Refused):
                    checked(phase,line(dict(state=phase,**{**FIXED,key:invalid})),LABEL)
        for idx in range(7):
            changed=FIXED['document'].split(':'); changed[idx]='6'
            fixture=Fixture(change={'absent':dict(state='absent',**{**FIXED,'document':':'.join(changed)})})
            self.assertEqual(fixture.run(),2); self.assertNotIn('lease_release',fixture.calls)
        audit=Fixture(fail='audit_after'); self.assertEqual(audit.run(),2); self.assertNotIn('lease_release',audit.calls)

    def test_close_only_on_dropped_ack_does_not_accept_lost_stage_or_launch(self):
        absent={name:{'state':'already_absent'} for name in ('stop','restore','close')}
        for restored,navigated,expected in ((False,True,2),(True,True,0),(False,False,0)):
            with self.subTest(restored=restored,navigated=navigated):
                fixture=Fixture(change=absent); fixture.journal.update(stage_possible=True,navigation_possible=navigated,
                     phase='restored' if restored else 'tap_requested' if navigated else 'original_observed',
                     restored=restored,original={'state':'original',**FIXED},audit_before=navigated)
                self.assertTrue(valid_journal(fixture.journal,LABEL))
                self.assertEqual(fixture.run(True),expected); self.assertNotIn('tap',fixture.calls)
                self.assertNotIn('open_request',fixture.calls); self.assertNotIn('qualified',fixture.report)
                self.assertEqual('lease_release' in fixture.calls,expected==0)

    def test_foreign_output_or_numeric_type_is_not_archived(self):
        for mode, row in [('original',dict(state='original',**{**FIXED,'document':'secret'})),
                          ('audit_before',dict(state='audited',private_path='/private')),
                          ('request_armed',dict(ARM,request_armed_ns=True)),('tap',dict(START,private='secret'))]:
            with self.subTest(mode=mode), self.assertRaises(Refused): checked(mode,line(row),LABEL)


class TransportTests(unittest.TestCase):
    def test_one_frozen_shell_channel_keeps_both_receipts_across_one_collect_and_eof(self):
        shell=b'''printf '{"state":"armed"}\\n'; IFS= read -r cmd; [ "$cmd" = collect ] || exit 1
if IFS= read -r line; then exit 1; fi; printf '{"result":1}\\n' '''
        cmd, header=invocation(shell,'request',LABEL)
        channel=RequestChannel(['sh','-c',cmd],header,os.environ,lambda:None)
        self.addCleanup(channel.close); self.assertEqual(channel.armed(),b'{"state":"armed"}\n')
        self.assertEqual(channel.collect(),b'{"result":1}\n'); self.assertFalse(channel.running())
        with self.assertRaises(Refused): channel.collect()

    def test_channel_cancellation_or_extra_private_output_is_not_a_sample(self):
        for tail in ("printf 'private\\n'",'exit 5'):
            shell=("printf 'ARM\\n'; IFS= read -r cmd; [ \"$cmd\" = collect ] || exit 0; printf 'END\\n'; "+tail).encode()
            cmd,header=invocation(shell,'request',LABEL); channel=RequestChannel(['sh','-c',cmd],header,os.environ,lambda:None)
            try:
                self.assertEqual(channel.armed(),b'ARM\n')
                with self.assertRaises(Refused): channel.collect()
            finally:
                channel.close()
        cmd,header=invocation(b"printf 'ARM\\n'; read cmd; [ \"$cmd\" = cancel ]",'request',LABEL)
        channel=RequestChannel(['sh','-c',cmd],header,os.environ,lambda:None); channel.armed(); channel.close()
        self.assertEqual(channel.process.returncode,0)

    def test_install_and_audit_transport_carry_exact_binary_after_header(self):
        with tempfile.TemporaryDirectory() as work:
            file=Path(work)/'inputs.tgz'; data=b'\x1f\x8b\n\x00\xff'; file.write_bytes(data)
            program=(b'[ "$2" = '+LABEL.encode()+b' ] || exit 64; cat; '
                     b'if [ "$1" = install ]; then printf "WEREAD_STOCK_INSTALL_RETIRED %s\\n" "$2" >&2; fi')
            wire=staged_wire(file,invocation(program,'install',LABEL)[1],Path(work)/'install.stream')
            sent=[]; transport=Transport(['sh','-c'],program,LABEL,file,b'DOC\nLAUNCH\n',os.environ,lambda:sent.append('check'),wire=wire)
            self.assertEqual(transport.call('install'),data)
            self.assertEqual(transport.call('audit_after'),b'DOC\nLAUNCH\n'); self.assertEqual(sent,['check','check'])

    def test_arm_framing_limits_exec_length_and_does_not_consume_any_archive_byte(self):
        with tempfile.TemporaryDirectory() as work:
            root=Path(work); archive=root/'payload'; data=bytes(range(256))*10+b'\n\x00\r\xff'; archive.write_bytes(data)
            secret='private-content-in-program'; baseline=("#"+secret+"x"*47000+"\ncat\n").encode()
            command,header=invocation(baseline,'install',LABEL); self.assertLess(len(command.encode()),256)
            self.assertNotIn(secret,command); self.assertGreater(len(header),47000)
            compare=staged_wire(archive,header,root/'wire'); self.assertEqual(compare.stat().st_size,len(header)+len(data))
            with compare.open('rb') as stream:
                completed=subprocess.run(['sh','-c',command],stdin=stream,capture_output=True,timeout=8)
            self.assertEqual((completed.returncode,completed.stdout,completed.stderr),(0,data,HEADER_RECEIVED+b'\n'))
            short=subprocess.run(['sh','-c',command],input=header[:-1],capture_output=True,timeout=8)
            self.assertEqual((short.returncode,short.stdout,short.stderr),(64,b'',b''))

    def test_exit_diagnostics_preserve_sequence_without_exporting_private_ssh_text(self):
        for code,raw,reason,observed in [(255,b'private auth failure\n','ssh_transport_exit',False),
                (None,HEADER_RECEIVED+b'\nprivate path\n','transport_timeout',True),
                (64,b'private path\n','install_header_unconfirmed',False),
                (9,HEADER_RECEIVED+b'\nprivate path\n','unconfirmed_after_header',True),
                (1,HEADER_RECEIVED+b'\nprivate path\nrefused install\n','install',True),
                (1,b'refused bind_mount_call_22\n','bind_mount_call_22',False),
                (1,b'refused bind_unmount_call_16\n','bind_unmount_call_16',False),
                (1,b'refused bind_mount_call_0\n','install_header_unconfirmed',False),
                (1,b'refused bind_mount_call_4096\n','install_header_unconfirmed',False),
                (1,b'refused bind_mount_call_13:/private\n','install_header_unconfirmed',False),
                (1,b'refused bind_mount_call_16\nrefused bind_mount_call_22\n','install_header_unconfirmed',False)]:
            fixture=Fixture(); previous=fixture.call
            def call(mode):
                if mode=='install': raise transport_failure(mode,code,raw)
                return previous(mode)
            fixture.call=call
            self.assertEqual(fixture.run(),2)
            failed=next(row for row in fixture.report['operations'] if row['mode']=='install')
            self.assertEqual(failed.get('ssh_returncode'),code); self.assertIs(failed['install_header_observed'],observed)
            self.assertEqual(failed['reason'],reason); self.assertNotIn('private',str(fixture.report)); self.assertFalse(fixture.report['released'])
            self.assertEqual(fixture.report['refused'], 'install:'+reason)
            self.assertEqual(fixture.journal['load_pending'],'install')
            self.assertEqual(fixture.report['close_reason'],'remote_load_retirement_unconfirmed')

    def test_no_install_wire_or_missing_header_sentinel_cannot_activate(self):
        with tempfile.TemporaryDirectory() as work:
            root=Path(work); archive=root/'payload'; archive.write_bytes(b'fixture')
            program=b'cat >/dev/null; echo installed'; remote=Transport(['sh','-c'],program,LABEL,archive,b'',os.environ,lambda:None)
            with self.assertRaisesRegex(Refused,'install_wire_not_prepared'): remote.call('install')
            remote.wire=staged_wire(archive,invocation(program,'install',LABEL)[1],root/'wire')
            marker=LOAD_CLOSE_RECEIPTS['install']+LABEL.encode('ascii')+b'\n'
            simulated=subprocess.CompletedProcess([],0,b'{"state":"installed"}\n',marker)
            with patch('run_stock_ab_visit.subprocess.run',return_value=simulated), self.assertRaises(WireRefusal) as refused:
                remote.call('install')
            self.assertEqual((str(refused.exception),refused.exception.returncode,refused.exception.header_observed),('missing_install_header_receipt',0,False))

    def test_retirement_requires_one_final_visit_bound_marker_and_normal_ssh_exit(self):
        remote=Transport(['unused'],b':',LABEL,None,b'',os.environ,lambda:None)
        remote.wire=Path(__file__)
        for mode,state in (('stage','staged'),('install','installed')):
            prefix=HEADER_RECEIVED+b'\n' if mode=='install' else b''
            good=LOAD_CLOSE_RECEIPTS[mode]+LABEL.encode('ascii')+b'\n'
            other=LOAD_CLOSE_RECEIPTS[('install' if mode=='stage' else 'stage')]+LABEL.encode('ascii')+b'\n'
            for code,tail,expected in ((0,good,True),(1,good,True),(255,good,False),(-9,good,False),
                                       (0,b'',False),(0,good[:-1],False),(0,good+good,False),(0,other,False),
                                       (0,good[:-17]+b'f'*16+b'\n',False),(0,good+b'extra\n',False)):
                with self.subTest(mode=mode,code=code,tail=tail):
                    result=subprocess.CompletedProcess([],code,line({'state':state}),prefix+tail)
                    with patch('run_stock_ab_visit.subprocess.run',return_value=result):
                        if code==0 and expected:
                            self.assertEqual(remote.call(mode),result.stdout)
                        else:
                            with self.assertRaises(WireRefusal) as failure:
                                remote.call(mode)
                            self.assertIs(failure.exception.load_retired,expected)
            with patch('run_stock_ab_visit.subprocess.run',side_effect=subprocess.TimeoutExpired([],1,stderr=prefix+good)):
                with self.assertRaises(WireRefusal) as failure:
                    remote.call(mode)
                self.assertIs(failure.exception.load_retired,False)

    def test_pinned_original_reference_order_does_not_depend_on_tar_order(self):
        with tempfile.TemporaryDirectory() as work:
            path=Path(work)/'in.tgz'
            with tarfile.open(path,'w:gz') as output:
                for name,body in (('launch-reference',b'LAUNCH\n'),('document-reference',b'DOC\n')):
                    member=tarfile.TarInfo(name); member.size=len(body); output.addfile(member,BytesIO(body))
            self.assertEqual(references(path),b'DOC\nLAUNCH\n')
            bad=Path(work)/'ref'; bad.write_bytes(b'data'); bad.chmod(0o644)
            with self.assertRaises(Refused): audit_reference(bad)

    def test_no_second_attempt_and_only_original_owner_may_recover(self):
        with tempfile.TemporaryDirectory() as root:
            plan=Path(root); args=SimpleNamespace(owner='orig',recover=None); attempt=original_attempt(plan,CONFIG,args)
            report=initial(CONFIG,'orig'); checkpoint(attempt/'operations.json',report)
            with self.assertRaises(Refused): original_attempt(plan,CONFIG,args)
            (attempt/'private-token').write_text('fixture-only'); args.recover=attempt
            self.assertEqual(original_attempt(plan,CONFIG,args),attempt); args.owner='other'
            with self.assertRaises(Refused): original_attempt(plan,CONFIG,args)
            args.owner='orig'; report['released']=True; checkpoint(attempt/'operations.json',report)
            with self.assertRaises(Refused): original_attempt(plan,CONFIG,args)

    def test_shared_lock_busy_before_the_owner_record_or_device_access(self):
        events, config, args = self.host_visit_fixture()

        @contextmanager
        def busy():
            events.append('busy')
            raise Refused('physical_operation_busy')
            yield

        with patch('run_stock_ab_visit.physical_operation', new=busy):
            self.assertEqual(execute(args), 3)
        self.assertEqual(events, ['wire_prepared', 'busy'])
        attempt = args.plan / 'attempt'
        self.assertFalse((attempt / 'private-token').exists())
        result = json.loads((attempt / 'operations.json').read_bytes())
        self.assertEqual(result['acquisition'], 'busy')
        self.assertFalse(result['device_accessed'])
        self.assertEqual(result['label'], config['label'])

    def test_shared_lock_spans_transport_and_handback_but_not_qualification(self):
        events, _, args = self.host_visit_fixture()

        @contextmanager
        def operation():
            events.append('locked')
            try:
                yield
            finally:
                events.append('unlocked')

        def drive(transport, release, report, journal, save, persist, *, recovering):
            self.assertFalse(recovering)
            transport.owned()
            events.append('device_cleanup')
            release()
            self.assertEqual(events[-1], 'unlocked')
            events.append('qualification')
            report['released'] = True
            return 0

        with patch('run_stock_ab_visit.physical_operation', new=operation), patch('run_stock_ab_visit.drive', new=drive):
            self.assertEqual(execute(args), 0)
        self.assertEqual(events, ['wire_prepared', 'locked', 'owner_acquired', 'owner_checked',
                                  'device_cleanup', 'owner_released', 'unlocked', 'qualification'])

    def test_canonical_lock_entry_does_not_translate_errors_in_its_body(self):
        class Busy(RuntimeError):
            pass

        @contextmanager
        def busy():
            raise Busy('fixture collision')
            yield

        with patch('run_stock_ab_visit.runpy.run_path', return_value={'operation_lock': busy, 'DeviceBusy': Busy}):
            with self.assertRaisesRegex(Refused, 'physical_operation_busy'), physical_operation():
                self.fail('Canonical lock collision did not block the body')

        @contextmanager
        def acquired():
            yield

        with patch('run_stock_ab_visit.runpy.run_path', return_value={'operation_lock': acquired, 'DeviceBusy': Busy}):
            with self.assertRaisesRegex(Busy, 'fixture body'), physical_operation():
                raise Busy('fixture body')

    def host_visit_fixture(self):
        work = Path(self.enterContext(tempfile.TemporaryDirectory()))
        plan = work / 'plan'
        plan.mkdir()
        program = plan / 'device.sh'
        program.write_text('true\n')
        args = SimpleNamespace(work=work, plan=plan, preflight=False, recover=None, owner='owner', lease_tool=program)
        events = []

        def acquire(request):
            events.append('owner_acquired')
            return {'acquired_at_utc': '2026-11-28T00:00:00+00:00'}

        def owned(request):
            events.append('owner_checked')
            return {'ownership_verified': True}

        def release(request, *, native_restored_and_connections_closed):
            self.assertTrue(native_restored_and_connections_closed)
            events.append('owner_released')
            return dict(owner_and_token_checked=True, native_restored_and_connections_closed=True,
                        only_owned_owner_json_removed_then_rmdir=True, lease_directory_absent_after_release=True,
                        released_at_utc='2026-11-28T00:01:00+00:00')

        def wire(*args):
            events.append('wire_prepared')
            return None

        lease = SimpleNamespace(LeaseRequest=lambda path, token, *other: SimpleNamespace(token_file=token),
                                acquire=acquire, status=owned, release=release)
        shared = dict(ssh_command=lambda *a: ['fixture-only'], load_lease=lambda p: lease, LEASE=work / 'fixture-lease')
        self.enterContext(patch.dict(sys.modules, stock_ab_freeze=SimpleNamespace(verify_visit=lambda *a, **k: CONFIG)))
        for name, value in (('WORKER_RUNNER', program), ('references', lambda _: b'doc\nlaunch\n'),
                            ('password_input', lambda: 'fixture-only'), ('staged_wire', wire)):
            self.enterContext(patch(f'run_stock_ab_visit.{name}', new=value))
        self.enterContext(patch('run_stock_ab_visit.runpy.run_path', return_value=shared))
        self.enterContext(patch('run_stock_ab_visit.signal.signal'))
        self.enterContext(patch.object(sys, 'stdout', StringIO()))
        return events, CONFIG, args

    def test_preflight_does_not_read_password_resolve_ssh_or_create_attempt(self):
        with tempfile.TemporaryDirectory() as root:
            work=Path(root); plan=work/'plan'; plan.mkdir(); (plan/'device.sh').write_text('true\n')
            args=SimpleNamespace(work=work,plan=plan,preflight=True,recover=None,owner=None,lease_tool=None)
            frozen=SimpleNamespace(verify_visit=lambda *a,**k: CONFIG)
            with patch.dict(sys.modules,stock_ab_freeze=frozen), patch('run_stock_ab_visit.WORKER_RUNNER',plan/'device.sh'), \
                    patch('run_stock_ab_visit.references',return_value=b'doc\nlaunch\n'), \
                    patch('run_stock_ab_visit.password_input',side_effect=AssertionError),patch('run_stock_ab_visit.runpy.run_path',side_effect=AssertionError):
                self.assertEqual(execute(args),0)
            self.assertFalse((plan/'attempt').exists())


if __name__ == '__main__':
    unittest.main()
