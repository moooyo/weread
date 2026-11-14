# Accepted control shapes and Reader surfaces

Accepted: 2026-11-14 (Asia/Shanghai).

Status: the accepted control shapes are implemented in the current UI source.
The earlier rectangular ARM candidate, identified by SHA-256 prefix
`60d643dacf6e`, passed 26 native checks and produced 34 PNGs in the two firmware
guests recorded in [the rectangular UI review](RECTANGULAR_UI.md). That evidence
belongs to that earlier candidate. It does not establish native acceptance of
later font-slider, contents-page, and More-menu builds.

The third intermediate ARM candidate has SHA-256 prefix `6c88111cb398`. Its
build and public-package checks passed, but its Paperwhite 6 capture still
shows an incorrect contents origin. Its layout is not accepted as final.
Two Rust contents-navigation regressions and 25 software slider
checks passed for this work; those software gates remain separate from native
layout acceptance.

The fourth ARM candidate of the slider/standalone-contents iteration has
SHA-256 prefix `4bc076430a55`. Its build and public-package checks completed with exit code
0. The repair only adds explicit `x: 0px` and `y: 0px` to `contents-content`.
The final main-profile native report passed 32/32, with
`checked_profiles: ["scribe", "pw6"]` and `final_report_complete: true`.
Both profiles' native drag, ordinary-UI exit restoration, and final cleanup
checks passed within the recorded scope. The strict 37-chapter public-fixture
aggregate also passed. All software, emulator, artifact, and cleanup gates
authorized for this round are complete within their recorded scopes. Hardware
and physical acceptance, native cancellation, native commit-count observation,
and the unresolved CRC cause remain independent follow-up boundaries. See
[the slider, contents, and More-menu record](SLIDER_CONTENTS_MENU.md) for the
candidate provenance and acceptance gates.

The subsequent [Reader detail hub](DETAIL_HUB.md), ARM `f2c0f429540b`, is the
current implementation. Reader directory navigation now opens shared book
details. Its paired main and Scribe stress evidence is independent of `4bc`;
its additional PW6 long-catalog stress remains Blocked/Unverified.

The operator's direct follow-up authorized implementing the earlier shared
input/switch agreement and extending straight rectangular styling to the
remaining rectangular application surfaces. The subsequent request accepted
the interaction and page amendments recorded below.

## Text inputs and search fields

All text-entry fields and search boxes use a straight rectangular outline with
`border-radius: 0`. Apply the rule to the component that owns the visible frame,
including a containing Rectangle around a TextInput or a search activation
field. An unrounded inner TextInput does not satisfy the rule when its visible
container remains rounded. Matching topmost frame overlays must also use zero
radius.

Preserve the accepted border weight, typography, padding, focus indication,
pressed and disabled states, and touch area. This shape amendment does not
define new sizes or change composition, editing, submission, or navigation.

## Binary switches

Every two-state switch uses a straight rectangular track and a square thumb.
Both track and thumb have `border-radius: 0`; the thumb has equal width and
height. Disabled dashed outlines must follow the same rectangular track.
Capsule tracks and circular thumbs are excluded from this control.

The thumb's left and right positions must clearly distinguish the two states.
Preserve the checked state, binary semantics, visible state contrast, and the
accepted effective touch area, including any full-row touch region.

## Font-size sliders

The font-size control is a real stepped slider with a thin, straight,
continuous track and a circular thumb. This circle is intentional;
it distinguishes a multi-value slider from the rectangular track and square
thumb of a binary switch. The rectangular-surface rule must not turn the
slider thumb into a square or the thin track into a capsule.

Pointer Down and Move update only the local thumb, fill, and numeric preview.
Pointer Up selects one value from the existing supported font-size list and
submits the existing font-size use case exactly once. Cancellation, a late Up
without an active drag, disabling, value-list changes, and component
destruction do not submit a value. The existing decrement and increment
controls remain available.

## Rectangular tabs and surfaces

