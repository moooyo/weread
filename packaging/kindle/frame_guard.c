/* Retain an application frame in a native window across a display handoff. */
#define _GNU_SOURCE
#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define MAX_PIXELS (UINT32_C(1860) * UINT32_C(2480))
#define MAX_STRIDE UINT32_C(4096)
#define MAX_FRAMEBUFFER_BYTES (UINT64_C(32) * 1024 * 1024)
#define MAX_IMAGE_BYTES (UINT64_C(1) * 1024 * 1024)
#define IMAGE_ROWS UINT32_C(64)
#define RECORD_BYTES 256
#define OPERATION_SECONDS 4
#define MAX_SEQUENCE UINT64_C(2147483647)
#define MAX_ROOT_CHILDREN 512
/* ID:system is the native dialog layer's global ownership sentinel. */
#define GUARD_TITLE "L:D_N:wereadGuard_ID:system_BS:true_O:UD"

/* Only GETVAR/GETFIX are used. These layouts follow the Linux framebuffer ABI. */
struct fb_bitfield { uint32_t offset, length, msb_right; };
struct fb_var_info {
    uint32_t xres, yres, xres_virtual, yres_virtual, xoffset, yoffset;
    uint32_t bits_per_pixel, grayscale;
    struct fb_bitfield red, green, blue, transp;
    uint32_t nonstd, activate, height, width, accel_flags, pixclock;
    uint32_t left_margin, right_margin, upper_margin, lower_margin;
    uint32_t hsync_len, vsync_len, sync, vmode, rotate, colorspace, reserved[4];
};
struct fb_fix_info {
    char id[16];
    unsigned long smem_start;
    uint32_t smem_len, type, type_aux, visual;
    uint16_t xpanstep, ypanstep, ywrapstep;
    uint32_t line_length;
    unsigned long mmio_start;
    uint32_t mmio_len, accel;
    uint16_t capabilities, reserved[2];
};
_Static_assert(sizeof(struct fb_var_info) == 160, "GETVAR ABI size");
#if ULONG_MAX == UINT32_MAX
_Static_assert(sizeof(struct fb_fix_info) == 68, "ARM GETFIX ABI size");
#endif

#define X_FUNCTIONS(F) \
    F(XOpenDisplay) F(XCloseDisplay) F(XDefaultRootWindow) \
    F(XGetWindowAttributes) F(XCreateWindow) F(XDestroyWindow) \
    F(XCreatePixmap) F(XFreePixmap) F(XCreateGC) F(XFreeGC) \
    F(XCreateImage) F(XPutImage) F(XSetWindowBackgroundPixmap) \
    F(XClearWindow) F(XStoreName) F(XInternAtom) F(XChangeProperty) \
    F(XSetClassHint) F(XSetWMHints) F(XMapWindow) F(XSync) \
    F(XTranslateCoordinates) F(XQueryTree) F(XFree) F(XAllocColor) F(XPending) F(XNextEvent) \
    F(XConnectionNumber) F(XMaxRequestSize) F(XSetErrorHandler) F(XSetIOErrorHandler)
#define DECLARE_X(name) static __typeof__(name) *api_##name;
X_FUNCTIONS(DECLARE_X)
#undef DECLARE_X

struct process_identity { pid_t parent; uint64_t birth; char state; };
struct request {
    uint64_t sequence, generation, birth;
    pid_t application, parent;
    int align;
};
struct guard {
    int context, commands;
    pid_t owner;
    uint64_t owner_birth, self_birth, sequence;
    char token[33];
    uint32_t width, height, source_rotation;
    unsigned char *snapshot;
    void *library;
    Display *display;
    Window root, window;
    Pixmap pixmap;
    GC gc;
    XWindowAttributes root_attributes;
    unsigned long gray_pixels[256];
};
static volatile sig_atomic_t stopping;
static volatile sig_atomic_t x_error;

