import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).resolve().with_name('mount-table-diagnose.sh')
LABEL = '3a578c34b99e748b'
SECRET = 'private_sentinel_that_must_not_be_output'


class TableDiagnosticTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.volume = self.root / 'volume'
        self.volume.mkdir()
        self.table = self.root / 'table'
        self.info = self.root / 'fdinfo'
        self.info.write_text('pos:\t0\nmnt_id:\t51\n')
        self.body = SOURCE.read_text().replace('volume=/mnt/us\n', 'volume=' + shlex.quote(str(self.volume))+'\n')
        self.body = self.body.replace('table=/proc/self/mountinfo\n','table=' + shlex.quote(str(self.table))+'\n')
        self.body = self.body.replace('fdinfo=/proc/self/fdinfo/8\n','fdinfo=' + shlex.quote(str(self.info))+'\n')
        self.shell = shlex.split(os.environ.get('WEREAD_TABLE_SHELL', 'sh'))
        tool = os.environ.get('WEREAD_TABLE_BUSYBOX')
        if tool:
            self.body = self.body.replace('    awk -v ', '    ' + shlex.quote(tool) + ' awk -v ', 1)

    def entry(self, root='/', point=None, fields='rw - fuse dev rw', identity=51, parent=31):
        return f'{identity} {parent} 0:56 {root} {point or self.volume} {fields}\n'

    def run_body(self, data=None, body=None):
        if data is not None:
            self.table.write_text(data)
        candidate = subprocess.run(self.shell, input=body or self.body, text=True, capture_output=True, timeout=8)
        self.assertEqual(candidate.returncode, 0, candidate.stderr)
        self.assertEqual(candidate.stderr, '')
        self.assertNotIn(SECRET, candidate.stdout)
        self.assertNotIn(str(self.root), candidate.stdout)
        self.assertEqual(len(candidate.stdout.splitlines()),1)
        observed=json.loads(candidate.stdout)
        self.assertEqual(set(observed),set('policy table fd base_rows base_fd_rows fd_rows base_shape first_error error_scope repeat seek propagation'.split()))
        self.assertEqual((observed['policy'],observed['seek']),('mount-table-read-v1','not_observed'))
        return observed

    def test_supported_base_projection_does_not_result_in_a_bind(self):
        for root,props,shape,propagation in [('/', 'rw - fuse dev rw','root','private'),('/sub','rw shared:1 - fuse dev rw','subdir','shared_or_slave')]:
            actual=self.run_body(self.entry(root=root,fields=props))
            self.assertEqual((actual['table'], actual['fd'], actual['base_rows'], actual['base_fd_rows'],actual['fd_rows'],actual['first_error'],
                              actual['base_shape'],actual['repeat'],actual['propagation']),('ok','ok','one','one','one','none',shape,'same',propagation))

    def test_unrelated_nonabsolute_root_reports_its_class_but_no_raw_row(self):
        actual=self.run_body(self.entry(root='socket:[123]',point='/'+SECRET,identity=17)+self.entry())
        self.assertEqual((actual['first_error'],actual['error_scope'],actual['base_rows'],actual['base_fd_rows']),('root_nonabsolute','other','one','one'))
        actual=self.run_body(self.entry(root='socket:[321]'))
        self.assertEqual((actual['first_error'],actual['error_scope'],actual['base_shape']),('root_nonabsolute','volume','nonabsolute'))

    def test_generic_errors_match_first_failure_and_do_not_launder_later_rows(self):
        for row,reason,scope in [
            ('1 2\n','missing_field','indeterminate'),(self.entry(identity='-1'),'mount_id','volume'),(self.entry(parent=0),'parent_id','volume'),
            (self.entry().replace('0:56','broken'),'device_field','volume'), (self.entry(point=SECRET),'point_nonabsolute','other'),
            (self.entry(fields='rw'),'base_separator','volume'),(self.entry(root='/'+SECRET+'\\040x'),'base_escaped','volume'),
            (self.entry(root='/'+'x'*4080),'base_root_limit','volume'), (self.entry()+self.entry(identity=999),'base_duplicate','volume'),
            (self.entry(point='/'+SECRET),'base_absent','indeterminate'),
        ]:
            with self.subTest(reason=reason):
                report=self.run_body(row)
                self.assertEqual((report['first_error'],report['error_scope']),(reason,scope))

    def test_bounds_and_second_pass_target_fields(self):
        for data,code in [(self.entry(fields='rw - fuse '+('x'*32768)+' rw'),'line_limit'),
                          (self.entry()+self.entry(point='/other',identity=22)*8192,'row_limit')]:
            result=self.run_body(data);self.assertEqual((result['first_error'],result['repeat']),(code,'unavailable'))
        target=str(self.volume)+'/.weread-bind-diagnostic-'+LABEL+'/target'
        for fields,root,code in [('rw','/','target_separator'),('rw - fuse dev rw','/'+'x'*4096,'target_root_limit')]:
            result=self.run_body(self.entry()+self.entry(root, target, fields, 42)); self.assertEqual(result['first_error'],code)

    def test_fd_and_base_not_confused(self):
        self.info.write_text('pos: 0\n');result=self.run_body(self.entry());self.assertEqual((result['fd'],result['base_rows'],result['base_fd_rows']),('missing','one','zero'))
        self.info.write_text('mnt_id: 999\nmnt_id: 51\n');self.assertEqual(self.run_body(self.entry())['fd'],'format')
        self.info.write_text('mnt_id: 16\n');result=self.run_body(self.entry()+self.entry(point='/'+SECRET,identity=16))
        self.assertEqual((result['first_error'],result['base_fd_rows'],result['fd_rows']),('none','zero','one'))
        self.info.write_text('mnt_id: 51\n');result=self.run_body(self.entry()+self.entry(point='/'+SECRET,identity=51))
        self.assertEqual((result['first_error'],result['error_scope'],result['fd_rows']),('base_changed','other','multiple'))
        self.table.unlink();self.assertEqual(self.run_body()['first_error'],'table_open')
