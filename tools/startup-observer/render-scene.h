#ifndef WEREAD_RENDER_SCENE_H
#define WEREAD_RENDER_SCENE_H

#include <stddef.h>
#include <stdint.h>

enum { R_WIDTH = 1860, R_HEIGHT = 2480, R_STRIDE = 1872 };
enum scene_kind { NO_SCENE, GRID_SCENE, LIST_SCENE, EMPTY_SCENE };

struct surface { const volatile uint8_t *base; unsigned stride; };
struct observed_scene { enum scene_kind kind; };

static unsigned tone(struct surface surface, unsigned column, unsigned row) {
    return surface.base[row * surface.stride + column];
}

static int chrome(struct surface surface) {
    static const struct { unsigned column, row; uint32_t ink, paper; } rows[] = {
        {76,201,0x3000ffff,0xc7f20000}, {124,195,0x01e1ce70,0xfe00318f},
        {124,199,0x00e1ce30,0xff1e3187}, {67,215,0x00fffffe,0xee000000},
        {111,221,0x7fffffc3,0x00000038}, {124,229,0x000739c0,0xffe0c01f},
        {196,2364,0xfc0000ff,0x03ffff00}, {197,2368,0xfe7e7e7f,0x01818180},
        {207,2392,0xfff00003,0x000ffffc}
    };
    for (size_t row = 0; row < sizeof rows / sizeof *rows; ++row) {
        for (unsigned index = 0; index < 32; ++index) {
            uint32_t bit = UINT32_C(1) << index;
            if (!((rows[row].ink | rows[row].paper) & bit)) continue;
            unsigned value = tone(surface, rows[row].column + index * 2, rows[row].row);
            if (((rows[row].ink & bit) && value > 100) || ((rows[row].paper & bit) && value < 155)) return 0;
        }
    }
    for (unsigned column = 56; column <= 148; column += 8) {
        if (tone(surface,column,355) > 85 || tone(surface,column,351) < 198) return 0;
    }
    for (unsigned column = 183; column < 640; column += 47) if (tone(surface,column,355) < 198) return 0;
    return 1;
}

static int marks(struct surface surface, unsigned left, unsigned top, unsigned width, unsigned height) {
    unsigned hits = 0, lines = 0;
    for (unsigned row = 0; row < height; row += 3) {
        unsigned count = 0;
        for (unsigned col = 0; col < width; col += 3) count += tone(surface,left+col,top+row) <= 120;
        hits += count;
        lines += count >= 2;
    }
    return hits >= 8 && lines >= 3;
}

static int bounded_cover(struct surface surface, unsigned left, unsigned top, unsigned width, unsigned height) {
    unsigned bands = 0;
    for (unsigned dy = 0; dy < height; dy += height / 7) {
        unsigned row = top + dy, paper = tone(surface,left-12,row);
        if (paper < 198 || tone(surface,left+width+10,row) + 10 < paper) continue;
        unsigned count = 0;
        for (unsigned dx = 0; dx < width; dx += width / 13) {
            count += tone(surface,left+dx,row) + 20 < paper;
        }
        bands += count >= 2;
    }
    return bands >= 3;
}

static int first_grid_item(struct surface surface, unsigned content) {
    unsigned top = content + 44;
    unsigned extent = 0;
    unsigned upper_paper = tone(surface,40,top-5), lower_paper = tone(surface,40,top+425);
    if (upper_paper < 198 || lower_paper < 198) return 0;
    for (unsigned column = 70; column < 355; column += 25) {
        unsigned upper = tone(surface,column,top-5), lower = tone(surface,column,top+425);
        if (upper+8 < upper_paper || upper > upper_paper+8 || lower+8 < lower_paper || lower > lower_paper+8) return 0;
    }
    for (unsigned row = top + 15; row < top + 415; row += 33) {
        unsigned left=0, right=0, paper=tone(surface,52,row);
        for (unsigned distance=7;distance<58;distance+=8) {
            left += tone(surface,64+distance,row)+20 < paper;
            right += tone(surface,366-distance,row)+20 < paper;
        }
        extent += left >= 2 && right >= 2;
    }
    return extent >= 3 && bounded_cover(surface,64,top+8,302,400) && marks(surface,64,top+436,300,56);
}

static int first_list_item(struct surface surface, unsigned content) {
    for (unsigned column = 440; column < 1600; column += 111) {
        unsigned rule = tone(surface,column,content+142);
        if (rule < 128 || rule > 195 || tone(surface,column,content+140) < 198) return 0;
    }
    return bounded_cover(surface, 64, content + 18, 88, 100) && marks(surface, 180, content + 16, 630, 67);
}

static int empty_controls(struct surface surface, unsigned top) {
    for (unsigned row = top + 62; row < top + 137; ++row) {
        unsigned matches = 0;
        for (unsigned column = 695; column <= 865; column += 34) {
            matches += tone(surface,column,row-3) > 198 && tone(surface,column,row) < 70 &&
                       tone(surface,column,row+9) < 70 && tone(surface,column,row+93) < 70;
        }
        for (unsigned column = 984; column <= 1164; column += 36) {
            matches += tone(surface,column,row-3) > 198 && tone(surface,column,row) < 70 &&
                       tone(surface,column,row+9) > 198 && tone(surface,column,row+93) < 70;
        }
        if (matches == 12 && tone(surface,927,row+12) > 198) return 1;
    }
    return 0;
}

static int empty_message(struct surface surface) {
    static const uint16_t ink[][2] = {{20,4},{20,15},{20,26},{20,37},{59,4},{68,13},{89,28},{72,41},
        {137,5},{105,18},{105,29},{116,41},{155,5},{155,16},{156,29},{167,36},
        {215,4},{198,12},{215,28},{232,41},{252,4},{259,12},{270,25},{257,34}};
    static const uint16_t paper[][2] = {{3,4},{14,15},{26,27},{44,39},{51,4},{63,15},{82,32},{76,41},
        {109,4},{110,15},{137,30},{106,41},{147,4},{184,15},{162,31},{157,41},
        {195,4},{202,14},{195,27},{211,40},{243,4},{250,13},{250,27},{250,41}};
    for (unsigned left = 782; left <= 790; ++left) for (unsigned top = 1100; top <= 1390; ++top) {
        if (tone(surface,left+20,top+4) > 100 || tone(surface,left+59,top+4) > 100) continue;
        unsigned dark = 0, light = 0;
        for (unsigned index = 0; index < sizeof ink / sizeof *ink; ++index) {
            dark += tone(surface,left+ink[index][0],top+ink[index][1]) < 100;
            light += tone(surface,left+paper[index][0],top+paper[index][1]) > 184;
        }
        if (dark >= 22 && light >= 22 && empty_controls(surface,top)) return 1;
    }
    return 0;
}

static struct observed_scene scene(const volatile uint8_t *base, unsigned stride) {
    struct surface surface = {base,stride};
    if (!chrome(surface)) return (struct observed_scene){0};
    static const unsigned starts[] = {360,424,432,456};
    int grid = 0, list = 0;
    for (unsigned index = 0; index < sizeof starts / sizeof *starts; ++index) {
        grid |= first_grid_item(surface,starts[index]);
        list |= first_list_item(surface,starts[index]);
    }
    int empty = !grid && !list && empty_message(surface);
    if (grid + list + empty != 1) return (struct observed_scene){0};
    return (struct observed_scene){grid ? GRID_SCENE : list ? LIST_SCENE : EMPTY_SCENE};
}

static const char *scene_name(enum scene_kind kind) {
    static const char *const names[] = {"none", "grid", "list", "empty"};
    return names[kind];
}

#endif
