#define _GNU_SOURCE
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

static int number(const char *text, size_t max) {
    size_t length = strlen(text);
    return length && length <= max && text[0] != '0' && strspn(text, "0123456789") == length;
}

static int record(int dir, const char *path, const char *expected) {
    char buffer[192];
    int descriptor = openat(dir, path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (descriptor < 0) return 0;
    ssize_t bytes_read = read(descriptor, buffer, sizeof(buffer));
    close(descriptor);
    return bytes_read > 0 && (size_t)bytes_read == strlen(expected) && !memcmp(buffer, expected, (size_t)bytes_read);
}

int main(int argc, char **argv) {
    if (argc != 4 || strlen(argv[1]) != 16 || strspn(argv[1], "0123456789abcdef") != 16 ||
            !number(argv[2], 7) || !number(argv[3], 19)) return 64;
    int pid = atoi(argv[2]);
    if (pid <= 1) return 64;
    int target = (int)syscall(SYS_pidfd_open, pid, 0);
    if (target < 0) return 2;
    char stage[96], proc[96], path[160], wanted[160], link[160], stat[2048], generation[36];
    snprintf(stage, sizeof(stage), "/mnt/us/.weread-orientation-%s", argv[1]);
    snprintf(proc, sizeof(proc), "/proc/%d", pid);
    int sdir = open(stage,O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    int pdir = open(proc,O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    int lock = open("/tmp/weread-launcher.lock",O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    if (sdir < 0 || pdir < 0 || lock < 0) return 2;
    int fd = openat(pdir,"stat",O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    ssize_t bytes_read = fd < 0 ? -1 : read(fd, stat, sizeof(stat)-1);
    if (bytes_read <= 0 || bytes_read >= (ssize_t)sizeof(stat)-1) return 2;
    close(fd); stat[bytes_read]=0;
    char *end = strrchr(stat,')');
    if (!end || end[1] != ' ') return 2;
    char *value = strtok(end+2, " ");
    if (!value || (strcmp(value,"R") && strcmp(value,"S"))) return 2;
    for (int field=4; value && field<=22; ++field) value = strtok(NULL," ");
    if (!value || strcmp(value, argv[3])) return 2;
    snprintf(path,sizeof(path),"%s/B/app/weread",stage);
    bytes_read=readlinkat(pdir,"exe",link,sizeof(link));
    if (bytes_read<=0 || (size_t)bytes_read!=strlen(path) || memcmp(link,path,(size_t)bytes_read)) return 2;
    snprintf(wanted,sizeof(wanted),"%s\n",argv[1]);
    if (!record(sdir,"owner",wanted) || !record(sdir,"out/B-reader/entered/arm","B-reader\n")) return 2;
    fd=openat(lock,"generation",O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    bytes_read=fd<0 ? -1 : read(fd,generation,sizeof(generation));
    if (bytes_read!=35 || generation[0]!='1' || generation[1]!=':' || generation[34]!='\n') return 2;
    close(fd); generation[34]=0;
    if (strspn(generation+2,"0123456789abcdef")!=32) return 2;
    snprintf(wanted,sizeof(wanted),"%s:%s:%s\n",generation,argv[2],argv[3]);
    if (!record(lock,"application-owned",wanted) || !record(sdir,"out/B-reader/wake-owner",wanted)) return 2;
    snprintf(wanted,sizeof(wanted),"%s:1\n",generation);
    if (!record(lock,"managed-wake-request",wanted) || !record(lock,"managed-wake-ready",wanted)) return 2;
    return syscall(SYS_pidfd_send_signal,target,SIGTERM,NULL,0) == 0 ? 0 : 2;
}
