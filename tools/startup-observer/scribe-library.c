/* Research only: recognize the ordinary Library chrome without exporting pixels. */
#define _GNU_SOURCE
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fb.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

enum { WIDTH = 1860, HEIGHT = 2480, STRIDE = 1872 };

/* Bit i is the reference ink/paper at x + 2*i. Ignore antialiased edges.
 * Source: startup-performance-20261117-3423e73/candidate-library.png.
 * Six rows of the fixed Library heading; three of the selected navigation icon. */
static const struct { unsigned x, y; uint32_t ink, paper; } rows[] = {
    {76, 201, 0x3000ffff, 0xc7f20000}, {124, 195, 0x01e1ce70, 0xfe00318f},
    {124, 199, 0x00e1ce30, 0xff1e3187}, {67, 215, 0x00fffffe, 0xee000000},
    {111, 221, 0x7fffffc3, 0x00000038}, {124, 229, 0x000739c0, 0xffe0c01f},
    {196, 2364, 0xfc0000ff, 0x03ffff00}, {197, 2368, 0xfe7e7e7f, 0x01818180},
    {207, 2392, 0xfff00003, 0x000ffffc},
};

static int matches(const volatile uint8_t *pixels, size_t stride) {
    for (size_t r = 0; r < sizeof rows / sizeof *rows; r++) {
        for (unsigned i = 0; i < 32; i++) {
            uint32_t bit = UINT32_C(1) << i;
            if (!((rows[r].ink | rows[r].paper) & bit)) continue;
            uint8_t value = pixels[rows[r].y * stride + rows[r].x + i * 2];
            if (((rows[r].ink & bit) && value > 100) ||
                ((rows[r].paper & bit) && value < 155)) return 0;
        }
    }
    return 1;
}

#ifdef PUBLIC_FRAME_REPLAY
/* Host build only. Input: one public 1860x2480 Gray8 fixture on stdin. */
int main(void) {
    static uint8_t pixels[WIDTH * HEIGHT];
    if (fread(pixels, 1, sizeof pixels, stdin) != sizeof pixels ||
        fgetc(stdin) != EOF || ferror(stdin)) return 2;
    puts(matches(pixels, WIDTH) ? "MATCH" : "NO_MATCH");
    return 0;
}
#else
static int supported(int fd) {
    struct fb_fix_screeninfo f;
    struct fb_var_screeninfo v;
    return ioctl(fd, FBIOGET_FSCREENINFO, &f) == 0 && ioctl(fd, FBIOGET_VSCREENINFO, &v) == 0 &&
        v.xres == WIDTH && v.yres == HEIGHT && v.xres_virtual == STRIDE &&
        v.yres_virtual == 2 * HEIGHT && !v.xoffset && !v.yoffset && v.rotate == 3 &&
        (v.grayscale == 1 || v.grayscale == 2) && v.bits_per_pixel == 8 &&
        f.line_length == STRIDE && f.smem_len >= STRIDE * HEIGHT;
}

static int64_t boottime_us(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_BOOTTIME, &t) != 0) { perror("clock_gettime"); exit(2); }
    return (int64_t)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}

static int watch(int fd, const volatile uint8_t *map) {
    int64_t start = boottime_us(), last_negative = -1, first_positive = -1;
    const struct timespec pause = { .tv_nsec = 50000000 };
    while (boottime_us() - start < 45000000) {
        if (!supported(fd)) { fputs("Unsupported framebuffer\n", stderr); return 2; }
        int64_t before = boottime_us();
        int match = matches(map, STRIDE);
        int64_t after = boottime_us();
        if (!supported(fd)) { fputs("Framebuffer changed\n", stderr); return 2; }
        if (match) {
            if (last_negative < 0) { fputs("Already matches at arm\n", stderr); return 2; }
            if (first_positive >= 0) {
                printf("LIBRARY_CHROME boottime_us last_negative_begin=%" PRId64
                       " first_positive_end=%" PRId64 " confirmed_end=%" PRId64 "\n",
                       last_negative, first_positive, after);
                return 0;
            }
            first_positive = after;
        } else {
            if (last_negative < 0) { puts("ARMED"); fflush(stdout); }
            last_negative = before;
            first_positive = -1;
        }
        if (nanosleep(&pause, NULL) != 0) return 2;
    }
    fputs("No confirmed Library chrome before timeout\n", stderr);
    return 1;
}

int main(void) {
    int fd = open("/dev/fb0", O_RDONLY | O_CLOEXEC);
    if (fd < 0) { perror("open framebuffer"); return 2; }
    int result = 2;
    if (supported(fd)) {
        void *map = mmap(NULL, STRIDE * HEIGHT, PROT_READ, MAP_SHARED, fd, 0);
        if (map == MAP_FAILED) perror("mmap");
        else { result = watch(fd, map); munmap(map, STRIDE * HEIGHT); }
    } else { fputs("Unsupported framebuffer\n", stderr); }
    close(fd);
    return result;
}
#endif
