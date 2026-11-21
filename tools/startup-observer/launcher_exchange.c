#define _GNU_SOURCE
#define _FILE_OFFSET_BITS 64
#include <errno.h>
#include <fcntl.h>
#include <linux/fs.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef EXCHANGE_TEST_USERSTORE
#define USERSTORE "/mnt/us"
#else
#define USERSTORE EXCHANGE_TEST_USERSTORE
#endif

static const char production[] = "launch.sh";
static const char slot[] = "selector-original-slot";
static const char left_probe[] = "exchange-probe-left";
static const char right_probe[] = "exchange-probe-right";
static const char probe_folder[] = "exchange-probe-parent";

static int refuse(const char *reason) {
    fprintf(stderr, "refused exchange_%s\n", reason);
    return 1;
}

static int directory(int parent, const char *name) {
    return openat(parent, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
}

static int regular(int parent, const char *name) {
    return openat(parent, name, O_PATH | O_NOFOLLOW | O_CLOEXEC);
}

static int mount_identity(int descriptor, uint64_t *mount) {
    char path[64], line[128];
    snprintf(path, sizeof(path), "/proc/self/fdinfo/%d", descriptor);
    FILE *info = fopen(path, "r");
    if (!info) return 0;
    unsigned long long value = 0;
    int found = 0;
    while (fgets(line, sizeof(line), info)) {
        char extra;
        if (sscanf(line, "mnt_id: %llu %c", &value, &extra) == 1) {
            *mount = (uint64_t)value;
            found++;
        }
    }
    int valid = !ferror(info) && found == 1;
    fclose(info);
    return valid;
}

static int same_inode(const struct stat *first, const struct stat *second) {
    return first->st_dev == second->st_dev && first->st_ino == second->st_ino;
}

static int same_metadata(const struct stat *first, const struct stat *second) {
    return same_inode(first, second) && first->st_mode == second->st_mode && first->st_uid == second->st_uid &&
        first->st_gid == second->st_gid && first->st_nlink == second->st_nlink && first->st_size == second->st_size &&
        first->st_mtim.tv_sec == second->st_mtim.tv_sec && first->st_mtim.tv_nsec == second->st_mtim.tv_nsec;
}

static int pinned(int parent, const char *name, int descriptor, uint64_t mount, int is_directory) {
    struct stat open_file, named;
    uint64_t current = 0;
    return descriptor >= 0 && !fstat(descriptor, &open_file) && !fstatat(parent, name, &named, AT_SYMLINK_NOFOLLOW) &&
        same_inode(&open_file, &named) && mount_identity(descriptor, &current) && current == mount &&
        (is_directory ? S_ISDIR(named.st_mode) : S_ISREG(named.st_mode) && named.st_nlink == 1 &&
        named.st_size > 0 && named.st_size <= 1024 * 1024);
}

static int named_file(int parent, const char *name, const struct stat *expected) {
    struct stat named;
    return !fstatat(parent, name, &named, AT_SYMLINK_NOFOLLOW) && same_metadata(expected, &named);
}

struct transaction {
    int volume, home, stage, owner;
    uint64_t mount;
    char stage_name[48];
    const char *label;
};

static int open_owner(const struct transaction *run) {
    int entry = regular(run->stage, "owner");
    if (!pinned(run->stage, "owner", entry, run->mount, 0)) {
        if (entry >= 0) close(entry);
        return -1;
    }
    char path[64];
    snprintf(path, sizeof(path), "/proc/self/fd/%d", entry);
    int reader = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    close(entry);
    return reader;
}

static int authorized(const struct transaction *run) {
    char text[18];
    struct stat path, opened;
    return !lstat(USERSTORE, &path) && !fstat(run->volume, &opened) && S_ISDIR(path.st_mode) && same_inode(&path, &opened) &&
        pinned(run->volume, "weread", run->home, run->mount, 1) &&
        pinned(run->volume, run->stage_name, run->stage, run->mount, 1) &&
        pinned(run->stage, "owner", run->owner, run->mount, 0) &&
        pread(run->owner, text, sizeof(text), 0) == 17 && !memcmp(text, run->label, 16) && text[16] == '\n';
}

static int exchange(const struct transaction *run, int left_dir, const char *left_name, int left_fd,
                    int right_dir, const char *right_name, int right_fd) {
    struct stat left, right;
    if (!authorized(run) || !pinned(left_dir, left_name, left_fd, run->mount, 0) ||
        !pinned(right_dir, right_name, right_fd, run->mount, 0) || fstat(left_fd, &left) || fstat(right_fd, &right) ||
        same_inode(&left, &right) || left.st_dev != right.st_dev) return 0;
    if (syscall(SYS_renameat2, left_dir, left_name, right_dir, right_name, RENAME_EXCHANGE) != 0) return 0;
    return !fsync(left_dir) && !fsync(right_dir) && authorized(run) &&
        named_file(left_dir, left_name, &right) && named_file(right_dir, right_name, &left) &&
        pinned(left_dir, left_name, right_fd, run->mount, 0) && pinned(right_dir, right_name, left_fd, run->mount, 0);
}

static int create_probe(int directory_fd, const char *name, const char *text, int *descriptor) {
    *descriptor = openat(directory_fd, name, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    size_t size = strlen(text);
    return *descriptor >= 0 && write(*descriptor, text, size) == (ssize_t)size && !fsync(*descriptor);
}

static int remove_probe(int directory_fd, const char *name, int first, int second) {
    struct stat named, opened;
    if (fstatat(directory_fd, name, &named, AT_SYMLINK_NOFOLLOW)) return errno == ENOENT;
    return S_ISREG(named.st_mode) && named.st_nlink == 1 &&
        ((first >= 0 && !fstat(first, &opened) && same_inode(&named, &opened)) ||
         (second >= 0 && !fstat(second, &opened) && same_inode(&named, &opened))) &&
        !unlinkat(directory_fd, name, 0);
}

static int probe(const struct transaction *run, int live) {
    struct stat original;
    if (fstat(live, &original) || mkdirat(run->stage, probe_folder, 0700)) return 0;
    int other = directory(run->stage, probe_folder);
    if (!pinned(run->stage, probe_folder, other, run->mount, 1)) return 0;
    int first = -1, second = -1;
    int passed = create_probe(run->stage, left_probe, "left", &first) && create_probe(other, right_probe, "right", &second) &&
        exchange(run, run->stage, left_probe, first, other, right_probe, second) &&
        exchange(run, run->stage, left_probe, second, other, right_probe, first);
    int left_clear = remove_probe(run->stage, left_probe, first, second);
    int right_clear = remove_probe(other, right_probe, first, second);
    int removed = right_clear && pinned(run->stage, probe_folder, other, run->mount, 1) &&
        !unlinkat(run->stage, probe_folder, AT_REMOVEDIR);
    if (first >= 0) close(first);
    if (second >= 0) close(second);
    close(other);
    return passed && left_clear && removed && !fsync(run->stage) &&
        named_file(run->home, production, &original) && pinned(run->home, production, live, run->mount, 0);
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--policy")) {
        puts("selector-exchange-v1");
        return 0;
    }
    if (argc != 3 || (strcmp(argv[1], "probe") && strcmp(argv[1], "swap")) || strlen(argv[2]) != 16 ||
        strspn(argv[2], "0123456789abcdef") != 16) return 64;
    struct transaction run = {.label = argv[2]};
    snprintf(run.stage_name, sizeof(run.stage_name), ".weread-render-%s", run.label);
    run.volume = directory(AT_FDCWD, USERSTORE);
    run.home = directory(run.volume, "weread");
    run.stage = directory(run.volume, run.stage_name);
    if (run.volume < 0 || !mount_identity(run.volume, &run.mount)) return refuse("ownership");
    run.owner = open_owner(&run);
    if (!authorized(&run)) return refuse("ownership");
    int live = regular(run.home, production);
    if (!pinned(run.home, production, live, run.mount, 0)) return refuse("launcher");
    if (!strcmp(argv[1], "probe")) {
        if (!probe(&run, live)) return refuse("probe");
        puts("exchange_supported");
    } else {
        int saved = regular(run.stage, slot);
        if (!exchange(&run, run.home, production, live, run.stage, slot, saved)) return refuse("unconfirmed");
        puts("exchanged");
    }
    return 0;
}
