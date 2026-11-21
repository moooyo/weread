"""Replay captured-shape mount tables against the actual parser without mounting anything."""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
LABEL = '0123456789abcdef'
VOLUME = '/mnt/us'
STAGE = VOLUME + '/.weread-bind-diagnostic-' + LABEL
SOURCE_ROOT = STAGE[len(VOLUME):] + '/source'
BASE = '51 31 0:56 / /mnt/us rw - fuse /dev/fuse rw\n'
HARNESS = '''
#define _GNU_SOURCE
#define _FILE_OFFSET_BITS 64
#include <stdio.h>
#include <string.h>
static const char *table_path;
static FILE *fixture_open(const char *name,const char *mode) {
  return fopen(!strcmp(name,"/proc/self/mountinfo")?table_path:name,mode);
}
#define fopen fixture_open
#define main production_main
#include "launcher_file_bind.c"
#undef main
int main(int argc,char **argv) {
    struct operation op={.label="0123456789abcdef",.profile="scratch",.authority=-1};struct table table={0};
    if(argc!=2||!paths(&op)) return 2;
    table_path=argv[1];
    puts(mounts(&op,&table)?"pass":reason);
    return 0;
}
'''


def row(point='/other',parent='0',identity=22,root='/',dev='0:61'):
    return f'{identity} {parent} {dev} {root} {point} rw - tmpfs tmpfs rw\n'


class ParentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        stay=tempfile.TemporaryDirectory();cls.addClassCleanup(stay.cleanup)
        cls.root=Path(stay.name)
        harness=cls.root/'parser.c';harness.write_text(HARNESS)
        cls.binary=cls.root/'parse'
        compiler=shlex.split(os.environ.get('CC','cc'));cls.front=shlex.split(os.environ.get('WEREAD_BIND_EXEC',''))
        subprocess.run([*compiler,'-std=c11','-Wall','-Wextra','-Werror','-O2','-I',str(HERE),str(harness),'-o',str(cls.binary)],check=True)

    def replay(self, table, result):
        path=self.root/'mounts';path.write_text(table)
        run=subprocess.run([*self.front,str(self.binary),str(path)],check=True,capture_output=True,text=True,timeout=4)
        self.assertEqual(run.stdout,result+'\n');self.assertEqual(run.stderr,'')

    def test_canonical_zero_only_when_unrelated_or_an_unambiguous_top_root(self):
        for unrelated in [row(),row('/'),row('/mnt/other'),row('/mnt/user'),row('/mnt/us/weread-neighbor'),row('/var/tmp/other')]:
            with self.subTest(unrelated=unrelated): self.replay(unrelated+BASE,'pass')
        for bad in ['00','00000000','-1','+0','0x0','n/a','4294967296','99999999999']:
            with self.subTest(bad=bad): self.replay(row(parent=bad)+BASE,'parent_value')
        self.replay(row('/',root='/chroot')+BASE,'parent_scope')
        self.replay(row('/')+row('/',parent='1',identity=23)+BASE,'parent_scope')

    def test_relevant_zero_is_not_authorization(self):
        for point in ['/mnt', VOLUME+'/documents/item',VOLUME+'/weread',VOLUME+'/weread/app',STAGE,
                      STAGE+'/source',STAGE+'/target',STAGE+'/target/child',STAGE+'/other','/var','/var/tmp',
                      '/var/tmp/weread-bind-diagnostic-'+LABEL]:
            with self.subTest(point=point): self.replay(row(point)+BASE,'parent_scope')
        self.replay(BASE.replace('51 31','51 0'),'parent_scope')
        self.replay(row(identity=51)+BASE,'parent_scope')

    def test_alternate_mount_of_the_source_is_checked_even_before_the_base(self):
        for prior in ['',BASE]:
            for parent,why in [('0','parent_scope'),('8','foreign_mount')]:
                with self.subTest(prior=prior,parent=parent):
                    alias=row('/unrelated/alias',parent=parent,root=SOURCE_ROOT,dev='0:56')
                    self.replay(prior+alias+('' if prior else BASE),why)

    def test_other_fields_and_mount_guards_are_never_bypassed(self):
        for unrelated in [row(root='socket:[111]'),row(point='relative'),row(dev='not-a-device'),row(identity='0'),row(identity='-1')]:
            with self.subTest(unrelated=unrelated): self.replay(unrelated+BASE,'mount_table')
        self.replay(row()+BASE+BASE,'mount_table')
        self.replay(row()+BASE.replace(' rw -',' rw shared:95 -'),'propagation')
        self.replay(row()+BASE+row(STAGE+'/source',parent='51'),'foreign_mount')
