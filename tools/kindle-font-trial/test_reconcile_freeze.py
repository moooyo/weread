import json
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import assisted_shelf
import prepare_pose_flip as draw
import run_pose_flip
from freeze import ROOT
from prepare_orientation import HELPERS
from test_draw_freeze import draw_receipt, elf


def reconciled_build_evidence(app_bytes):
    common = draw_receipt(app_bytes)
    control = common | dict(lane='baseline', cargo_target=f'target/{draw.ABI}/release/weread',
                            cargo_artifacts='logs/baseline-01234567/events.jsonl', source_provenance={'base':'1'*40,'paths':['Cargo.toml']},
                            cargo_graphs={'weread':common['application_features'], 'slint-backend-kindle':common['backend_features']})
    changed = control | dict(lane='reconcile', cargo_artifacts='logs/reconcile-76543210/events.jsonl',
                             application_features=sorted([*common['application_features'],'startup-orientation-reconcile']),
                             backend_features=['startup-orientation-reconcile','startup-orientation-reuse'],
                             command=[arg.replace(',startup-orientation-reuse',',startup-orientation-reconcile') for arg in common['command']])
    changed['cargo_graphs'] = {'weread':changed['application_features'], 'slint-backend-kindle':changed['backend_features']}
    return {'draw_build':changed, 'control_build':control, 'members':{'B/app/weread':app_bytes}}


def write_json(path, obj):
    path.write_text(json.dumps(obj))


def events(proof):
    def artifact(target, kind, manifest, executable, features):
        return dict(reason='compiler-artifact', target={'name':target,'kind':[kind]}, profile={'test':False}, fresh=False,
                    manifest_path=manifest, executable=executable, features=features)
    return [artifact('slint_backend_kindle', 'lib', '/work/src/vendor/slint-backend-kindle/Cargo.toml', None, proof['backend_features']),
            artifact('weread', 'bin', '/work/src/Cargo.toml', '/work/' + proof['cargo_target'], proof['application_features']),
            {'reason':'build-finished','success':True}]


class ReconciledBuildTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name).resolve()
        (self.work/'src').mkdir()
        shutil.copyfile(ROOT/'Cargo.toml', self.work/'src/Cargo.toml')
        self.changed = elf(b'new opt in')
        self.stable = elf(b'control v1')
        self.proof = reconciled_build_evidence(len(self.changed))
        self.plan = draw.RECONCILED_CONTRACT | self.proof | {'label':'0123456789abcdef'}
        self.result = self.work / draw.RECONCILED_RESULT
        self.target = self.work / draw.CARGO_TARGET
        self.target.parent.mkdir(parents=True)
        self.target.write_bytes(self.changed)
        for key, blob in [('control_build', self.stable), ('draw_build', self.changed)]:
            proof = self.proof[key]
            directory = self.work / proof['lane']
            directory.mkdir()
            (directory/'weread').write_bytes(blob)
            write_json(directory/'result.json', proof)
            path = self.work / proof['cargo_artifacts']
            path.parent.mkdir(parents=True)
            path.write_text(''.join(json.dumps(row) + '\n' for row in events(proof)))
        write_json(self.work/'final-source.json', self.proof['draw_build']['source_provenance'])

    def test_archive_schedule_requires_two_corresponding_artifacts_and_a_matching_member(self):
        self.assertEqual(draw.reconciled_schedule(self.plan), [draw.CASE])
        for change in [{'purpose':draw.DRAW_CONTRACT['purpose']}, {'draw_build':self.proof['control_build']}, {'members':{}},
                       {'control_build':self.proof['draw_build']}, {'members':{'B/app/weread':True}},
                       {'control_build':self.proof['control_build'] | {'source_provenance':{'base':'2'*40,'paths':['Cargo.toml']}}}]:
            with self.subTest(change=change), self.assertRaises(ValueError): draw.reconciled_schedule(self.plan | change)
        with self.assertRaises(ValueError): draw.draw_schedule(self.plan)

    def test_reject_changed_feature_closure_journal_manifest_target_and_freshness(self):
        proof = self.proof['draw_build']
        path = self.work / proof['cargo_artifacts']
        for index, edits in [(0, {'fresh':True}), (1, {'fresh':True}), (1, {'profile':{'test':True}}),
                             (0, {'manifest_path':'/wrong/Cargo.toml'}), (1, {'manifest_path':'/work/old/Cargo.toml'}),
                             (0, {'features':['startup-orientation-reuse']}), (1, {'features':self.proof['control_build']['application_features']}),
                             (1, {'executable':'/work/other/weread'}), (2, {'success':False})]:
            rows = events(proof); rows[index].update(edits)
            path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            with self.subTest(index=index, edits=edits), self.assertRaises(ValueError): draw.paired_builds(self.work, self.result)
        for malformed in [events(proof)[1:], events(proof)[:-1], [events(proof)[0], *events(proof)]]:
            path.write_text(''.join(json.dumps(row) + '\n' for row in malformed))
            with self.assertRaises(ValueError): draw.paired_builds(self.work, self.result)

    def test_same_size_substitutions_or_different_source_are_not_a_paired_build(self):
        draw.paired_builds(self.work, self.result)
        self.target.write_bytes(self.stable)
        with self.assertRaisesRegex(ValueError, 'match the Cargo output'):
            draw.paired_builds(self.work, self.result)
        self.target.write_bytes(self.changed)
        (self.work/'baseline/weread').write_bytes(self.changed)
        with self.assertRaisesRegex(ValueError, 'same-source baseline'):
            draw.paired_builds(self.work, self.result)
        (self.work/'baseline/weread').write_bytes(self.stable)
        write_json(self.work/'final-source.json', {'base':'3'*40,'paths':['Cargo.toml']})
        with self.assertRaisesRegex(ValueError, 'same admitted source'):
            draw.paired_builds(self.work, self.result)

    def test_one_freeze_preflight_and_recovery_do_not_change_the_old_device_program(self):
        retained = self.work/'retained'
        helpers = retained/'orientation-inputs/helpers'
        helpers.mkdir(parents=True)
        for name in ['packaging/kindle/launch.sh','packaging/kindle/document-launcher.sh','docs/review/library-startup-latency-20261119/rebuild-arm.py']:
            destination = retained/'src'/name
            destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(ROOT/name,destination)
        for name in ('measure',*HELPERS): (helpers/name).write_bytes(b'fixture')
        (helpers.parent/'seed.tar.gz').write_bytes(b'public fixture')
        sensor = self.work/'sensor'; scene = self.work/'scene'
        sensor.write_bytes(elf(b'paired sensor')); scene.write_bytes(elf(b'public scene'))
        plan = self.work/'fresh'
        with patch.object(assisted_shelf,'public_seed'), patch.object(draw,'check_policy'), \
                patch.object(run_pose_flip,'run',side_effect=AssertionError('preflight may not acquire')):
            with self.assertRaises(ValueError): draw.freeze(retained, plan, sensor, scene, self.result)
            result = draw.freeze(retained, plan, sensor, scene, self.result, reconciled=True)
            self.assertEqual(result['draw_build'], self.proof['draw_build'])
            self.assertEqual(result['control_build'], self.proof['control_build'])
            self.assertEqual(run_pose_flip.execute(SimpleNamespace(work=self.work,plan=plan,preflight=True,recover=None)), 0)
            original = draw.device_program(len(self.changed))
            self.assertNotIn('reconciled', original)
            self.assertNotEqual((plan/'device.sh').read_text(), original)
            self.target.unlink()
            self.assertEqual(draw.verify_recovery(plan), result)
            (plan/'device.sh').write_text(original)
            with self.assertRaisesRegex(ValueError, 'frozen startup-turn'):
                draw.verify_recovery(plan)


if __name__ == '__main__':
    unittest.main()
