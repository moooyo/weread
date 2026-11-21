"""Exercise the field observer with metadata-only evdev and framebuffer fixtures."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
SHIM = r'''
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <linux/input.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

enum { FRAME_FD = 400 };
static int discovery_round, sampled, rotated, clock_reads, opened_frame;
static int variant(const char *name) { return !strcmp(getenv("POSE_TEST"), name); }
static int paired(void) { return getenv("POSE_DISPLAY") || getenv("POSE_DRAW_FIXTURE"); }

int __wrap_open(const char *path, int flags, ...) {
    if (!strcmp(path, "/dev/fb0")) {
        if (flags != (O_RDONLY | O_CLOEXEC | O_NOFOLLOW) || !paired() || opened_frame++) _exit(75);
        if (variant("no_screen")) { errno = ENOENT; return -1; }
        return FRAME_FD;
    }
    int number = -1;
    char trailing;
    if (flags != (O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW) ||
        sscanf(path, "/dev/input/event%d%c", &number, &trailing) != 1 || number < 0 || number > 31) _exit(70);
    if (!number) ++discovery_round;
    if (number == 4 && variant("broken_node")) { errno = EACCES; return -1; }
    if (number == 2 || number == 1 || (number == 3 &&
        (variant("ambiguous") || (discovery_round == 2 && variant("late_ambiguity")))))
        return discovery_round * 100 + number;
    errno = ENOENT;
    return -1;
}

int __wrap_fstat(int fd, struct stat *info) {
    memset(info, 0, sizeof(*info));
    info->st_mode = variant("regular") || (fd == FRAME_FD && variant("screen_file")) ? S_IFREG : S_IFCHR;
    info->st_ino = fd % (variant("replaced") ? 1000 : 100);
    info->st_dev = 77;
    info->st_rdev = 88;
    return 0;
}
int __wrap_close(int fd) { if (fd < 100) _exit(71); return 0; }

int __wrap_lstat(const char *path, struct stat *info) {
    if (strcmp(path, "/dev/fb0") || !paired() || rotated != 401) _exit(80);
    if (variant("fb_gone")) { errno = ENOENT; return -1; }
    __wrap_fstat(FRAME_FD, info);
    if (variant("fb_replaced")) ++info->st_rdev;
    if (variant("fb_symlink")) info->st_mode = S_IFLNK;
    return 0;
}

int __wrap_clock_gettime(clockid_t clock, struct timespec *value) {
    if (clock != CLOCK_MONOTONIC) _exit(76);
    ++clock_reads;
    if (paired()) {
        if (sampled != clock_reads / 2 || rotated != clock_reads / 2) _exit(77);
        if (getenv("POSE_DISPLAY") && clock_reads > 2) _exit(79);
    }
    value->tv_sec = 1;
    value->tv_nsec = variant("reversed") ? 9 - clock_reads : clock_reads;
    return 0;
}

int __wrap_ioctl(int fd, unsigned long request, ...) {
    va_list arguments;
    va_start(arguments, request);
    void *data = va_arg(arguments, void *);
    va_end(arguments);
    if (fd == FRAME_FD) {
        if (request != FBIOGET_VSCREENINFO || clock_reads != 2 * rotated + 1 || sampled != rotated + 1) _exit(74);
        if (variant("bad_screen") || (variant("fb_error_mid") && rotated == 49)) { errno = EIO; return -1; }
        ++rotated;
        struct fb_var_screeninfo *screen = data;
        const char *angle = getenv("POSE_ROTATE"), *change = getenv("POSE_ROTATE_3_AT");
        screen->rotate = variant("bad_rotate") ? 4U : change && rotated >= atoi(change) ? 3U : angle ? (unsigned)atoi(angle) : 1U;
        return 0;
    }
    if (request == EVIOCGBIT(0, sizeof(unsigned long))) {
        *(unsigned long *)data = 1UL << EV_ABS;
        if (fd % 100 == 1 || variant("key")) *(unsigned long *)data |= 1UL << EV_KEY;
        if (variant("relative")) *(unsigned long *)data |= 1UL << EV_REL;
    } else if (request == EVIOCGBIT(EV_ABS, 8)) {
        unsigned long *bits = data;
        bits[0] = 1UL << ABS_PRESSURE;
        if (variant("touch")) bits[ABS_MT_POSITION_X / (8 * sizeof(*bits))] |=
            1UL << (ABS_MT_POSITION_X % (8 * sizeof(*bits)));
    } else if (request == EVIOCGNAME(128)) {
        strcpy(data, variant("bma") ? "BMA4xY_feature" : variant("wrong_name") ? "KX1329 touch" : "I2C-Kx132 node");
    } else if (request == EVIOCGABS(ABS_PRESSURE)) {
        if (fd != 102 || (paired() && clock_reads != 2 * sampled + 1)) _exit(72);
        if (variant("unreadable_pressure")) { errno = ENODEV; return -1; }
        if (sampled && variant("read_error")) { errno = ENODEV; return -1; }
        ((struct input_absinfo *)data)->value = (sampled && variant("unknown")) || variant("unmapped") ? 19 : 15 + sampled % 4;
        ++sampled;
    } else _exit(73);
    return 0;
}
'''

NO_WAIT = '''int __wrap_nanosleep(const struct timespec *interval, struct timespec *remaining) {
    (void)remaining;
    if (interval->tv_sec != 0 || interval->tv_nsec != 50000000) _exit(81);
    return 0;
}
'''


class SnapshotTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        root = Path(cls.temp.name)
        shim = root / "syscalls.c"
        shim.write_text(SHIM + NO_WAIT)
        cls.program = root / "snapshot"
        subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", str(HERE / "pose_snapshot.c"), str(shim),
                        *("-Wl,--wrap=" + name for name in ("open", "ioctl", "close", "fstat", "lstat", "clock_gettime", "nanosleep")),
                        "-o", cls.program],
                       check=True)

    def invoke(self, variant, arguments, status, rotation=0, change=None, ready_only=False):
        env = {"POSE_TEST": variant, "POSE_ROTATE": str(rotation)}
        if arguments == ["--display-sample"]:
            env["POSE_DISPLAY"] = "1"
        if arguments == ["--startup-draw"]:
            env["POSE_DRAW_FIXTURE"] = "1"
        if change is not None:
            env["POSE_ROTATE_3_AT"] = str(change)
        result = subprocess.run([self.program, *arguments], capture_output=True, text=True, timeout=4,
                                env={**os.environ, **env})
        self.assertEqual(result.returncode, status, result.stderr)
        self.assertEqual(result.stderr, "")
        if ready_only:
            self.assertEqual(result.stdout, f"READY 1000000001 1000000002 15 {rotation}\n")
        elif status:
            self.assertEqual(result.stdout, "")
        return result.stdout.splitlines()

    def test_samples_raw_values_from_the_same_descriptor_and_brackets_each_read(self):
        lines = self.invoke("kx132", ["4"], 0)
        self.assertEqual(len(lines), 4)
        previous = 0
        for index, line in enumerate(lines, 1):
            self.assertRegex(line, r"^[1-4] [0-9]+ [0-9]+ 1[5-8]$")
            sequence, before, after, raw = map(int, line.split())
            self.assertEqual((sequence, raw), (index, 14 + index))
            self.assertGreater(before, previous)
            self.assertGreaterEqual(after, before)
            previous = after

    def test_alternate_trusted_name_and_single_read_default(self):
        self.assertEqual(len(self.invoke("bma", [], 0)), 1)

    def test_rejects_nonexclusive_source_or_incomplete_capture_without_partial_output(self):
        for variant in ("ambiguous", "late_ambiguity", "replaced", "key", "relative", "touch", "wrong_name", "regular",
                        "broken_node", "unknown", "read_error"):
            with self.subTest(variant=variant):
                self.invoke(variant, ["2"], 2)

    def test_bounded_arguments(self):
        for arguments in (["0"], ["129"], ["01"], ["+2"], ["3", "100"], ["/dev/input/event2"], ["--display-sample", "3"]):
            with self.subTest(arguments=arguments):
                self.invoke("kx132", arguments, 64)

    def test_display_sample_brackets_pressure_and_getvar_once_and_reports_all_four_rotations(self):
        for rotation in range(4):
            self.assertEqual(self.invoke("kx132", ["--display-sample"], 0, rotation),
                             [f"sample 1000000001 1000000002 15 {rotation}"])

    def test_display_sample_refuses_missing_invalid_or_unbound_metadata(self):
        for variant in ("ambiguous", "late_ambiguity", "replaced", "wrong_name", "no_screen", "screen_file",
                        "bad_screen", "bad_rotate", "unreadable_pressure", "unmapped", "reversed"):
            with self.subTest(variant=variant):
                self.invoke(variant, ["--display-sample"], 2)

    def test_startup_draw_uses_paired_reads_on_the_held_descriptors(self):
        self.assertEqual(self.invoke("kx132", ["--draw-policy"], 0),
                         ["startup-draw 401 50000000 27000000000 CLOCK_MONOTONIC ABS_PRESSURE FBIOGET_VSCREENINFO"])
        ready, *rows = self.invoke("kx132", ["--startup-draw"], 0, rotation=1, change=201)
        self.assertEqual(len(rows), 401)
        self.assertEqual(ready, "READY " + rows[0].split(" ", 1)[1])
        for index, row in enumerate(rows):
            before = 1000000001 + index * 2
            self.assertEqual(row, f"{index + 1} {before} {before + 1} {15 + index % 4} {1 if index < 200 else 3}")

    def test_startup_draw_never_substitutes_for_missing_or_rebound_framebuffer(self):
        for variant in ("no_screen", "screen_file", "bad_screen", "bad_rotate", "ambiguous", "unmapped"):
            with self.subTest(variant=variant):
                self.invoke(variant, ["--startup-draw"], 2)
        for variant in ("fb_error_mid", "fb_replaced", "fb_gone", "fb_symlink", "replaced", "late_ambiguity", "read_error"):
            with self.subTest(variant=variant):
                self.invoke(variant, ["--startup-draw"], 2, ready_only=True)

    def test_original_startup_flip_remains_pressure_only(self):
        self.assertEqual(self.invoke("kx132", ["--flip-policy"], 0),
                         ["startup-flip 401 50000000 27000000000 CLOCK_MONOTONIC"])
        ready, *rows = self.invoke("kx132", ["--startup-flip"], 0)
        self.assertEqual(len(rows), 401)
        self.assertEqual(ready, "READY " + rows[0].split(" ", 1)[1])
        self.assertTrue(all(len(row.split()) == 4 for row in rows))


if __name__ == "__main__":
    unittest.main()
