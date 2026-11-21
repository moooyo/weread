#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/timerfd.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#if defined(WEREAD_POSE_VISIT_TEST) && defined(__arm__)
#error "The simulation clock must not be built for the Kindle"
#endif

static int command(char *const arguments[]) {
    pid_t child = fork();
    if (child < 0) return 0;
    if (child == 0) {
        if (setpgid(0, 0) != 0) _exit(125);
        int devnull = open("/dev/null", O_RDWR);
        if (devnull < 0 || dup2(devnull, 0) < 0 || dup2(devnull, 1) < 0 || dup2(devnull, 2) < 0) _exit(125);
        if (devnull > 2) close(devnull);
        close(8);
        execvp(arguments[0], arguments);
        _exit(127);
    }
    int status;
    for (unsigned attempt = 0; attempt < 30; ++attempt) {
        pid_t result = waitpid(child, &status, WNOHANG);
        if (result == child) return WIFEXITED(status) && WEXITSTATUS(status) == 0;
        if (result < 0 && errno != EINTR) return 0;
        usleep(50000);
    }
    kill(-child, SIGKILL);
    kill(child, SIGKILL);
    while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
    return 0;
}

static int finish(const char *text, int status) {
    puts(text);
    return fflush(stdout) == 0 ? status : 1;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--policy") == 0) {
#ifdef WEREAD_POSE_VISIT_TEST
        return finish("simulation CLOCK_BOOTTIME", 0);
#else
        return finish("production CLOCK_BOOTTIME_ALARM", 0);
#endif
    }
    if (argc != 4 || strcmp(argv[1], "guarded-cycle") != 0) return 64;
    if (setsid() < 0 || signal(SIGHUP, SIG_IGN) == SIG_ERR || signal(SIGPIPE, SIG_IGN) == SIG_ERR)
        return finish("not_run guardian_unavailable", 77);
#ifdef WEREAD_POSE_VISIT_TEST
    const clockid_t clock = CLOCK_BOOTTIME;
    const int delay = 2;
#else
    const clockid_t clock = CLOCK_BOOTTIME_ALARM;
    const int delay = 8;
#endif
    int alarm = timerfd_create(clock, TFD_CLOEXEC);
    if (alarm < 0) return finish("not_run alarm_unavailable", 77);
    char *owner[] = {"sh", "-c", "g=$(cat \"$2/generation\") && p=$(cat \"$2/application-pid\") && b=$(cat \"$2/application-birth\") && "
        "test \"$g:$p:$b\" = \"$(cat \"$1/power-owner\")\" && cmp -s \"$1/power-owner\" \"$2/application-owned\" && "
        "test \"$(awk '{print $22}' \"/proc/$p/stat\")\" = \"$b\" && test \"$(readlink \"/proc/$p/exe\")\" = \"${1%/out/B-reader}/B/app/weread\"",
        "pose-owner", argv[2], argv[3], NULL};
    char *cycle[] = {"sh", "-c", "g=$(cat \"$1/generation\") && printf '%s:1\\n' \"$g\" | cmp -s - \"$1/managed-sleep-request\" && "
        "cmp -s \"$1/managed-sleep-request\" \"$1/managed-sleep-ready\"", "pose-cycle", argv[3], NULL};
    if (!command(owner)) return finish("not_run guardian_unavailable", 77);
    char *wake[] = {"lipc-set-prop", "-i", "com.lab126.powerd", "wakeUp", "1", NULL};
    if (!command(wake)) return finish("not_run wake_command_unavailable", 77);
    char *active[] = {"sh", "-c", "test \"$(lipc-get-prop -s com.lab126.powerd state)\" = active", NULL};
    if (!command(active)) return finish("not_run power_state_unavailable", 77);
    struct itimerspec schedule = {.it_value = {.tv_sec = delay}};
    if (timerfd_settime(alarm, 0, &schedule, NULL) < 0 || timerfd_gettime(alarm, &schedule) < 0 ||
            schedule.it_value.tv_sec < delay - 1) return finish("not_run alarm_unavailable", 77);
    int directory = open(argv[2], O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    int intent = directory < 0 ? -1 : openat(directory, "power-intent", O_WRONLY | O_CLOEXEC | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
    if (intent < 0 || write(intent, "power-cycle\n", 12) != 12 || close(intent) != 0) return finish("not_run guardian_unavailable", 77);
    close(directory);
    char *enter[] = {"powerd_test", "-p", NULL};
    int requested = command(enter);
    uint64_t expirations = 0;
    struct pollfd waiting = {.fd = alarm, .events = POLLIN};
    ssize_t count = -1;
    if (poll(&waiting, 1, (delay + 1) * 1000) == 1 && (waiting.revents & POLLIN))
        count = read(alarm, &expirations, sizeof(expirations));
    close(alarm);
    int woke = command(active);
    for (unsigned attempt = 0; !woke && attempt < 3; ++attempt) {
        if (!command(owner) || (attempt && !command(cycle))) break;
        command(wake);
        for (unsigned tick = 0; !woke && tick < 5; ++tick) {
            usleep(100000);
            woke = command(active);
        }
    }
    if (count != (ssize_t)sizeof(expirations) || !expirations || !requested || !woke)
        return finish("not_qualified power_cycle_unconfirmed", 1);
    return finish("requested", 0);
}
