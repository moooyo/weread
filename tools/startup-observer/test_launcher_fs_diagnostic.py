"""Check scratch-only filesystem diagnostics, including uncertain exchange outcomes."""

import errno
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
LABEL = '0123456789abcdef'
INTERPOSE = r'''
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>
static int crossed;
static int active(const char *mode) { return getenv("FIXTURE") && !strcmp(getenv("FIXTURE"),mode); }
extern long __real_syscall(long, ...);
long __wrap_syscall(long number, ...) {
    if(number!=SYS_renameat2){errno=ENOSYS;return -1;}
    va_list args; va_start(args,number);
    int left=va_arg(args,int); const char *left_name=va_arg(args,const char *);
    int right=va_arg(args,int); const char *right_name=va_arg(args,const char *);
    int flags=va_arg(args,int); va_end(args);
    if(active("syscall_missing")){errno=ENOSYS;return -1;}
    if(active("filesystem_rejects_exchange")){errno=EOPNOTSUPP;return -1;}
    long result=__real_syscall(number,left,left_name,right,right_name,flags);
    if(!result){crossed++;if(active("changed_mode") && crossed==1 && fchmodat(left,left_name,0400,0)) return -1;}
    return result;
}
extern int __real_fsync(int);
int __wrap_fsync(int fd){
    struct stat st;
    if(crossed && active("directory_sync") && !fstat(fd,&st) && S_ISDIR(st.st_mode)){
        crossed=0;errno=EIO;return -1;
    }
    return __real_fsync(fd);
}
extern FILE *__real_fopen64(const char *,const char *);
FILE *__wrap_fopen64(const char *name,const char *mode){
    if(active("no_mnt_id") && !strncmp(name,"/proc/self/fdinfo/",18)) return __real_fopen64("/dev/null",mode);
    return __real_fopen64(name,mode);
}
extern int __real_linkat(int,const char *,int,const char *,int);
int __wrap_linkat(int left,const char *old,int right,const char *next,int flags){
    if(active("no_hardlinks")){errno=EPERM;return -1;}
    return __real_linkat(left,old,right,next,flags);
}
'''


class DiagnosticTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.work = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.work.cleanup)
        cls.store = tempfile.TemporaryDirectory(dir=os.environ.get('WEREAD_EXCHANGE_TEST_FS'))
        cls.addClassCleanup(cls.store.cleanup)
        cls.root = Path(cls.store.name).resolve()
        cls.binary = Path(cls.work.name) / 'diagnose'
        compiler = shlex.split(os.environ.get('CC', 'cc'))
        cls.front = shlex.split(os.environ.get('WEREAD_EXCHANGE_TEST_EXEC', ''))
        flags = ['-std=c11', '-Wall', '-Wextra', '-Werror', '-O2', '-DDIAG_TEST_USERSTORE=' + json.dumps(str(cls.root))]
        compiled = [*compiler, *flags, HERE / 'launcher_fs_diagnostic.c']
        subprocess.run([*compiled, '-o', cls.binary], check=True)
        cls.wrapped = None
        if os.environ.get('WEREAD_EXCHANGE_FAULTS') == '1':
            interposer = Path(cls.work.name) / 'simulate.c'
            interposer.write_text(INTERPOSE)
            cls.wrapped = cls.binary.with_name('with-fixtures')
            subprocess.run([*compiled, interposer, '-Wl,--wrap=syscall,--wrap=fsync,--wrap=fopen64,--wrap=linkat',
                            '-o', cls.wrapped], check=True)

    def setUp(self):
        self.stage = self.root / ('.weread-exchange-diagnostic-' + LABEL)
        self.stage.mkdir()
        self.addCleanup(shutil.rmtree, self.stage)
        self.owner = self.stage / 'owner'
        self.owner.write_text(LABEL + '\n')

    def invoke(self, *args, fault=None):
        executable = self.wrapped if fault else self.binary
        return subprocess.run([*self.front, executable, *args], capture_output=True, text=True, timeout=5,
                              env=dict(os.environ, FIXTURE=fault or ''))

    def run_case(self, fault=None, fatal=False):
        result = self.invoke('diagnose', LABEL, fault=fault)
        self.assertEqual(result.returncode, int(fatal), result.stdout + result.stderr)
        self.assertEqual(result.stderr, '')
        observed = json.loads(result.stdout)
        self.assertEqual(set(observed), {'policy','filesystem','admission','admission_errno','exchange','exchange_errno',
                                         'hardlink','hardlink_errno','clean'})
        self.assertEqual(observed['policy'], 'selector-diagnostic-v1')
        self.assertEqual(observed['clean'], not fatal)
        return observed

    def test_positive_roundtrip_on_scratch_without_any_production_directory(self):
        observed = self.run_case()
        self.assertEqual(observed['admission'], 'verified')
        self.assertEqual(observed['exchange'], 'supported')
        if observed['filesystem'] == 'vfat':
            self.assertEqual(observed['hardlink'], 'link_create')
            self.assertIn(observed['hardlink_errno'], (errno.EPERM, errno.EOPNOTSUPP, errno.ENOSYS))
        else:
            self.assertEqual(observed['hardlink'], 'supported')
        self.assertEqual([item.name for item in self.root.iterdir()], [self.stage.name])
        self.assertEqual([item.name for item in self.stage.iterdir()], ['owner'])

    def test_unsupported_mode_and_foreign_scratch_are_untouched(self):
        self.assertEqual(self.invoke('--policy').stdout, 'selector-diagnostic-v1\n')
        for bad in [('swap', LABEL), ('probe', LABEL), ('diagnose', LABEL.upper()), ('diagnose', LABEL, '/tmp')]:
            self.assertEqual(self.invoke(*bad).returncode, 64)
        self.owner.write_text('not the requested owner\n')
        report = self.run_case(fatal=True)
        self.assertEqual((report['admission'], report['exchange'], report['hardlink']), ('scratch_owner', 'not_run', 'not_run'))
        self.owner.write_text(LABEL + '\n')
        unknown = self.stage / 'sub'
        unknown.mkdir()
        (unknown / 'untouched').write_text('retain')
        self.assertEqual(self.run_case(fatal=True)['admission'], 'scratch_mkdir')
        self.assertEqual((unknown / 'untouched').read_text(), 'retain')

    def test_syscall_rejection_post_exchange_and_independent_link_errors(self):
        if not self.wrapped:
            self.skipTest('GNU link-wrapper failure fixtures are opt-in')
        for fault, area, code, error in [
            ('syscall_missing','exchange','exchange_out',errno.ENOSYS),
            ('filesystem_rejects_exchange','exchange','exchange_out',errno.EOPNOTSUPP),
            ('directory_sync','exchange','out_fsync_left',errno.EIO),
            ('changed_mode','exchange','post_owner_mode',0),
            ('no_hardlinks','hardlink','link_create',errno.EPERM),
            ('no_mnt_id','admission','mnt_id_missing',0),
        ]:
            with self.subTest(fault=fault):
                observed = self.run_case(fault)
                self.assertEqual((observed[area],observed[area+'_errno']), (code,error))
                if area != 'hardlink':
                    self.assertEqual(observed['hardlink'], 'supported')
                if area != 'exchange':
                    self.assertEqual(observed['exchange'], 'supported')
                self.assertEqual([item.name for item in self.stage.iterdir()], ['owner'])


if __name__ == '__main__':
    unittest.main()
