import io
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import assisted_shelf
from freeze import ROOT
import prepare_pose_flip as draw
from prepare_orientation import FACES, HELPERS
import run_pose_flip


def elf(payload):
    data = bytearray(84)
    data[:7] = b'\x7fELF\1\1\1'
    struct.pack_into('<HH', data, 16, 2, 40)
    struct.pack_into('<I', data, 28, 52)
    struct.pack_into('<I', data, 36, 0x5000400)
    struct.pack_into('<HH', data, 42, 32, 1)
    struct.pack_into('<I', data, 52, 1)
    return bytes(data) + payload


def draw_receipt(app_bytes):
    return dict(source='/work/src', lane='startup-first-draw', abi=draw.ABI, command=draw.COMMAND,
                release_profile={'lto':'thin', 'codegen-units':1, 'strip':True}, application_features=sorted(draw.CANDIDATE),
                backend_features=['startup-orientation-reuse'], local_dependencies=['i-slint-core','swash','zeno','zip'],
                generated_receipts=FACES, both_recompiled=True, qemu_cpu='cortex-a8', version_output='weread 0.2.0',
                runtime_policy='first-application-render-begin-v1',
                toolchain={'rustc':'1.99.1','cargo':'1.99.1','cargo_zigbuild':'0.24.0','zig':'0.15.2'},
                elf={'bytes':app_bytes,'elf_flags':'0x5000400','program_types':[1]})


