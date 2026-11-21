#include "native_tile_reference.h"
#include <fcntl.h>
#include <sys/file.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>

static int tile_private_process(void) {
    const struct rlimit zero = {0, 0};
    return setrlimit(RLIMIT_CORE, &zero) == 0 && prctl(PR_SET_DUMPABLE, 0) == 0;
}

#ifdef BILI_NATIVE_INPUT
static int tile_label_file(int record, const char *line) {
    char content[18];
    struct stat identity;
    return record >= 0 && !fstat(record, &identity) && S_ISREG(identity.st_mode) && identity.st_size == 17 &&
        read(record, content, sizeof(content)) == 17 && !memcmp(line, content, 17);
}

static int tile_commit_press_intent(void) {
    const char *label = getenv("WEREAD_TILE_LABEL");
    if (!label || strlen(label) != 16 || strspn(label, "0123456789abcdef") != 16) return 0;
    char stage_name[48], line[18];
    snprintf(stage_name, sizeof(stage_name), ".weread-render-%s", label);
    snprintf(line, sizeof(line), "%s\n", label);
    int parent = open("/mnt/us", O_RDONLY | O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW);
    if (parent < 0) return 0;
    int stage = openat(parent, stage_name, O_RDONLY | O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW);
    close(parent);
    if (stage < 0) return 0;
    int record = openat(stage, "owner", O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK);
    struct stat identity;
    int ok = tile_label_file(record, line);
    if (record >= 0) close(record);
    int monitor = openat(stage, "observer-active", O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK);
    if (ok) {
        ok = tile_label_file(monitor, line) && fstatat(stage, "render-cancel", &identity, AT_SYMLINK_NOFOLLOW) < 0 && errno == ENOENT;
        if (ok) {
            if (flock(monitor, LOCK_EX | LOCK_NB) == 0) { flock(monitor, LOCK_UN); ok = 0; }
            else ok = errno == EWOULDBLOCK;
        }
    }
    if (monitor >= 0) close(monitor);
    if (ok) {
        record = openat(stage, "tile-dispatch-intent", O_WRONLY | O_CLOEXEC | O_NOFOLLOW | O_CREAT | O_EXCL, 0600);
        ok = record >= 0 && write(record, line, 17) == 17 && !fsync(record);
        if (record >= 0 && close(record)) ok = 0;
    }
    close(stage);
    return ok;
}
#endif

static int public_tile_score(const struct frame *frame, struct bounds box) {
    int total = 0;
    for (int band = 0; band < 4; ++band) {
        int good[2] = {0, 0};
        for (int index = 0; index < 32; ++index) {
            const unsigned short *point = tile_points[band][index];
            int column = box.left + (point[0] * (box.right - box.left) + 128) / 256;
            int row = box.top + (point[1] * (box.bottom - box.top) + 192) / 384;
            int sum = 0;
            for (int delta = -1; delta <= 1; ++delta) {
                const unsigned char *line = frame->gray + (size_t)(row + delta) * frame->width + column;
                sum += line[-1] + line[0] + line[1];
            }
            good[index / 16] += index < 16 ? sum < 111 * 9 : sum > 160 * 9;
        }
        if (good[0] < 14 || good[1] < 14) return 0;
        total += good[0] + good[1];
    }
    return total;
}

static struct match public_tile(const struct frame *frame) {
    struct match result = {0};
    const int top = frame->height / 16, bottom = frame->height * 85 / 100;
    unsigned candidates = 0;
    int best = 0;
    for (int left = 2; left < frame->width - 72; left += 2) {
        int start = top;
        const unsigned char *first = frame->gray + (size_t)top * frame->width + left;
        int previous = first[-1] + first[0] + first[1];
        int current = previous;
        for (int row = top; row < bottom; ++row) {
            const unsigned char *line = frame->gray + (size_t)(row + 1) * frame->width + left;
            int following = line[-1] + line[0] + line[1];
            int ink = previous + current + following < 128 * 9;
            previous = current; current = following;
            if (ink) continue;
            int length = row - start;
            if (length >= 110 && length <= 930) {
                if (++candidates > 30000) return (struct match){0};
                for (int edge = 64; edge <= 74; edge += 2) {
                    int reference_height = edge == 64 ? 182 : edge == 66 ? 186 : edge == 68 ? 189 : 191;
                    int width = (length * 256 + reference_height / 2) / reference_height;
                    int reference_top = edge == 64 ? 133 : edge == 66 ? 131 : 129;
                    struct bounds box = {left - width * edge / 256, start - width * reference_top / 256, 0, 0};
                    box.right = box.left + width;
                    box.bottom = box.top + width * 3 / 2;
                    if (box.left < 0 || box.top < top || box.right >= frame->width || box.bottom >= bottom) continue;
                    int score = public_tile_score(frame, box);
                    if (!score) continue;
                    if (result.count && (abs(box.left - result.box.left) > width / 10 ||
                                         abs(box.top - result.box.top) > width / 10)) {
                        result.count = 2;
                        return result;
                    }
                    if (score > best) { best = score; result.count = 1; result.box = box; }
                }
            }
            start = row + 1;
        }
    }
    return result;
}
