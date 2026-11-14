# Font slider, full contents page, and anchored More menu

Recorded: 2026-11-14 (Asia/Shanghai).

Scope update: [the Reader detail hub](DETAIL_HUB.md) supersedes this record's
accepted default directory entry and layout. The new implementation is in
the `f2c0f429540b` candidate, with focused host, ARM/package, paired main
native, gallery, and Scribe catalog/notes evidence recorded separately. PW6
long-catalog stress remains Blocked/Unverified by font-startup failures.
The `4bc` results below remain historical acceptance of the former standalone
contents path, not acceptance of the new hub. Font-slider, rectangular-control,
and anchored-More decisions remain in effect. The new normal-navigation return
may finish and restart reading sessions while retaining the character anchor
and Reader menu context.

Historical candidate status: this software, firmware-emulator, capture/archive, and
cleanup iteration is complete. The fourth candidate passed paired main native
scanout, drag, ordinary UI restoration, and separate long-catalog gates. Its
gallery passed JavaScript/DOM/metadata checks. The startup limitation and
independent hardware/native-input conditions below remain outside this
completed scope. No physical-device deployment is part of this update.

The operator requested a real font-size slider and a complete contents page,
then corrected the reference project to AidokuForKindle and requested that More
actions appear beside their trigger instead of in a bottom sheet. This record
keeps the accepted behavior separate from software and physical acceptance.

## Accepted behavior

- Font size uses a real discrete slider over the existing supported value
  list. Left-button Down and Move preview only the local thumb, fill, and
  numeric value; the Reader font and persisted preference remain unchanged.
  Up selects the final listed value and clears the gesture before invoking
  the existing use case exactly once. Cancel restores the accepted preview
  without committing; a late or repeated Up cannot submit the retired gesture.
  Component destruction does not commit. Existing decrement/increment controls
  remain available. The circular thumb is retained over a thin, straight,
  continuous track. Brightness/warmth segment controls keep their separate tap
  behavior; binary switches still use rectangular tracks and square thumbs.
- The Reader contents view is an opaque full-window page. Book context and
  contents/bookmarks/notes tabs remain. Titles wrap at their available width;
  a bounded current page can scroll vertically. Header and pagination stay at
  fixed viewport positions, and Locate reveals the current chapter.
- Returning or selecting the current chapter preserves Reader pagination,
  history, reading position, session/report state, and menu/tool/subtool
  context. A different chapter opens at its normal start. Catalog positions
  use actual catalog order rather than assuming consecutive chapter indexes.
- More uses a rectangular, opaque, right-aligned menu below the detail-header
  ellipsis. Outside clicks dismiss it; its rows retain the existing refresh,
  import, cache, and removal actions. Other confirmation dialogs remain centered.

The design reference is AidokuForKindle's chapter-hub review proposal and its
existing detail MoreMenu. The proposal's full-page navigation and context
return were adapted to WeRead's own contents/bookmarks/notes use cases; manga
filters and source protocols were not imported. The reference is neither a
runtime dependency nor verification performed by WeRead.

The chapter-hub proposal is a design-only artifact, not evidence that
AidokuForKindle's current application implements it. Its older small contents
panel was not adopted. The user corrected the reference from BiliComics to
AidokuForKindle before this implementation was finalized.

## Source and runtime boundary

The changes are in `ui/widgets.slint`, `ui/reader.slint`, `ui/app.slint`, and
the Reader action use cases with dedicated contents-navigation regressions.
`return_reading` is an interface label. No account, acquisition, database,
preference-layout, or compatibility format was added.

The native guard remains
`689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d`.
Remote sources, mutable caches, logs, and artifacts are task-owned under
`/root/weread-slider-contents-20261114-r1/`. Earlier source and target caches
are read-only namespace lowers. Operator-local output is under
`artifacts/slider-contents-20261114/` and is ignored by Git.

Scribe 2022/DVT and Paperwhite 6/production firmware guests use 5.20.1 with
native viewports 1860 x 2480 and 1272 x 1696 respectively. These are emulator
results; physical installation and acceptance of this candidate remain open.

