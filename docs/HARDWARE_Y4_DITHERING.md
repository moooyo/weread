# Hardware Y4 image processing

Recorded: 2026-11-13 (Asia/Shanghai).

## Objective and existing behavior

The operator requested that WeRead inspect the device-gated hardware Y4 work
shared by BiliComics and implement missing capability in this application's own
architecture. The inspected baseline is WeRead `6a1d070`.

The baseline already retained canonical RGB in the renderer, used independent
EPDC update markers, checked sleep presentation, and restored native display
ownership. It did not identify Y4-capable hardware, request hardware dithering,
or preserve Gray8 chapter/article pictures: acquired pictures were rounded to
sixteen levels before display. Adding an ioctl flag alone could not recover the
discarded luminance values.

The current implementation preserves the existing default: `off` means
sixteen-level rounding without Bayer dithering. `software` selects a four-by-four
Bayer conversion, and `hardware` requests native processing with an explicit
software fallback. This is not a claim that the panel supports 256 independently
addressable, stable physical reflectance levels.

## Current preference and cache formats

`AppPreferences.image_dithering` is one required current field with the values
`off`, `software`, and `hardware`. First use defaults to `off`. An explicit unknown
method resolves to software; UI choice assignment accepts only known values.
There was no older WeRead dithering boolean, and none is introduced as a
compatibility alias.

The existing pre-release policy remains in force: preference snapshots missing
required current fields are rejected, without rewriting or deleting private
data. Use a fresh data directory for an unsupported development snapshot; no
automatic database or preference migration is added.

Chapter image assets now use the single current `book-image:v2:` namespace,
conversion version 2, and an explicit 256-level representation. The prepared
cache's gray-level field can represent 256. Existing sixteen-level assets are
not reinterpreted as Gray8 through an older-key alias. Article canonical keys bind
article identity, image identity, URL, and the current Gray8 conversion format.
The network source and authentication boundaries remain unchanged.

Canonical PNGs preserve eight-bit luminance after white-background alpha
compositing. Download preparation remains independent of mutable view
preferences. Book-list covers and temporary sleep covers retain their separate
fixed sixteen-level preparation and do not request Y4.

## Actual image-scene ownership

Display preparation resizes or crops a verified canonical picture to its actual
physical image dimensions before applying Off or Software processing. Physical
dimensions use the window's real scale, not a design profile's assumed pixel
count. A Hardware request prepares both raw Gray8 and its software replacement
before publishing either. There is one retained Reader or Article scene, not an
unbounded cache of historical display variants.

The picture method, source identity, target geometry, and prepared model belong
to that published scene. A pending preference or failed replacement cannot
change the old visible picture's processing. Hardware availability may change
after startup or driver rejection; selecting the already prepared replacement
requires no SQL, file loading, or image decoding.

Only complete, unclipped picture rectangles with integral physical pixel edges
are eligible for raw processing. Unsafe geometry uses software pictures. The
rectangles exclude captions and body text. Controls, selection, modal dialogs,
toasts, and the opaque sleep screen use prepared software images or the sleep
screen's independent fixed palette. Leaving the reading scene clears raw-image
ownership. Wake restores the retained actual scene, not a pending preference.

Existing transport, decode, and preparation limits remain. A staged display
scene admits its total picture area against the viewport, caps the viewport at
eight megapixels, and caps primary plus fallback RGB storage at 64 MiB. Source
decoding remains bounded by 8 MiB encoded input, 8192-pixel edges, sixteen
megapixels, and 64 MiB decoder allocation. The task PNG preparation cache remains
8 MiB. Metadata ownership continues through the existing layout admission.

## Hardware and update policy

Eligibility requires the real hardware identity, a reviewed monochrome Scribe
2022/2024 code, an actual eight-bit framebuffer, and the MediaTek update
interface. A UI design profile, resolution, or successful generic ioctl is not
sufficient. Paperwhite 5, unknown identities, color framebuffers, and unsupported
interfaces use software processing. The existing device/firmware launch
allowlist is not broadened.

The identity reader consumes exactly the first sixteen `/proc/usid` bytes using
a bounded complete read and strict uppercase/digit validation. A seventeenth NUL
transport byte is not consumed. Short reads, read errors, embedded NUL, malformed
identities, and unknown codes cannot establish support. Complete serial values
are neither retained nor logged.

The MTK Y4 enable flag is `0x4000`; `0x8000` is REGAL. Y4 requests use GC16 with
`dither_mode=0` and `quant_bit=0`. Refresh regions containing raw pictures expand
outward to eight-pixel boundaries and are clipped to the framebuffer. A local
update entirely outside the raw pictures retains its original AUTO/DU/A2 policy.
An intersecting or full update uses the conservative grayscale-safe path.

