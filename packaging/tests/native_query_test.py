#!/usr/bin/env python3
"""Exercise the firmware query bridge against a private host-only mock library."""

from pathlib import Path
import os
import struct
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "kindle/native_query.c"
MOCK = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static int handle;
static void record(const char *text) {
    FILE *file = fopen(getenv("WEREAD_TEST_TRACE"), "a");
    if (file) { fprintf(file, "%s\n", text); fclose(file); }
}
void *LipcOpenNoName(void) {
    record("open");
    return getenv("WEREAD_TEST_NO_CONNECTION") ? NULL : &handle;
}
void LipcClose(void *value) { (void)value; record("close"); }
int LipcGetStringProperty(void *value, const char *service, const char *name, char **output) {
    (void)value;
    char row[128]; snprintf(row, sizeof(row), "%s:%s", service, name); record(row);
    const char *mode = getenv("WEREAD_TEST_MODE");
    if (mode && !strcmp(mode, "null")) { *output = NULL; return 0; }
    if (mode && !strcmp(mode, "long")) {
        *output = malloc(130); memset(*output, 'x', 129); (*output)[129] = 0;
    } else if (mode && !strcmp(mode, "limit")) {
        *output = malloc(129); memset(*output, 'x', 128); (*output)[128] = 0;
    } else {
        *output = strdup(!strcmp(name, "state") ? "active" : "U");
    }
    return mode && !strcmp(mode, "reject") ? 1 : 0;
}
void LipcFreeString(char *value) { record("free"); free(value); }
'''


class NativeQueryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="weread-native-query-test-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.root = Path(cls.temporary.name)
        source = cls.root / "library.c"
        source.write_text(MOCK)
        cls.library = cls.root / "libquery-fixture.so"
        subprocess.run(["cc", "-std=c11", "-D_GNU_SOURCE", "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC", str(source), "-o", str(cls.library)], check=True)
        cls.worker = cls.root / "worker"
        cls.missing = cls.root / "missing-library-worker"
        for output, library in [(cls.worker, cls.library), (cls.missing, cls.root / "absent.so")]:
            subprocess.run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", f'-DWEREAD_QUERY_TEST_LIBRARY="{library}"', str(SOURCE), "-ldl", "-o", str(output)], check=True)

    def setUp(self):
        self.trace = self.root / (self.id().split(".")[-1] + ".log")
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("WEREAD_TEST_")}
        self.env["WEREAD_TEST_TRACE"] = str(self.trace)

    def run_worker(self, request, **environment):
        return subprocess.run([str(self.worker), "--session"], input=request,
                              capture_output=True, env=self.env | environment, timeout=7)

    def records(self):
        return self.trace.read_text().splitlines() if self.trace.exists() else []

    def test_one_connection_and_only_two_fixed_read_only_properties(self):
        result = self.run_worker(bytes([1, 2, 2, 1]))
        self.assertEqual(result.returncode, 0)
        frames = b"".join(bytes([operation, 0]) + struct.pack(">H", len(value)) + value
                          for operation, value in [(1,b"active"),(2,b"U"),(2,b"U"),(1,b"active")])
        self.assertEqual(result.stdout, b"WQ1\n" + frames)
        self.assertEqual(self.records().count("open"), 1)
        self.assertEqual(self.records().count("close"), 1)
        self.assertEqual(self.records().count("free"), 4)
        self.assertEqual([row for row in self.records() if row.startswith("com.")],
                         ["com.lab126.powerd:state", "com.lab126.winmgr:accelerometer",
                          "com.lab126.winmgr:accelerometer", "com.lab126.powerd:state"])

    def test_unknown_opcode_and_fifth_query_are_refused(self):
        for request in [b"\0", b"\3", b"\xff", b"\1"*5]:
            with self.subTest(request=request):
                result = self.run_worker(request)
                self.assertEqual(result.returncode, 64)
                self.assertTrue(result.stdout.startswith(b"WQ1\n"))

    def test_oversized_null_and_rejected_replies_have_no_payload(self):
        for mode, status in [("long",2),("null",2),("reject",1)]:
            with self.subTest(mode=mode):
                result = self.run_worker(b"\1", WEREAD_TEST_MODE=mode)
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"WQ1\n" + bytes([1,status,0,0]))
        self.assertEqual(self.records().count("free"), 2)

    def test_value_limit_is_exact(self):
        result = self.run_worker(b"\1", WEREAD_TEST_MODE="limit")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"WQ1\n\1\0\0\x80" + b"x"*128)

    def test_unavailable_library_or_initial_connection_has_no_handshake(self):
        missing = subprocess.run([str(self.missing), "--session"], input=b"", capture_output=True, env=self.env, timeout=7)
        denied = self.run_worker(b"", WEREAD_TEST_NO_CONNECTION="1")
        for result in [missing, denied]:
            self.assertEqual((result.returncode, result.stdout), (69, b""))

    def test_arguments_and_extra_data_are_not_commands(self):
        result = subprocess.run([str(self.worker), "com.lab126.powerd", "state"], capture_output=True, env=self.env, timeout=7)
        self.assertEqual((result.returncode, result.stdout), (64, b""))
        self.assertEqual(self.records(), [])
        result = self.run_worker(b"state; echo injected\n")
        self.assertEqual(result.returncode, 64)
        self.assertEqual([row for row in self.records() if row.startswith("com.")], [])


if __name__ == "__main__":
    unittest.main()
