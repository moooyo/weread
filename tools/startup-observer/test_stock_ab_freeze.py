import io
import json
from pathlib import Path
import sqlite3
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import stock_ab_freeze as freeze


class BatchFreezeTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(); self.addCleanup(directory.cleanup)
        self.work = Path(directory.name).resolve(); self.root = self.work/freeze.BATCH
        self.arms = {lane:{'tile/models/eng.traineddata':b'public language','arm/app/weread':lane.encode()} for lane in 'AB'}
        pair = {'source_commit':'1'*40, 'lanes':{'baseline':{},'candidate':{}}}
        self.materials = self.enterContext(patch.object(freeze,'materials',return_value=(pair,self.arms,{'library.sqlite':23})))
        def shell(config):
            return '#!/bin/sh\n: ' + config['label'] + ' ' + config['lane'] + '\n'
        self.enterContext(patch.dict(sys.modules,stock_ab_device=SimpleNamespace(render_device=shell)))

    def test_two_archives_fourteen_labels_and_six_fixed_balanced_pairs(self):
        result = freeze.freeze(self.work)
        self.assertEqual(result,freeze.verify_batch(self.work))
        self.assertEqual(''.join(slot['lane'] for slot in result['schedule']), 'ABABBAABBAABBA')
        self.assertEqual([slot['pair'] for slot in result['schedule']], [0,0,1,1,2,2,3,3,4,4,5,5,6,6])
        self.assertEqual(len({slot['label'] for slot in result['schedule']}),14)
        self.assertNotEqual((self.root/'A.tgz').stat().st_ino,(self.root/'B.tgz').stat().st_ino)
        for lane in 'AB': self.assertEqual((self.root/(lane+'.tgz')).stat().st_nlink,8)
        with self.assertRaisesRegex(ValueError,'overwrite'): freeze.freeze(self.work)

    def test_swapped_lane_or_foreign_slot_is_not_an_executable_visit(self):
        batch = freeze.freeze(self.work)
        first, second = [self.root/row['visit'] for row in batch['schedule'][:2]]
        (first/'inputs.tgz').unlink(); (first/'inputs.tgz').hardlink_to(self.root/'B.tgz')
        with self.assertRaisesRegex(ValueError,'Wrong shared arm'): freeze.verify_visit(self.work,first)
        (first/'plan.json').write_bytes((second/'plan.json').read_bytes())
        with self.assertRaisesRegex(ValueError,'visit plan'): freeze.verify_visit(self.work,first,recovery=True)

    def test_recovery_is_independent_of_the_arm_but_not_the_frozen_cleanup_program(self):
        batch = freeze.freeze(self.work); first = self.root/batch['schedule'][0]['visit']
        (first/'inputs.tgz').unlink(); (self.root/'A.tgz').unlink(); (first/'attempt').mkdir()
        with patch.object(freeze,'materials',side_effect=AssertionError('Recovery must not unpack or verify a payload')):
            self.assertEqual(freeze.verify_visit(self.work,first,recovery=True)['slot'],0)
            (first/'device.sh').write_bytes(b'#!/bin/sh\n:\n')
            with self.assertRaisesRegex(ValueError,'frozen device program'): freeze.verify_visit(self.work,first,recovery=True)

    def test_archive_comparison_refuses_same_length_change(self):
        batch = freeze.freeze(self.work); first = self.root/batch['schedule'][0]['visit']
        self.arms['A']['arm/app/weread'] = b'B'
        with self.assertRaisesRegex(ValueError,'Archive bytes differ'): freeze.verify_visit(self.work,first)

    def test_seed_normalizes_public_directory_and_preserves_long_paths_and_bytes(self):
        bindings={'targets':[{'book_id':f'book-{index}','cover_url':f'https://example.invalid/{index}',
                              'thumbnail':f'demo/covers/{index:064x}-'+'a'*64+'.png'} for index in range(15)]}
        source = self.work/'source.db'
        db = sqlite3.connect(source)
        db.executescript('CREATE TABLE books(id,cover_url,on_shelf); CREATE TABLE download_tasks(job);')
        db.executemany('INSERT INTO books VALUES(?,?,1)',[(row['book_id'],row['cover_url']) for row in bindings['targets']]); db.commit(); db.close()
        artifacts = {'demo/library.sqlite':source.read_bytes(),'library-covers.json':json.dumps(bindings).encode(),
                     'demo/images/cache-index.sqlite3':b'cache', 'demo/covers/cache-index.sqlite3':b'cache',
                     'demo/images/'+'b'*64+'-'+'c'*64+'.png':b'art'} | {row['thumbnail']:b'cover' for row in bindings['targets']}
        path = self.work/'inputs/public-seed.tgz'; path.parent.mkdir()
        def pack():
            with path.open('wb') as out: freeze.write_bundle(out,artifacts,inner=True)
        pack()
        with patch.object(freeze,'record',return_value=bindings):
            normalized = freeze.seed(self.work)
            self.assertEqual(len(normalized),21); self.assertEqual(normalized['library.sqlite'],artifacts['demo/library.sqlite'])
            self.assertNotIn('library-covers.json',normalized); self.assertTrue(all(not name.startswith('demo/') for name in normalized))
            content = io.BytesIO(); freeze.write_bundle(content,normalized,inner=True)
            with tarfile.open(fileobj=io.BytesIO(content.getvalue())) as archive:
                self.assertEqual(set(archive.getnames()),set(normalized))
                self.assertTrue(all(info.isfile() and info.mode == 0o644 for info in archive))
            artifacts['demo/account.json'] = b'not private test data'; pack()
            with self.assertRaisesRegex(ValueError,'unexpected seed member'): freeze.seed(self.work)


if __name__ == '__main__':
    unittest.main()