class FirstDrawFreeze(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name).resolve()
        self.retained = self.work / 'public-inputs'
        for name in ['packaging/kindle/launch.sh', 'packaging/kindle/document-launcher.sh',
                     'docs/review/library-startup-latency-20261119/rebuild-arm.py']:
            path = self.retained / 'src' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        helpers = self.retained / 'orientation-inputs/helpers'
        helpers.mkdir(parents=True)
        for name in ['measure', *HELPERS]:
            (helpers / name).write_bytes(b'fixture')
        (helpers.parent / 'seed.tar.gz').write_bytes(b'public fixture')
        self.result = self.work / draw.BUILD_RESULT
        self.result.parent.mkdir()
        self.candidate = self.result.with_name('weread')
        self.candidate.write_bytes(elf(b'new build'))
        self.app_bytes = self.candidate.stat().st_size
        self.sensor = self.work / 'sensor'; self.sensor.write_bytes(elf(b'new sensor'))
        self.scene = self.work / 'scene'; self.scene.write_bytes(elf(b'scene fixture'))
        self.plan = self.work / 'one-visit'
        self.receipt = draw_receipt(self.app_bytes)
        self.result.write_text(json.dumps(self.receipt))
        self.enterContext(patch.object(assisted_shelf, 'public_seed'))
        self.probes = []
        self.enterContext(patch.object(draw, 'check_policy', side_effect=self.probe))

    def probe(self, worker, program, option, expected):
        self.assertEqual(worker, self.work)
        expected_program = b'new sensor' if option == '--draw-policy' else b'new build'
        if program.read_bytes() != elf(expected_program):
            raise ValueError(f'The packaged ARM executable does not implement {option}')
        self.probes.append((program.parent, option, expected))

    def freeze(self):
        return draw.freeze(self.retained, self.plan, self.sensor, self.scene, self.result)

    def preflight(self):
        return run_pose_flip.execute(SimpleNamespace(work=self.work, plan=self.plan, preflight=True, recover=None))

    def replace_member(self, member_name, old, new):
        path = self.plan / 'inputs.tgz'
        with tarfile.open(path) as original:
            entries = [(member, original.extractfile(member).read()) for member in original]
        with tarfile.open(path, 'w:gz') as output:
            for member, data in entries:
                if member.name == member_name:
                    self.assertEqual(data, old)
                    self.assertEqual(len(new), member.size)
                    data = new
                output.addfile(member, io.BytesIO(data))
        metadata = self.plan / 'plan.json'
        record = json.loads(metadata.read_text())
        record['archive_bytes'] = path.stat().st_size
        metadata.write_text(json.dumps(record))

    def test_only_fresh_elf_and_new_witness_are_packaged_and_rechecked_without_a_lease(self):
        plan = self.freeze()
        self.assertFalse((self.retained / 'arm-pair').exists())
        self.assertEqual(plan['draw_build'], self.receipt)
        self.assertEqual(plan['members']['B/app/weread'], self.app_bytes)
        script = (self.plan / 'device.sh').read_text()
        guard = '[ "$(stat -c %s "$stage/B/app/weread")" = {} ]'
        self.assertIn(guard.format(self.app_bytes), script)
        self.assertIn(guard.format(assisted_shelf.ARM_B), assisted_shelf.device_program(fixed_down=True))
        self.probes.clear()
        with patch.object(run_pose_flip, 'run', side_effect=AssertionError('preflight must never acquire')):
            self.assertEqual(self.preflight(), 0)
        self.assertEqual([(option, policy) for _, option, policy in self.probes],
                         [('--draw-policy', draw.SENSOR_POLICY), ('--startup-draw-trace-policy', draw.APP_POLICY),
                          ('--version', 'weread 0.2.0')])
        self.assertTrue(all(parent.parent == self.work and not parent.exists() for parent, _, _ in self.probes))

    def test_archive_same_size_candidate_substitution_cannot_reuse_the_build_receipt(self):
        self.freeze()
        self.replace_member('B/app/weread', elf(b'new build'), elf(b'old build'))
        with self.assertRaisesRegex(ValueError, 'differs from the fresh build'):
            self.preflight()

    def test_archived_pressure_only_helper_fails_even_with_matching_length(self):
        self.freeze()
        self.replace_member('pose-snapshot', elf(b'new sensor'), elf(b'old sensor'))
        with self.assertRaisesRegex(ValueError, 'draw-policy'):
            self.preflight()

    def test_an_old_elf_at_the_new_path_cannot_pass_a_fabricated_policy_claim(self):
        self.candidate.write_bytes(elf(b'old build'))
        with self.assertRaisesRegex(ValueError, 'startup-draw-trace-policy'):
            self.freeze()

    def test_candidate_result_path_symlinks_and_elf_metadata_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'independently built'):
            draw.built_candidate(self.work, self.candidate)
        self.candidate.unlink(); self.candidate.symlink_to(self.sensor)
        with self.assertRaisesRegex(ValueError, 'regular project-local'):
            draw.built_candidate(self.work, self.result)
        self.candidate.unlink(); self.candidate.write_bytes(elf(b'a different size'))
        with self.assertRaisesRegex(ValueError, 'ELF does not match'):
            draw.built_candidate(self.work, self.result)

    def test_build_features_toolchain_font_receipts_and_freshness_are_mandatory(self):
        faults = {'runtime_policy':'older', 'lane':'candidate', 'source':'/work/old', 'generated_receipts':[],
                  'both_recompiled':1, 'application_features':['kindle'], 'backend_features':[], 'release_profile':{},
                  'command':draw.COMMAND[1:], 'abi':'armv7', 'local_dependencies':[], 'qemu_cpu':'host',
                  'toolchain':self.receipt['toolchain'] | {'zig':'1.0.0'}, 'elf':self.receipt['elf'] | {'program_types':[1,3]}}
        for key, invalid in faults.items():
            with self.subTest(field=key), self.assertRaises(ValueError):
                draw.checked_receipt(self.receipt | {key:invalid})

    def test_old_plans_can_only_be_analyzed_and_recovery_remains_bound_to_the_draw_script(self):
        plan = self.freeze()
        old = draw.CONTRACT | {'label':plan['label']}
        self.assertEqual(draw.schedule(old), [draw.CASE])
        with self.assertRaises(ValueError): draw.draw_schedule(old)
        with self.assertRaises(ValueError): draw.schedule(plan)
        script = self.plan / 'device.sh'
        script.write_text(draw.device_program(self.app_bytes + 1))
        with self.assertRaisesRegex(ValueError, 'frozen startup-turn transaction'):
            draw.verify_recovery(self.plan)
        (self.plan / 'plan.json').write_text(json.dumps(old))
        with self.assertRaisesRegex(ValueError, 'measurement contract'):
            self.preflight()


class PolicyOutput(unittest.TestCase):
    def test_qemu_policy_requires_the_single_exact_line_success_and_no_stderr(self):
        program = Path('/private/sensor')
        expected = (draw.SENSOR_POLICY + '\n').encode()
        for code, out, err in [(0, expected + b'\n', b''), (2, expected, b''), (0, expected, b'warning'), (0, b'', b'')]:
            with self.subTest(code=code, stdout=out, stderr=err), \
                    patch.object(draw.shutil, 'which', return_value='/bin/qemu-arm'), \
                    patch.object(draw.subprocess, 'run', return_value=subprocess.CompletedProcess([], code, out, err)) as execute:
                with self.assertRaisesRegex(ValueError, 'draw-policy'):
                    draw.check_policy(program.parent, program, '--draw-policy', draw.SENSOR_POLICY)
                self.assertEqual(execute.call_args.args[0], ['/bin/qemu-arm', '-cpu', 'cortex-a8', '/private/sensor', '--draw-policy'])


if __name__ == '__main__':
    unittest.main()
