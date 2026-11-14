# Native UI capture review and contents-frame correction

Recorded: 2026-11-14 (Asia/Shanghai).

The operator requested actual Scribe and Paperwhite UI captures through
`ssh test-env`, followed by a correction for the interrupted Scribe contents
frame shown in a photograph. The resulting review uses the current Rust/Slint
application in two disposable firmware guests and retains their QEMU scanout
PNG files unchanged. Physical installation and acceptance of this candidate
remain open.

The subsequent [rectangular UI implementation](RECTANGULAR_UI.md) applies the
later shape agreement and its broader follow-up. It has a separate application,
capture set, and verification record; this earlier package and its PNGs do not
represent the new rectangular styling.

## Implemented corrections

`ReaderContents` previously painted its parent frame before its full-width tab,
chapter-row, and pager backgrounds. Those opaque children covered the frame at
the same locations as the photographed gaps. The panel now clips its children
and paints the existing 2px frame after them. Its dimensions, opaque paper
background, application events, and Reader layout remain the current design.

The native frame guard also needed the already recorded PW6 experimental
viewport. Its normal PW6 geometry remains 1264 x 1680. Only the existing
`pw6-5201-emulator` name, matching approved `pw6` profile, and exact firmware
`5.20.1` select 1272 x 1696. An absent or mismatched approved profile, missing
firmware, unknown experiment, wrong experimental pair, or incorrect actual
framebuffer geometry is rejected. This does not extend the physical support
matrix or introduce arbitrary geometry overrides.

## Recorded environments

| Guest | Firmware | Runtime logical viewport | Visible native capture |
| --- | --- | --- | --- |
| Scribe first generation, `kindle-scribe-1`, DVT profile | 5.20.1 | 930 x 1240 | 1860 x 2480 |
| Paperwhite 6, `kindle-paperwhite-6`, production profile | 5.20.1 | 636 x 848 | 1272 x 1696 |

Both application metadata records report `backend=kindle` and scale 2. The PW6
design reference remains 632 x 840; it is not substituted for the measured
firmware viewport. Emulator revision is
`c96062d51a1a54dd445d1668e740786d160a80bb`, with QEMU submodule
`b75b93cdc82d60d8c3f844dd114fd38dfb481df5`. Both source checkouts were clean when
their identities were read. Guest disks use independent copy-on-write overlays;
the existing instances and their data were preserved.

## Artifact identities

The source base is Git commit `6142bc168561c612367d78c145835c742a76072a` plus
the Reader frame correction and the independently compiled native guard fix.
The ARM application was built from 563 verified source inputs with only
`ui/reader.slint` changed. The subsequent C helper has its own source and build
receipt; it does not require another Rust compilation.

| Artifact | SHA-256 |
| --- | --- |
| ARM application | `2a97beaf69b1097a992447a693b2a7d9f2161d256b0551499b054ec28a0558df` |
| Native frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Development ZIP | `9011f2abd67c3e225fd58a6aa12ea041ab3f9744e373c508cc15502cb62a89af` |
| Guard C source | `66561073580d4c57662bab70558fec07ded47cb82d7fa0c22b5f32ad234328aa` |

Remote work is under `/root/weread-ui-review-20261114-r1/`. The operator-local
generated evidence is under `artifacts/ui-review-20261113/`; this folder name
reflects the conversation's start date. Generated images, packages, and runtime
evidence are ignored by Git.

## Verification facts

All compilation, application execution, QMP interaction, and executable checks
ran on `test-env`. Source editing and generated-image inspection on Windows
were separate from application verification.

- The static ARM application and independent ARM guard compiled successfully.
  Public package member CRCs and both executable identities were checked.
- The first packaging attempt stopped at an omitted `THIRD_PARTY_NOTICES.md`
  after Rust compilation passed. The notice was copied from the verified
  public baseline, the successful retry was recorded separately, and the
  earlier failure log was retained.
- The original guard failed the new correct PW6 experimental-geometry case.
  The corrected helper passed all 36 remote fixture tests: 22 normal PW6,
  eight named-emulator cases, and six Scribe cases, without skips.
- Both firmware guests ran the exact application and guard above. QMP taps
  exercised the emulated native input path. Twenty-eight selected scanout
  captures cover fourteen states on each device: covers, list, details,
  search/keyboard, Reader, Reader menu, contents, typography, Settings,
  downloads/storage, bookmarks, notes, publications, and statistics.
- The pre-fix Scribe QMP image and application snapshot both missed 404 rows
  of the right frame, with a longest gap of 210 pixels. Contents, bookmarks,
  notes, reopened contents, and the final application snapshot now have zero
  missing rows across `x=[1076,1080)` and all 2480 native rows.
- Closing contents restored the previous Reader-menu PNG bytes exactly on
  both devices. Selecting another chapter changed the visible body. PW6
  retained its full-screen contents layout without the Scribe side frame.
- Final metadata confirmed both runtime viewports. Application exit restored
  each guest's status bar and unstopped window manager. Both task-owned QEMU
  processes were then closed.
- The actual conversation fragment passed 20 remote JavaScript syntax and DOM
  interaction checks using Node 20.19.2 and jsdom 26.1.0 in a task-owned tools
  directory. Default contents, page switching, device switching, bookmarks,
  notes, and optional host state persistence passed. This is not a CSS or
  actual Codex host-browser acceptance result; the generated image boards were
  inspected separately.

The dedicated pixel check is `scripts/verify-reader-contents-border.py`. Run it
on the worker against completed native or logical Scribe captures, for example:

```sh
python3 scripts/verify-reader-contents-border.py \
  /path/to/scribe/toc.png /path/to/scribe/marks.png /path/to/scribe/notes.png \
  --negative-control /path/to/scribe/baseline-toc-menu.png \
  --closed-control /path/to/scribe/reader-menu-after-toc.png \
  --output /path/to/border-report.json
```

## Review deliverables and remaining gate

`delivery/kindle-native-ui-originals.zip` contains the selected native PNG bytes
and their identity receipts. `delivery/toc-before-after.png` uses the same
native crop coordinates on both sides; its labels are outside the crops.
The conversation preview switches between actual captured states. Its default
contents pair retains native pixels in lossless WebP; other page previews are
resampled for the conversation's size limit. The native PNG archive remains the
pixel baseline for further iteration.

Earlier blank/startup/stock-Home captures are excluded from the review archive.
Only isolated public offline demonstration data was used. These observations
do not establish authentication, book-download entitlement, physical panel
response, or acceptance of another model/firmware combination. The next device
gate is installation and visual/touch acceptance of this exact candidate on an
explicitly identified physical Kindle and firmware.
