#define _GNU_SOURCE
#define _FILE_OFFSET_BITS 64
#include <errno.h>
#include <fcntl.h>
#include <linux/fs.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef DIAG_TEST_USERSTORE
#define USERSTORE "/mnt/us"
#else
#define USERSTORE DIAG_TEST_USERSTORE
#endif

struct result { const char *step; int error; };
struct pair { int first, second; struct stat original, replacement; };
struct scratch { int volume, stage, subdir; const char *label; char name[64]; };

static struct result outcome(const char *step, int error) { return (struct result){step, error}; }
static int inode(const struct stat *first, const struct stat *second) {
    return first->st_dev == second->st_dev && first->st_ino == second->st_ino;
}

static int dir(int parent, const char *name) { return openat(parent, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC); }

static int named(int parent, const char *name, int opened, int is_dir) {
    struct stat file, path;
    errno = 0;
    return opened >= 0 && !fstat(opened, &file) && !fstatat(parent, name, &path, AT_SYMLINK_NOFOLLOW) &&
        inode(&file, &path) && (is_dir ? S_ISDIR(file.st_mode) : S_ISREG(file.st_mode) && file.st_nlink == 1);
}

static int owned(const struct scratch *area) {
    struct stat volume;
    if (!named(AT_FDCWD, USERSTORE, area->volume, 1) || !named(area->volume, area->name, area->stage, 1) ||
        fstat(area->volume, &volume)) return 0;
    int entry = openat(area->stage, "owner", O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC);
    struct stat record, stage;
    char text[18];
    int valid = entry >= 0 && named(area->stage, "owner", entry, 0) && !fstat(entry, &record) &&
        !fstat(area->stage, &stage) && record.st_dev == volume.st_dev && stage.st_dev == volume.st_dev &&
        record.st_size == 17 && pread(entry, text, sizeof(text), 0) == 17 && !memcmp(text, area->label, 16) && text[16] == '\n';
    if (entry >= 0) close(entry);
    return valid;
}

static struct result mount_id(int descriptor, unsigned long long *mount) {
    char path[64], line[160];
    snprintf(path, sizeof(path), "/proc/self/fdinfo/%d", descriptor);
    FILE *info = fopen(path, "r");
    if (!info) return outcome("fdinfo_open", errno);
    struct result result = outcome("mnt_id_missing", 0);
    while (fgets(line, sizeof(line), info)) {
        if (!strncmp(line, "mnt_id:", 7)) {
            char trailing;
            if (sscanf(line + 7, " %llu %c", mount, &trailing) == 1 && *mount && !strcmp(result.step, "mnt_id_missing")) {
                result = outcome("verified", 0);
            } else { result = outcome("mnt_id_format", 0); break; }
        }
    }
    if (ferror(info)) result = outcome("fdinfo_read", errno);
    fclose(info);
    return result;
}

static struct result admission(const struct scratch *area) {
    int owner = openat(area->stage, "owner", O_PATH | O_NOFOLLOW | O_CLOEXEC);
    if (owner < 0) return outcome("owner_opath", errno);
    struct result result = outcome("verified", 0);
    unsigned long long reference = 0;
    int observed[] = {area->volume, area->stage, area->subdir, owner};
    for (size_t index = 0; index < sizeof(observed) / sizeof(*observed); index++) {
        unsigned long long value = 0;
        result = mount_id(observed[index], &value);
        if (strcmp(result.step, "verified")) break;
        if (index && value != reference) { result = outcome("mnt_id_mismatch", 0); break; }
        reference = value;
    }
    if (!strcmp(result.step, "verified")) {
        char path[64], text[18];
        snprintf(path, sizeof(path), "/proc/self/fd/%d", owner);
        int reader = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if (reader < 0) result = outcome("owner_proc_reopen", errno);
        else {
            if (!named(area->stage, "owner", reader, 0) || pread(reader, text, sizeof(text), 0) != 17 ||
                memcmp(text, area->label, 16) || text[16] != '\n') result = outcome("owner_identity", 0);
            close(reader);
        }
    }
    close(owner);
    return result;
}

