import unittest

from analyze_pose_flip import CHECKPOINT
from prepare_pose_flip import CONTRACT, DRAW_CONTRACT
import test_pose_flip_host as fixtures


class FirstDrawTests(unittest.TestCase):
    def setUp(self):
        self.visit = fixtures.FlipEvidenceTests()
        self.visit.setUp()
        self.addCleanup(self.visit.doCleanups)
        self.visit.plan = DRAW_CONTRACT | dict(label=fixtures.LABEL)
        self.visit.receipt.update(purpose=DRAW_CONTRACT['purpose'],operator_cue_acknowledged='2026-11-21T02:10:03Z')
        for op in self.visit.receipt['operations']:
            op['at'] = '2026-11-21T02:10:01Z' if op['mode']=='stage' else '2026-11-21T02:10:10Z'
        self.make()

    def make(self, switch=10, new_scan=1, draw='101.145000000', angle=180):
        entries = self.visit.make_stream(switch=switch)
        lines = entries['sensor'].splitlines()
        entries['sensor'] = ''.join(line + (' 1\n' if number < switch else f' {new_scan}\n') for number,line in enumerate(lines))
        entries['sensor-ready'] = entries['sensor-ready'].strip()+' 1\n'
        self.line = f'WEREAD_STARTUP {draw} first_application_frame_draw_begin_rot{angle}\n'
        entries['markers'] = entries['markers'].replace('WEREAD_STARTUP 101.200000000 first_application_frame_send',
                                                        self.line+'WEREAD_STARTUP 101.200000000 first_application_frame_send')
        self.visit.entries = entries

    def draw(self):
        return self.visit.analyze()['first_application_draw']

    def test_new_and_prior_renderer_transforms_are_distinguishable_when_scanout_is_unchanged(self):
        for angle, result in [(180,'matches_new_mapping'),(0,'matches_prior_not_new_mapping')]:
            self.make(angle=angle)
            draw=self.draw()
            self.assertEqual(draw['turn_in_splash_to_draw_window'],'after_splash_before_draw')
            self.assertEqual(draw['software_mapping']['status'],result)
            self.assertEqual(draw['software_mapping']['expected_new_rotation_degrees'],180)
            self.assertEqual(draw['orientation_correctness'],'not_verified')
            self.assertEqual(draw['pixels'],'not_observed_for_this_draw')

    def test_independent_reference_or_scanout_cannot_be_supplied_by_the_renderer_angle(self):
        for changed, angle, reason in [(3,0,'prior_and_new_mappings_have_same_angle'),(2,180,'no_new_portrait_scanout_mapping'),
                                       (1,90,'renderer_angle_outside_tested_portraits'),(1,270,'renderer_angle_outside_tested_portraits')]:
            self.make(new_scan=changed, angle=angle)
            draw=self.draw(); self.assertEqual(draw['renderer_rotation_degrees'],angle)
            self.assertEqual(draw['software_mapping']['reason'],reason)
        for anchor in ('missing','malformed','foreign_generation','FIXED:3','U:3','D:3'):
            self.make(); self.visit.entries['portrait-launch']=anchor+'\n'
            self.assertEqual(self.draw()['software_mapping']['status'],'not_qualified')
        self.make(); self.visit.receipt['initial_native_reference']='U:3'
        self.assertEqual(self.draw()['software_mapping']['status'],'not_qualified')

    def test_strictly_bounded_turn_and_adjacent_scanout_at_draw_are_required(self):
        for switch, relation in [(40,'after_draw'),(24,'overlaps_draw_boundary'),(1000,'no_single_observed_change')]:
            self.make(switch=switch); draw=self.draw()
            self.assertEqual(draw['turn_relative_to_draw'],relation); self.assertNotIn('software_mapping',draw)
        self.make(switch=5); draw=self.draw()
        self.assertEqual(draw['turn_relative_to_draw'],'before_draw'); self.assertEqual(draw['turn_in_splash_to_draw_window'],'before_splash')
        self.assertNotIn('software_mapping',draw)
        self.make(draw='101.134000023')
        self.assertEqual(self.draw()['software_mapping']['reason'],'draw_not_strictly_between_sample_intervals')
        self.make(); lines=self.visit.entries['sensor'].splitlines(); lines[24]=lines[24][:-1]+'3'
        self.visit.entries['sensor']='\n'.join(lines)+'\n'
        self.assertEqual(self.draw()['software_mapping']['reason'],'draw_not_bracketed_by_equal_new_holding_and_scanout')
        self.make(); lines=self.visit.entries['sensor'].splitlines()
        for index in range(24,len(lines)):
            number,start,end,holding,scan=map(int,lines[index].split()); lines[index]=f'{number} {start+70000000} {end+70000000} {holding} {scan}'
        self.visit.entries['sensor']='\n'.join(lines)+'\n'
        self.assertEqual(self.draw()['software_mapping']['reason'],'draw_sample_bracket_over_100ms')

    def test_delayed_output_from_same_clock_does_not_break_existing_phase_timeline(self):
        self.make(); text=self.visit.entries['markers']
        self.visit.entries['markers']=text.replace(self.line,'WEREAD_STARTUP 101.160000123 other_phase\n'+self.line)
        report=self.visit.analyze(); self.assertEqual(report['first_application_draw']['status'],'observed')
        self.assertEqual(report['startup_turn']['coverage'],'hit'); self.assertIsNotNone(report['orientation_queries'])
        self.visit.entries['markers']=text.replace(self.line, self.line.replace('rot180','rot135'))
        self.assertEqual(self.draw()['reason'],'invalid_draw_marker')
        self.assertEqual(self.visit.analyze()['startup_turn']['coverage'],'hit')

    def test_missing_wrong_order_unpaired_and_duplicate_draw_do_not_overturn_the_old_gate(self):
        self.make(); text=self.visit.entries['markers']
        for replace, reason in [('', 'draw_marker_not_recorded'), (self.line+self.line.replace('rot180','rot0'),'multiple_draw_markers'),
                                (self.line.replace('101.145','101.200'),'unpaired_or_unordered_draw_and_submit'),
                                (self.line+'WEREAD_STARTUP 101.150000000 unrelated_between\n','unpaired_or_unordered_draw_and_submit')]:
            self.visit.entries['markers']=text.replace(self.line,replace)
            report=self.visit.analyze(); self.assertEqual(report['first_application_draw']['reason'],reason)
            self.assertEqual(report['startup_turn']['coverage'],'hit')
        self.visit.plan=CONTRACT | dict(label=fixtures.LABEL); self.visit.receipt['purpose']=CONTRACT['purpose']
        self.visit.entries=self.visit.make_stream(); self.visit.entries['markers']=text
        self.assertEqual(self.draw()['status'],'not_in_historical_plan')

    def test_legacy_pressure_only_trace_wrong_binding_or_absent_ack_cannot_pass(self):
        self.visit.entries=self.visit.make_stream()
        with self.assertRaises(ValueError): self.draw()
        self.make(); self.visit.entries['sensor-binding']=self.visit.entries['sensor-binding'].replace(' 343 ',' 344 ')
        with self.assertRaises(ValueError): self.draw()
        self.make(); self.visit.receipt['operator_cue_acknowledged']='2026-11-21T02:10:01Z'
        with self.assertRaises(ValueError): self.draw()

    def test_partial_pose_checkpoint_does_not_claim_a_shelf(self):
        self.visit.receipt['passed']=False; self.visit.receipt['operations'][3].update(status=1,diagnostic='refused capture_metadata')
        self.visit.entries={key:value for key,value in self.visit.entries.items() if key in CHECKPOINT}
        self.visit.entries['observation']='partial capture_metadata\n'
        report=self.visit.analyze()
        self.assertEqual(report['first_application_draw']['software_mapping']['status'],'matches_new_mapping')
        self.assertEqual(report['digital_shelf']['status'],'not_assessed'); self.assertIsNone(report['final_route'])

    def test_fallback_is_not_changed_into_reuse_by_draw_agreement(self):
        self.visit.entries['markers']=self.visit.entries['markers'].replace('_orientation_reused','_orientation_timeout')
        report=self.visit.analyze()
        self.assertEqual(report['first_application_draw']['software_mapping']['status'],'matches_new_mapping')
        self.assertEqual(report['orientation_queries'],{'splash':'timeout','event_loop':'timeout'})

    def test_unmapped_scanout_or_missing_current_generation_keeps_only_the_measured_angle(self):
        for launch, raw in [('D:3',1),('D:1',2),('missing',1)]:
            self.make(); self.visit.entries['portrait-launch']=launch+'\n'
            if raw!=1:
                self.visit.entries['sensor']=self.visit.entries['sensor'].replace(' 1\n',f' {raw}\n',1)
                self.visit.entries['sensor-ready']=self.visit.entries['sensor-ready'].replace(' 1\n',f' {raw}\n')
            result=self.draw(); self.assertEqual(result['status'],'observed'); self.assertEqual(result['renderer_rotation_degrees'],180)
            self.assertEqual(result['software_mapping']['status'],'not_qualified')


if __name__ == '__main__': unittest.main()
