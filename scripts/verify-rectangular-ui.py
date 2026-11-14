#!/usr/bin/env python3
"""Check rectangular controls and concise Library rendering on test-env.

Inputs are actual QEMU scanout PNGs from isolated firmware guests. Earlier native
captures serve as negative controls, not golden images for the changed design.
The checks do not establish physical panel behavior or device compatibility.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

from PIL import Image


PROFILES = {
    'scribe': {'size': (1860, 2480), 'margin': 64, 'menu_height': 152, 'tabs': 6},
    'pw6': {'size': (1272, 1696), 'margin': 40, 'menu_height': 128, 'tabs': 5},
}


def image(directory: Path, profile: str, name: str) -> Image.Image:
    with Image.open(directory / profile / f'{name}.png') as source:
        if source.size != PROFILES[profile]['size']:
            raise ValueError(f'Unexpected {profile}/{name} capture geometry: {source.size}')
        return source.convert('L')


def pixels(source: Image.Image, box: tuple) -> list:
    return list(source.crop(box).getdata())


def corner_boxes(box: tuple, inset: int = 1, edge: int = 2) -> list:
    left, top, right, bottom = box
    return [(x, y, x + edge, y + edge)
            for x in (left + inset, right - inset - edge)
            for y in (top + inset, bottom - inset - edge)]


def find_switches(source: Image.Image, profile: str) -> list:
    right = source.width - PROFILES[profile]['margin'] - 16
    left = right - 96
    row_states = [source.getpixel((left + 1, y)) < 32
                  and source.getpixel((right - 2, y)) < 32
                  for y in range(180, min(800, source.height))]
    result, start = [], None
    for offset, active in enumerate([*row_states, False]):
        if active and start is None:
            start = offset
        elif not active and start is not None:
            length = offset - start
            if 52 <= length <= 60:
                result.append((left, 180 + start, right, 180 + offset))
            start = None
    return result


def switch_thumb(source: Image.Image, box: tuple, checked: bool) -> bool:
    left, top, right, bottom = box
    thumb_left = left + (48 if checked else 8)
    thumb = (thumb_left, top + 8, thumb_left + 40, top + 48)
    values = [value for corner in corner_boxes(thumb, inset=2)
              for value in pixels(source, corner)]
    # The opposite state's background can mimic thumb corners; check both roles.
    inner = (left + 4, top + 4, right - 4, bottom - 4)
    background = [value for corner in corner_boxes(inner)
                  for value in pixels(source, corner)]
    return (all(value >= 230 for value in values) and all(value < 32 for value in background)
            if checked else all(value <= 32 for value in values) and all(value >= 230 for value in background))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--captures', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    checks = []

    def record(name: str, passed: bool, **details):
        checks.append({'name': name, 'passed': bool(passed), **details})

    for profile, geometry in PROFILES.items():
        width, height = geometry['size']
        library = image(args.captures, profile, 'library')
        old_library = image(args.baseline, profile, 'library')
        header_box = (200, 166, 760 if profile == 'pw6' else 1150, 250)
        new_ink = sum(value < 100 for value in pixels(library, header_box))
        old_ink = sum(value < 100 for value in pixels(old_library, header_box))
        record(f'{profile}/library-summary-removed', new_ink < 20 and old_ink > 100,
               region=list(header_box), current_ink_pixels=new_ink, baseline_ink_pixels=old_ink)

        strip = (20, 362, width - 20, 374)
        current_mean = sum(pixels(library, strip)) / ((width - 40) * 12)
        previous_mean = sum(pixels(old_library, strip)) / ((width - 40) * 12)
        record(f'{profile}/duplicate-offline-banner-removed', current_mean >= 230 and previous_mean < 225,
               current_mean=current_mean, baseline_mean=previous_mean)

        search_frame = (geometry['margin'], 64, width - geometry['margin'], 152)
        current_corners = [value for box in corner_boxes(search_frame)
                           for value in pixels(library, box)]
        old_corners = [value for box in corner_boxes(search_frame)
                       for value in pixels(old_library, box)]
        record(f'{profile}/library-search-square-corners', all(value < 32 for value in current_corners),
               baseline_rejected=any(value > 230 for value in old_corners))
        record(f'{profile}/rounded-search-negative-control', any(value > 230 for value in old_corners))

        menu = image(args.captures, profile, 'reader-menu')
        old_menu = image(args.baseline, profile, 'reader-menu')
        tab_width = width / geometry['tabs']
        top = height - geometry['menu_height']
        active_tab = (math.ceil(tab_width + 12), top + 8,
                      math.floor(tab_width * 2 - 12), height - 8)
        corners = [value for box in corner_boxes(active_tab, inset=2)
                   for value in pixels(menu, box)]
        old_corners = [value for box in corner_boxes(active_tab, inset=2)
                       for value in pixels(old_menu, box)]
        record(f'{profile}/whole-reader-tab-is-rectangular', all(value < 32 for value in corners),
               region=list(active_tab))
        record(f'{profile}/icon-pill-negative-control', any(value > 230 for value in old_corners))
        label_top = top + (88 if profile == 'scribe' else 76)
        label_box = (round(tab_width * 1.5) - 65, label_top + 8,
                     round(tab_width * 1.5) + 65, height - 16)
        white_label = sum(value >= 230 for value in pixels(menu, label_box))
        record(f'{profile}/selected-tab-label-remains-visible', white_label > 100,
               white_label_pixels=white_label, region=list(label_box))

        storage = image(args.captures, profile, 'storage')
        off = image(args.captures, profile, 'storage-switch-off')
        restored = image(args.captures, profile, 'storage-switch-restored')
        switches = find_switches(storage, profile)
        record(f'{profile}/storage-has-rectangular-switch-tracks', bool(switches),
               regions=[list(box) for box in switches])
        if switches:
            box = switches[0]
            values = [value for corner in corner_boxes(box)
                      for value in pixels(storage, corner)]
            record(f'{profile}/switch-track-corners-are-square', all(value < 32 for value in values))
            record(f'{profile}/checked-thumb-is-a-square', switch_thumb(storage, box, True))
            record(f'{profile}/unchecked-thumb-is-a-square', switch_thumb(off, box, False))
            record(f'{profile}/switch-restored-after-taps', pixels(storage, box) == pixels(restored, box))

        before_close = image(args.captures, profile, 'reader-menu')
        after_close = image(args.captures, profile, 'reader-menu-after-toc')
        body = (0, 160, width, height)
        record(f'{profile}/contents-close-restores-reader-menu',
               pixels(before_close, body) == pixels(after_close, body))

    report = {'passed': all(item['passed'] for item in checks), 'checks': checks,
              'scope': 'Native rectangular-control pixels and QMP interaction results',
              'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'physical_device_acceptance': False}
    args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'passed': report['passed'], 'checks': len(checks),
                      'failures': [item for item in checks if not item['passed']]}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