static const char *filesystem(int volume) {
    struct statfs info;
    if (volume < 0 || fstatfs(volume, &info)) return "unknown";
    switch ((unsigned long)info.f_type) {
        case 0x4d44: return "vfat";
        case 0x2011bab0: return "exfat";
        case 0x65735546: return "fuse";
        case 0xef53: return "ext";
        case 0x58465342: return "xfs";
        case 0x01021994: return "tmpfs";
        default: return "other";
    }
}

static int marker(int directory, const char *name, const char *label, char kind, int *fd, struct stat *saved, struct result *result) {
    *fd = openat(directory, name, O_CREAT | O_EXCL | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (*fd < 0) { *result = outcome(kind == 'A' ? "create_left" : "create_right", errno); return 0; }
    char text[21];
    int size = snprintf(text, sizeof(text), "%s:%c\n", label, kind);
    if (write(*fd, text, (size_t)size) != size) { *result = outcome("marker_write", errno); return 0; }
    if (fsync(*fd)) { *result = outcome("marker_fsync", errno); return 0; }
    if (fstat(*fd, saved) || !named(directory, name, *fd, 0)) { *result = outcome("marker_stat", errno); return 0; }
    return 1;
}

static const char *match(const struct scratch *area, int directory, const char *name, const struct stat *expected, char kind, unsigned links) {
    struct stat path, open_file;
    if (fstatat(directory, name, &path, AT_SYMLINK_NOFOLLOW)) return "post_stat";
    if (!S_ISREG(path.st_mode) || !inode(&path, expected)) return "post_inode";
    if (path.st_nlink != links) return "post_nlink";
    if (path.st_mode != expected->st_mode || path.st_uid != expected->st_uid || path.st_gid != expected->st_gid) return "post_owner_mode";
    if (path.st_size != expected->st_size) return "post_size";
    if (path.st_mtim.tv_sec != expected->st_mtim.tv_sec || path.st_mtim.tv_nsec != expected->st_mtim.tv_nsec) return "post_mtime";
    int file = openat(directory, name, O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC);
    if (file < 0) return "post_open";
    char text[21], expected_text[21];
    int length = snprintf(expected_text, sizeof(expected_text), "%s:%c\n", area->label, kind);
    int valid = !fstat(file, &open_file) && inode(&open_file, &path) && pread(file, text, sizeof(text), 0) == length && !memcmp(text, expected_text, length);
    close(file);
    return valid ? NULL : "post_content";
}

static struct result after(const struct scratch *area, const struct pair *files, int links, int back, int on_left) {
    if (fsync(area->stage)) return outcome(back ? "back_fsync_left" : "out_fsync_left", errno);
    if (fsync(area->subdir)) return outcome(back ? "back_fsync_right" : "out_fsync_right", errno);
    const char *why = NULL;
    if (links) {
        why = match(area, area->stage, "left", back ? &files->original : &files->replacement, back ? 'A' : 'B', 1);
        if (!why && !back) why = match(area, area->subdir, "held", &files->original, 'A', 1);
    } else {
        why = match(area, area->stage, "left", on_left ? &files->original : &files->replacement, on_left ? 'A' : 'B', 1);
        if (!why) why = match(area, area->subdir, "right", on_left ? &files->replacement : &files->original, on_left ? 'B' : 'A', 1);
    }
    return why ? outcome(why, 0) : outcome("supported", 0);
}

static struct result perform(const struct scratch *area, struct pair *files, int links) {
    struct result result;
    if (!marker(area->stage,"left",area->label,'A',&files->first,&files->original,&result) ||
        !marker(area->subdir,"right",area->label,'B',&files->second,&files->replacement,&result)) return result;
    if (!links) {
        if (syscall(SYS_renameat2, area->stage, "left", area->subdir, "right", RENAME_EXCHANGE)) return outcome("exchange_out", errno);
        result = after(area, files, 0, 0, 0);
        if (strcmp(result.step, "supported")) return result;
        if (syscall(SYS_renameat2, area->stage, "left", area->subdir, "right", RENAME_EXCHANGE)) return outcome("exchange_back", errno);
        return after(area, files, 0, 1, 1);
    }
    if (linkat(area->stage, "left", area->subdir, "held", 0)) return outcome("link_create", errno);
    const char *why = match(area, area->stage,"left",&files->original,'A',2);
    if (!why) why = match(area, area->subdir,"held",&files->original,'A',2);
    if (why) return outcome(why, 0);
    if (fsync(area->subdir)) return outcome("link_fsync", errno);
    if (renameat(area->subdir, "right", area->stage, "left")) return outcome("rename_out", errno);
    result = after(area, files, 1, 0, 0);
    if (strcmp(result.step, "supported")) return result;
    if (renameat(area->subdir, "held", area->stage, "left")) return outcome("rename_back", errno);
    return after(area, files, 1, 1, 1);
}

static int clean_marker(const struct scratch *area, int directory, const char *name, const struct pair *files) {
    struct stat path, opened;
    if (fstatat(directory, name, &path, AT_SYMLINK_NOFOLLOW)) return errno == ENOENT;
    if (!S_ISREG(path.st_mode) || path.st_nlink > 2 || !owned(area) || !named(area->stage, "sub", area->subdir, 1)) return 0;
    int created = (files->first >= 0 && !fstat(files->first, &opened) && inode(&path, &opened)) ||
                  (files->second >= 0 && !fstat(files->second, &opened) && inode(&path, &opened));
    if (!created) return 0;
    return !unlinkat(directory, name, 0);
}

static int exists(int directory, const char *name) {
    struct stat path;
    if (!fstatat(directory, name, &path, AT_SYMLINK_NOFOLLOW)) return 1;
    return errno == ENOENT ? 0 : -1;
}

static struct result probe(const struct scratch *area, int links, int *clean) {
    if (exists(area->stage, "left") || exists(area->subdir, "right") || exists(area->subdir, "held")) {
        *clean = 0;
        return outcome("scratch_collision", 0);
    }
    struct pair files = {.first = -1, .second = -1};
    struct result result = perform(area, &files, links);
    int cleared = clean_marker(area, area->subdir, "held", &files);
    cleared &= clean_marker(area, area->subdir, "right", &files);
    cleared &= clean_marker(area, area->stage, "left", &files);
    if (files.first >= 0) close(files.first);
    if (files.second >= 0) close(files.second);
    *clean = cleared;
    return result;
}

static int report(int volume, struct result admission, struct result exchange, struct result hardlink, int clean) {
    printf("{\"policy\":\"selector-diagnostic-v1\",\"filesystem\":\"%s\",\"admission\":\"%s\",\"admission_errno\":%d,\"exchange\":\"%s\",\"exchange_errno\":%d,"
           "\"hardlink\":\"%s\",\"hardlink_errno\":%d,\"clean\":%s}\n", filesystem(volume), admission.step, admission.error, exchange.step, exchange.error,
           hardlink.step, hardlink.error, clean ? "true" : "false");
    return clean ? 0 : 1;
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--policy")) { puts("selector-diagnostic-v1"); return 0; }
    if (argc != 3 || strcmp(argv[1], "diagnose") || strlen(argv[2]) != 16 || strspn(argv[2], "0123456789abcdef") != 16) return 64;
    struct scratch area = {.label = argv[2]};
    snprintf(area.name, sizeof(area.name), ".weread-exchange-diagnostic-%s", area.label);
    area.volume = dir(AT_FDCWD, USERSTORE);
    area.stage = dir(area.volume, area.name);
    struct result skipped = outcome("not_run", 0);
    if (area.stage < 0 || !owned(&area)) return report(area.volume, outcome("scratch_owner", errno), skipped, skipped, 0);
    if (mkdirat(area.stage, "sub", 0700)) return report(area.volume, outcome("scratch_mkdir", errno), skipped, skipped, 0);
    area.subdir = dir(area.stage, "sub");
    if (area.subdir < 0 || !named(area.stage, "sub", area.subdir, 1)) return report(area.volume, outcome("scratch_subdir", errno), skipped, skipped, 0);
    struct result admissible = admission(&area);
    int clean = 0;
    struct result exchange = probe(&area, 0, &clean);
    struct result hardlink = clean ? probe(&area, 1, &clean) : skipped;
    if (clean) clean = owned(&area) && named(area.stage, "sub", area.subdir, 1) &&
        !unlinkat(area.stage, "sub", AT_REMOVEDIR) && !exists(area.stage, "sub") && !exists(area.stage, "left");
    return report(area.volume, admissible, exchange, hardlink, clean);
}
