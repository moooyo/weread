#define _GNU_SOURCE
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fb.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#ifndef POSE_STAGE_PREFIX
#define POSE_STAGE_PREFIX "/mnt/us/.weread-orientation-"
#endif
#ifndef POSE_LOCK
#define POSE_LOCK "/tmp/weread-launcher.lock"
#endif

enum { WIDTH = 1860, HEIGHT = 2480, STRIDE = 1872, CAP = 512, LOG_MAX = 32768 };
#define SECOND UINT64_C(1000000000)
static const struct { uint16_t x, y; uint8_t gray, group; } points[] = {
#include "pose_splash_points.h"
};
struct geometry { unsigned rotation, gray, page, xv, yv, bytes; };
struct scores { unsigned zero, turned; };
struct gate { uint64_t ready, finished; };
struct owner { unsigned pid; uint64_t birth; char nonce[34]; struct stat log; };

static int refuse(const char *reason) { fprintf(stderr, "splash_refused %s\n", reason); return 1; }
static uint64_t now(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) || value.tv_sec < 0) _exit(2);
    return (uint64_t)value.tv_sec * SECOND + (uint64_t)value.tv_nsec;
}
static int read_at(int directory, const char *path, char *buffer, size_t cap) {
    int fd = openat(directory, path, O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
    if (fd < 0) return -1;
    struct stat status;
    ssize_t got = !fstat(fd, &status) && S_ISREG(status.st_mode) ? read(fd, buffer, cap) : -1;
    close(fd);
    if (got < 0 || (size_t)got >= cap) return -1;
    buffer[got] = '\0';
    return (int)got;
}
static bool equal_at(int directory, const char *path, const char *expected) {
    char data[CAP];
    return read_at(directory, path, data, sizeof data) == (int)strlen(expected) && !strcmp(data, expected);
}
static bool same_file(struct stat left, struct stat right) {
    return left.st_dev == right.st_dev && left.st_ino == right.st_ino;
}
static bool number(const char *text, uint64_t *value) {
    char *end;
    if (!isdigit((unsigned char)*text)) return false;
    errno = 0;
    *value = strtoull(text, &end, 10);
    return !errno && *value && !strcmp(end, "\n");
}
static bool owned(int stage, int lock, const char *base, const char *visit, const struct owner *prior,
                  struct owner *current) {
    char data[CAP], path[CAP], expected[CAP], actual[CAP], exe[CAP], shelf[CAP], png[CAP];
    uint64_t pid, birth;
    if (read_at(lock, "application-pid", data, sizeof data) < 0 || !number(data, &pid) || pid > INT32_MAX ||
        read_at(lock, "application-birth", data, sizeof data) < 0 || !number(data, &birth) ||
        read_at(lock, "nonce", current->nonce, sizeof current->nonce) != 33 ||
        strspn(current->nonce, "0123456789abcdef") != 32 || current->nonce[32] != '\n') return false;
    current->pid = (unsigned)pid; current->birth = birth; current->nonce[32] = 0;
    snprintf(expected, sizeof expected, "1:%s\n", current->nonce);
    if (!equal_at(lock, "generation", expected)) return false;
    snprintf(expected, sizeof expected, "1:%s:%u:%" PRIu64 "\n", current->nonce, current->pid, birth);
    if (!equal_at(lock, "application-owned", expected)) return false;
    snprintf(path, sizeof path, "out/%s/entered/arm", visit);
    snprintf(expected, sizeof expected, "%s\n", visit);
    if (!equal_at(stage, path, expected)) return false;
    snprintf(path, sizeof path, "/proc/%u", current->pid);
    int proc = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    if (proc < 0) return false;
    struct stat installed, live, errlog;
    snprintf(exe, sizeof exe, "%s/B/app/weread", base);
    snprintf(shelf, sizeof shelf, "%s/B/data", base);
    snprintf(png, sizeof png, "%s/out/%s/final.png", base, visit);
    const char *args[] = {exe, "--kindle", "--device", "scribe", "--data-dir", shelf, "--demo", "--offline",
                          "--screen", visit + 2, "--exit-after-ms", !strcmp(visit,"B-reader") ? "35000" : "12000",
                          "--capture", png};
    size_t written = 0;
    for (size_t index = 0; index < sizeof args / sizeof *args; ++index) {
        size_t length = strlen(args[index]) + 1;
        if (length > sizeof expected - written) { close(proc); return false; }
        memcpy(expected + written, args[index], length); written += length;
    }
    bool valid = fstatat(stage, "B/app/weread", &installed, AT_SYMLINK_NOFOLLOW) == 0 && S_ISREG(installed.st_mode) &&
        !fstatat(proc, "exe", &live, 0) && same_file(installed, live) &&
        !fstatat(stage, "B/data/logs/application-bootstrap.log", &current->log, AT_SYMLINK_NOFOLLOW) &&
        S_ISREG(current->log.st_mode) && !fstatat(proc, "fd/2", &errlog, 0) && same_file(current->log, errlog) &&
        read_at(proc, "cmdline", actual, sizeof actual) == (int)written && !memcmp(actual, expected, written) &&
        read_at(proc, "stat", data, sizeof data) > 0;
    close(proc);
    if (!valid) return false;
    char *field = strrchr(data, ')');
    if (!field || field[1] != ' ') return false;
    field += 2;
    for (unsigned index = 3; index < 22; ++index) {
        field = strchr(field, ' '); if (!field) return false; ++field;
    }
    char *end = strchr(field, ' ');
    if (!end) return false;
    *end = '\n'; end[1] = 0;
    uint64_t observed_birth;
    return number(field, &observed_birth) && observed_birth == birth && (!prior ||
        (current->pid == prior->pid && birth == prior->birth && !strcmp(current->nonce, prior->nonce) && same_file(current->log, prior->log)));
}

static bool phases(int stage, const struct owner *who, struct gate *gate) {
    char buffer[LOG_MAX];
    int fd = openat(stage, "B/data/logs/application-bootstrap.log", O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return false;
    struct stat status;
    ssize_t got = !fstat(fd, &status) && same_file(status, who->log) ? read(fd, buffer, sizeof buffer) : -1;
    close(fd);
    if (got < 0 || (size_t)got == sizeof buffer || memchr(buffer, 0, (size_t)got)) return false;
    buffer[got] = 0; *gate = (struct gate){0};
    char *line = buffer, *end;
    while ((end = strchr(line, '\n'))) {
        *end = 0;
        if (!strncmp(line, "WEREAD_STARTUP ", 15)) {
            uint64_t sec, nsec; char name[64], extra;
            const char *point = strchr(line + 15, '.');
            if (!point || strspn(point + 1, "0123456789") != 9 ||
                sscanf(line + 15, "%" SCNu64 ".%9" SCNu64 " %63s%c", &sec, &nsec, name, &extra) != 3 ||
                !sec || sec >= UINT64_MAX / SECOND) return false;
            uint64_t *target = !strcmp(name, "startup_feedback_ready") ? &gate->ready :
                               !strcmp(name, "startup_feedback_finished") ? &gate->finished : NULL;
            if (target) { if (*target) return false; *target = sec * SECOND + nsec; }
        }
        line = end + 1;
    }
    return !gate->finished || (gate->ready && gate->ready < gate->finished);
}

static struct scores score(const volatile uint8_t *page, unsigned stride, bool inverse) {
    unsigned sum[2][4] = {{0}}, count[4] = {0}, error[2] = {0};
    for (size_t index = 0; index < sizeof points / sizeof *points; ++index) {
        unsigned col = points[index].x, row = points[index].y, group = points[index].group;
        int gray = inverse ? 255 - points[index].gray : points[index].gray;
        sum[0][group] += (unsigned)abs((int)page[row * stride + col] - gray);
        sum[1][group] += (unsigned)abs((int)page[(HEIGHT - 1 - row) * stride + WIDTH - 1 - col] - gray);
        ++count[group];
    }
    for (unsigned angle = 0; angle < 2; ++angle) for (unsigned group = 0; group < 4; ++group) {
        unsigned mean = (sum[angle][group] + count[group] - 1) / count[group];
        if (mean > error[angle]) error[angle] = mean;
    }
    return (struct scores){error[0], error[1]};
}

static bool geometry(int fd, struct geometry *mode) {
#ifdef POSE_SPLASH_FIXTURE
    struct stat file;
    if (fstat(fd, &file) || !S_ISREG(file.st_mode) || file.st_size != STRIDE * HEIGHT) return false;
    *mode = (struct geometry){3, getenv("POSE_SPLASH_TEST_INVERSE") ? 2u : 1u, 0, WIDTH, HEIGHT, STRIDE * HEIGHT};
#else
    struct fb_fix_screeninfo fixed; struct fb_var_screeninfo var;
    if (ioctl(fd, FBIOGET_FSCREENINFO, &fixed) || ioctl(fd, FBIOGET_VSCREENINFO, &var) ||
        var.xres != WIDTH || var.yres != HEIGHT || var.xoffset || fixed.line_length != STRIDE ||
        var.bits_per_pixel != 8 || (var.rotate != 1 && var.rotate != 3) || (var.grayscale != 1 && var.grayscale != 2) ||
        (var.xres_virtual != WIDTH && var.xres_virtual != STRIDE) || (var.yres_virtual != HEIGHT && var.yres_virtual != 2 * HEIGHT) ||
        (var.yoffset != 0 && var.yoffset != HEIGHT) || var.yoffset + HEIGHT > var.yres_virtual ||
        fixed.smem_len < STRIDE * (var.yoffset + HEIGHT)) return false;
    *mode = (struct geometry){var.rotate, var.grayscale, var.yoffset, var.xres_virtual, var.yres_virtual, fixed.smem_len};
#endif
    return true;
}

static bool sample(struct scores *result, unsigned *rotation) {
#ifdef POSE_SPLASH_FIXTURE
    const char *path = getenv("POSE_SPLASH_TEST_PAGE");
    if (!path) return false;
#else
    const char *path = "/dev/fb0";
#endif
    int fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
    if (fd < 0) return false;
    struct geometry before, after;
    bool valid = geometry(fd, &before);
    if (valid) {
        size_t length = (before.page + HEIGHT) * STRIDE;
        const uint8_t *mapping = mmap(NULL, length, PROT_READ, MAP_SHARED, fd, 0);
        valid = mapping != MAP_FAILED;
        if (valid) {
            *result = score(mapping + before.page * STRIDE, STRIDE, before.gray == 2);
            munmap((void *)mapping, length);
            valid = geometry(fd, &after) && !memcmp(&before, &after, sizeof before);
            *rotation = before.rotation;
        }
    }
    close(fd); return valid;
}

int main(int argc, char **argv) {
    size_t prefix = strlen(POSE_STAGE_PREFIX);
    if (argc != 3 || strncmp(argv[1], POSE_STAGE_PREFIX, prefix) || strlen(argv[1] + prefix) != 16 ||
        strspn(argv[1] + prefix, "0123456789abcdef") != 16 || strlen(argv[1]) > 120 ||
        (strcmp(argv[2], "B-reader") && strcmp(argv[2], "B-library"))) return 64;
    int stage = open(argv[1], O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    char label[18]; snprintf(label, sizeof label, "%s\n", argv[1] + prefix);
    if (stage < 0 || !equal_at(stage, "owner", label)) return refuse("stage");
#ifdef POSE_SPLASH_FIXTURE
    uint64_t deadline = now() + 2 * SECOND;
#else
    uint64_t deadline = now() + 45 * SECOND;
#endif
    uint64_t begin = 0, end = 0;
    struct owner bound = {0}, current = {0}; struct scores result = {0}; unsigned rotation = 0;
    int lock = -1;
    const struct timespec delay = {0, 10000000};
    do {
        if (lock < 0) lock = open(POSE_LOCK, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        if (lock >= 0 && owned(stage, lock, argv[1], argv[2], bound.pid ? &bound : NULL, &current)) {
            if (!bound.pid) bound = current;
            struct gate gate;
            if (!phases(stage, &bound, &gate)) return refuse("markers");
            if (gate.finished) {
                if (!(gate.ready < begin && begin <= end && end < gate.finished)) return refuse("window_missed");
                bool rotated = result.turned <= 12 && result.zero >= result.turned + 64;
                if (!rotated && (result.zero > 12 || result.turned < result.zero + 64)) return refuse("ambiguous");
                printf("SPLASH %" PRIu64 " %" PRIu64 " %u %u %u\n", begin, end, rotation, result.zero, result.turned);
                if (rotated) { fprintf(stderr, "splash_refused rotated\n"); return 20; }
                return 0;
            }
            if (!begin && gate.ready && gate.ready < now()) {
                begin = now();
                if (!sample(&result, &rotation)) return refuse("geometry");
                end = now();
                if (!owned(stage, lock, argv[1], argv[2], &bound, &current)) return refuse("owner_changed");
            }
        } else if (bound.pid) return refuse("owner_changed");
        nanosleep(&delay, NULL);
    } while (now() < deadline);
    return refuse(bound.pid ? "window_missed" : "owner_missing");
}