The native Y4 flag applies to the submitted update rectangle. Consequently, a
full mixed page also sends its glyph pixels through that native processing.
Only the CPU conversion is restricted to picture spans; no per-pixel hardware
mask is claimed. Ordinary scenes without raw pictures retain their existing
refresh policy.

This revision conservatively uses software picture processing while actual
hardware inversion is enabled. It does not assume that the previously accepted
GCK16 night waveform can be combined with Y4. Existing nonraw night-mode and
single output-polarity compensation rules remain unchanged.

## Submission failure and native handoff

Submission results retain errno rather than collapsing every failure into a
boolean. Only definite unsupported-parameter errors (`EINVAL`, `ENOTTY`, or
`EOPNOTSUPP`) permit hardware fallback. The renderer first completes the preceding
accepted update, then rewrites the current raw picture spans from canonical
RGB using software Bayer, and submits a fresh update with a new marker.
Hardware rejection remains disabled for that session; simply removing the Y4
flag from unprocessed Gray8 is not a fallback.

Uncertain submission or completion errors, including `EIO`, `EINTR`, and timeout,
latch delivery failure. They do not authorize a new ABI probe, an in-flight
memory rewrite, or blind retry. Later scene revisions cannot clear the latch;
the display loop exits through its existing native-restoration path.

The renderer retains the scene that actually produced its RGB frame separately
from pending application requests. Image-local Bayer phase follows the final
physical-sized picture through the renderer's inverse rotation mapping, rather
than restarting at a dirty rectangle or clipped edge. Normal glyph pixels
outside picture spans retain the existing conversion.

Before native handoff, raw picture spans are software-completed under the native
baseline so the frame guard does not receive pixels that depend on a missing
Y4 flag. The canonical RGB and retained scene remain available for the complete
wake render. Existing portrait calibration, input draining, nonce/generation
checks, sleep ownership, and startup/native appearance restoration remain in
force. No second image rotation is added for Down holding or Down scanout.

## Fixed references and evidence boundaries

