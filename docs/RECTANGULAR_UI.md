# Rectangular UI and concise Library review

Recorded: 2026-11-14 (Asia/Shanghai).

The operator requested removal of redundant Library descriptions, rectangular
Reader-menu tabs covering icon and label, and corresponding treatment of other
rounded rectangular application surfaces. This implements the previously
accepted straight input frames and rectangular binary switches. The scope is
application UI and remote firmware-emulator review; physical installation and
acceptance remain open.

## Resulting presentation

- The Library heading omits the book-count/local-save phrase. Its repeated
  offline banner is removed, preserving the global offline/demo indication,
  source errors, recovery prompts, and download-task feedback.
- Reader, Article, and main-navigation tabs highlight icon and label together
  inside one straight rectangle. Selected and pressed content is white on
  black, and the full existing touch region remains.
- Input wrappers, search fields, keyboard keys, segmented controls, buttons,
  chips, cards, cover frames, dialog outlines, toasts, and disabled dashed
  outlines use zero corner radius. The four existing radius-token categories
  in `assets/design-tokens.json` are zero.
- ToggleRow retains its 48 x 28 logical track and 20 x 20 logical thumb, with
  square corners and unchanged left/right positions. Circular slider handles,
  avatars, radio marks, and circular selection/status indicators remain.
- The earlier contents-panel clipping and final-layer frame are preserved.
  The UI changes do not alter account, acquisition, database, or preference
  formats. All six Slint source files were updated; Rust use cases are unchanged.

## Remote artifact and environment

| Artifact | SHA-256 |
| --- | --- |
| ARM application | `60d643dacf6e07ff48321785173a3d88dbf7400b8f6344bdf00668f7c91bbbff` |
| Native frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Development ZIP | `6003efe54a2ccf155ea6b47601b3a9e8dc30e26a003d032afc08baea240d3384` |

The guard source and executable are unchanged from the previous independently
verified named-viewport correction. The application was compiled on `test-env`
from the current normalized source, with task-owned mutable layers and previous
target/tool caches used read-only inside a private namespace. Historical source
and target paths appear only as namespace bindings for cache fingerprints.

Remote work is under `/root/weread-ui-rectangular-20261114-r1/`; operator-local
generated output is under `artifacts/ui-rectangular-20261114/`. Source and package
receipts bind the actual ELF, public ZIP members, and their CRCs.

Both guests use firmware 5.20.1: first-generation Scribe/DVT with native
1860 x 2480 and logical 930 x 1240, and Paperwhite 6/production with native
1272 x 1696 and logical 636 x 848. These are firmware-emulator viewports. The
PW6 design reference remains 632 x 840. No new physical support claim is made.

## Verification and capture facts

All builds, executable checks, application execution, QMP taps, and pixel
verification ran on `test-env`. Only source editing and generated-image
inspection took place on the Windows operator machine.

- The static ARM build and public-package identity/CRC checks passed.
- `scripts/verify-rectangular-ui.py` passed 26 native pixel and interaction
  checks across both devices. They cover the empty Library summary region,
  removed banner, square search corners, full rectangular selected Reader tab,
  visible white labels, both switch states, exact restored switch pixels, and
  unchanged Reader-menu pixels after closing contents. Earlier rounded
  screenshots are negative controls for the changed shapes.
- The initial switch locator omitted ToggleRow's 8-logical-pixel inner inset.
  The locator was corrected without changing pixel thresholds. Early rapid-tap
  PW6 captures did not show the requested two-state pair; settled-state captures
  and explicit track-background/thumb assertions then passed. Initial reports
  are retained independently from the passing report.
- Contents, bookmarks, and notes retained a continuous Scribe right frame at
  all 2480 native rows. The previous frame correction remains verified after
  the shared-control changes.
- Thirty-four current native PNGs cover seventeen states on each device:
  the fourteen main states plus numeric jump, more actions, and exit
  confirmation. Main images are actual QMP scanout captures from the exact
  application/guard pair above using isolated public offline demonstration data.
- Both guests exited through the actual application confirmation button. Their
  final metadata confirmed the native backend and runtime viewports; status-bar
  and window-manager ownership were restored before the task-owned QEMU
  processes were closed.
- The final conversation fragment passed 26 remote JavaScript syntax/DOM checks
  using existing read-only Node/jsdom validation dependencies. Page and device
  switching, before/after comparison isolation, optional host persistence, and
  host-free operation passed. This scope does not include CSS or the actual
  Codex host browser. Generated image boards were inspected separately.

The current figure package is separate from the earlier native capture review.
Old images appear only in explicitly marked before comparisons; their identities
are retained. Native PNG bytes are not retouched. Captured inputs and switches
do not establish live authentication, upstream download rights, panel timing,
or physical touch acceptance.

The next device gate is installation and visual/touch acceptance of this exact
candidate on an explicitly identified physical Kindle and firmware.