static void stop_signal(int signal_number) {
    (void)signal_number;
    stopping = 1;
}
static void operation_timeout(int signal_number) {
    (void)signal_number;
    /* Closing this process's X connection releases its window and pixmaps. */
    _exit(1);
}
static int report(const char *step) {
    fprintf(stderr, "weread frame guard: %s\n", step);
    return -1;
}
static int x_protocol_error(Display *display, XErrorEvent *event) {
    (void)display;
    (void)event;
    x_error = 1;
    return 0;
}
static int x_connection_error(Display *display) {
    (void)display;
    _exit(1);
}
static void clear_bytes(void *buffer, size_t bytes) {
    volatile unsigned char *destination = buffer;
    while (bytes-- != 0) { *destination++ = 0; }
}
static int decimal(const char *value, uint64_t maximum, uint64_t *result) {
    if (!value || !*value || (*value == '0' && value[1])) { return -1; }
    uint64_t parsed = 0;
    for (const unsigned char *cursor = (const unsigned char *)value; *cursor; ++cursor) {
        if (*cursor < '0' || *cursor > '9' || parsed > (maximum - (*cursor - '0')) / 10) {
            return -1;
        }
        parsed = parsed * 10 + (*cursor - '0');
    }
    *result = parsed;
    return 0;
}
static int token_valid(const char *token) {
    if (strlen(token) != 32) { return 0; }
    for (size_t index = 0; index < 32; ++index) {
        if (!((token[index] >= '0' && token[index] <= '9') ||
              (token[index] >= 'a' && token[index] <= 'f'))) { return 0; }
    }
    return 1;
}
static int read_all(int descriptor, char *buffer, size_t capacity) {
    size_t used = 0;
    unsigned interrupted = 0;
    while (used + 1 < capacity) {
        ssize_t count = read(descriptor, buffer + used, capacity - used - 1);
        if (count < 0 && errno == EINTR && !stopping && interrupted++ < 16) { continue; }
        if (count < 0) { return -1; }
        if (!count) { buffer[used] = 0; return (int)used; }
        if (memchr(buffer + used, 0, (size_t)count)) { return -1; }
        used += (size_t)count;
    }
    return -1;
}
static int context_record(struct guard *guard, const char *name, char *buffer, size_t capacity) {
    int descriptor = openat(guard->context, name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (descriptor < 0) { return -1; }
    struct stat info;
    int result = -1;
    if (fstat(descriptor, &info) == 0 && S_ISREG(info.st_mode) &&
        info.st_uid == getuid() && info.st_size > 0 && (uint64_t)info.st_size < capacity) {
        result = read_all(descriptor, buffer, capacity);
    }
    close(descriptor);
    if (result < 1 || buffer[result - 1] != '\n') { return -1; }
    buffer[result - 1] = 0;
    return memchr(buffer, '\n', (size_t)result - 1) ? -1 : 0;
}
static int process_identity(pid_t process, struct process_identity *identity) {
    char path[64], buffer[4096];
    snprintf(path, sizeof(path), "/proc/%ld/stat", (long)process);
    int descriptor = open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (descriptor < 0) { return -1; }
    int bytes = read_all(descriptor, buffer, sizeof(buffer));
    close(descriptor);
    if (bytes < 0) { return -1; }
    char *tail = strrchr(buffer, ')');
    if (!tail || tail[1] != ' ' || !tail[2] || tail[3] != ' ') { return -1; }
    identity->state = tail[2];
    char *cursor = tail + 4;
    uint64_t parsed;
    for (unsigned field = 4; field <= 22; ++field) {
        char *end = strchr(cursor, ' ');
        if (!end) { return -1; }
        *end = 0;
        if (field == 4 || field == 22) {
            if (decimal(cursor, field == 4 ? INT_MAX : UINT64_MAX, &parsed)) { return -1; }
            if (field == 4) { identity->parent = (pid_t)parsed; }
            else { identity->birth = parsed; }
        }
        cursor = end + 1;
    }
    return identity->birth ? 0 : -1;
}
static int owner_current(struct guard *guard) {
    struct process_identity identity;
    char record[RECORD_BYTES], expected[RECORD_BYTES];
    if (getppid() != guard->owner || process_identity(guard->owner, &identity) ||
        identity.birth != guard->owner_birth || identity.state == 'Z' || identity.state == 'X') {
        return 0;
    }
    snprintf(expected, sizeof(expected), "%ld", (long)guard->owner);
    return !context_record(guard, "pid", record, sizeof(record)) && !strcmp(record, expected) &&
        !context_record(guard, "nonce", record, sizeof(record)) && !strcmp(record, guard->token);
}
static int protect_owner(struct guard *guard) {
    struct sigaction action = {0};
    action.sa_handler = stop_signal;
    sigemptyset(&action.sa_mask);
    sigaction(SIGHUP, &action, NULL);
    sigaction(SIGINT, &action, NULL);
    sigaction(SIGTERM, &action, NULL);
    action.sa_handler = operation_timeout;
    sigaction(SIGALRM, &action, NULL);
    signal(SIGPIPE, SIG_IGN);
    if (prctl(PR_SET_PDEATHSIG, SIGTERM) || !owner_current(guard)) { return -1; }
    struct rlimit no_core = { .rlim_cur = 0, .rlim_max = 0 };
    if (setrlimit(RLIMIT_CORE, &no_core)) {
        return report("private frame core dump prevention failed");
    }
    return 0;
}
static int application_frozen(struct guard *guard, const struct request *request) {
    struct process_identity identity;
    char record[RECORD_BYTES], expected[RECORD_BYTES];
    if (!owner_current(guard) || process_identity(request->application, &identity) ||
        identity.birth != request->birth || identity.parent != request->parent ||
        (identity.state != 'T' && identity.state != 't')) { return 0; }
    snprintf(expected, sizeof(expected), "%" PRIu64 ":%s", request->generation, guard->token);
    if (context_record(guard, "generation", record, sizeof(record)) || strcmp(record, expected)) {
        return 0;
    }
    snprintf(expected, sizeof(expected), "%ld", (long)request->application);
    if (context_record(guard, "application-pid", record, sizeof(record)) || strcmp(record, expected)) {
        return 0;
    }
    snprintf(expected, sizeof(expected), "%" PRIu64, request->birth);
    return !context_record(guard, "application-birth", record, sizeof(record)) && !strcmp(record, expected);
}
static int write_bytes(int descriptor, const void *data, size_t bytes) {
    const unsigned char *cursor = data;
    unsigned interrupted = 0;
    while (bytes) {
        ssize_t count = write(descriptor, cursor, bytes);
        if (count < 0 && errno == EINTR && !stopping && interrupted++ < 16) { continue; }
        if (count <= 0) { return -1; }
        cursor += count;
        bytes -= (size_t)count;
    }
    return 0;
}
static int publish_record(struct guard *guard, const char *pending, const char *destination,
        const char *record) {
    size_t bytes = strlen(record);
    if (!bytes || bytes >= RECORD_BYTES) { return -1; }
    int descriptor = openat(guard->context, pending,
        O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (descriptor < 0) { return -1; }
    struct stat info;
    int result = fstat(descriptor, &info) || !S_ISREG(info.st_mode) || info.st_uid != getuid() ||
        fchmod(descriptor, 0600) || write_bytes(descriptor, record, bytes);
    if (close(descriptor)) { result = -1; }
    if (!result && owner_current(guard)) {
        result = renameat(guard->context, pending, guard->context, destination);
    } else { result = -1; }
    if (result) { unlinkat(guard->context, pending, 0); }
    return result;
}
static int publish_ready(struct guard *guard, uint64_t sequence, uint64_t generation) {
    char record[RECORD_BYTES];
    snprintf(record, sizeof(record), "%" PRIu64 ":%" PRIu64 ":%s:%ld:%" PRIu64 "\n",
        sequence, generation, guard->token, (long)getpid(), guard->self_birth);
    return publish_record(guard, "frame-guard-ready.pending", "frame-guard-ready", record);
}
static int publish_owned_window(struct guard *guard) {
    char record[RECORD_BYTES];
    snprintf(record, sizeof(record), "%s:%ld:%" PRIu64 ":0x%lx\n",
        guard->token, (long)getpid(), guard->self_birth, guard->window);
    return publish_record(guard, "frame-guard-window.pending", "frame-guard-window", record);
}
static int query_framebuffer(int descriptor, struct fb_var_info *variable, struct fb_fix_info *fixed) {
#ifdef WEREAD_FRAME_GUARD_TEST
    char metadata[128];
    const char *fixture = getenv("WEREAD_TEST_FRAMEBUFFER_METADATA");
    const char *metadata_file = getenv("WEREAD_TEST_FRAMEBUFFER_METADATA_FILE");
    if (metadata_file) {
        int metadata_descriptor = open(metadata_file, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (metadata_descriptor < 0) { return -1; }
        int bytes = read_all(metadata_descriptor, metadata, sizeof(metadata));
        close(metadata_descriptor);
        if (bytes < 0) { return -1; }
        fixture = metadata;
    }
    if (fixture) {
        memset(variable, 0, sizeof(*variable));
        memset(fixed, 0, sizeof(*fixed));
        if (sscanf(fixture, "%u:%u:%u:%u", &variable->xres, &variable->yres,
                &fixed->line_length, &variable->rotate) != 4) { return -1; }
        variable->xres_virtual = variable->xres;
        variable->yres_virtual = variable->yres;
        variable->bits_per_pixel = 8;
        fixed->smem_len = fixed->line_length * variable->yres;
        return 0;
    }
#endif
    return ioctl(descriptor, 0x4600UL, variable) || ioctl(descriptor, 0x4602UL, fixed) ? -1 : 0;
}
static int open_framebuffer(void) {
    const char *path = "/dev/fb0";
#ifdef WEREAD_FRAME_GUARD_TEST
    const char *fixture = getenv("WEREAD_TEST_FRAMEBUFFER");
    if (fixture) { path = fixture; }
#endif
    return open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
}
static int framebuffer_valid(struct guard *guard, const struct fb_var_info *variable,
        const struct fb_fix_info *fixed) {
    uint64_t bytes = (uint64_t)fixed->line_length * variable->yres;
    return variable->xres == guard->width && variable->yres == guard->height &&
        variable->xoffset == 0 && variable->yoffset == 0 && variable->bits_per_pixel == 8 &&
        variable->xres_virtual >= variable->xres && variable->yres_virtual >= variable->yres &&
        variable->rotate <= 3 && fixed->line_length >= variable->xres &&
        fixed->line_length <= MAX_STRIDE && bytes <= MAX_FRAMEBUFFER_BYTES && bytes <= fixed->smem_len;
}
static int read_frame(struct guard *guard, int capture, uint32_t *rotation) {
    struct fb_var_info before = {0}, after = {0};
    struct fb_fix_info fixed_before = {0}, fixed_after = {0};
    int descriptor = open_framebuffer();
    if (descriptor < 0) { return report("framebuffer open failed"); }
    int result = query_framebuffer(descriptor, &before, &fixed_before);
    if (!result && !framebuffer_valid(guard, &before, &fixed_before)) { result = -1; }
    if (!result && capture) {
        for (uint32_t row = 0; row < guard->height && !result; ++row) {
            size_t used = 0;
            unsigned interrupted = 0;
            while (used < guard->width) {
                ssize_t count = pread(descriptor, guard->snapshot + (size_t)row * guard->width + used,
                    guard->width - used, (off_t)((uint64_t)row * fixed_before.line_length + used));
                if (count < 0 && errno == EINTR && !stopping && interrupted++ < 16) { continue; }
                if (count <= 0) { result = -1; break; }
                used += (size_t)count;
            }
        }
    }
    if (!result && (query_framebuffer(descriptor, &after, &fixed_after) ||
        !framebuffer_valid(guard, &after, &fixed_after) ||
        before.rotate != after.rotate || fixed_before.line_length != fixed_after.line_length)) {
        result = -1;
    }
    close(descriptor);
    if (result) { return report("framebuffer snapshot was not stable"); }
    *rotation = before.rotate;
    return 0;
}
static int load_xlib(struct guard *guard) {
    guard->library = dlopen("libX11.so.6", RTLD_NOW | RTLD_LOCAL);
    if (!guard->library) { return report("X11 runtime unavailable"); }
#define LOAD_X(name) do { \
    void *symbol = dlsym(guard->library, #name); \
    _Static_assert(sizeof(symbol) == sizeof(api_##name), "X11 function pointer ABI"); \
    if (!symbol) { return report("X11 entry point unavailable"); } \
    memcpy(&api_##name, &symbol, sizeof(symbol)); \
} while (0);
    X_FUNCTIONS(LOAD_X)
#undef LOAD_X
    api_XSetErrorHandler(x_protocol_error);
    api_XSetIOErrorHandler(x_connection_error);
#ifdef WEREAD_FRAME_GUARD_TEST
    guard->display = api_XOpenDisplay(NULL);
#else
    guard->display = api_XOpenDisplay(":0");
#endif
    if (!guard->display) { return report("native X display unavailable"); }
    guard->root = api_XDefaultRootWindow(guard->display);
    if (!api_XGetWindowAttributes(guard->display, guard->root, &guard->root_attributes)) {
        return report("native X geometry unavailable");
    }
    return 0;
}
static unsigned long masked_gray(unsigned value, unsigned long mask) {
    if (!mask) { return 0; }
    unsigned shift = 0;
    while (!(mask & 1UL)) { mask >>= 1; ++shift; }
    return (((unsigned long)value * mask + 127) / 255) << shift;
}
static int prepare_colors(struct guard *guard) {
    Visual *visual = guard->root_attributes.visual;
    if (!visual || guard->root_attributes.depth < 8 || guard->root_attributes.depth > 32) {
        return report("native X pixel format unsupported");
    }
    for (unsigned value = 0; value < 256; ++value) {
        if (visual->class == TrueColor) {
            unsigned long depth_mask = guard->root_attributes.depth == 32 ? UINT32_MAX :
                (1UL << (unsigned)guard->root_attributes.depth) - 1;
            unsigned long opaque_planes = depth_mask &
                ~(visual->red_mask | visual->green_mask | visual->blue_mask);
            guard->gray_pixels[value] = masked_gray(value, visual->red_mask) |
                masked_gray(value, visual->green_mask) | masked_gray(value, visual->blue_mask) |
                opaque_planes;
        } else if (visual->class == StaticGray) {
            if (guard->root_attributes.depth != 8 || visual->map_entries != 256) {
                return report("native X grayscale map is not 8-bit");
            }
            XColor color = {0};
            color.red = color.green = color.blue = (unsigned short)(value * 257);
            color.flags = DoRed | DoGreen | DoBlue;
            if (!api_XAllocColor(guard->display, guard->root_attributes.colormap, &color)) {
                return report("native X grayscale map unavailable");
            }
            /* A snapshot byte must retain its exact neutral intensity. */
            if (color.pixel > UINT8_MAX || color.red != value * 257 ||
                color.green != value * 257 || color.blue != value * 257) {
                return report("native X grayscale map is not exact");
            }
            guard->gray_pixels[value] = color.pixel;
        } else { return report("native X visual unsupported"); }
    }
    return 0;
}
static int window_fullscreen(struct guard *guard) {
    XWindowAttributes attributes;
    int x, y;
    Window child;
    return api_XGetWindowAttributes(guard->display, guard->window, &attributes) &&
        attributes.map_state == IsViewable && attributes.width == (int)guard->width &&
        attributes.height == (int)guard->height && attributes.border_width == 0 &&
        api_XTranslateCoordinates(guard->display, guard->window, guard->root, 0, 0, &x, &y, &child) &&
        x == 0 && y == 0 && !x_error;
}
static int drain_x_events(struct guard *guard) {
    while (api_XPending(guard->display) && !stopping) {
        XEvent event;
        api_XNextEvent(guard->display, &event);
        if (event.type == DestroyNotify && event.xdestroywindow.window == guard->window) {
            return report("native guard window was destroyed");
        }
    }
    return x_error ? -1 : 0;
}
static int owned_surface_is_topmost(struct guard *guard) {
    Window surface = guard->window;
    Window root, parent, *children = NULL;
    unsigned children_count;
    /* Check clipping and siblings along only our client's ancestor chain. */
    for (unsigned level = 0; level < 8; ++level) {
        children = NULL;
        if (!api_XQueryTree(guard->display, surface, &root, &parent, &children, &children_count)) {
            return 0;
        }
        if (children) { api_XFree(children); }
        if (root != guard->root || !parent || parent == surface) { return 0; }
        XWindowAttributes parent_attributes;
        int parent_x, parent_y;
        Window translated_child;
        if (!api_XGetWindowAttributes(guard->display, parent, &parent_attributes) ||
            parent_attributes.map_state != IsViewable ||
            !api_XTranslateCoordinates(guard->display, parent, guard->root, 0, 0,
                &parent_x, &parent_y, &translated_child) || parent_x > 0 || parent_y > 0 ||
            (int64_t)parent_x + parent_attributes.width < guard->width ||
            (int64_t)parent_y + parent_attributes.height < guard->height) { return 0; }
        children = NULL;
        Window parent_of_parent;
        if (!api_XQueryTree(guard->display, parent, &root, &parent_of_parent, &children, &children_count)) {
            return 0;
        }
        if (root != guard->root || children_count > MAX_ROOT_CHILDREN) {
            if (children) { api_XFree(children); }
            return 0;
        }
        int seen_surface = 0, covered = 0;
        /* XQueryTree orders immediate children from bottom to top. */
        for (unsigned index = 0; index < children_count; ++index) {
            if (children[index] == surface) { seen_surface = 1; continue; }
            if (!seen_surface) { continue; }
            XWindowAttributes attributes;
            int x, y;
            if (!api_XGetWindowAttributes(guard->display, children[index], &attributes)) {
                covered = 1;
                break;
            }
            if (attributes.map_state != IsViewable || attributes.class != InputOutput ||
                attributes.width <= 0 || attributes.height <= 0) { continue; }
            if (!api_XTranslateCoordinates(guard->display, children[index], guard->root, 0, 0,
                &x, &y, &translated_child)) { covered = 1; break; }
            if (x < (int)guard->width && y < (int)guard->height &&
                (int64_t)x + attributes.width > 0 && (int64_t)y + attributes.height > 0) {
                covered = 1;
                break;
            }
        }
        if (children) { api_XFree(children); }
        if (!seen_surface || covered || x_error) { return 0; }
        if (parent == guard->root) { return 1; }
        surface = parent;
    }
    return 0;
}
static int make_pixmap(struct guard *guard, uint32_t rotation, Pixmap *result) {
    uint32_t delta = (rotation + 4 - guard->source_rotation) % 4;
    if (delta != 0 && delta != 2) { return report("portrait scanout delta unsupported"); }
    XImage *image = api_XCreateImage(guard->display, guard->root_attributes.visual,
        (unsigned)guard->root_attributes.depth, ZPixmap, 0, NULL, guard->width, IMAGE_ROWS, 32, 0);
    if (!image || image->bits_per_pixel < 8 || image->bits_per_pixel > 32 ||
        image->bits_per_pixel % 8 || image->bytes_per_line < (int)guard->width ||
        (uint64_t)image->bytes_per_line * IMAGE_ROWS > MAX_IMAGE_BYTES) {
        if (image) { XDestroyImage(image); }
        return report("native X image layout unsupported");
    }
    size_t image_bytes = (size_t)image->bytes_per_line * IMAGE_ROWS;
    long request_words = api_XMaxRequestSize(guard->display);
    if (request_words < 64 || request_words > 65535) {
        XDestroyImage(image);
        return report("native X request limit unsupported");
    }
    uint32_t band_rows = (uint32_t)((request_words * 4 - 64) / image->bytes_per_line);
    if (!band_rows) { XDestroyImage(image); return report("native X scanline exceeds request limit"); }
    if (band_rows > IMAGE_ROWS) { band_rows = IMAGE_ROWS; }
    image->data = calloc(1, image_bytes);
    if (!image->data) { XDestroyImage(image); return report("image allocation failed"); }
    Pixmap pixmap = api_XCreatePixmap(guard->display, guard->root, guard->width,
        guard->height, (unsigned)guard->root_attributes.depth);
    if (!pixmap) { XDestroyImage(image); return report("native pixmap allocation failed"); }
    unsigned pixel_bytes = (unsigned)image->bits_per_pixel / 8;
    for (uint32_t row = 0; row < guard->height; row += band_rows) {
        uint32_t rows = guard->height - row < band_rows ? guard->height - row : band_rows;
        for (uint32_t offset = 0; offset < rows; ++offset) {
            for (uint32_t column = 0; column < guard->width; ++column) {
                uint32_t source_y = delta ? guard->height - row - offset - 1 : row + offset;
                uint32_t source_x = delta ? guard->width - column - 1 : column;
                unsigned long pixel = guard->gray_pixels[guard->snapshot[(size_t)source_y * guard->width + source_x]];
                unsigned char *destination = (unsigned char *)image->data +
                    (size_t)offset * (size_t)image->bytes_per_line + (size_t)column * pixel_bytes;
                for (unsigned byte = 0; byte < pixel_bytes; ++byte) {
                    unsigned shift = image->byte_order == LSBFirst ? byte * 8 : (pixel_bytes - byte - 1) * 8;
                    destination[byte] = (unsigned char)(pixel >> shift);
                }
            }
        }
        api_XPutImage(guard->display, pixmap, guard->gc, image, 0, 0, 0, (int)row, guard->width, rows);
    }
    clear_bytes(image->data, image_bytes);
    XDestroyImage(image);
    api_XSync(guard->display, False);
    if (x_error || stopping) { api_XFreePixmap(guard->display, pixmap); return -1; }
    *result = pixmap;
    return 0;
}
static int publish_snapshot(struct guard *guard, uint32_t rotation) {
    XWindowAttributes root;
    if (!api_XGetWindowAttributes(guard->display, guard->root, &root) ||
        root.width != (int)guard->width || root.height != (int)guard->height ||
        root.depth != guard->root_attributes.depth || root.visual != guard->root_attributes.visual ||
        !window_fullscreen(guard) || drain_x_events(guard) || !owned_surface_is_topmost(guard)) {
        return report("guard window no longer covers the display");
    }
    Pixmap next = None;
    if (make_pixmap(guard, rotation, &next)) { return -1; }
    api_XSetWindowBackgroundPixmap(guard->display, guard->window, next);
    api_XClearWindow(guard->display, guard->window);
    api_XSync(guard->display, False);
    if (x_error || stopping || !window_fullscreen(guard) || !owned_surface_is_topmost(guard)) {
        api_XFreePixmap(guard->display, next);
        return -1;
    }
    Pixmap previous = guard->pixmap;
    guard->pixmap = next;
    if (previous) { api_XFreePixmap(guard->display, previous); }
    return 0;
}
static int create_window(struct guard *guard) {
    if (guard->root_attributes.width != (int)guard->width ||
        guard->root_attributes.height != (int)guard->height || prepare_colors(guard)) {
        return report("native X viewport does not match the approved portrait framebuffer");
    }
    guard->gc = api_XCreateGC(guard->display, guard->root, 0, NULL);
    if (!guard->gc || make_pixmap(guard, guard->source_rotation, &guard->pixmap)) { return -1; }
    XSetWindowAttributes attributes = {0};
    attributes.background_pixmap = guard->pixmap;
    attributes.backing_store = WhenMapped;
    attributes.event_mask = ExposureMask | StructureNotifyMask | KeyPressMask | ButtonPressMask;
    guard->window = api_XCreateWindow(guard->display, guard->root, 0, 0, guard->width,
        guard->height, 0, guard->root_attributes.depth, InputOutput, guard->root_attributes.visual,
        CWBackPixmap | CWBackingStore | CWEventMask, &attributes);
    if (!guard->window) { return report("guard window creation failed"); }
    api_XStoreName(guard->display, guard->window, GUARD_TITLE);
    Atom name = api_XInternAtom(guard->display, "_NET_WM_NAME", False);
    Atom utf8 = api_XInternAtom(guard->display, "UTF8_STRING", False);
    api_XChangeProperty(guard->display, guard->window, name, utf8, 8, PropModeReplace,
        (const unsigned char *)GUARD_TITLE, (int)strlen(GUARD_TITLE));
    Atom pid_property = api_XInternAtom(guard->display, "_NET_WM_PID", False);
    unsigned long process_id = (unsigned long)getpid();
    api_XChangeProperty(guard->display, guard->window, pid_property, XA_CARDINAL, 32,
        PropModeReplace, (const unsigned char *)&process_id, 1);
    XClassHint class_hint = { .res_name = "weread-frame-guard", .res_class = "WeReadFrameGuard" };
    api_XSetClassHint(guard->display, guard->window, &class_hint);
    XWMHints hints = {0};
    hints.flags = InputHint | StateHint;
    hints.input = True;
    hints.initial_state = NormalState;
    api_XSetWMHints(guard->display, guard->window, &hints);
    api_XMapWindow(guard->display, guard->window);
    api_XSync(guard->display, False);
    struct timespec tick = { .tv_sec = 0, .tv_nsec = 20000000 };
    for (unsigned checks = 0; checks < 100 && !stopping; ++checks) {
        if (!owner_current(guard) || drain_x_events(guard)) { return -1; }
        if (window_fullscreen(guard) && owned_surface_is_topmost(guard)) { return 0; }
        nanosleep(&tick, NULL);
    }
    return report("native guard window was not confirmed fullscreen");
}
static int parse_request(struct guard *guard, char *record, struct request *request) {
    char *values[7], *cursor = record;
    for (size_t index = 0; index < 7; ++index) {
        values[index] = strsep(&cursor, ":");
        if (!values[index] || !*values[index]) { return -1; }
    }
    if (cursor || strcmp(values[2], guard->token)) { return -1; }
    uint64_t application, parent;
    if (decimal(values[0], MAX_SEQUENCE, &request->sequence) || !request->sequence ||
        request->sequence != guard->sequence + 1 ||
        decimal(values[1], MAX_SEQUENCE, &request->generation) || !request->generation ||
        decimal(values[3], INT_MAX, &application) || application < 2 ||
        decimal(values[4], UINT64_MAX, &request->birth) || !request->birth ||
        decimal(values[5], INT_MAX, &parent) || parent < 2) { return -1; }
    if (!strcmp(values[6], "capture")) { request->align = 0; }
    else if (!strcmp(values[6], "align")) { request->align = 1; }
    else { return -1; }
    request->application = (pid_t)application;
    request->parent = (pid_t)parent;
    return 0;
}
static int perform_request(struct guard *guard, char *record) {
    struct request request = {0};
    uint32_t rotation;
    if (parse_request(guard, record, &request) || !application_frozen(guard, &request)) {
        return report("frame request identity was not current and frozen");
    }
    if (read_frame(guard, !request.align, &rotation)) { return -1; }
    if (!request.align) { guard->source_rotation = rotation; }
    if (!application_frozen(guard, &request) || publish_snapshot(guard, rotation) ||
        !application_frozen(guard, &request) ||
        publish_ready(guard, request.sequence, request.generation)) { return -1; }
    guard->sequence = request.sequence;
    return 0;
}
static int command_loop(struct guard *guard) {
    char record[RECORD_BYTES];
    size_t used = 0;
    struct pollfd descriptors[2] = {
        { .fd = guard->commands, .events = POLLIN },
        { .fd = api_XConnectionNumber(guard->display), .events = POLLIN },
    };
    while (!stopping) {
        /* Expose is deliberately consumed without repainting an old App frame. */
        if (drain_x_events(guard)) { return -1; }
        if (poll(descriptors, 2, -1) < 0) {
            if (errno == EINTR) { continue; }
            return -1;
        }
        if ((descriptors[0].revents | descriptors[1].revents) & (POLLERR | POLLHUP | POLLNVAL)) {
            return report("frame guard channel closed");
        }
        if (descriptors[0].revents & POLLIN) {
            char bytes[RECORD_BYTES];
            ssize_t count = read(guard->commands, bytes, sizeof(bytes));
            if (count <= 0) { return -1; }
            for (ssize_t index = 0; index < count; ++index) {
                if (!bytes[index] || used + 1 >= sizeof(record)) { return report("frame request exceeded its limit"); }
                if (bytes[index] == '\n') {
                    record[used] = 0;
                    alarm(OPERATION_SECONDS);
                    int result = perform_request(guard, record);
                    alarm(0);
                    if (result) { return result; }
                    used = 0;
                } else { record[used++] = bytes[index]; }
            }
        }
    }
    return 0;
}
static void notify_process_exit(struct guard *guard, pid_t helper, uint64_t birth, int status) {
    if (!owner_current(guard)) { return; }
    int descriptor = openat(guard->context, "power-events", O_WRONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
    if (descriptor < 0) { return; }
    struct stat info;
    if (!fstat(descriptor, &info) && S_ISFIFO(info.st_mode) && info.st_uid == getuid()) {
        char record[RECORD_BYTES];
        int bytes = snprintf(record, sizeof(record), "_weread_frame_guard_exit %s %ld %" PRIu64 " %d\n",
            guard->token, (long)helper, birth, status);
        /* One bounded PIPE_BUF record, never a blocking event-channel write. */
        (void)write(descriptor, record, (size_t)bytes);
    }
    close(descriptor);
}
static int watch_lifetime(int argc, char **argv) {
    struct guard guard = { .context = -1, .commands = -1 };
    uint64_t owner, helper, helper_birth, descriptor;
    if (argc != 15 || strcmp(argv[1], "--context") || strcmp(argv[3], "--token") ||
        strcmp(argv[5], "--owner") || strcmp(argv[7], "--owner-birth") ||
        strcmp(argv[9], "--watch-helper") || strcmp(argv[11], "--helper-birth") ||
        strcmp(argv[13], "--lifetime-fd") || !token_valid(argv[4]) ||
        decimal(argv[6], INT_MAX, &owner) || owner < 2 ||
        decimal(argv[8], UINT64_MAX, &guard.owner_birth) || !guard.owner_birth ||
        decimal(argv[10], INT_MAX, &helper) || helper < 2 ||
        decimal(argv[12], UINT64_MAX, &helper_birth) || !helper_birth ||
        decimal(argv[14], INT_MAX, &descriptor)) { return 1; }
    guard.owner = (pid_t)owner;
    memcpy(guard.token, argv[4], sizeof(guard.token));
    guard.context = open(argv[2], O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    struct stat directory, channel;
    if (guard.context < 0 || fstat(guard.context, &directory) || !S_ISDIR(directory.st_mode) ||
        directory.st_uid != getuid() || (directory.st_mode & 0077) ||
        fstat((int)descriptor, &channel) || !S_ISFIFO(channel.st_mode) ||
        channel.st_uid != getuid() ||
        (fcntl((int)descriptor, F_GETFL) & O_ACCMODE) != O_RDONLY || protect_owner(&guard)) {
        return 1;
    }
    /* No polling, X connection, or frame buffer is needed to observe EOF. */
    unsigned char discarded[128];
    int status = 0;
    while (!stopping) {
        ssize_t bytes = read((int)descriptor, discarded, sizeof(discarded));
        if (bytes < 0 && errno == EINTR) { continue; }
        if (bytes < 0) { status = 1; break; }
        if (!owner_current(&guard)) { status = 1; break; }
        if (!bytes) {
            notify_process_exit(&guard, (pid_t)helper, helper_birth, 1);
            break;
        }
    }
    close(guard.context);
    return status;
}
static void cleanup_guard(struct guard *guard) {
    alarm(OPERATION_SECONDS);
    if (guard->display) {
        if (guard->window) { api_XDestroyWindow(guard->display, guard->window); }
        if (guard->pixmap) { api_XFreePixmap(guard->display, guard->pixmap); }
        if (guard->gc) { api_XFreeGC(guard->display, guard->gc); }
        api_XSync(guard->display, False);
        api_XCloseDisplay(guard->display);
    }
    if (guard->snapshot) {
        clear_bytes(guard->snapshot, (size_t)guard->width * guard->height);
        free(guard->snapshot);
    }
    if (guard->library) { dlclose(guard->library); }
    alarm(0);
}
static int probe_x11(void) {
    struct guard guard = { .context = -1, .commands = -1 };
    alarm(OPERATION_SECONDS);
    if (load_xlib(&guard)) { return 1; }
    XImage *image = api_XCreateImage(guard.display, guard.root_attributes.visual,
        (unsigned)guard.root_attributes.depth, ZPixmap, 0, NULL,
        (unsigned)guard.root_attributes.width, 1, 32, 0);
    if (!image) { cleanup_guard(&guard); return 1; }
    Visual *visual = guard.root_attributes.visual;
    int exact_gray = -1, identity_gray = -1;
    if (visual->class == StaticGray) {
        exact_gray = prepare_colors(&guard) == 0;
        identity_gray = exact_gray;
        for (unsigned value = 0; value < 256 && identity_gray; ++value) {
            if (guard.gray_pixels[value] != value) { identity_gray = 0; }
        }
    }
    printf("{\"width\":%d,\"height\":%d,\"depth\":%d,\"visual_class\":%d,"
        "\"red_mask\":%lu,\"green_mask\":%lu,\"blue_mask\":%lu,"
        "\"bits_per_pixel\":%d,\"bytes_per_line\":%d,\"byte_order\":%d,\"max_request_words\":%ld,"
        "\"gray_map_exact\":%d,\"gray_map_identity\":%d}\n",
        guard.root_attributes.width, guard.root_attributes.height, guard.root_attributes.depth,
        visual->class, visual->red_mask, visual->green_mask, visual->blue_mask,
        image->bits_per_pixel, image->bytes_per_line, image->byte_order, api_XMaxRequestSize(guard.display),
        exact_gray, identity_gray);
    XDestroyImage(image);
    cleanup_guard(&guard);
    return x_error ? 1 : 0;
}
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--probe-native-x11")) { return probe_x11(); }
    if (argc == 15) { return watch_lifetime(argc, argv); }
    struct guard guard = { .context = -1, .commands = -1 };
    uint64_t owner, commands;
    if (argc != 13 || strcmp(argv[1], "--context") || strcmp(argv[3], "--token") ||
        strcmp(argv[5], "--owner") || strcmp(argv[7], "--owner-birth") ||
        strcmp(argv[9], "--profile") || strcmp(argv[11], "--command-fd") ||
        !token_valid(argv[4]) || decimal(argv[6], INT_MAX, &owner) || owner < 2 ||
        decimal(argv[8], UINT64_MAX, &guard.owner_birth) || !guard.owner_birth ||
        decimal(argv[12], INT_MAX, &commands) || commands < 3) {
        return report("invalid launch arguments") ? 1 : 0;
    }
    if (!strcmp(argv[10], "scribe")) { guard.width = 1860; guard.height = 2480; }
    else if (!strcmp(argv[10], "pw6")) { guard.width = 1264; guard.height = 1680; }
    else { return report("unsupported approved profile") ? 1 : 0; }
    if ((uint64_t)guard.width * guard.height > MAX_PIXELS) { return 1; }
    guard.owner = (pid_t)owner;
    guard.commands = (int)commands;
    memcpy(guard.token, argv[4], sizeof(guard.token));
    guard.context = open(argv[2], O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    struct stat directory, channel;
    struct process_identity self;
    if (guard.context < 0 || fstat(guard.context, &directory) || !S_ISDIR(directory.st_mode) ||
        directory.st_uid != getuid() || (directory.st_mode & 0077) ||
        fstat(guard.commands, &channel) || !S_ISFIFO(channel.st_mode) ||
        channel.st_uid != getuid() || process_identity(getpid(), &self)) { return 1; }
    guard.self_birth = self.birth;
    if (protect_owner(&guard)) { return 1; }
    guard.snapshot = malloc((size_t)guard.width * guard.height);
    if (!guard.snapshot) { return 1; }
    /* Bootstrap contains no stock pixels. The App replaces it before handoff. */
    memset(guard.snapshot, 255, (size_t)guard.width * guard.height);
    alarm(OPERATION_SECONDS);
    int result = read_frame(&guard, 0, &guard.source_rotation) || load_xlib(&guard) ||
        create_window(&guard) || publish_owned_window(&guard) || publish_ready(&guard, 0, 0);
    alarm(0);
    if (!result) { result = command_loop(&guard); }
    notify_process_exit(&guard, getpid(), guard.self_birth, result ? 1 : 0);
    cleanup_guard(&guard);
    close(guard.context);
    return result ? 1 : 0;
}
