"""Exercise the parser, real watcher and generated shells in a Linux-only fixture."""

import argparse
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parent))
import analyze
import prepare


PREFIX = analyze.PREFIX + ' '
WATCH = '--wait-portrait-completion'
PUBLISH = '--publish-portrait-completion'
FINAL = 'native-theme-done'
TRACE = 'portrait-notification.diag'
FAULT = r'''
#define _GNU_SOURCE
#include <dlfcn.h>
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <unistd.h>
static int injected;
int poll(struct pollfd *fds, nfds_t count, int wait_ms) {
    const char *fault = getenv("WEREAD_FIXTURE_FAULT");
    if (fault && count == 1) {
        fds[0].revents = !strcmp(fault,"poll") ? POLLERR : POLLIN;
        injected = 1;
        return 1;
    }
    int (*native)(struct pollfd *, nfds_t, int) = dlsym(RTLD_NEXT,"poll");
    return native(fds,count,wait_ms);
}
ssize_t read(int fd, void *target, size_t count) {
    if (injected) {
        const char *fault = getenv("WEREAD_FIXTURE_FAULT");
        if (!strcmp(fault,"read")) return 0;
        struct inotify_event event = { .wd = -1, .mask = IN_Q_OVERFLOW };
        if (count < sizeof(event)) return 0;
        memcpy(target,&event,sizeof(event));
        return sizeof(event);
    }
    ssize_t (*native)(int, void *, size_t) = dlsym(RTLD_NEXT,"read");
    return native(fd,target,count);
}
int renameat(int from_dir, const char *from, int to_dir, const char *to) {
    int (*native)(int, const char *, int, const char *) = dlsym(RTLD_NEXT,"renameat");
    int result = native(from_dir,from,to_dir,to);
    const char *fault = getenv("WEREAD_FIXTURE_FAULT");
    if (!result && fault && !strcmp(fault,"reap")) raise(SIGKILL);
    return result;
}
'''


def trace(*rows):
    return ''.join(PREFIX+row+'\n' for row in rows).encode('ascii')


def launch(command, **kwargs):
    return subprocess.run(command,cwd='/',capture_output=True,timeout=10,**kwargs)


class ReportTests(unittest.TestCase):
    def test_orders_publication_against_observation_without_kernel_claim(self):
        cases=(('publish_begin 140','publish_end 150 ok','wait 1 100 110 200 205 210 211 215 deadline',
                'in_window_publication_processed_after_deadline'),
               ('publish_begin 206','publish_end 214 ok','wait 1 100 110 200 0 0 0 203 deadline',
                'publication_at_or_after_deadline'),
               ('publish_begin 199','publish_end 202 ok','wait 1 100 110 200 0 0 0 203 deadline',
                'publication_crosses_deadline'),
               ('publish_begin 120','publish_end 150 ok','wait 1 100 110 200 0 0 0 203 deadline',
                'in_window_publication_no_timely_observation'),
               ('publish_begin 120','publish_end 150 ok','wait 1 100 110 200 0 0 0 115 poll_error','watch_failure'),
               ('publish_begin 120','publish_end 150 ok','wait 1 100 110 200 155 156 158 160 hint',
                'completion_seen_before_deadline'))
        for begin,end,wait,expected in cases:
            with self.subTest(expected=expected):
                report=analyze.analyze(trace(begin,wait,end),1)
                self.assertEqual(report['waits'][0]['classification'],expected)
                self.assertEqual(report['missing_wait_slots'],[])
                self.assertFalse(report['startup_timing_eligible'])

    def test_missing_and_failed_telemetry_never_manufacture_a_bracket(self):
        wait='wait 2 100 110 200 0 0 0 205 deadline'
        for rows in ((wait,),('publish_begin 140',wait),('publish_begin 140',wait,'publish_end 143 rename_error')):
            with self.subTest(rows=rows):
                report=analyze.analyze(trace(*rows),2)
                self.assertIsNone(report['publication']['bracket_ns'])
                self.assertEqual(report['missing_wait_slots'],[1])
                self.assertEqual(report['waits'][0]['classification'],'publication_order_unknown')

    def test_rejects_repeated_foreign_unbounded_and_inconsistent_rows(self):
        row='wait 1 100 110 200 0 0 0 205 deadline'
        items=(trace(row,row),trace('publish_end 100 ok'),trace('publish_begin 200','publish_end 100 ok'),
               trace('wait 29 100 110 200 0 0 0 206 deadline'),trace('wait 1 100 110 200 0 0 210 211 hint'),
               trace('wait 1 100 110 200 0 0 0 117 deadline'),trace('wait 1 100 110 200 112 0 0 115 setup_error'),
               trace(row,'wait 2 199 210 299 0 0 0 305 deadline'),trace(row).rstrip(),b'x'*8193,
               b'secret data\n',trace(row.replace('100','9223372036854775808',1)))
        for data in items:
            with self.subTest(data=data[:120]), self.assertRaises(ValueError):
                analyze.analyze(data)
        with self.assertRaises(ValueError):
            analyze.analyze(trace(row),0)


class WorkerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root=Path(cls.enterClassContext(tempfile.TemporaryDirectory(dir='/tmp',prefix='weread-notice-regression-')))
        cls.inputs=cls.root/'inputs'
        prepare.prepare(*FROZEN,cls.inputs)
        cls.worker=cls.root/'helper'; cls.fault=cls.root/'fault.so'
        test_source=cls.root/'fault.c'; test_source.write_text(FAULT,encoding='ascii')
        delay_source=cls.root/'usleep.c'
        delay_source.write_text('''#define _DEFAULT_SOURCE
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) { return argc != 2 || usleep((useconds_t)strtoul(argv[1],0,10)); }
''',encoding='ascii')
        subprocess.run(['cc','-std=c11','-O2','-Wall','-Wextra','-Werror',str(delay_source),'-o',str(cls.root/'usleep')],
                       check=True,timeout=15,cwd='/')
        subprocess.run(['cc','-std=c11','-O2','-Wall','-Wextra','-Werror','-DWEREAD_QUERY_TEST_COMPLETION_MS=350',
                        str(cls.inputs/'native_query.c'),'-ldl','-o',str(cls.worker)],check=True,timeout=15,cwd='/')
        subprocess.run(['cc','-shared','-fPIC','-O2','-Wall','-Wextra','-Werror',str(test_source),'-ldl','-o',str(cls.fault)],
                       check=True,timeout=15,cwd='/')

    def setUp(self):
        self.directory=Path(self.enterContext(tempfile.TemporaryDirectory(dir=self.root)))
        self.trace=self.directory/TRACE; self.trace.touch(mode=0o600)
        self.pending=self.directory/(FINAL+'.pending'); self.final=self.directory/FINAL
        self.pending.write_bytes(b'0\n')

    def run_helper(self, mode, *args, status=0, **kwargs):
        result=launch([str(self.worker),mode,str(self.directory),*args],**kwargs)
        self.assertEqual((result.returncode,result.stdout,result.stderr),(status,b'',b''))

    def parse(self, wait_count=None):
        return analyze.analyze(self.trace.read_bytes(),wait_count)

    def watch(self, slot):
        proc=subprocess.Popen([str(self.worker),WATCH,str(self.directory),str(slot)],stdin=subprocess.DEVNULL,
                              stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,cwd='/')
        def stop():
            if proc.poll() is None: proc.kill()
            proc.wait(timeout=2)
        self.addCleanup(stop)
        limit=time.monotonic()+0.09
        while time.monotonic()<limit and proc.poll() is None:
            try:
                process = Path(f'/proc/{proc.pid}')
                if 'poll' not in (process/'wchan').read_text('ascii'):
                    time.sleep(0.001)
                    continue
                for fd in (process/'fdinfo').iterdir():
                    if 'inotify wd:' in fd.read_text('ascii'):
                        return proc
            except FileNotFoundError:
                pass
            time.sleep(0.001)
        self.fail('Owned fixture watcher did not register in time')

    def test_generated_arms_keep_one_difference_and_original_session(self):
        a=(self.inputs/'A.sh').read_text('ascii'); b=(self.inputs/'B.sh').read_text('ascii')
        b_anchor=prepare.DISPLAY.replace('take_display\n','take_display startup\n')
        self.assertEqual(prepare.replace(a,prepare.DISPLAY,b_anchor),b)
        for path in ('A.sh','B.sh','collect.sh'):
            result=launch(['/bin/sh','-n',str(self.inputs/path)])
            self.assertEqual((result.returncode,result.stdout,result.stderr),(0,b'',b''))
        base='    signal(SIGPIPE, SIG_IGN);'
        self.assertEqual((self.inputs/'native_query.c').read_text('ascii').split(base,1)[1],
                         FROZEN[0].read_text('ascii').split(base,1)[1])
        self.assertIn('if read_single_record "$LOCK_DIR/native-theme-done"',b)
        self.assertIn('wait_portrait_completion_hint "$portrait_trace_slot"',b)
        self.assertIn('case "$native_theme_wait_status" in',b)
        self.assertIn('1) ;; # The watcher consumed its timeout.',b)

    def test_actual_rename_and_watcher_use_one_clock(self):
        proc=self.watch(1)
        self.run_helper(PUBLISH)
        self.assertEqual(proc.wait(timeout=2),0)
        self.assertEqual(self.final.read_bytes(),b'0\n')
        report=self.parse(1); wait,=report['waits']
        begin,end=report['publication']['bracket_ns']
        self.assertLessEqual(wait['armed_sample_ns'],begin)
        self.assertLessEqual(begin,wait['wake_sample_ns'])
        self.assertLessEqual(wait['wake_sample_ns'],wait['named_event_sample_ns'])
        self.assertLessEqual(wait['named_event_sample_ns'],wait['visible_sample_ns'])
        self.assertLess(end,wait['deadline_ns'])
        self.assertEqual(wait['classification'],'completion_seen_before_deadline')

    def test_real_timeout_then_authoritative_publication(self):
        self.run_helper(WATCH,'1',status=1)
        self.run_helper(PUBLISH)
        self.assertEqual(self.final.read_bytes(),b'0\n')
        wait,=self.parse(1)['waits']
        self.assertEqual(wait['outcome'],'deadline')
        self.assertEqual(wait['classification'],'publication_at_or_after_deadline')

    def test_armed_event_processed_after_deadline(self):
        started=time.monotonic()
        proc=self.watch(1)
        proc.send_signal(signal.SIGSTOP)
        limit=started+0.15
        while time.monotonic()<limit:
            if '\nState:\tT ' in Path(f'/proc/{proc.pid}/status').read_text('ascii'):
                break
            time.sleep(0.001)
        else:
            self.fail('Owned fixture watcher did not stop in time')
        self.run_helper(PUBLISH)
        time.sleep(max(0,started+0.445-time.monotonic()))
        proc.send_signal(signal.SIGCONT)
        self.assertEqual(proc.wait(timeout=2),1)
        wait,=self.parse(1)['waits']
        self.assertEqual(wait['classification'],'in_window_publication_processed_after_deadline')
        self.assertGreaterEqual(wait['visible_sample_ns'],wait['deadline_ns'])

    def test_injected_poll_read_and_overflow_errors_do_not_report_timeout(self):
        for slot,(fault,reason) in enumerate((('poll','poll_error'),('read','read_error'),('overflow','watch_lost')),1):
            with self.subTest(fault=fault):
                env=os.environ | {'LD_PRELOAD':str(self.fault),'WEREAD_FIXTURE_FAULT':fault}
                self.run_helper(WATCH,str(slot),status=70,env=env)
                self.assertEqual(self.parse(slot)['waits'][-1]['outcome'],reason)
        self.assertTrue(all(w['classification']=='watch_failure' for w in self.parse(3)['waits']))

    def test_interrupted_publisher_preserves_completion_and_only_the_known_boundary(self):
        env=os.environ | {'LD_PRELOAD':str(self.fault),'WEREAD_FIXTURE_FAULT':'reap'}
        self.run_helper(PUBLISH,status=-signal.SIGKILL,env=env)
        self.assertEqual(self.final.read_bytes(),b'0\n')
        self.run_helper(WATCH,'1')
        report=self.parse(1)
        self.assertGreater(report['publication']['before_ns'],0)
        self.assertIsNone(report['publication']['bracket_ns'])
        self.assertEqual(report['waits'][0]['outcome'],'hint')

    def test_launcher_counts_runtime_fault_and_still_validates_owned_status(self):
        self.trace.unlink()
        app=self.directory/'app'; app.mkdir(mode=0o700)
        stage=self.directory/'stage'; stage.mkdir(mode=0o700)
        shim=app/'weread-native-query'
        shim.write_text('''#!/bin/sh
if [ "$1" = --wait-portrait-completion ]; then
    export WEREAD_FIXTURE_FAULT=poll LD_PRELOAD='''+shlex.quote(str(self.fault))+'''
fi
exec '''+shlex.quote(str(self.worker))+''' "$@"
''',encoding='ascii')
        shim.chmod(0o700)
        probe=app/'weread'
        probe.write_text('#!/bin/sh\nusleep 300000\nprintf "%s\\n" fixture-owned-output\n',encoding='ascii')
        probe.chmod(0o700)
        text=(self.inputs/'B.sh').read_text('ascii')
        functions='\n'.join(text[text.index(begin):text.index(end)] for begin,end in (
            ('read_single_record() {','# Display and input ownership'),
            ('native_theme_group_is_owned() {','log_native_command_failure() {'),
            ('begin_owned_registration() {','launcher_signal_exit() {')))
        stub=r'''set -eu
umask 077
LOCK_DIR=$1 APP_DIR=$2 weread_handoff_stage=$3
PROFILE=scribe CLEANUP_RUNNING=0 LAUNCH_TOKEN=fixture-token
NATIVE_THEME_TIMEOUT_TICKS=15 NATIVE_THEME_COMMAND_SEQUENCE=0 NATIVE_COMMAND_FILE_BLOCKS=256
NATIVE_WINDOW_OUTPUT_LIMIT=65536 NATIVE_THEME_COMMAND_PID='' NATIVE_THEME_COMMAND_BIRTH=''
weread_portrait_shell_phase=called weread_notify_waits=0 weread_notify_wakes=0 weread_notify_failures=0
weread_portrait_shell_mark() { if [ "$1" = portrait_permission ]; then weread_portrait_shell_phase=permitted; fi; }
weread_handoff_clock() { return 1; }
weread_portrait_poll_begin() { :; }
weread_portrait_poll_end() { :; }
note_cleanup_failure() { return 1; }
'''
        run=r'''
trap 'stop_native_theme_command || :' 0
owned_native_command 192 --notify-portrait-completion "$APP_DIR/weread" --kindle --probe-native-portrait-calibration --device scribe
printf '%s %s %s %s\n' "$NATIVE_COMMAND_EXIT_STATUS" "$weread_notify_waits" "$weread_notify_wakes" "$weread_notify_failures"
'''
        result=launch(['/bin/sh','-c',stub+functions+run,'fixture',str(self.directory),str(app),str(stage)],
                      env=os.environ | {'PATH':str(self.root)+':'+os.environ['PATH']})
        self.assertEqual((result.returncode,result.stdout),(0,b'0 1 0 1\n'),result.stderr)
        self.assertEqual((self.directory/'native-theme-output').read_bytes(),b'fixture-owned-output\n')
        self.assertEqual(self.final.read_bytes(),b'0\n')
        report=self.parse(1)
        self.assertEqual(report['publication']['result'],'ok')
        self.assertEqual(report['waits'][0]['outcome'],'poll_error')

    def test_unavailable_trace_does_not_change_publication_or_hint(self):
        self.trace.unlink()
        foreign=self.directory/'foreign'; foreign.write_bytes(b'leave intact\n')
        self.trace.symlink_to(foreign)
        self.run_helper(PUBLISH)
        self.run_helper(WATCH,'1')
        self.assertEqual(self.final.read_bytes(),b'0\n')
        self.assertEqual(foreign.read_bytes(),b'leave intact\n')

    def test_invalid_slot_and_unsafe_input_do_not_write(self):
        self.run_helper(WATCH,'01',status=64)
        self.run_helper(WATCH,'29',status=64)
        self.pending.unlink(); self.pending.symlink_to(self.trace)
        self.run_helper(PUBLISH,status=69)
        self.assertFalse(self.final.exists()); self.assertEqual(self.trace.read_bytes(),b'')

    def test_frozen_producer_fragment_renames_status_only_once(self):
        content=(self.inputs/'B.sh').read_text('ascii')
        anchor='        printf "%s\\n" "$command_status" >"$completed.pending"'
        start=content.index(anchor)
        producer=content[start:content.index('        # Retain this private group anchor',start)]
        stub='set -eu\ncompleted=$1/native-theme-done\nportrait_trace_query=$2\nportrait_publish_file=\ncommand_status=0\n'
        result=launch(['/bin/sh','-c',stub+producer,'fixture',str(self.directory),str(self.worker)])
        self.assertEqual((result.returncode,result.stdout,result.stderr),(0,b'',b''))
        self.assertEqual(self.final.read_bytes(),b'0\n'); self.assertFalse(self.pending.exists())
        self.assertEqual(self.parse(0)['publication']['result'],'ok')

    def test_collection_requires_owned_bounded_trace_without_mixed_output(self):
        stub='''lock=$1; allow=$2
startup_trace_identity() { [ "$allow" = yes ]; }
stock_private_record() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(stat -c '%u:%a:%h' "$1")" = 0:600:1 ]; }
'''
        shell=stub+(self.inputs/'collect.sh').read_text('ascii')+'\ncollect_portrait_notification'
        value=trace('publish_begin 100','publish_end 110 ok')
        self.trace.write_bytes(value)
        result=launch(['/bin/sh','-c',shell,'fixture',str(self.directory),'yes'])
        self.assertEqual((result.returncode,result.stdout,result.stderr),(0,value,b''))
        for allow,data in (('no',value),('yes',value+b'private\n'),('yes',value.rstrip()),('yes',b'x'*8193)):
            with self.subTest(allow=allow,count=len(data)):
                self.trace.write_bytes(data)
                result=launch(['/bin/sh','-c',shell,'fixture',str(self.directory),allow])
                self.assertEqual((result.returncode,result.stdout),(1,b''))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('frozen_sources',type=Path,nargs=3,metavar='SOURCE')
    args=p.parse_args()
    if os.geteuid()!=0 or sys.platform!='linux' or {r.split(':',1)[0].strip() for r in Path('/proc/net/dev').read_text('ascii').splitlines()[2:]}!={'lo'}:
        p.error('Run as root in a disposable networkless Linux fixture')
    FROZEN=args.frozen_sources
    unittest.main(argv=[sys.argv[0],'-v'])
