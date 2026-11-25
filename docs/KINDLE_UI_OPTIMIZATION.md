# Kindle e-ink UI handoff

The November 25, 2026 handoff supersedes the preceding e-ink pass. The
interactive Scribe and Paperwhite 6 boards and the per-screen HTML files define
the intended layout. The previous source patch supplies the common rectangular
controls, press/release refresh feedback, and the four-tab Settings design.

## Shared layout

- Keep the six grayscale tokens, square controls, and non-animated interaction.
  The touch targets of compact controls remain at least 48 logical pixels high.
- Library, Publications, Detail contents and reviews, Search results, and the
  Article drawer place a compact pager in the header. The Reader note popup
  centers the same control in its own bottom row. At `1 / 1` the pager remains
  visible and both arrows and the jump label are inert. On multi-page lists the
  label opens the existing bounded jump dialog. Swipe paging remains available.
  The font picker's local pager remains separate.
- Toasts clear the bottom navigation or Library selection bar, the Settings
  account footer, or the Search keyboard. Reader and Article clear their page
  footers when closed and follow the measured top of the open tool panel. A
  content-sized panel has 8 pixels of top padding and 16 at the bottom. The
  Article menu with no selected tool has no content band.
- Remove explanatory count lines, not meaningful state: tab counts, the
  download decision summary, Reader thread metadata, and actionable offline,
  retry, or source notices remain.

## Screen changes

| Screen | Implementation |
| --- | --- |
| Library | Two header rows, pager after sort tabs, 72-pixel list rows, and a 5 × 3 / 3 × 2 cover grid extending toward navigation. The full-size profiles accommodate up to 14 / 9 list rows; a status or task row can reduce the visible capacity. The selection bar reports the selected book count. |
| Publications | Search and sync share the first row. The second row contains the account sort controls or an icon-only Back action and ellipsized account title, followed by the pager. At full height 14 / 9 rows fit without an offline or action notice. |
| Detail | The Scribe side pane keeps a serif introduction beside all three tabs; clipping provides a Full Text dialog. Paperwhite retains Introduction, Contents, and Reviews and opens My Notes from Introduction. On Scribe an Introduction navigation request resolves to Contents. The normal and selection headers share the same catalog viewport (20 / 8 rows for the reference profiles). The More menu no longer repeats Library import. The running download dialog shows completed chapters and the progress track, without replacing the Library's actionable retry notice. |
| Search | The results pager lives on the scope row and no result-count band is reserved. Full-height Scribe layouts fit 15 results, or 11 with the keyboard; Paperwhite fits 9 or 5. Recent search chips have 36-pixel outlines inside 48-pixel targets. |
| Reader | Both devices use Contents, Progress, Thoughts, Light, and Font. The saved Thoughts position is authoritative, with Center as the initial setting. Scribe can switch to a side panel without a duplicate toggle. The primary Contents action opens Detail; the independent contents view keeps the locate-current control, with a header pager. |
| Article | The Images and Typography panels fit their controls. The image count reports loaded versus total. The list drawer uses a one-line account header and pager, and can fit 12 / 9 whole rows on the emulator viewports. |
| Settings | Sync, Reading, Downloads, and Display fit their own screens. The stylus highlighting preference defaults off and is exposed only on a Scribe with reported pen capability. The Reader uses that preference and does not add a new input path. Settings presents account actions where applicable; existing offline local-shelf access is retained. |
| Statistics | Remove the routine local-record subtitle; preserve real source or partial-data notices. |
| Login and Sleep | Login aligns the existing QR, Skills guide, and verification-code layout to each profile without simulating an upstream service. The sleep title/author frame already follows the supplied proportions; real cached book images are not replaced with the prototype's sample illustration. |

The development preference snapshot has one current layout, including the new
stylus value. No legacy preference migration or implicit data reset is added.
The cache confirmation describes the actual operation without quoting the
Downloads page's broader storage total as reclaimable bytes.

## Verification boundaries

Builds, tests, and rendering use the designated Linux worker. The firmware
walkthrough runs the native application under the launcher using isolated,
offline demonstration data. `verify-firmware-emulator.py --case walkthrough`
records QMP touch and scanout checkpoints, labels any setup intents, and records
final route, geometry, and launcher restoration separately. The comparisons
use the handoff HTML layouts and declared metrics, not a whole-screen golden
raster with matching book data and dates. The desktop layout fixtures are
supplementary evidence, not native scanout. The initial Search result row
placed the cover, add button, and on-shelf mark above the prototype's vertical
center. The focused follow-up below records the correction and recheck.

The design's signed-in assumption removes redundant sign-in copy from book
detail; it does not change the existing local-shelf startup rule. The sleep
cover mock artwork is not an application asset. The default title/author
frame follows the design and actual cached cover images keep their aspect ratio.
Live login and verification, remote reviews and downloads, and physical pen,
power, and panel behavior remain independent acceptance gates. A fixture may
demonstrate a screen state without proving its service or hardware path.

