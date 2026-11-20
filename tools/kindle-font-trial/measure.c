#define _POSIX_C_SOURCE 200809L
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fb.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

enum { WIDTH = 1860, HEIGHT = 2480, STRIDE = 1872, COLS = 48, ROWS = 64 };

static int geometry(int descriptor, struct fb_var_screeninfo *info) {
    struct fb_fix_screeninfo fixed;
    return ioctl(descriptor, FBIOGET_FSCREENINFO, &fixed) == 0 &&
        ioctl(descriptor, FBIOGET_VSCREENINFO, info) == 0 &&
        info->xres == WIDTH && info->yres == HEIGHT && info->xoffset == 0 &&
        (info->xres_virtual == WIDTH || info->xres_virtual == STRIDE) &&
        (info->yres_virtual == HEIGHT || info->yres_virtual == 2 * HEIGHT) &&
        (info->yoffset == 0 || info->yoffset == HEIGHT) &&
        info->yoffset + HEIGHT <= info->yres_virtual && (info->rotate == 1 || info->rotate == 3) &&
        info->bits_per_pixel == 8 && (info->grayscale == 1 || info->grayscale == 2) &&
        fixed.line_length == STRIDE && fixed.smem_len >= STRIDE * (info->yoffset + HEIGHT);
}

static int capture(int thumbnail) {
    int descriptor = open("/dev/fb0", O_RDONLY | O_CLOEXEC);
    struct fb_var_screeninfo before, after;
    if (descriptor < 0 || !geometry(descriptor, &before)) return 2;
    size_t length = STRIDE * (HEIGHT + before.yoffset);
    const uint8_t *map = mmap(NULL, length, PROT_READ, MAP_SHARED, descriptor, 0);
    if (map == MAP_FAILED) return 2;
    const uint8_t *pixels = map + STRIDE * before.yoffset;
    printf("P5\n%d %d\n255\n", thumbnail ? COLS : WIDTH, thumbnail ? ROWS : HEIGHT);
    if (thumbnail) {
        for (size_t row = 0; row < ROWS; row++) {
            size_t top = row * HEIGHT / ROWS, bottom = (row + 1) * HEIGHT / ROWS;
            for (size_t col = 0; col < COLS; col++) {
                size_t left = col * WIDTH / COLS, right = (col + 1) * WIDTH / COLS;
                unsigned sum = 0, count = 0;
                for (size_t scan = top; scan < bottom; scan += 2)
                    for (size_t pos = left; pos < right; pos += 2) {
                        sum += pixels[scan * STRIDE + pos];
                        count++;
                    }
                if (putchar((int)(sum / count)) == EOF) return 2;
            }
        }
    } else {
        for (size_t row = 0; row < HEIGHT; row++)
            if (fwrite(pixels + row * STRIDE, 1, WIDTH, stdout) != WIDTH) return 2;
    }
    int ok = !ferror(stdout) && fflush(stdout) == 0 && geometry(descriptor, &after) &&
        before.rotate == after.rotate && before.yoffset == after.yoffset && before.grayscale == after.grayscale;
    munmap((void *)map, length);
    close(descriptor);
    return ok ? 0 : 2;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "thumb") == 0) return capture(1);
    if (argc == 2 && strcmp(argv[1], "panel") == 0) return capture(0);
    if (argc != 3 || strcmp(argv[1], "request") != 0 ||
        strlen(argv[2]) != 16 || strspn(argv[2], "0123456789abcdef") != 16) return 64;
    char uri[128];
    snprintf(uri, sizeof(uri), "file:///mnt/us/documents/weread-font-trial-%s.sh", argv[2]);
    struct timespec time;
    if (clock_gettime(CLOCK_MONOTONIC, &time) != 0 ||
        printf("REQUEST %jd.%09ld\n", (intmax_t)time.tv_sec, time.tv_nsec) < 0 || fflush(stdout) != 0) return 2;
    execlp("lipc-set-prop", "lipc-set-prop", "com.lab126.appmgrd", "start", uri, (char *)NULL);
    return 2;
}