## Current source geometry

The complete page is explicitly sized to its parent viewport. Its outer
VerticalLayout also has explicit viewport width and height. The fixed header
is 200 logical pixels: clock 28, return header 52, book context 68, and tabs 52.
The list height is the actual logical viewport height minus that header and
the fixed footer. It does not depend on the catalog's preferred height.

| Firmware guest | Logical viewport | Visible list height | Footer height | Native footer top |
| --- | --- | --- | --- | --- |
| Scribe 2022 / 5.20.1 | 930 x 1240 | 976 | 64 | 2352 |
| Paperwhite 6 / 5.20.1 | 636 x 848 | 592 | 56 | 1584 |

These values describe the required viewport geometry. With a short unscrolled
catalog, the first chapter must begin at logical y=200, native y=400. Correct
footer placement alone does not establish that list-origin requirement. Both
fourth-candidate main profiles passed the short-list origin and footer checks.
The separately passed long-title fixture remains outside that main result. PW6 desktop
design captures retain 632 x 840 and must not be substituted for the guest's
measured runtime viewport.

Wrapped chapter rows grow with their measured preferred text height. The
Flickable contains only the existing page model, at most eleven PW6 chapters
or sixteen Scribe chapters. Its content extent is the greater of the visible
list height and that page's preferred height. This allows access to the final
wrapped row without moving the footer or loading the whole catalog into the
scrolling view. Initial entry and Locate use the current row's actual layout
position; user scrolling ends automatic following. No new persisted route or
compatibility field is introduced.

## Retained correction history

The earlier [rectangular UI candidate](RECTANGULAR_UI.md), ARM `60d643dacf6e`,
retains its 26 native checks and 34 original PNGs. Those runs are not executions
of the slider/contents candidate. The older 540-pixel contents-frame checks
belong to their original panel implementation, not this full-page layout.

The first focused host compilation stopped because MoreMenuItem specified both
`height` and `min-height`. No application run is attributed to that failed
compilation. Its log is retained as
`build/logs/unit-lib-1beadf092b-20261113T233413Z.log`; removing the redundant
minimum-height constraint allowed the focused test run to complete.

The three ARM checkpoints remain incomplete layout results:

| Intermediate ARM | Native observation | Remaining defect |
| --- | --- | --- |
| `8281790b24d7` | First footer appeared near native y=832 | Preferred-size layout and centered short rows |
| `14ccca0b4481` | Explicit full-height layout moved the footer to Scribe y=2352 and PW6 y=1584 | Three-row catalogs remained centered |
| `6c88111cb398` | Start alignment retained the correct PW6 footer at y=1584 | PW6's first short-catalog row still began at y=791 instead of y=400 |

The second intermediate ZIP is
`66a6b93cf009e310c16cb42eaab25f30cd59472f2abd4f8e15eb79b2cbd586f2`.
The third intermediate ARM, ZIP, and source identities are
`6c88111cb3988a84765df899130423fdbe7aba78ddc5d3399e79171044593e0b`,
`2967793e98ed9638690f9ccb00220ead00fb06734324a40445c5f77350064b56`, and
`b0a6c3b5422328db8706790d810d5a7c56d94de54154887d52f22b4753bb41e9`.
Neither build success nor footer correction is full-page layout acceptance.