Reader and Article menu tabs use one rectangular selected/pressed area around
both icon and label, with black fill and white content. Main navigation follows
the same state treatment. Original tab bounds and full touch regions remain.

Buttons, keyboard keys, segmented controls, filter chips, cards, cover frames,
input wrappers, dialog outlines, and toast frames use zero corner radius. The
current `assets/design-tokens.json` radius categories are all zero; other
accepted numeric tokens retain their values.

The Library heading omits its redundant book-count/local-save summary. The
repeated offline banner is removed while the global offline/demo indication,
actionable errors, recovery notices, and task feedback remain discoverable.

## Full contents page and anchored More menu

The Reader contents view is an opaque full-window page with book context and
contents, bookmarks, and notes tabs. It uses AidokuForKindle's chapter-hub
design proposal reviewed for this amendment as a design reference only. The
proposal is not a runtime dependency or verification performed by this
project. The adaptation retains this application's own Reader use cases and
bounded pagination.

Chapter titles wrap at the available row width and determine row height. Only
the current application-supplied page scrolls in a vertical Flickable: 11
chapters on Paperwhite 6 and 16 on Scribe. Header and pagination footer stay
at fixed viewport positions. Initial entry and Locate use the current row's
actual layout position. While contents remains open, a user scroll stops
automatic following until Locate is selected again.

The accepted layout targets for the two firmware-guest profiles are recorded
below. Correct footer placement in an intermediate capture does not establish
correct contents origin. Both main-profile captures of the fourth candidate
passed after the explicit origin repair. The strict 37-chapter public-fixture
aggregate also passed; these dimensions do not establish physical-device
acceptance.

| Firmware-guest profile | Fixed header (logical px) | Visible list (logical px) | Fixed footer (logical px) | Footer top (physical px) |
| --- | --- | --- | --- | --- |
| Scribe 2022/DVT, firmware 5.20.1 | 200 | 976 | 64 | 2352 |
| Paperwhite 6/production, firmware 5.20.1 | 200 | 592 | 56 | 1584 |

The contents layout candidates have the following observed provenance. All
hashes in this table are SHA-256 prefixes, and all coordinates are physical
pixels from firmware-guest captures.

| Intermediate ARM candidate | Observed footer placement | Observed contents origin |
| --- | --- | --- |
| First, `828179` | Footer approximately Y = 832 | Short rows were centered |
| Second, `14ccca` | Scribe Y = 2352; Paperwhite 6 Y = 1584 after explicit full-height layout | The short three-row list was still centered |
| Third, `6c88111cb398` | Paperwhite 6 Y = 1584 after `alignment: start` | First short-list row at Y = 791; expected header end Y = 400 |

The third candidate passed build and package checks but did not pass contents
origin acceptance. The fourth candidate only adds explicit `contents-content`
`x` and `y` bindings at zero. Its build and package checks exited successfully;
its identity is recorded below. The final two-main-profile native report and
the separate strict 37-chapter public-fixture aggregate are complete.

| Fourth-candidate artifact | SHA-256 |
| --- | --- |
| ARM application | `4bc076430a559eba01d7b1cc8e246d77c28e42776bfaf6f9f8d7bf63e5e60e5d` |
| Development ZIP | `9919688eb953c56413ae2f43df7bca590c0d74b778d17e7513b06a5e89840814` |
| Source digest | `672d2b7fe9495812df2b2eaafd726977aa32b8eb675dd5109b003c4b153d8bfa` |
| Raw `ui/reader.slint` | `f13d4800e23554d9284c367ad5342973344fa875133b4cf5dc48f55a9d7aa38b` |
| Main HTML gallery, 964,646 bytes | `64cee5d922a515796bfa28d8afecb10dd0d2e5f6073683cff008ae8c8df7840d` |
| Public fixture archive, `public-synthetic-native-4bc076.zip` | `78905aec24c2369c140f1e90744eaf0f7e8717cb87b95a85b2129a4e3b20bb64` |

