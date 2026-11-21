"""Build the read-only Scribe observer's reference from public startup artwork."""

import struct
import sys
from pathlib import Path


def reference(directory):
    def artwork(name):
        data = (directory / (name + ".gray")).read_bytes()
        width, height = struct.unpack_from("<II", data)
        if not (0 < width <= 2048 and 0 < height <= 256 and len(data) == 8 + width * height):
            raise ValueError("Invalid public startup image")
        return width, height, data[8:]

    title, starting = artwork("app-name"), artwork("starting")
    stage_height = max(artwork(name)[1] for name in ("fonts", "display", "library"))
    top = (2480 - (title[1] + 56 + starting[1] + 24 + stage_height + 76)) // 2
    points = []
    for label, (width, height, pixels) in enumerate((title, starting)):
        left = (1860 - width) // 2
        for row in range(0, height, 3):
            for col in range(0, width, 3):
                gray = pixels[row * width + col]
                if gray <= 64 or gray >= 245:
                    points.append((left + col, top + row, gray, 2 * label + int(gray >= 245)))
        top += height + 56
    if any(sum(item[3] == group for item in points) < 60 for group in range(4)):
        raise ValueError("Both labels must have enough independent ink and paper samples")
    return points


if __name__ == "__main__":
    for item in reference(Path(sys.argv[1])):
        print("{%d,%d,%d,%d}," % item)
