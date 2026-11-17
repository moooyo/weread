/* Firmware-ABI bridge for a bounded, read-only portrait calibration session. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <signal.h>
#include <stddef.h>
#include <string.h>
#include <sys/prctl.h>
#include <unistd.h>

typedef void Connection;
struct Api {
    Connection *(*open)(void);
    void (*close)(Connection *);
    int (*get)(Connection *, const char *, const char *, char **);
    void (*free_value)(char *);
};

static int send_bytes(const void *data, size_t remaining) {
    const unsigned char *cursor = data;
    while (remaining) {
        ssize_t count = write(STDOUT_FILENO, cursor, remaining);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return -1;
        cursor += count;
        remaining -= (size_t)count;
    }
    return 0;
}

static void *load(struct Api *api) {
#ifdef WEREAD_QUERY_TEST_LIBRARY
    void *library = dlopen(WEREAD_QUERY_TEST_LIBRARY, RTLD_NOW | RTLD_LOCAL);
#else
    void *library = dlopen("/usr/lib/liblipc.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!library) library = dlopen("/usr/lib/liblipc.so.1.0", RTLD_NOW | RTLD_LOCAL);
#endif
    if (!library) return NULL;
    api->open = (Connection *(*)(void))dlsym(library, "LipcOpenNoName");
    api->close = (void (*)(Connection *))dlsym(library, "LipcClose");
    api->get = (int (*)(Connection *, const char *, const char *, char **))dlsym(library, "LipcGetStringProperty");
    api->free_value = (void (*)(char *))dlsym(library, "LipcFreeString");
    if (api->open && api->close && api->get && api->free_value) return library;
    dlclose(library);
    return NULL;
}

int main(int argc, char **argv) {
    if (argc != 2 || strcmp(argv[1], "--session")) return 64;
    pid_t owner = getppid();
    if (owner <= 1 || prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != owner) return 70;
    signal(SIGPIPE, SIG_IGN);
    alarm(5);
    struct Api api = {0};
    void *library = load(&api);
    if (!library) return 69;
    Connection *connection = api.open();
    if (!connection) { dlclose(library); return 69; }
    int result = send_bytes("WQ1\n", 4) ? 74 : 0;
    unsigned int requests = 0;
    while (!result) {
        unsigned char operation;
        ssize_t count = read(STDIN_FILENO, &operation, 1);
        if (count < 0 && errno == EINTR) continue;
        if (!count) break;
        if (count != 1 || (operation != 1 && operation != 2) || requests++ >= 4) {
            result = 64;
            break;
        }
        /* No provider, property name, setter, or executable comes from input. */
        const char *service = operation == 1 ? "com.lab126.powerd" : "com.lab126.winmgr";
        const char *property = operation == 1 ? "state" : "accelerometer";
        char *value = NULL;
        int status = api.get(connection, service, property, &value);
        size_t length = value ? strnlen(value, 129) : 0;
        unsigned char error = status ? 1 : (!value || length > 128 ? 2 : 0);
        if (error) length = 0;
        unsigned char header[4] = {operation, error, 0, (unsigned char)length};
        if (send_bytes(header, sizeof(header)) || (length && send_bytes(value, length))) result = 74;
        if (value) api.free_value(value);
    }
    api.close(connection);
    dlclose(library);
    return result;
}