The source cause was checked against the
[official Slint v1.17.1 compiler source](https://raw.githubusercontent.com/slint-ui/slint/v1.17.1/internal/compiler/passes/default_geometry.rs).
Lines 144-153 call `maybe_center_in_parent` for a non-layout child in the
non-legacy geometry path, excluding Window, when its height does not fill its
parent. Lines 489-497 do not inject vertical centering when a `y` binding
already exists. The observed short-list centering is a compiler
default-geometry issue, not a native-driver fault. Explicit content coordinates
address that source cause. Both main-profile short-list origins and the
separate strict 37-chapter public-fixture aggregate passed.

The two Rust contents-navigation regressions and 25 software slider checks
retain their existing software scope. They are not native checks of the
fourth ARM application. Reuse of the slider results is limited to the matching
widgets-source SHA-256 prefix `b738abd`, font assets, and dependency lockfile.
The whole final application has not been rerun through that software suite.

The fourth candidate's final main native record is
`native-ui-verification/final/report.json`: 32/32 checks passed, with
`checked_profiles: ["scribe", "pw6"]` and `final_report_complete: true`.
The earlier `native-ui-verification/final/report-pw6.json` remains an
independent historical partial report: 17/17 passed, only `pw6` checked, and
`final_report_complete: false`. Its count is not added to the final 32 checks.

| Main profile | Short contents boundaries Y (physical px) | Full-width footer Y (physical px) | More popup bounds (physical px) |
| --- | --- | --- | --- |
| Scribe | 400, 544, 688, 832 | 2352 | (1156, 176)-(1844, 696) |
| Paperwhite 6 | 400, 534, 668, 802 | 1584 | (728, 176)-(1256, 664) |

The recorded Paperwhite 6 checks retained the More popup after an inside blank
click and restored the detail view after an outside click. Blank contents
clicks retained the contents view, and Return retained the Reader menu context.

Both `gesture-verification/{scribe,pw6}/final-verified/report.json` receipts
passed the observed preview/release behavior:

| Main profile | Body pixels changed while held | Slider pixels changed while held | Body pixels changed on release |
| --- | --- | --- | --- |
| Scribe | 0 | 19,403 | 311,125 |
| Paperwhite 6 | 0 | 12,820 | 121,667 |

`native_model_commit_count` was not observed, and native cancellation was not
tested. Both `restoration/{profile}/final-ui-exit/receipt.json` receipts
confirmed ordinary-UI exit, absence of the lock, and restoration of the status
bar and `awesome`. No process exit code is inferred from those UI receipts.

The main gallery contains 36 current PNGs: 15 main and 3 optional captures for
each of the two main profiles. Six before comparisons remain separate
historical evidence. The HTML size and SHA-256 are recorded above.
`gallery-validation/report.json` passed 45/45 JavaScript, DOM, and metadata
checks; that result is not CSS or host-render verification.

The separate strict 37-chapter public catalog fixture gate passed in
`/root/weread-slider-contents-20261114-r1/public-catalog/final-public-synthetic-4bc076-report.json`.
The report accepted 42 original PNG/sidecar pairs (22 Scribe and 20 Paperwhite
6) with matching identities and dimensions. All 20 pixel-consistency checks
passed (12 Scribe and 8 Paperwhite 6), as did all 5 Paperwhite 6 post-native
data checks. Both profiles placed the first row at Y = 400, kept the footer
fixed during vertical scrolling, located chapter 001, and retained the Reader
menu and body on Return. Scribe also passed entry, scrolling, Locate, and
Return for non-first current chapter 003.

The fixture contains 37 catalog entries: 3 cached original demo chapters and
34 added entries with NULL bodies and unknown status. Original identities,
body hashes, authority, and the persisted chapter/offset anchor 0/0 were
preserved. This is fixture acceptance, not evidence of 37 downloaded chapters
or book rights. The original archive has 91 entries, passed all 42 PNG/sidecar
SHA-256 and CRC checks, and excludes databases, logs, setup/startup files, and
the earlier `6c881` candidate. Its SHA-256 is recorded above. See the
[public-fixture acceptance record](SLIDER_CONTENTS_MENU.md) for the detailed
receipts and archive scope.

Final cleanup passed, as recorded in
`/root/weread-slider-contents-20261114-r1/cleanup-receipt.json`. Both task-owned
main QEMU instances exited, the status bar and `awesome` were restored, and
`original_machine_modified: false` was recorded. Source and capture artifacts
were preserved, and all three task-owned tmpfs layers were unmounted
(`task_tmpfs_mounts_removed: 3`). The canonical-path reuse correction is an
operational helper change; it does not change the ARM application or establish
the cause of the CRC startup failure.

An unresolved startup limitation remains: the first tmpfs launch of this
candidate failed a SerifRegular gzip CRC check, while a retry with the same
hash and path succeeded. Host and guest ELF hashes matched, and CRC validation
remains enabled. The cause is undetermined; source or runtime faults have not
been excluded, and no physical-device behavior is inferred. See
[Startup limitation](SLIDER_CONTENTS_MENU.md#observed-emulator-startup-limitation).

Opening contents hides the Reader menu visually while preserving `menu`,
`tool`, `subtool`, and the Reader page. Back or selecting the current chapter
restores the prior reading and menu context. Selecting another chapter opens
it at its normal start. Contents entry must not clear those context values.

More actions use an opaque rectangular popup beside the detail-header trigger,
aligned to its right edge. This replaces the bottom-sheet presentation.
Outside clicks dismiss the popup, and its existing action meanings remain.
Other confirmation dialogs retain their centered placement. See
[the interaction and acceptance record](SLIDER_CONTENTS_MENU.md).

## Scope and precedence

This amendment supersedes older rounded-rectangle choices for the controls and
surfaces above. Genuine circular slider handles, avatars, radio marks, and
selection/status indicators retain their semantics. Device bezels are outside
the application shape rule. The current request does not include physical
device deployment.

## Current capture baseline

The [native UI capture review](UI_CAPTURE_REVIEW.md) and its original PNGs
record the earlier application before this shape amendment was implemented.
Their previously verified frame correction and artifact identities remain
their own evidence. Those images must not be retouched or relabeled as
implementation of the new shapes.

The current source coverage is:

| Area | Current visible frame owner | Source implementation |
| --- | --- | --- |
| Library search activation field | `ui/bookstore.slint`, visible Rectangle | Zero radius |
| Search, API-key, verification-code, and thought-entry fields | `ui/auth.slint`, containing Rectangles | Zero radius |
| Publication search | `ui/auxiliary.slint`, parent and topmost outlines | Zero radius |
| Reader thought editor | `ui/reader.slint`, containing frame | Zero radius |
| Numeric page entry | `ui/app.slint`, input frame | Zero radius |
| Reader, Article, and Settings binary switches | `ui/widgets.slint` ToggleRow | Rectangular track and square thumb |
| Reader font-size slider | `ui/widgets.slint`, slider track and thumb | Thin straight track and circular thumb |
| Reader directory destination | `ui/reader.slint`, entry; `ui/bookstore.slint`, DetailContent | Shared book details, adaptive layout, bounded-page vertical Flickable |
| Auxiliary Reader bookmark/note contents | `ui/reader.slint`, ReaderContents | Opaque auxiliary page; historical default directory implementation |
| Detail More actions | `ui/app.slint`, popup frame | Anchored rectangular popup |

The coverage table describes source implementation; it does not imply live
authentication or acceptance of every input flow. The rectangular UI review
retains native pixel and interaction evidence for its own earlier candidate.
Current slider, contents, and More-menu software, emulator, artifact, and
cleanup gates are complete within the scopes in the record linked above.
Hardware and physical acceptance, untested native cancellation, unobserved
native commit counts, and the unresolved CRC cause remain separate follow-up
boundaries.
