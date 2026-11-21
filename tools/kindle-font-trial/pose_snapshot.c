#define _POSIX_C_SOURCE 200809L
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <linux/fb.h>
#include <linux/input.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

enum { MAX_SAMPLES = 128, EVENT_NODES = 32, NAME_BYTES = 128, WORD_BITS = sizeof(unsigned long) * CHAR_BIT };

static int trusted_name(const char *name) {
    if (!strcasecmp(name, "bma4xy_feature")) return 1;
    while (*name) {
        while (*name && !isalnum((unsigned char)*name)) ++name;
        const char *part = name;
        while (isalnum((unsigned char)*name)) ++name;
        if (name - part == 5 && !strncasecmp(part, "kx132", 5)) return 1;
    }
    return 0;
}

static int inspect(int descriptor) {
    struct stat status;
    unsigned long events = 0, axes[64 / WORD_BITS] = {0};
    char name[NAME_BYTES] = {0};
    if (fstat(descriptor, &status) < 0) return -1;
    if (!S_ISCHR(status.st_mode)) return 0;
    if (ioctl(descriptor, EVIOCGBIT(0, sizeof(events)), &events) < 0) return -1;
    if (!(events & (1UL << EV_ABS)) || events & ((1UL << EV_KEY) | (1UL << EV_REL))) return 0;
    if (ioctl(descriptor, EVIOCGBIT(EV_ABS, sizeof(axes)), axes) < 0) return -1;
    for (size_t index = 0; index < sizeof(axes) / sizeof(*axes); ++index) {
        unsigned long expected = index == ABS_PRESSURE / WORD_BITS ? 1UL << (ABS_PRESSURE % WORD_BITS) : 0;
        if (axes[index] != expected) return 0;
    }
    if (ioctl(descriptor, EVIOCGNAME(sizeof(name)), name) < 0 || !memchr(name, '\0', sizeof(name))) return -1;
    return trusted_name(name);
}

static int discover(void) {
    int found = -1;
    for (int index = 0; index < EVENT_NODES; ++index) {
        char path[32];
        snprintf(path, sizeof(path), "/dev/input/event%d", index);
        int descriptor = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
        if (descriptor < 0) {
            if (errno == ENOENT) continue;
            goto invalid;
        }
        int match = inspect(descriptor);
        if (match == 1 && found < 0) {
            found = descriptor;
            continue;
        }
        close(descriptor);
        if (match != 0) goto invalid;
    }
    return found;
invalid:
    if (found >= 0) close(found);
    return -1;
}

struct sample { uint64_t before, after; int raw; };

static int monotonic_ns(uint64_t *value) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) < 0 || now.tv_sec < 0) return 0;
    *value = (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
    return *value > 0;
}

static int sample(int descriptor, struct sample *result) {
    struct input_absinfo pressure;
    if (!monotonic_ns(&result->before) || ioctl(descriptor, EVIOCGABS(ABS_PRESSURE), &pressure) < 0 ||
        !monotonic_ns(&result->after) || result->before > result->after) return 0;
    result->raw = pressure.value;
    return result->raw >= 15 && result->raw <= 18;
}

static int still_unique(int descriptor) {
    struct stat original, current;
    int verified = discover();
    if (verified < 0) return 0;
    int same = fstat(descriptor, &original) == 0 && fstat(verified, &current) == 0 &&
        original.st_dev == current.st_dev && original.st_ino == current.st_ino && original.st_rdev == current.st_rdev;
    close(verified);
    return same;
}

static int display_sample(void) {
    int sensor = discover();
    if (sensor < 0) return 2;
    int frame = open("/dev/fb0", O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    struct stat status;
    struct input_absinfo pressure;
    struct fb_var_screeninfo screen;
    uint64_t before, after;
    int valid = frame >= 0 && fstat(frame, &status) == 0 && S_ISCHR(status.st_mode) &&
        monotonic_ns(&before) && ioctl(sensor, EVIOCGABS(ABS_PRESSURE), &pressure) == 0 &&
        ioctl(frame, FBIOGET_VSCREENINFO, &screen) == 0 && monotonic_ns(&after) && before <= after &&
        pressure.value >= 15 && pressure.value <= 18 && screen.rotate <= 3 && still_unique(sensor);
    if (frame >= 0) close(frame);
    close(sensor);
    if (!valid) return 2;
    if (printf("sample %" PRIu64 " %" PRIu64 " %d %u\n", before, after, pressure.value, screen.rotate) < 0) return 2;
    return fflush(stdout) == 0 ? 0 : 2;
}

static int startup_flip(void) {
    enum { count = 401 };
    struct sample points[count];
    const struct timespec interval = {.tv_nsec = 50000000};
    int descriptor = discover();
    if (descriptor < 0) return 2;
    int valid = 1;
    for (int index = 0; index < count && valid; ++index) {
        valid = (!index || !nanosleep(&interval, NULL)) && sample(descriptor, &points[index]) &&
            (!index || points[index].before > points[index - 1].after) &&
            points[index].after - points[0].before <= UINT64_C(27000000000);
        if (valid && !index)
            valid = printf("READY %" PRIu64 " %" PRIu64 " %d\n", points[0].before, points[0].after, points[0].raw) > 0 && !fflush(stdout);
    }
    valid = valid && still_unique(descriptor);
    close(descriptor);
    if (!valid) return 2;
    for (int index = 0; index < count; ++index)
        if (printf("%d %" PRIu64 " %" PRIu64 " %d\n", index + 1, points[index].before, points[index].after, points[index].raw) < 0) return 2;
    return fflush(stdout) == 0 ? 0 : 2;
}

int main(int argc, char **argv) {
    int count = 1;
    if (argc == 2) {
        if (!strcmp(argv[1], "--flip-policy")) return puts("startup-flip 401 50000000 27000000000 CLOCK_MONOTONIC") < 0 || fflush(stdout) ? 2 : 0;
        if (!strcmp(argv[1], "--startup-flip")) return startup_flip();
        if (!strcmp(argv[1], "--display-sample")) return display_sample();
        size_t length = strlen(argv[1]);
        if (length == 0 || length > 3 || argv[1][0] == '0' || strspn(argv[1], "0123456789") != length) return 64;
        count = atoi(argv[1]);
    }
    if (argc > 2 || count < 1 || count > MAX_SAMPLES) return 64;
    int descriptor = discover();
    if (descriptor < 0) return 2;
    struct sample samples[MAX_SAMPLES];
    const struct timespec pause = {.tv_nsec = 50000000};
    int valid = 1;
    for (int index = 0; index < count && valid; ++index) {
        valid = (!index || !nanosleep(&pause, NULL)) && sample(descriptor, &samples[index]) &&
            (!index || samples[index].before > samples[index - 1].after) &&
            samples[index].after - samples[0].before <= UINT64_C(8000000000);
    }
    valid = valid && still_unique(descriptor);
    close(descriptor);
    if (!valid) return 2;
    for (int index = 0; index < count; ++index) {
        if (printf("%d %" PRIu64 " %" PRIu64 " %d\n", index + 1, samples[index].before, samples[index].after,
                   samples[index].raw) < 0) return 2;
    }
    return fflush(stdout) == 0 ? 0 : 2;
}
