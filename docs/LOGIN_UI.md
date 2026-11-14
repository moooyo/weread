# QR Login UI

Accepted: 2026-11-15 (Asia/Shanghai).

Status: the approved Skills guide and the existing larger-QR layout are
implemented. The final guidance snapshot passed 89 unique selected host cases,
77 actual software-interface checks, and the separate Detail capacity captures.
The earlier QR-only redesign and its physical startup retain their own evidence
below. Neither this guide nor its software screenshots establish real scanning,
authenticated device use, or a new model/firmware compatibility claim.

## Accepted interface

The title uses exactly `Strings.login-title`. Its Chinese locale value is
`\u767b\u5f55WeRead` in escaped form, with no separating whitespace. The page
has no brand eyebrow, manual API Key editor, or API Key/local-Library/demo
footer. There is no hidden replacement entry for manual credentials. Internal
manual-key reducers remain in the controller, but the Login UI does not expose
them, including when a test forces the internal `key` step.

The QR content is centered in one responsive column. Its maximum edge is 480
logical pixels on Scribe and 360 on Paperwhite, reduced when available space
requires it. The QR has no frame or separate white backing: its surrounding
surface uses `Theme.paper`. The shared encoder emits an RGBA alpha mask with
opaque dark modules and transparent light modules and a four-module quiet
zone. The Slint image uses `colorize: Theme.ink` and
`image-rendering: pixelated`. This describes image construction and software
rendering, not a physical phone-scanning result.

Refresh retains its existing quiet-button treatment: 18-pixel text, normal
400 weight, and a 48-logical-pixel touch height. Loading, expiry, and offline
states show one state message rather than duplicate prompts. A valid QR
retains a fixed position for its expiry label, separate from Refresh.

The four-digit verification step retains its numeric keyboard, four-digit
limit, visible Backspace, rescan, submit, and close actions. The close control
uses the existing cancellation and ordinary-exit behavior. This change does
not alter authentication protocols, account data structures, credential
persistence, database layout, or the existing local-book reading capability.

A first ordinary start without a session or local shelf opens QR Login.
Existing ordinary-start Library selection when a session or local books are
available remains unchanged. Removing the Login footer does not remove local
import or reading behavior from Library.

## Skills setup guidance

Accepted: 2026-11-15 (Asia/Shanghai), using the final V3 guidance proposal.
The accepted guidance is implemented and passed its final host and software
verification. All execution for this scope used `test-env`.

The QR page includes a first-use Skills prerequisite, the official page address,
and a quiet help action beneath Refresh. The guide opens as a full page with a
separate public QR for `https://weread.qq.com/r/weread-skills`. Its four rows
share a numbered column and text origin; the widest text determines the width
of the complete centered group. The fourth row explains that the Kindle does
not require manual API Key entry. Back is at the upper left. The existing X
remains at the upper right and requests ordinary application exit in every
Login state; it never acts as Back. There is no bottom Back button.

The official page's public setup section and JavaScript describe generating
a personal API Key after signing into the same account. The application does not
generate or enable an upstream key. The public guide QR uses the same admitted
RGBA encoder and transparent quiet zone as Login. Source fields `skills-open`
and `skills-qr` are transient UI state, not persistence or credential fields.
The guide replicates the admitted alpha mask at eight source pixels per module,
with a maximum 512-pixel raster edge. This caps the replicated raster at one MiB and reduces
fixed-point texture-sampling drift while keeping the 360/288 logical display
sizes, four-module quiet zone, and pixelated rendering. The general QR encoder
and its byte-admission contract remain unchanged.

Opening help retires the current challenge and fences queued Login results.
Back obtains a fresh challenge through the existing Login flow. A missing
Skills key after the four-digit step returns to the QR page with its setup
hint and clears temporary code state. Close retains its existing session/data
preservation and exit behavior. Authentication protocols, database layout,
preferences, and upstream key generation are unchanged.

The original guidance PNGs and interactive proposals remain design records;
they are not application, firmware, or physical-device screenshots.

### Final guidance verification

The task-owned root is `/root/weread-skills-implementation-20261115-r1`.
The runtime manifest is
`e34dbeb1be35032604500eef17344a55f035e955db3a5e8a9c6c46d00c4eec0f`,
with 620 files on baseline main `12c1315`. Fifteen source/test paths differ;
documentation closeout edits are outside that frozen runtime snapshot.

The Login filter passed 46 parent cases. The QR and eight Detail filters passed
their exact 44-case union through the same libtest executable and private source
namespace; case IDs were verified before and after execution. The complete
scope contains 90 executions and 89 unique cases, with one Login/QR overlap,
zero failures, and zero ignored cases. Thirteen changed Rust files passed the
read-only format check. These selected scopes are not a full application suite.

