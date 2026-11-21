#define _GNU_SOURCE
#include "../startup-observer/render-scene.h"
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fb.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

static int geometry(int descriptor, struct fb_var_screeninfo *info) {
    struct fb_fix_screeninfo fixed;
    return !ioctl(descriptor, FBIOGET_FSCREENINFO, &fixed) && !ioctl(descriptor, FBIOGET_VSCREENINFO, info) &&
        info->xres == R_WIDTH && info->yres == R_HEIGHT && info->bits_per_pixel == 8 && !info->xoffset &&
        (info->xres_virtual == R_WIDTH || info->xres_virtual == R_STRIDE) &&
        (info->yres_virtual == R_HEIGHT || info->yres_virtual == 2 * R_HEIGHT) &&
        (info->yoffset == 0 || info->yoffset == R_HEIGHT) && info->yoffset + R_HEIGHT <= info->yres_virtual &&
        (info->rotate == 1 || info->rotate == 3) && (info->grayscale == 1 || info->grayscale == 2) &&
        fixed.line_length == R_STRIDE && fixed.smem_len >= R_STRIDE * (info->yoffset + R_HEIGHT);
}

static uint64_t monotonic(void) {
    struct timespec time;
    if (clock_gettime(CLOCK_MONOTONIC, &time)) return 0;
    return (uint64_t)time.tv_sec * UINT64_C(1000000000) + (unsigned long)time.tv_nsec;
}

int main(void) {
    int descriptor = open("/dev/fb0", O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    struct fb_var_screeninfo before, after;
    uint64_t begin = monotonic();
    if (descriptor < 0 || !begin || !geometry(descriptor, &before)) return 1;
    size_t size = (size_t)R_STRIDE * (R_HEIGHT + before.yoffset);
    const uint8_t *page = mmap(NULL, size, PROT_READ, MAP_SHARED, descriptor, 0);
    if (page == MAP_FAILED) return 1;
    struct observed_scene current = scene(page + (size_t)R_STRIDE * before.yoffset, R_STRIDE);
    int unchanged = geometry(descriptor, &after) && before.rotate == after.rotate && before.grayscale == after.grayscale &&
        before.yoffset == after.yoffset && before.xres_virtual == after.xres_virtual && before.yres_virtual == after.yres_virtual;
    uint64_t end = monotonic();
    munmap((void *)page, size);
    close(descriptor);
    if (!unchanged || end < begin) return 1;
    printf("SHELF %" PRIu64 " %" PRIu64 " %u %s\n", begin, end, before.rotate, scene_name(current.kind));
    return fflush(stdout) == 0 ? 0 : 1;
}
