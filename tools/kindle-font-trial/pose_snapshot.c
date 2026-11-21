#define _POSIX_C_SOURCE 200809L
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
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

int main(int argc, char **argv) {
    int count = 1;
    if (argc == 2) {
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