The actual App/Slint fixture is `weread-fixture-2944aee7cc88`, SHA-256
`2944aee7cc883a98acf0c2ab6da8637307635aae27e8873dfca488e79678df67`.
It renders Scribe 930 x 1240 and Paperwhite 632 x 840 windows at scale 1.
The final report `software-skills-final-dense/report.json` passed 77/77 checks
and retains 24 unchanged PNGs with metadata and model sidecars. A preceding
PW-only 24-check preflight is marked as partial and is not added to that total.

The initial Login QR uses an explicitly named, non-authenticating display
overlay. Guide opening, its public QR, left Back, cleared state, and fresh
offline Login use the real controller and pointer path. The overlay is hidden
before the second guide/back cycle. Four rendered instruction rows and their
group centering were checked and visually reviewed; exact text is source-bound,
not an OCR claim. Existing numeric-code, Backspace, four-digit maximum, and
rescan pointer checks passed. Actual X exit and stale-result fencing belong to
the separate host closing regression, rather than a fabricated UI exit result.

The initial `75d17167` candidate passed 89 selected host cases and built an ARM
package, but its interface report passed 75 checks and failed two PW guide-QR
geometry checks. That source, package, report, and original PNGs remain
historical. The renderer's eight-fractional-bit texture coordinates explain the
observed four-pixel drift; the admitted guide mask's bounded pixel replication
fixed it without changing display sizes or acceptance thresholds. Final guide
ink-center error is 1 logical pixel on Scribe and 0.5 on PW, within the original
three-pixel allowance. The general QR encoder remains unchanged.

No firmware guest, physical Kindle, credential input, or upstream authentication
submission was used for the guidance verification. The previous physical
installation and acceptance boundaries below remain attached to their own ARM.

The ARM application built successfully in 186.87 seconds, with SHA-256
`4c4df58d13cde38df3b280d43ca004cdcf1039026638ea3bb60da6daa1ec6d61`.
The unchanged native guard remains `689ef367eabe` and launcher `71316711`.
Original development ZIP `bdf6f8cb27c092d27350972a91e6285ec86a74fe9cb0e862571f0259764fe2d6`
passed CRC and contained the matching application and guard. The original
compiler package and any documentation-only delivery repack retain separate
receipts. The new application has not been installed or tested on a physical
Kindle in this scope.

Final reviewed delivery is `weread-kindle-skills-reviewed-dev.zip`, 68,276,537
bytes, SHA-256
`8446935f6d9dcd8481d312ec4d96e837c0bba25f7a0fe38e74b8caad12788d36`.
Only the public packaged README changed from the successful original ZIP;
the other 35 members, application, guard, and launcher are byte-identical.
The member list, ordering, CRC, and source integrity checks passed. This is a
documentation-only repack, not a new compiler output. Original packages and
all initial failure receipts remain preserved.

## Initial QR redesign verification snapshot

All execution for this iteration runs on `test-env`. The task-owned root is
`/root/weread-login-implementation-20261115-r1`.

| Source identity | Value |
| --- | --- |
| Baseline main commit | `3a45c2a` |
| Runtime verification manifest | `d049eb0bd8c35fdbcd75e13960c9ba4d62893492b106bd8865801caad4b4da14` |
| Files in the frozen snapshot | 617 |

The six changed source/test files in that snapshot are `ui/auth.slint`,
`assets/i18n/zh-CN.json`, `src/app/library.rs`,
`src/app/qr_input_budget_tests.rs`, `src/app/native_edit_tests.rs`, and
`tools/offline-ui-fixture/src/main.rs`. This document and the other
documentation-only closeout edits are outside that 617-file runtime snapshot;
they do not require a new executable build.

The positive password-editor UI cases were retired because that interface was
removed. Their replacements check that actual QR and code pages, and a forced
internal key state, expose no credential editor. This is not a claim that the
internal manual-key reducers or shared input components were deleted.

## Completed host and software-interface evidence

| Current scope | Result | Receipt identity or report | Duration |
| --- | --- | --- | --- |
| `login` host filter | 43 passed, including Close and the new absence checks | `unit-lib-428821350e` | 51.07 seconds |
| `qr_input_budget_tests` host filter | Nine passed | `unit-lib-f4e58133cb` | 53.49 seconds |
| Offline interface fixture build | Exit 0 | Build log timestamp `174153Z` | 63.65 seconds |
| Actual App/Slint software interface | 85/85 checks passed across both profiles | `software-login-final/report.json`, status `passed` | Not a timing claim |

The case-ID comparison in `unit-scope-summary.json` identifies exactly one
overlap between the two host filters: 51 unique parent cases, not a 52-case
total. These remain scoped results rather than a complete application-suite
run. Earlier initial nine-result and Close-only observations remain under
`checkpoints/pre-qr-only-test-8fd5`; they belong to the preceding
test-maintenance snapshot and are not added to the current counts.

