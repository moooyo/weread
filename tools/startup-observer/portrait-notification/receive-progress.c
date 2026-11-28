/* Copy a bounded archive and sample actual reads, writes and EOF separately. */
#define _POSIX_C_SOURCE 200809L
#define _FILE_OFFSET_BITS 64
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define MAX_ARCHIVE (180u * 1024u * 1024u)
#define NS_PER_SECOND UINT64_C(1000000000)

struct progress {
    const char *label;
    int fd;
    unsigned int rows;
    uint64_t started, last_read, reported;
    unsigned int bytes_read, bytes_written;
};

static uint64_t monotonic_ns(void) {
    struct timespec time;
    if (clock_gettime(CLOCK_MONOTONIC, &time) || time.tv_sec < 0 ||
        (uint64_t)time.tv_sec >= INT64_MAX / NS_PER_SECOND ||
        time.tv_nsec < 0 || time.tv_nsec >= (long)NS_PER_SECOND) return 0;
    return (uint64_t)time.tv_sec * NS_PER_SECOND + (uint64_t)time.tv_nsec;
}

static int progress_file(const char *path, const char *identity) {
    if (!*identity) return -1;
    int fd = open(path, O_WRONLY | O_APPEND | O_NOFOLLOW | O_NONBLOCK);
    if (fd < 0) return -1;
    struct stat status;
    if (!fstat(fd, &status) && S_ISREG(status.st_mode) && status.st_nlink == 1 && status.st_size == 0) {
        char pinned[64];
        int length = snprintf(pinned, sizeof(pinned), "%ju:%ju", (uintmax_t)status.st_dev, (uintmax_t)status.st_ino);
        if (length > 0 && length < (int)sizeof(pinned) && !strcmp(identity, pinned)) return fd;
    }
    close(fd);
    return -1;
}

static void disable_progress(struct progress *p) {
    if (p->fd >= 0) close(p->fd);
    p->fd = -1;
}

static void sample(struct progress *p, const char *state, int force) {
    if (p->fd < 0 || p->rows >= (force ? 160u : 156u)) return;
    uint64_t now = monotonic_ns();
    if (!p->started || now < p->started || now < p->last_read || now < p->reported) {
        disable_progress(p);
        return;
    }
    if (!force && now - p->reported < NS_PER_SECOND) return;
    char row[160];
    int length = snprintf(row, sizeof(row), "WEREAD_STOCK_RECEIVE_PROGRESS %s %" PRIu64 " %" PRIu64
                          " %" PRIu64 " %u %u %s\n", p->label, p->started, now, p->last_read,
                          p->bytes_read, p->bytes_written, state);
    if (length <= 0 || length >= (int)sizeof(row) || write(p->fd, row, (size_t)length) != length) {
        disable_progress(p);
        return;
    }
    p->rows++;
    p->reported = now;
}

static int receive(unsigned int limit, struct progress *p) {
    unsigned char buffer[64 * 1024];
    struct pollfd input = { .fd = STDIN_FILENO, .events = POLLIN };
    p->started = monotonic_ns();
    sample(p, "receiving", 1);
    while (p->bytes_read < limit) {
        int ready = poll(&input, 1, 1000);
        if (!ready || (ready < 0 && errno == EINTR)) {
            sample(p, "receiving", 0);
            continue;
        }
        if (ready < 0) {
            sample(p, "read_error", 1);
            return 1;
        }
        size_t remaining = limit - p->bytes_read;
        ssize_t size = read(STDIN_FILENO, buffer, remaining < sizeof(buffer) ? remaining : sizeof(buffer));
        if (size < 0) {
            if (errno == EINTR) continue;
            sample(p, "read_error", 1);
            return 1;
        }
        if (!size) {
            sample(p, "eof", 1);
            return 0;
        }
        int exact_boundary = p->bytes_read < limit - 1 && p->bytes_read + (unsigned int)size >= limit - 1;
        p->bytes_read += (unsigned int)size;
        if (p->fd >= 0 && !(p->last_read = monotonic_ns())) disable_progress(p);
        sample(p, "receiving", exact_boundary);
        for (size_t offset = 0; offset < (size_t)size;) {
            ssize_t written = write(STDOUT_FILENO, buffer + offset, (size_t)size - offset);
            if (written < 0 && errno == EINTR) continue;
            if (written <= 0) {
                sample(p, "write_error", 1);
                return 1;
            }
            offset += (size_t)written;
            p->bytes_written += (unsigned int)written;
        }
        if (exact_boundary && p->bytes_read < limit) sample(p, "receiving", 1);
    }
    /* Stop after observing byte N+1. The installer will reject the size. */
    sample(p, "overrun", 1);
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 5 || strlen(argv[2]) != 16 || strspn(argv[2], "0123456789abcdef") != 16 ||
        !argv[1][0] || argv[1][0] == '0') return 64;
    unsigned int limit = 0;
    for (const char *c = argv[1]; *c; c++) {
        if (*c < '0' || *c > '9') return 64;
        limit = limit * 10u + (unsigned int)(*c - '0');
        if (limit > MAX_ARCHIVE + 1u) return 64;
    }
    if (limit < 2) return 64;
    struct progress p = { .label = argv[2], .fd = progress_file(argv[3], argv[4]) };
    int result = receive(limit, &p);
    disable_progress(&p);
    return result;
}
