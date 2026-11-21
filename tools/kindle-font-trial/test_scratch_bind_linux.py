import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

from scratch_bind import observation, program


LABEL = 'a0123b456c789def'
SOURCE = Path(__file__).resolve().parents[1]/'startup-observer'/'launcher_file_bind.c'


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, timeout=15)


@unittest.skipUnless(os.environ.get('WEREAD_BIND_FIXTURE') == 'isolated' and os.getpid() == 1 and os.geteuid() == 0,
                     'Requires a private mount and PID namespace on the Linux test worker')
class ScratchBindLinuxTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temp = tempfile.TemporaryDirectory(); cls.addClassCleanup(temp.cleanup)
        base = Path(temp.name).resolve(); cls.volume = base/'userstore'; cls.runtime = base/'runtime'
        cls.volume.mkdir(); cls.runtime.mkdir(); cls.runtime.chmod(0o2770)
        binary = base/'helper'
        run('cc','-std=c11','-O2','-Wall','-Wextra','-Werror',f'-DBIND_TEST_USERSTORE="{cls.volume}"',
            f'-DBIND_TEST_TMP="{cls.runtime}"',SOURCE,'-o',binary)
        cls.helper = binary.read_bytes(); cls.original = program(len(cls.helper))
        cls.original = cls.original.replace('volume=/mnt/us\n','volume='+shlex.quote(str(cls.volume))+'\n')
        cls.original = cls.original.replace('tmp=/var/tmp\n','tmp='+shlex.quote(str(cls.runtime))+'\n')
        cls.original = cls.original.replace('admission() {','native() { return 0; }\nadmission() {')

    def setUp(self):
        run('mount','-t','tmpfs','-o','size=2m','tmpfs',self.volume)
        self.addCleanup(run,'umount','-R',self.volume)
        self.body = self.original
        self.scratch = self.volume/('.weread-bind-diagnostic-'+LABEL)
        self.stage = self.runtime/('weread-bind-diagnostic-'+LABEL)
        self.source = self.scratch/'source'; self.target = self.scratch/'target'
        self.assertFalse(self.stage.exists() or self.scratch.exists())

    def phase(self, mode, ok=True):
        result = subprocess.run(['sh','-c',self.body,'isolated-wire',mode,LABEL,'none'],
                                capture_output=True,input=self.helper if mode=='probe' else b'',timeout=15)
        self.assertEqual(result.returncode==0,ok,result.stderr.decode()); return result.stdout

    def finish(self):
        self.body = self.original
        self.assertEqual(self.phase('close'),b'closed\n'); self.assertEqual(self.phase('absent'),b'absent\n')
        self.assertFalse(self.stage.exists() or self.scratch.exists()); self.phase('close')

    def test_independent_real_mount_view_and_verified_restoration(self):
        self.assertEqual(self.phase('admit'),b'admitted\n'); raw=self.phase('probe')
        self.assertTrue(self.target.samefile(self.source)); self.assertEqual(self.target.read_text(),LABEL+':S\n')
        raw+=self.phase('witness'); self.phase('absent',ok=False); self.finish()
        result=observation(raw); self.assertEqual(result['result'],'current_views_verified')
        self.assertIs(result['on_demand_handler_observed'],False)

    def test_disconnect_immediately_after_real_mount_only_restores(self):
        needle='helper_step bind bound || finish_refusal bind'; self.assertEqual(self.body.count(needle),1)
        self.body=self.body.replace(needle,needle+'\n        kill -KILL "$$"')
        self.phase('probe',ok=False); self.assertTrue(self.target.samefile(self.source)); self.assertTrue((self.stage/'bind-journal').is_file())
        self.assertFalse((self.stage/'bound-evidence').exists()); self.finish()

    def test_close_refuses_to_unmount_a_foreign_layer(self):
        raw=self.phase('probe')+self.phase('witness'); self.assertEqual(observation(raw)['result'],'current_views_verified')
        outsider=self.volume/'foreign'; outsider.write_text('keep this')
        run('mount','--bind',outsider,self.target); self.phase('close',ok=False); self.assertTrue(outsider.samefile(self.target))
        run('umount',self.target); self.assertTrue(self.source.samefile(self.target)); self.finish()
        self.assertEqual(outsider.read_text(),'keep this')

    def test_different_userspace_root_refuses_before_mount_and_still_closes(self):
        ready, announce=os.pipe(); wait, finish=os.pipe(); pid=os.fork()
        if pid==0:
            try:
                os.close(ready); os.close(finish); os.chroot(self.runtime); os.chdir('/'); os.write(announce,b'1'); os.read(wait,1)
                os._exit(0)
            except BaseException: os._exit(2)
        os.close(wait); os.close(announce)
        try:
            self.assertEqual(os.read(ready,1),b'1'); raw=self.phase('probe')+self.phase('witness')
            self.assertFalse((self.stage/'bind-journal').exists() or self.target.samefile(self.source))
            self.finish(); result=observation(raw)
            self.assertEqual((result['stage'],result['reason']),('prepare','bind_native_view'))
            self.assertIs(result['mount_call_not_reached'],True)
        finally:
            os.close(ready); os.close(finish); os.waitpid(pid,0)


if __name__ == '__main__':
    unittest.main()
