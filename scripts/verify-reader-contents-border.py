#!/usr/bin/env python3
"""Check Scribe contents-frame continuity in remote-generated screenshots.

Run on ssh test-env. Inputs may be logical desktop snapshots or native QEMU
scanout PNGs. A passing result is a software rendering observation, not physical
panel acceptance. The optional negative control must retain the original gaps.
"""

import argparse
import json
from pathlib import Path

from PIL import Image


def inspect(path: Path, *, inverted: bool = False) -> dict:
    with Image.open(path) as source:
        image = source.convert('L')
    if image.size not in {(930, 1240), (1860, 2480)}:
        raise ValueError(f'Unsupported Scribe capture geometry: {image.size}')
    scale = image.width // 930
    panel_width, border = 540 * scale, 2 * scale
    target = 255 if inverted else 0
    strip = image.crop((panel_width - border, 0, panel_width, image.height))
    pixels = strip.load()
    missing_rows = [y for y in range(strip.height)
                    if any(abs(pixels[x, y] - target) > 16 for x in range(strip.width))]
    longest_gap = current_gap = 0
    missing = set(missing_rows)
    for y in range(strip.height):
        current_gap = current_gap + 1 if y in missing else 0
        longest_gap = max(longest_gap, current_gap)
    return {'path': str(path), 'size': list(image.size), 'scale': scale,
            'right_edge': [panel_width - border, 0, panel_width, image.height],
            'expected_tone': target, 'tolerance': 16, 'missing_rows': len(missing_rows),
            'longest_gap': longest_gap, 'continuous': not missing_rows}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', nargs='+', type=Path)
    parser.add_argument('--inverted', action='store_true')
    parser.add_argument('--negative-control', type=Path)
    parser.add_argument('--closed-control', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    results = [inspect(path, inverted=args.inverted) for path in args.captures]
    negative = inspect(args.negative_control) if args.negative_control else None
    closed = inspect(args.closed_control) if args.closed_control else None
    passed = all(result['continuous'] for result in results)
    if negative is not None:
        passed = passed and not negative['continuous'] and negative['longest_gap'] >= 48
    if closed is not None:
        passed = passed and not closed['continuous'] and closed['missing_rows'] > closed['size'][1] // 2
    report = {'passed': passed, 'scope': 'Scribe contents right-frame continuity',
              'captures': results, 'negative_control': negative, 'closed_control': closed,
              'physical_device_acceptance': False}
    encoded = json.dumps(report, indent=2) + '\n'
    if args.output:
        args.output.write_text(encoded, encoding='utf-8')
    print(encoded, end='')
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
