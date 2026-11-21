"""Prepare the WeRead selector from the pinned public native X11/OCR research source."""

import argparse
from pathlib import Path
import random
from PIL import Image, ImageFilter


REVISION = "9175002958174ff236e0f4c33072dc278189e88e"
SOURCE_PATH = "docs/physical/startup-optimization-20261119/resume/rebuilt/native-target.c"
HERE = Path(__file__).resolve().parent


def reference(artwork):
    image = Image.open(artwork).convert("L")
    if image.size != (256, 384):
        raise ValueError("Expected the approved 256 by 384 WeRead thumbnail")
    high = image.filter(ImageFilter.MaxFilter(5))
    low = image.filter(ImageFilter.MinFilter(5))
    rows = []
    rng = random.Random(1601)
    for top, bottom in [(48, 96), (124, 191), (192, 257), (258, 324)]:
        band = []
        for ink in (True, False):
            candidates = [(column, row) for row in range(top, bottom, 2) for column in range(20, 237, 2)
                          if (high.getpixel((column, row)) < 85 if ink else low.getpixel((column, row)) > 205)]
            rng.shuffle(candidates)
            chosen = []
            for column, row in candidates:
                if all((column - old_x)**2 + (row - old_y)**2 >= 36 for old_x, old_y in chosen):
                    chosen.append((column, row))
                    if len(chosen) == 16:
                        break
            if len(chosen) != 16:
                raise ValueError("Unexpected public artwork contrast")
            band.extend(chosen)
        rows.append("{" + ",".join("{%d,%d}" % pair for pair in band) + "}")
    return "static const unsigned short tile_points[4][32][2] = {\n" + ",\n".join(rows) + "\n};\n"


def once(source, old, new):
    if source.count(old) != 1:
        raise ValueError("Review the pinned native selector boundary before building")
    return source.replace(old, new, 1)


def prepare(original, artwork, directory):
    source = original.read_text(encoding="ascii")
    first, last = 'static int aligned(', 'static int recognize('
    source = once(source, source[source.index(first):source.index(last)], '#include "native_tile_artwork.c"\n\n')
    first, last = '        } else if (!navigation', '    } while (TessResultIteratorNext'
    source = once(source, source[source.index(first):source.index(last)], '        }\n')
    source = once(source, '    int prefix = 0;\n    struct bounds previous = {0};\n', '')
    source = once(source, 'if (!valid) { prefix = 0; continue; }', 'if (!valid) continue;')
    source = once(source, 'int top = navigation ? footer : 0;', 'int top = footer;')
    source = once(source, 'if (navigation) TessBaseAPISetRectangle', 'TessBaseAPISetRectangle')
    source = once(source, '    TessResultIteratorDelete(iterator);', '    TessResultIteratorDelete(iterator);\n    if (!navigation) result->title = public_tile(frame);')
    source = once(source, 'attr.width < 300 || attr.height < 500 ||\n        attr.width > 3000 || attr.height > 3200', 'attr.width != 1860 || attr.height != 2480')
    source = once(source, 'int main(int argc, char **argv) {', 'int main(int argc, char **argv) {\n    if (!tile_private_process()) return fail("private capture unavailable");')
    source = once(source, '    free(current.gray);', '''    if (stable && open_card) {
        struct match again = public_tile(&current);
        stable = again.count == 1 && again.box.left == target->box.left && again.box.top == target->box.top &&
            again.box.right == target->box.right && again.box.bottom == target->box.bottom;
    }
    free(current.gray);''')
    source = once(source, '    uint64_t down_before = now_ns();', '''    if (open_card && !tile_commit_press_intent()) {
        sigprocmask(SIG_SETMASK, &previous, NULL);
        return 0;
    }
    uint64_t down_before = now_ns();''')
    if 'bilicomicsforkindle' in source or 'TessBaseAPISetRectangle(ocr, 0, top' not in source:
        raise ValueError("Unexpected OCR scope")
    (directory / "native-target.c").write_text(source, encoding="ascii")
    (directory / "native_tile_reference.h").write_text(reference(artwork), encoding="ascii")
    if HERE != directory.resolve():
        (directory / "native_tile_artwork.c").write_bytes((HERE / "native_tile_artwork.c").read_bytes())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("original", "artwork", "directory"):
        parser.add_argument(key, type=Path)
    args = parser.parse_args()
    prepare(args.original, args.artwork, args.directory)
