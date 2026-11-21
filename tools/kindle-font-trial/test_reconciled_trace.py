"""Reconciliation remains a distinct, event-loop-only outcome in closed or simulated visits."""

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from analyze import ImpossibleOrientationTrace, metrics
from analyze_orientation import orientation_sample
from orientation_contract import GateError
from orientation_device import section
from orientation_gate import pilot_reference
from pose_flip_device import device_program
from prepare_pose_flip import RECONCILED_CONTRACT
from test_reconcile_freeze import reconciled_build_evidence
import test_first_draw_evidence as draw_fixtures
import test_orientation as balanced
import test_orientation_gate as pictures


def marker(trace, site, old):
    return trace.replace(site + '_orientation_' + old, site + '_orientation_reconciled')


class ReconciledTrace(unittest.TestCase):
    def export(self, case, trace, *, experiment=True, candidate_bytes=6):
        body = device_program(candidate_bytes, reconciled=experiment)
        program = section(body, 'export_markers() {', 'finish_case() {')
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            log = directory / 'bootstrap.log'
            log.write_text('unrelated private output\n' + trace)
            run = subprocess.run(['sh','-ec', 'arm=$1; log=$2; out=$3; plain() { test -f "$1"; };\n' + program + '\nexport_markers',
                                  'collector',case[0],log,directory], capture_output=True)
            if run.returncode:
                return None
            captured = (directory/'markers').read_text()
            self.assertNotIn('private', captured)
            return captured

    def test_only_explicit_B_event_loop_can_export_and_parse_the_new_marker(self):
        for case, terminal, site in [('A1','timeout','splash'),('A1','timeout','event_loop'),('B1','reused','splash'),('B1','reused','event_loop')]:
            with self.subTest(case=case,site=site):
                request, original = balanced.records(case,100)
                raw = marker(original,site,terminal)
                self.assertEqual(self.export(case,raw),raw if case=='B1' and site=='event_loop' else None)
                self.assertIsNone(self.export(case,raw,experiment=False))
                if case == 'B1' and site == 'event_loop':
                    observed=orientation_sample(request,self.export(case,raw),lane='B',allow_reconciliation=True)
                    self.assertEqual([observed[name]['outcome'] for name in ('splash','event_loop')],['reused','reconciled'])
                else:
                    with self.assertRaises(ImpossibleOrientationTrace): orientation_sample(request,raw,lane=case[0],allow_reconciliation=True)
                with self.assertRaises(ImpossibleOrientationTrace): orientation_sample(request,raw,lane=case[0])
                with self.assertRaises(ValueError): orientation_sample(request,raw)
                with self.assertRaises(ValueError): metrics(request,raw)

    def test_distinct_fallbacks_and_duplicate_or_extra_terminal(self):
        request, original = balanced.records('B1',100)
        for outcome in ['reused','timeout','unavailable','success','failed','reconciled']:
            text=original.replace('event_loop_orientation_reused','event_loop_orientation_'+outcome)
            parsed=orientation_sample(request,self.export('B1',text),lane='B',allow_reconciliation=True)
            self.assertEqual(parsed['event_loop']['outcome'],outcome)
        for extra in ('timeout','reused','reconciled'):
            combined = marker(original,'event_loop','reused')+f'WEREAD_STARTUP 101.000000000 event_loop_orientation_{extra}\n'
            captured = self.export('B1',combined)
            with self.assertRaises(ValueError): orientation_sample(request,captured,lane='B',allow_reconciliation=True)

    def test_only_new_validated_plan_and_raw_marker_can_credit_the_branch(self):
        case=draw_fixtures.FirstDrawTests(); case.setUp(); self.addCleanup(case.doCleanups)
        visit=case.visit
        old_plan, old_receipt = visit.plan, dict(visit.receipt)
        legacy_log = visit.entries['markers']
        new_log = marker(legacy_log, 'event_loop', 'reused')
        new_plan = RECONCILED_CONTRACT | {'label':balanced.LABEL} | reconciled_build_evidence(128)
        for terminal in ('reused','timeout','reconciled'):
            visit.plan=new_plan; visit.receipt['purpose']=new_plan['purpose']
            visit.entries['markers']=self.export('B-library',legacy_log.replace('event_loop_orientation_reused','event_loop_orientation_'+terminal))
            result=visit.analyze()
            self.assertEqual(result['orientation_queries'],{'splash':'reused','event_loop':terminal})
            self.assertEqual(result['first_application_draw']['renderer_rotation_degrees'],180)
            if terminal == 'reconciled':
                self.assertEqual(result['reconciliation_evidence'],{'status':'event_loop_marker_observed',
                                 'one_change_observed_before_event_loop':True, 'first_draw_mapping_observation':'matches_new_mapping'})
            else:
                self.assertEqual(result['reconciliation_evidence']['status'],'not_observed')
        case.make(switch=1000); visit.entries['markers']=new_log
        evidence=visit.analyze()['reconciliation_evidence']
        self.assertEqual(evidence['status'],'event_loop_marker_observed'); self.assertFalse(evidence['one_change_observed_before_event_loop'])
        self.assertEqual(evidence['first_draw_mapping_observation'],'not_assessed')
        visit.entries['markers']=marker(legacy_log,'splash','reused')
        with self.assertRaises(ImpossibleOrientationTrace): visit.analyze()
        visit.entries['markers']=new_log
        visit.plan=new_plan | {'draw_build': new_plan['control_build']}
        with self.assertRaises(ValueError): visit.analyze()
        visit.plan=old_plan; visit.receipt=old_receipt
        with self.assertRaises(ImpossibleOrientationTrace): visit.analyze()

    def test_a_changed_pose_without_a_marker_does_not_infer_reconciliation(self):
        case=draw_fixtures.FirstDrawTests(); case.setUp(); self.addCleanup(case.doCleanups)
        visit=case.visit; visit.plan=RECONCILED_CONTRACT|{'label':balanced.LABEL}|reconciled_build_evidence(128)
        visit.receipt['purpose']=visit.plan['purpose']
        original=visit.entries['markers']
        visit.entries['markers']=''.join(line for line in original.splitlines(keepends=True) if not line.endswith('event_loop_orientation_reused\n'))
        report=visit.analyze()
        self.assertEqual(report['startup_turn']['coverage'],'hit'); self.assertIsNone(report['orientation_queries'])
        self.assertEqual(report['reconciliation_evidence'],{'status':'invalid_or_missing_site_trace'})

    def test_reconciliation_in_either_historical_arm_is_an_impossible_picture_warmup_or_timed_trace(self):
        original = balanced.records
        for target in ['A-library','A0','A2','B-library','B0','B2']:
            def modified(case, *args, **kwargs):
                request, text = original(case, *args, **kwargs)
                return request, marker(text,'event_loop','timeout' if case[0]=='A' else 'reused') if case==target else text
            with self.subTest(target=target), patch.object(balanced,'records',modified), self.assertRaises(ImpossibleOrientationTrace):
                balanced.OrientationTests().run_batch()

    def test_cannot_establish_a_fixed_pose_pilot_from_reconciliation_in_either_lane(self):
        gate = pictures.PictureGateTests(); gate.setUpClass(); gate.setUp(); self.addCleanup(gate.doCleanups)
        for case, index, prior in [('A-library',0,'timeout'),('B-library',1,'reused')]:
            _, old = balanced.records(case,30+8*index)
            gate.make_archive(overrides={f'out/{case}/markers':marker(old,'event_loop',prior).encode()})
            with self.subTest(case=case), self.assertRaises(GateError):
                pilot_reference(pictures.plan_for(True),pictures.closeout(),gate.archive)


if __name__ == '__main__': unittest.main()