Fourth ARM `4bc076430a55` explicitly pins `contents-content` x/y to the scroll
viewport origin. The same-version [Slint 1.17.1 geometry pass](https://raw.githubusercontent.com/slint-ui/slint/v1.17.1/internal/compiler/passes/default_geometry.rs)
can inject a centered y binding for a non-layout child that does not fill its
parent when y is unspecified; an explicit position prevents that default.
This explains the contents-origin correction at the UI/compiler boundary,
without introducing a native-driver change. Fourth-candidate short-list
recapture passed on both profiles. Long-catalog acceptance also passed in its
separate final report; its counts are not added to the main report.

## Verified software and build gates

The two isolated Rust contents-navigation scenarios passed on `test-env`.
They exercise a non-first reading page, real page ranges/storage, current and
different chapters, sparse catalog ordering, bookmarks/notes pagination, and a
delayed Reader hold across contents entry. The receipt is
`build/artifacts/unit-lib-1beadf092b-receipt.json`: two parent cases passed,
zero failed or ignored, and nested isolated-child summaries are not counted
again. Its source snapshot is
`120e86cb51bef78f0200a600cd23e0c86d180e334160c81446d2c01763fd45f9`.
The later layout corrections did not change those Reader action use cases.
These are host test results on their recorded snapshot, not executions of the
fourth ARM or pixel validation of its contents layout.

The synthetic winit/software slider runner passed 25 checks with zero failures
on both desktop layouts. Held previews changed the slider pixels while
preserving the font and zero commits. PointerExited cancellation restored the
accepted pixels; late Up and component destruction kept zero commits. A fresh
release selected a listed value with one commit, and repeated Up retained that
count. Its report is `software-slider-check/report.json`, with standalone
software executable SHA-256
`9fc5a7a295982787c47b16f257751fe237b2429818a3f2f8a3e3b68ff6f7c74c`.
This is synthetic cancellation/commit-count evidence, not native evdev Cancel
or physical touch acceptance. The standalone runner has its own binary
identity; its 25 checks are not fourth-ARM native gesture results.

The locale check passed with 478 complete keys. The fourth ARM and public
bundle checks passed with exit 0. That ELF is ARM EABI5 hard-float and has no
dynamic section. Its Cargo release build took 3 minutes 32 seconds, and its
full build/package receipt took 214.63 seconds. The third intermediate build's
208.40-second receipt remains historical. These are worker build durations,
not Kindle interaction latency or corrected-layout acceptance.

| Fourth candidate artifact | SHA-256 |
| --- | --- |
| ARM application | `4bc076430a559eba01d7b1cc8e246d77c28e42776bfaf6f9f8d7bf63e5e60e5d` |
| Native frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Development ZIP | `9919688eb953c56413ae2f43df7bca590c0d74b778d17e7513b06a5e89840814` |
| Runtime source manifest | `672d2b7fe9495812df2b2eaafd726977aa32b8eb675dd5109b003c4b153d8bfa` |
| Uploaded `ui/reader.slint` | `f13d4800e23554d9284c367ad5342973344fa875133b4cf5dc48f55a9d7aa38b` |

The current `build/artifacts/arm-build-receipt.json` records the fourth
candidate, with log `build/logs/arm-20261114T005100Z.log`. The third candidate's
retained log is `build/logs/arm-20261114T002652Z.log`. All paths in this section
are relative to
`test-env:/root/weread-slider-contents-20261114-r1/`. Software tests and all
intermediate captures/build artifacts retain their separate identities.

## Completed fourth-candidate main firmware gates

`native-ui-verification/final/report.json` passed all 32 original-scanout and
sidecar checks for the exact fourth ARM/guard. Its `checked_profiles` contains
both `scribe` and `pw6`, and `final_report_complete` is true. The earlier
PW6-only 17-check report remains a separate partial record; its counts are not
added to 32. The read-only checker analyzed captures previously produced
through the guest input path and did not itself operate a guest.

| Main profile | Short-row native boundaries | Full-width footer top | More rectangle |
| --- | --- | --- | --- |
| Scribe | 400, 544, 688, 832 | 2352 | `(1156,176)-(1844,696)` |
| PW6 | 400, 534, 668, 802 | 1584 | `(728,176)-(1256,664)` |

Both short catalogs begin directly below the fixed header, with spare space
below the rows. In comparisons excluding the clock region, blank contents
taps retained the contents image and Return restored the original Reader/menu
image. Blank menu space retained More; outside taps restored Detail. All four
menu edges were continuous.

The prior software runner's widget source SHA, lock, and bundled fonts matched
the final control sources. The Reader layout had changed. This supports the
25 checks as unchanged-control evidence, not a whole-fourth-application rerun
or native cancellation.

Both `gesture-verification/{scribe,pw6}/final-verified/report.json` files
passed physical-coordinate QMP drag observations. Held samples retained the
body crop while the slider changed; body pixels changed after release:

| Profile | Held body changed pixels | Held slider changed pixels | Released body changed pixels |
| --- | --- | --- | --- |
| Scribe | 0 | 19,403 | 311,125 |
| PW6 | 0 | 12,820 | 121,667 |

Scanout cannot count model callbacks. Neither native report observes a commit
count or exercises native Cancel. The separate software chooser counter
remains the evidence for one release commit.

Both `restoration/{scribe,pw6}/final-ui-exit/receipt.json` files record ordinary
application exit through confirmation, absent launcher locks, running status
bars, and non-stopped window managers. The collector did not kill application
processes. No numeric launcher exit code is inferred from those receipts.

## Main capture gallery

`delivery/manifest.json` retains 36 unmodified original scanout PNGs: fifteen
main pages on each profile plus exit confirmation, held slider, and released
slider observations on each. Six before/after comparisons use explicitly
identified full-page or region views from their original candidates. They do
not rewrite older images as the new implementation.

`delivery/kindle-native-ui.html` opens on the native-resolution contents pair.
Its image conversion uses lossless WebP; the default contents pair is not
resampled. Type and More previews are 744 x 992 for Scribe and 510 x 680 for
PW6. Other displayed previews are smaller derivatives; full-resolution PNG
bytes remain unchanged. The device frame is illustrative presentation chrome.

The final fragment is 964,646 bytes, SHA-256
`64cee5d922a515796bfa28d8afecb10dd0d2e5f6073683cff008ae8c8df7840d`.
`gallery-validation/report.json` passed all 45 runtime-metadata, immutable-byte,
JavaScript syntax, and isolated jsdom DOM checks. Page/device switching,
comparison isolation, default contents, and optional host state handling
passed. Saved state contains selection keys and labels only; the largest
observed payload was 327 bytes, without image data. CSS layout, raster decoding,
and the actual Codex host's rendering are outside that check.

## Separate 37-chapter fixture

`public-catalog/final-public-synthetic-4bc076-report.json` passed for the exact
fourth ARM/guard and both firmware guests. It is independent of the ordinary
gallery and the 32-check main report. All 42 accepted original PNG/sidecar
pairs passed identity and dimension checks. The report passed 20 named pixel
consistency checks, with the following separate profile coverage:

| Profile | Accepted PNGs | Pixel checks | First-page last ordinal | Catalog pages | Final ordinal |
| --- | --- | --- | --- | --- | --- |
| Scribe | 22 | 12/12 passed | 016 | 3 | 037 |
| PW6 | 20 | 8/8 passed | 011 | 4 | 037 |

Both initial current rows start at native y=400. Recorded vertical drags change
the viewport while keeping the footer fixed; the first-page last items and
final catalog item remain accessible. Locate restores current chapter 001,
and Return restores the preceding reading/menu view. Scribe additionally
checks non-first current chapter 003, its automatic placement, scroll-away,
Locate, and reading/menu return. No extra PW6 chapter-003 native case is
inferred from that Scribe sequence.

PW6's `public-catalog/pw6/session-final-4bc076/post-native-data.json` records
37 chapter metadata entries with only three cached original chapters. The
34 added entries still have null bodies and unknown content status. Original
UIDs, body hashes, and content ownership remain intact; the post-run persistent
fixture position is chapter 0 / offset 0. This is synthetic metadata and
preservation evidence, not 37 downloaded chapters or network acquisition proof.
All five PW6 post-native data checks passed. Both fixture applications stopped
and restored native control before their VM closeout.

The independent `public-catalog/public-synthetic-native-4bc076.zip` archive is
12,374,964 bytes, SHA-256
`78905aec24c2369c140f1e90744eaf0f7e8717cb87b95a85b2129a4e3b20bb64`.
All 42 PNGs and their original sidecars passed archive SHA/CRC checks. Its 91
entries exclude databases, guest logs, setup/startup samples, intermediate
`6c881` captures, and acquired content. The accepted source PNGs were not
retouched. `public-catalog/archive-receipt.json` records the completed archive
gate. No new hardware, account, or upstream download qualification follows
from this synthetic fixture.

## Main emulator cleanup

`cleanup-receipt.json` passed for the exact fourth ARM. Both named task-owned
main QEMU processes exited after firmware status-bar/window-manager restoration.
Original emulator bases were not modified, and source/capture artifacts were
preserved. This cleanup is not physical-device access or acceptance.

The PW6 guest aliases `/tmp` to `/var/tmp`. Three task-owned tmpfs layers were
present at the execution directory and were all unmounted. The operational
`stage-pw6-tmpfs.py` helper now detects reuse through the canonical path to avoid
duplicate mounts. This helper correction did not change the ARM source or
binary, and it does not identify the cause of the earlier gzip checksum error.

## Observed emulator startup limitation

The first fourth-candidate PW6 attempt from the guest tmpfs executable failed
while loading bundled Serif Regular with a gzip checksum error. Host and guest
ELF SHA-256 matched. Restarting the same hash at the same executable path
succeeded on the second attempt. Integrity validation remained enabled; it was
not bypassed to obtain the successful captures.

`pw6-binary-staging.json` records the execution staging and enabled validation.
The shared log retained as
`restoration/pw6/final-ui-exit/application.log` includes checksum messages from
earlier starts as well as this recurrence; those earlier entries are not
additional counted fourth-candidate tmpfs failures. Startup captures remain
separate from ready application pages under `session/captures/pw6/`.

This is an observed emulator-startup limitation with an unconfirmed cause.
Identical ELF hashes and a successful retry do not exclude a source or runtime
defect, identify the emulator as the cause, or predict physical-device startup
behavior. The failure remains part of the record.

## Completed task gates and independent conditions

| Gate | Current status | Required evidence |
| --- | --- | --- |
| Fourth ARM and source identity | Passed: build/package only | Fourth candidate receipt and exact identities above |
| Main paired native screenshots | Passed: 32 checks | Fourth-candidate short-list origin, footer, overlays, and source identities |
| Native slider dragging | Passed on both profiles | Held preview and release through QMP touch; no native callback-count claim |
| Long-title contents | Passed: 20 pixel and five data checks | Separate 37-entry fixture on both profiles, 42 accepted PNG/sidecar pairs |
| Contents return and More | Passed on both main profiles | Original menu after Return, anchored menu retention and outside dismissal |
| Current-chapter native selection | Passed within recorded coverage | Chapter 001 on both profiles; additional non-first chapter 003 on Scribe |
| Native application restoration | Passed on both main profiles | Ordinary UI exit and restored firmware ownership |
| Main gallery | Passed: 45 JS/DOM/metadata checks | Exact 36-source-image manifest and HTML identity; excludes CSS/host rendering |
| Synthetic fixture archive | Passed | 42 PNG/sidecar SHA/CRC checks, 91 entries, archive identity above |
| Main task-owned emulator cleanup | Passed | Two QEMU processes exited, firmware restored, three task tmpfs layers removed; original bases unchanged |
| Startup repeatability | Limitation observed | Retained PW6 checksum failure, unchanged successful retry, cause unconfirmed |
| Physical installation and acceptance | Not performed | A separately authorized run on an explicitly identified Kindle model/firmware |

All software/emulator/artifact gates authorized for this iteration are closed.
Native Cancel and native callback counts were not observed; source-matched
software checks retain their own evidence. Physical hardware qualification and
the unconfirmed startup checksum cause require separate work. Emulator
networking and public fixture text do not establish authentication, upstream
reading rights, physical panel response, or a released support matrix.
