# QR Login UI

Accepted: 2026-11-15 (Asia/Shanghai).

Status: the approved V3 layout and larger QR code are implemented in source.
The current host Login and QR scopes passed, covering 51 unique parent cases
after case-ID deduplication. The two software-rendered profiles passed 85
interface checks. The ARM build/package retry and final documentation-only
repack passed. No firmware-guest or physical Kindle execution has been
performed for this Login implementation.

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

## Frozen verification snapshot

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

This iteration has not accessed a physical Kindle, started a firmware guest,
acquired or modified the shared device lease, or changed user data. The
physical installation remains the preceding ARM `64e66a8a243c`, launcher
`71316711`, and guard `689ef367eabe` on Scribe 2022 / firmware 5.19.5. The
operator's acceptance of that installation belongs to
[the introduction-preview deployment](PHYSICAL_TESTING.md#introduction-preview-deployment-and-operator-acceptance).
It is not acceptance of this Login redesign.

Physical layout, touch behavior, real scanning/verification, and another
model/firmware combination remain separate acceptance conditions. The current
software fixtures establish only their recorded presentation and offline
controller behavior.