The behavior reference is BiliComics
[`7241a85fac8aa032b79f4e7f2f6eb29aacdbebd3`](https://github.com/moooyo/bilicomics/commit/7241a85fac8aa032b79f4e7f2f6eb29aacdbebd3).
Selected vendor logic is adapted to WeRead's mixed text/image scenes, existing
renderer, and current storage rules. There is no BiliComics runtime dependency.
The existing backend MIT/Apache notices and upstream baseline remain; this
does not select a project license.

Primary implementation references are the
[FBInk MTK ABI](https://github.com/NiLuJe/FBInk/blob/ded92fa510c957e0157c92ae83bd5c51af710c05/eink/mtk-kindle.h),
[KOReader MTK submission and alignment](https://github.com/koreader/koreader-base/blob/f64c3eb11049e9a75189f186d7345d97c07d87b9/ffi/framebuffer_mxcfb.lua),
and [KOReader device policy](https://github.com/koreader/koreader/blob/7c38cf104cb9ea89df243e47ec820ef4aa97cab8/frontend/device/kindle/device.lua).
Interface facts were checked against those pinned sources.

BiliComics reports scoped R68 acceptance on Scribe 2022 / firmware 5.19.5 for
ARM `4dabdd791462ceb0bf2d18fab96ccb5a42d7d3f41c62baaa0b2e7abe9cd2d5f5`.
Its GC16/Y4 submission was accepted and its live-bound framebuffer input retained
all 256 values, including values outside the sixteen-tone palette. That is
BiliComics evidence, not verification performed by WeRead. It is not post-MDP
readback, a panel photograph, or measurement of 256 physical reflectance levels.

## Independent WeRead verification

All authoritative application builds and execution run on `test-env`. Source
preparation and generated-image inspection on the operator's Windows machine are
separate from application verification. A quoting error initially caused
unsuccessful local tool invocations; no local application check ran, and later
commands execute uploaded remote scripts.

The host was already short of filesystem space, with full swap. Verification
therefore uses a task-owned mount namespace and overlay, borrowing historical
Cargo/target/Zig caches read-only. Source, logs, mutable upper layers, and
transient service limits belong only to this run. No other project's cache or
process is removed. An early ENOSPC build and memory-interrupted build remain
failures, not passing checks. Final development/test compilation uses a
package-local 256 codegen units override and a bounded transient service; this
does not change application logic, dependency versions, or test assertions.

| Check | Result | Scope |
| --- | --- | --- |
| Backend | 137 passed, none ignored | Strict identity reads, raw scene ownership, rotation/phase, picture-only CPU conversion, native snapshots, MTK flags, checked markers, explicit rejection, and uncertain-delivery latching. |
| Application | 1,643 library tests and 52 Main tests passed; four existing benchmarks ignored | Complete `--features kindle --all-targets` run. One earlier full run had an existing eight-second download-event timeout; it did not recur in the complete passing run. Production download code and the timeout assertion were not changed. |
| Image display | 22 passed across PW6 and Scribe desktop layouts | Real App preparation, current Gray8 source/manifest integrity, Off palette, Bayer, unsupported Hardware fallback, surrounding text, and cold restart. No native hardware capability is mocked into this runner. |
| General UI | 133 passed, none failed | Complete desktop navigation, persistence, Reader, publication, and settings cases; display controls remain visible in PW6 and Scribe captures. |
| Formatting and locale | Three manifests formatted; 477 keys complete | Performed on the designated remote worker. |
| ARM package | Release build, four package contracts, and all 36 archive members' CRCs passed | A stripped static ARM ELF32 EABI5 executable was staged; it has no dynamic section. The release build completed in 6 minutes 48 seconds after the host-tool correction below. |

The first image-display run passed 20 checks and rejected two fixture queries
that used a nonexistent `chapters.id`. The queries were corrected to the actual
`source_id` schema field; UID, SHA, anchor, palette, and pixel assertions were
retained, and the complete 22-check run then passed.

Remote sources and logs are under `test-env:/run/weread-y4-20261113-r1/`.
The final image report is
`artifacts/image-ui-final/run-20261113T123727Z-1485127/report.json` there. It uses
only public demo text and a synthetic Gray8 ramp. Its fixture executable SHA-256
is `b3450b4bd3315e92d46e94a03cf4c07ef47d7fd91553a9d8993f41a52bc98e49`.
Screenshots are desktop evidence; they are not panel photographs or post-MDP
readback.

The general UI report is
`artifacts/general-ui/run-20261113T124856Z-1501325/report.json`; its desktop
executable SHA-256 is
`4c030c79d332af7ec4b6fc449f1fe4ce075a763aa6dea50215e79fc28bf0a68f`.
The isolated image fixture uses the same reviewed vendored core, Swash, and Zeno
patches as the application. Its checked-in lock now records the actual tested
resolution, aligned with the root lock, including `async-executor` 1.14.0 and
the new direct `hex`/`sha2` uses. Relative manifest patches replace the equivalent
task-only command-line paths; no implementation or dependency version changes
in that final path adjustment. Remote locked metadata and format checks verify
the standalone fixture configuration without rerunning the passing UI suite.

A source-identity receipt compares 563 current runtime, UI, asset, vendor,
build, package, and fixture files between the operator checkout and the remote
source. UTF-8 CRLF text is normalized to LF; binary bytes compare exactly.
The final receipt has zero mismatches and source-manifest SHA-256
`6da55fc9c450ed19c4a6c67fe426adcf9f442a4194b61b016a51df46f2ed7954`.
The relative fixture patch declaration is configuration-equivalent to the
paths used by the passing fixture build. Final documentation changes are
outside this runtime-source receipt.

The first ARM attempt stopped before Rust compilation because cargo-zigbuild's
generated compiler wrapper inherited `/run`'s `noexec` policy. A task-owned
cache bind is executable only inside this run's private namespace; the host
mount and historical caches retain their policies. The original EACCES log and
the before/after execution probes are retained. The corrected ARM build and
package chain exited successfully.

Final development artifacts:

| Artifact | SHA-256 |
| --- | --- |
| ARM executable | `d04ec035d08c156a9308d559b61104acb5f4e11cf94b2adbf121e810369425e7` |
| Development ZIP | `ff26515d2f8a8df95cb034c9b9e85a41f50b7ce4b1a396ec28b4e056075d04c0` |
| Launcher | `713167110d4839f14505cd03c6b30753476772c729fe92890a86fbd0085fe6be` |
| Native frame guard | `a235b41c80a21a35c99f8798f366a4f2e921b971a606c83f3f8d69d576676806` |

The package and checksum receipt are under the remote run's `artifacts/kindle/`.
The ZIP contains 36 public members and is 68,240,358 bytes. The complete
build/package/check chain took 411.62 seconds; the main release compilation
accounts for 6 minutes 48 seconds. The native guard remains a separate glibc
ARM process, while the application ELF is static.
These are development build results, not a deployment or physical acceptance.

WeRead's own native Y4 acceptance, physical picture quality, power use, and other
model/firmware combinations remain separate device gates. No Kindle connection
or device lease is part of this software run.