The current fixture executable is 209,192,664 bytes, with SHA-256
`2b1f7daf31346dda76f710302c2300d404dc8fb7435bc957e5f8805db62c84d4`.
Its captured interface is the actual application and Slint renderer on the
Linux host. Scribe PNGs are 930 x 1240 and Paperwhite PNGs are 632 x 840, both
at scale 1. They are software screenshots, not PIL drawings, proposal images,
firmware-emulator scanouts, or physical-device captures.

All 28 original PNGs are preserved without retouching in
`login-ui-software-d049eb0b-originals.zip`, 711,939 bytes, with SHA-256
`09a0cd5be6c60099884325a79c563946e2228f6b1f223c2b6238c46501c9edba`.
The archive CRC check passed. No separate desktop-main build was performed;
the fixture contains the current library and UI, and the ARM main was built
separately.

Ready QR, code, loading, and expired screens use explicitly identified
presentation fixtures. They contain no real authentication token, and no
authentication submission was made. After the fixture overlay was removed,
footer absence, Refresh, numeric input, the four-digit maximum, Backspace,
rescan, and final QR/code clearing were checked through the real offline
controller and pointer flow. Presentation-state checks do not prove an
authenticated upstream transition.

The Scribe/Paperwhite ready screens and Scribe code screen were visually
reviewed against the approved design. The QR has no white plate or border,
and the code keyboard's delete control is visible. The earlier V3 proposal
files remain historical design proposals; they must not be relabeled as these
application screenshots.

The three-lower read-only mount trial passed as build-environment preparation.
It is excluded from product test and interface-check totals.

## ARM and development package

The successful ARM/package retry completed with exit 0 in 195.598852 seconds.
The build log has timestamp `arm-20261114T175406Z`, under `build/logs/` in
the task-owned root. Its runtime source remains the frozen `d049eb0bd8c3`
manifest above.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM application | 84,644,760 | `8da54b9b111dab45f229b0d9c34db52b7aa25f63df1798c7c0761eba0da4bffe` |
| Native frame guard, unchanged | 25,792 | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Original build ZIP | 68,225,641 | `9bc61cffb370e475186649f97e7aaa07b2404c4b88f30a4d269780741fcbca85` |
| Final reviewed delivery ZIP | 68,225,804 | `a0cb9bd254e3d7971ffc1edbb0699bec329df0792109b9f06534fd20e9974197` |

The launcher is unchanged, with SHA-256
`713167110d4839f14505cd03c6b30753476772c729fe92890a86fbd0085fe6be`.

The first ARM attempt completed the Rust release step but failed the guard
link because the inherited Zig cache lacked `frame_guard.o`; its exit code
was 1. The original failed receipt and log remain preserved. The retry used
this task's fresh `build/clean-zig-local` and `build/clean-zig-global` caches,
without modifying the source or old caches or bypassing integrity checks.
`cache-repair.json` records that environment repair. The failed attempt is
not relabeled as a successful run.

The final delivery is
`build/artifacts/weread-kindle-login-v3-reviewed.zip` in the task-owned root.
`packaging-documentation-final-receipt.json` records its 36 members, passed
CRC check, and matching ARM/guard/launcher identities. Only the public
packaging README was refreshed from the original build ZIP; the other 35
members are unchanged. The packaged README's SHA-256 is
`dd2771553da477d10bcb47d0913cc407c223521c89fd3033577a37e4517a294d`.
The final ZIP is a documentation-only repack, not the original compiler/build
output. Both the original package and receipt remain preserved. An earlier
documentation package identified by prefix `7782` also remains historical;
it is not the delivery artifact.

Documentation-only closeout differences after the frozen runtime snapshot do
not change the executable or require recompilation.

## Deployment and acceptance boundary

After the software and packaging checkpoint, the operator requested deployment
to the identified Scribe 2022 / firmware 5.19.5. ARM `8da54b9b111d`, launcher
`71316711`, and guard `689ef367eabe` were installed and started through the
ordinary entry. All 36 public members matched the reviewed package; six
protected data/configuration files were unchanged across public replacement.
There was no reset, conversion, or forced logout. The
[physical deployment record](PHYSICAL_TESTING.md#qr-login-deployment-and-manual-trial-closeout)
separates startup and restoration evidence from authenticated acceptance.

At the authorized handoff, the application had already exited, so no stop
signal was sent. Native Home, statusbar, and the window manager were healthy;
application processes and the launcher lock were absent. All device transports
ended before the matching owner/token lease release at 2026-11-15 02:41:40
+08:00. No later physical access belongs to this task window. The preceding
introduction-preview acceptance is not reassigned to the Login redesign.

Physical layout, touch behavior, real scanning/verification, and another
model/firmware combination remain separate acceptance conditions. The current
software fixtures establish only their recorded presentation and offline
controller behavior.