## November 25 verification record

Final ARM candidate `weread-112500000003` (95,281,368 bytes) built and staged
on `test-env` using its isolated Ubuntu toolchain on the CentOS Stream 10
host. Rust all-target tests passed 1,722 library and 57 binary cases, with
seven library cases ignored. The translation check passed 486 keys.
There was no local application verification or physical-device run.

| Firmware guest | Design / guest logical viewport | Walkthrough |
| --- | --- | --- |
| Scribe 2022, 5.20.1 | 930 × 1240 / 930 × 1240, scale 2 | 14 of 14 groups passed; 83 operation checkpoints. The same candidate passed Library and Statistics on a focused replay. Managed sleep showed the title/author frame, and the Reader body matched at wake. |
| Paperwhite 6, 5.20.1 | 632 × 840 / 636 × 848, scale 2 | 13 of 14 groups passed; 79 operation checkpoints. A firmware reboot interrupted the first run; Statistics and offline Login passed after the stock Home window recovered. Managed sleep/wake remains blocked. |

The Paperwhite guest stayed `active` after the normal `powerButton=1` request,
both in Reader and on stock Home. Both observations reported
`preventScreenSaver=1` and `usbState=1`. No power policy was changed and no
managed cover or wake is counted for that guest. The app's Paperwhite cover
and title/author layouts have independent desktop presentation checks.

The interactive desktop suite passed 147 of 147 checks on the final app.
The desktop layout and offline presentation suite passed 375 of 375 checks,
including strict comparisons of cached inline Article images on both profiles.
The first separate sleep fixture run caught a checkpoint ordering bug: the
fixture could capture before publishing a visibility change delivered in the
same timer batch. The fixture now publishes that state before capture. The explicit same-batch
replay passed 16 of 16 checks across both profiles. This correction does not
change the shipped UI or Kindle wake behavior.

The ignored working-tree evidence is under
`artifacts/kindle-ui-20261125/final/INDEX.md`. It links each operation to the
original scanout and trace, and keeps separate contact sheets, app snapshots,
desktop fixtures, and reports. Counts omit the bootstrap fixture launch and
any checkpoints from blocked or failed groups. Both disposable guests were
shut down after launcher restoration; their overlays and the worker's task
workspace were removed. The shared emulator images were left unchanged.

## Search result row follow-up

The row, the left book-information layout, and the on-shelf group each center
on the vertical cross axis. The left action keeps its full 72-pixel height.
Control dimensions, the text stack, and the shared action button are unchanged.

ARM candidate `weread-112500000004` (95,291,016 bytes) passed the focused
Search walkthrough on both 5.20.1 firmware guests, including a same-build
Scribe gesture replay. Both layouts measured 8 logical pixels from row top to
cover, 12 to the add button and status group, and 16 from the status group top
to the check icon box. The ignored evidence is under
`artifacts/kindle-ui-20261125/search-row-alignment/INDEX.md`.

A subsequent physical visit used the same source, rebuilt on `test-env`, on
Scribe 2022 / firmware 5.19.5. The first visit stopped on an unsupported
fractional delay in the test script before collecting a frame; native control
was restored and the lease released before correcting it. The replay passed
with seven framebuffer checkpoints, confirming the same offsets, pinyin
candidates, local add-to-shelf, full-height press feedback and its release,
bottom-edge swipe pagination, and a top-edge tap opening Detail. The final
Detail capture matched the corresponding app snapshot below the status strip.
Input used synthetic Slint pointer events on the physical app, not a finger or
the evdev path. The screenshots are read-only framebuffer evidence, not panel
photographs or optical refresh measurements. The application exited normally,
native control and lighting were restored, the disposable installation was
removed, and the exact lease was released at 14:32:43 Asia/Shanghai. The daily
installation and private shelf were not used. The ignored physical evidence is
under `artifacts/kindle-ui-20261125/physical-search-row/`.

## Clean daily installation

On November 25, the operator authorized replacing the existing WeRead install
and discarding its local data. The normal package was rebuilt on `test-env`
from the physically walked source. The first install attempt rolled back after
it could not verify the Login framebuffer. It did not delete the previous
installation or data. The follow-up checked both documented Scribe framebuffer
pages and both portrait orientations on the worker before reacquiring the device.

On Scribe 2022 / firmware 5.19.5, the follow-up opened the normal indexed WeRead
document through the stock file-URI handler. The owned application acquired the
native framebuffer and input descriptors with normal arguments, displayed Login,
and opened a new database with no books or download tasks. The evidence retains
only the public Login title and Skills label, not the sign-in QR. No account was
signed in. After managed shutdown and native display and lighting restoration,
the old WeRead installation and its local data were removed from the device.
No other application or Kindle document was a deletion target. The stock Library
retains one WeRead entry, and the lease was released at 14:52:09 Asia/Shanghai.
The ignored deployment receipt and redacted capture are under
`artifacts/kindle-ui-20261125/clean-deployment/`.
