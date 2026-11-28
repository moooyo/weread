/* Included only in the experimental native-query helper, not the daily build. */
#include <stdarg.h>

#define DONE "native-theme-done"
#define NOTICE_TRACE "portrait-notification.diag"
#define TRACE_LIMIT 8192
#define TRACE_PREFIX "WEREAD_PORTRAIT_DIAG "

static int private_context(const char *path) {
    int fd = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    struct stat s;
    if (fd >= 0 && (fstat(fd, &s) || s.st_uid != geteuid() || (s.st_mode & 077))) {
        close(fd);
        return -1;
    }
    return fd;
}

static int open_notice_trace(int dir) {
    struct stat s;
    int fd = openat(dir, NOTICE_TRACE, O_WRONLY | O_APPEND | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC);
    if (fd >= 0 && (fstat(fd, &s) || !S_ISREG(s.st_mode) || s.st_uid != geteuid() ||
                    s.st_nlink != 1 || (s.st_mode & 0777) != 0600 || s.st_size >= TRACE_LIMIT)) {
        close(fd);
        return -1;
    }
    return fd;
}

static void notice_record(int fd, const char *format, ...) {
    if (fd < 0) return;
    char row[256] = TRACE_PREFIX;
    const size_t prefix = sizeof(TRACE_PREFIX) - 1;
    va_list args;
    va_start(args, format);
    int size = vsnprintf(row + prefix, sizeof(row) - prefix, format, args);
    va_end(args);
    if (size > 0 && (size_t)size < sizeof(row) - prefix) {
        /* One bounded append to the private tmpfs trace. Missing or partial
           evidence must not change the command or watcher result. */
        ssize_t ignored = write(fd, row, prefix + (size_t)size);
        (void)ignored;
    }
}

static int publish_portrait_completion(const char *context) {
    int dir = private_context(context);
    if (dir < 0) return 69;
    const char pending[] = DONE ".pending";
    struct stat s;
    if (fstatat(dir, pending, &s, AT_SYMLINK_NOFOLLOW) || !S_ISREG(s.st_mode) ||
        s.st_uid != geteuid() || s.st_nlink != 1) {
        close(dir);
        return 69;
    }
    int trace = open_notice_trace(dir);
    int64_t before = trace >= 0 ? monotonic_ns() : 0;
    if (before > 0) notice_record(trace, "publish_begin %lld\n", (long long)before);
    int renamed = renameat(dir, pending, dir, DONE);
    int64_t after = before > 0 ? monotonic_ns() : 0;
    if (after > 0) notice_record(trace, "publish_end %lld %s\n", (long long)after, renamed ? "rename_error" : "ok");
    if (trace >= 0) close(trace);
    close(dir);
    /* The trace result never changes the original completed-file result. */
    return renamed ? 74 : 0;
}

static int completion_visible(int dir) {
    struct stat s;
    if (fstatat(dir, DONE, &s, AT_SYMLINK_NOFOLLOW)) return errno == ENOENT ? 0 : -1;
    return S_ISREG(s.st_mode) && s.st_nlink == 1 && s.st_uid == geteuid() ? 1 : -1;
}

/* The timestamp marks a userspace read of a named event, not kernel delivery. */
static int scan_notice_queue(const char *bytes, size_t count) {
    int target = 0;
    while (count >= sizeof(struct inotify_event)) {
        struct inotify_event event;
        memcpy(&event, bytes, sizeof(event));
        bytes += sizeof(event);
        count -= sizeof(event);
        if (event.len > count) return -1;
        if (event.mask & (IN_IGNORED | IN_Q_OVERFLOW)) return -2;
        if ((event.mask & IN_MOVED_TO) && event.len >= sizeof(DONE) && !memcmp(bytes, DONE, sizeof(DONE))) target = 1;
        bytes += event.len;
        count -= event.len;
    }
    return count ? -1 : target;
}

static int wait_portrait_completion(const char *context, unsigned int slot) {
    int64_t began = monotonic_ns();
    if (began <= 0 || arm_waiter_watchdog()) return 69;
    int dir = private_context(context);
    if (dir < 0) return 69;
    int trace = slot ? open_notice_trace(dir) : -1;
    int watch = -1;
    int result = 69;
    const char *reason = "setup_error";
    int64_t armed = 0, wake = 0, named = 0, visible = 0;
    int64_t deadline = began + (int64_t)WEREAD_QUERY_TEST_COMPLETION_MS * 1000000;
    char pinned[32];
    int length = snprintf(pinned, sizeof(pinned), "/proc/self/fd/%d", dir);
    if (length < 0 || (size_t)length >= sizeof(pinned)) goto done;
    watch = inotify_init();
    if (watch < 0 || fcntl(watch, F_SETFL, O_NONBLOCK) < 0 ||
        inotify_add_watch(watch, pinned, IN_MOVED_TO | IN_ONLYDIR) < 0) goto done;
    armed = monotonic_ns();
    if (armed <= 0) goto clock_error;

    /* Register before looking for the final name. A hint still requires
       the original owned completion check in the launcher. */
    struct pollfd event = { .fd = watch, .events = POLLIN };
    for (;;) {
        int found = completion_visible(dir);
        int64_t now = monotonic_ns();
        if (now <= 0) goto clock_error;
        if (found < 0) {
            reason = "lookup_error";
            result = 70;
            break;
        }
        if (found == 1) visible = now;
        if (now >= deadline) {
            reason = "deadline";
            result = 1;
            break;
        }
        if (visible) {
            reason = "hint";
            result = 0;
            break;
        }
        int remaining = (int)((deadline - now + 999999) / 1000000);
        int ready = poll(&event, 1, remaining);
        if (ready == 0) {
            reason = "deadline";
            result = 1;
            break;
        }
        if (ready < 0 && errno == EINTR) continue;
        if (ready < 0 || event.revents != POLLIN) {
            reason = "poll_error";
            result = 70;
            break;
        }
        if (!wake && (wake = monotonic_ns()) <= 0) goto clock_error;
        char queue[512];
        ssize_t bytes = read(watch, queue, sizeof(queue));
        if (bytes < 0 && (errno == EINTR || errno == EAGAIN)) continue;
        int found_event = bytes > 0 ? scan_notice_queue(queue, (size_t)bytes) : -1;
        if (found_event < 0) {
            reason = found_event == -2 ? "watch_lost" : "read_error";
            result = 70;
            break;
        }
        if (found_event && !named && (named = monotonic_ns()) <= 0) goto clock_error;
    }
    goto done;
clock_error:
    reason = "clock_error";
    result = 70;
done:
    if (trace >= 0) {
        int64_t end = monotonic_ns();
        notice_record(trace, "wait %u %lld %lld %lld %lld %lld %lld %lld %s\n", slot,
                      (long long)began, (long long)(armed > 0 ? armed : 0), (long long)deadline,
                      (long long)(wake > 0 ? wake : 0), (long long)(named > 0 ? named : 0),
                      (long long)visible, (long long)(end > 0 ? end : 0), reason);
        close(trace);
    }
    if (watch >= 0) close(watch);
    close(dir);
    return result;
}

static int notice_slot(const char *text, unsigned int *slot) {
    if (text[0] == '0' && !text[1]) { *slot = 0; return 0; }
    if (text[0] < '1' || text[0] > '9') return -1;
    unsigned int value = (unsigned int)(text[0] - '0');
    if (text[1]) {
        if (text[1] < '0' || text[1] > '9' || text[2]) return -1;
        value = value * 10 + (unsigned int)(text[1] - '0');
    }
    if (value > 28) return -1;
    *slot = value;
    return 0;
}
