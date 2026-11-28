"""Exercise the bounded receive copy, samples and EOF independently of device access."""

import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


HERE = Path(__file__).resolve().parent
READER = None
sys.path.insert(0,str(HERE.parent))
from load_progress import HEADER_RECEIVED, checked_progress


LABEL = '0123456789abcdef'
LIMIT = 128 * 1024 + 19
PAYLOAD = (bytes(range(256)) * 513)[:LIMIT]


class ReceiveTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if READER:
            cls.reader = READER
        else:
            path = Path(cls.enterClassContext(tempfile.TemporaryDirectory()))/'receive-progress'
            subprocess.run(['cc','-std=c11','-O2','-Wall','-Wextra','-Werror',HERE/'receive-progress.c','-o',path],
                           check=True,timeout=15)
            cls.reader = [str(path)]

    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.records = self.root/'progress'
        self.records.touch(mode=0o600)
        self.output = self.root/'archive'
        state = self.records.stat()
        self.identity = f'{state.st_dev}:{state.st_ino}'

    def command(self, identity=None):
        return [*self.reader,str(LIMIT+1),LABEL,str(self.records),self.identity if identity is None else identity]

    def snapshot(self):
        rows = self.records.read_bytes().splitlines(keepends=True)
        if not rows or not rows[-1].endswith(b'\n'):
            return None
        self.assertLessEqual(len(rows),160)
        return checked_progress(HEADER_RECEIVED+b'\n'+rows[-1]+b'retired\n',LABEL,LIMIT)

    def run_reader(self, data, identity=None, output=None):
        with (output or self.output).open('wb') as target:
            done = subprocess.run(self.command(identity),input=data,stdout=target,capture_output=False,
                                  stderr=subprocess.PIPE,timeout=5)
        self.assertIn(done.returncode,(0,1)); self.assertEqual(done.stderr,b'')
        return done.returncode

    def receive_live(self):
        target = self.enterContext(self.output.open('xb'))
        child = self.enterContext(subprocess.Popen(self.command(),stdin=subprocess.PIPE,stdout=target,stderr=subprocess.PIPE))

        def stop():
            if child.poll() is None:
                child.kill(); child.wait(timeout=3)

        self.addCleanup(stop)
        return child

    def wait_snapshot(self, condition):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            row = self.snapshot()
            if row is not None and condition(row):
                return row
            time.sleep(.015)
        self.fail(f'Missing expected receiver sample: {self.snapshot()}')

    def test_exact_input_reports_eof_and_exact_counts(self):
        self.assertEqual(self.run_reader(PAYLOAD),0)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['state'],row['eof_observed']),(LIMIT,LIMIT,'eof',True))
        self.assertEqual(self.output.read_bytes(),PAYLOAD)

    def test_short_eof_is_not_inferred_from_the_declared_length(self):
        self.assertEqual(self.run_reader(PAYLOAD[:321]),0)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['state'],row['eof_observed']),(321,321,'eof',True))
        self.assertEqual(self.output.read_bytes(),PAYLOAD[:321])

    def test_empty_eof_has_no_last_data_read(self):
        self.assertEqual(self.run_reader(b''),0)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['eof_observed'],row['last_data_read_ns']),(0,0,True,None))
        self.assertEqual(self.output.read_bytes(),b'')

    def test_overrun_reads_only_the_first_extra_byte(self):
        surplus = b'\xffmore data'
        self.assertEqual(self.run_reader(PAYLOAD+surplus),0)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['state'],row['eof_observed']),
                         (LIMIT+1,LIMIT+1,'overrun',False))
        self.assertEqual(self.output.read_bytes(),PAYLOAD+b'\xff')

    def test_exact_bytes_are_written_before_eof_but_wait_remains_live(self):
        child = self.receive_live()
        child.stdin.write(PAYLOAD); child.stdin.flush()
        full = self.wait_snapshot(lambda row:row['bytes_written']==LIMIT)
        self.assertEqual((full['bytes_read'],full['state'],full['eof_observed']),(LIMIT,'receiving',False))
        self.assertEqual(self.output.read_bytes(),PAYLOAD); self.assertIsNone(child.poll())
        child.stdin.close()
        self.assertEqual(child.wait(timeout=3),0); self.assertEqual(child.stderr.read(),b'')
        eof = self.snapshot()
        self.assertTrue(eof['eof_observed']); self.assertEqual(eof['last_data_read_ns'],full['last_data_read_ns'])

    def test_partial_stall_keeps_last_data_time_until_watchdog_kill(self):
        child = self.receive_live()
        child.stdin.write(PAYLOAD[:7001]); child.stdin.flush()
        first = self.wait_snapshot(lambda row:row['bytes_written']==7001)
        child.stdin.write(PAYLOAD[7001:8000]); child.stdin.flush()
        latest = self.wait_snapshot(lambda row:row['bytes_written']==8000)
        idle = self.wait_snapshot(lambda row:row['sampled_ns']>latest['sampled_ns'])
        self.assertGreater(latest['last_data_read_ns'],first['last_data_read_ns'])
        self.assertEqual(idle['last_data_read_ns'],latest['last_data_read_ns'])
        child.kill(); self.assertEqual(child.wait(timeout=3),-signal.SIGKILL)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['state'],row['eof_observed']),(8000,8000,'receiving',False))

    def test_read_counts_are_separate_from_failed_archive_write(self):
        payload = PAYLOAD[:247]
        self.assertEqual(self.run_reader(payload,output=Path('/dev/full')),1)
        row = self.snapshot()
        self.assertEqual((row['bytes_read'],row['bytes_written'],row['state'],row['eof_observed']),
                         (len(payload),0,'write_error',False))

    def test_unavailable_or_replaced_progress_file_does_not_change_receive(self):
        foreign = self.root/'foreign'
        self.records.rename(foreign)
        for create, intact in ((lambda: self.records.symlink_to(foreign),None),
                               (lambda: self.records.touch(mode=0o600),b'')):
            with self.subTest(target=intact):
                create()
                self.assertEqual(self.run_reader(PAYLOAD),0)
                self.assertEqual(foreign.read_bytes(),b'')
                if intact is not None:
                    self.assertEqual(self.records.read_bytes(),intact)
                self.assertEqual(self.output.read_bytes(),PAYLOAD)
                self.records.unlink()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reader',nargs='+',help='Use a prebuilt receiver, optionally prefixed with qemu-arm')
    READER = parser.parse_args().reader
    if sys.platform != 'linux' or {row.split(':',1)[0].strip() for row in
                                  Path('/proc/net/dev').read_text('ascii').splitlines()[2:]} != {'lo'}:
        parser.error('Run in an isolated Linux network namespace')
    unittest.main(argv=[sys.argv[0],'-v'])
