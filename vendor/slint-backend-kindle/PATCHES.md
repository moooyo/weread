# Project-owned Kindle backend patches

## Provenance

The baseline is [`KindleTweaks/slint-kindle-backend-patch` at `03aa48d3531e9eae7b360bdc24e90a72e9f307a4`](https://github.com/KindleTweaks/slint-kindle-backend-patch/tree/03aa48d3531e9eae7b360bdc24e90a72e9f307a4). `UPSTREAM_REVISION` records the same identity. The original `LICENSE-MIT` and `LICENSE-APACHE` files are preserved without changes. This dependency's license metadata does not select the application's project license.

Only the library sources, manifest, README, and licenses are vendored. Example applications, screenshots, and unrelated repository material are omitted. The manifest's example workspace stanza is removed because those examples are not part of this source distribution. The upstream Slint `=1.17.1` requirement and Rust 1.92 minimum are retained.

The upstream manifest declares `fontique 0.11` and `fontdb 0.24`, whereas Slint 1.17.1 actually resolves `fontique 0.10` and `fontdb 0.23`. The compatibility patch changes the direct declarations to those actual versions. `fontique`'s `fontconfig-dlopen` feature then applies to the same instance used by `i-slint-common`, enabling `yeslogic-fontconfig-sys/dlopen` and avoiding a required native `libfontconfig` link. The application supplies its fonts explicitly. Direct dependencies with `default-features = false` do not turn off transitive default features; `fontdb`'s fontconfig parser is Rust code rather than a native fontconfig link.

## Public extension

`KindleBackend::refresh_handle()` returns a cloned `RefreshHandle` that is `Send + Sync`. `RefreshHandle::request()` accepts:

- `RefreshWaveform::{Auto, Gc16, Du, A2}`;
- a `flashing` flag;
- `RefreshArea::{Dirty, Full, Rect(PhysicalRect)}`.

Rectangles use physical window pixels before render rotation. Empty rectangles are rejected immediately. Other rectangles are intersected with the actual window geometry and rotated into panel coordinates before an update is submitted. Signed positions and unsigned dimensions use widened arithmetic, preventing wraparound when a caller supplies a negative origin or a very large size.

A successful call means that the request entered the producer queue. It is not a completion receipt or evidence of panel quality. Requests after event-loop termination are rejected. The queue holds at most 32 requests; overload collapses to one conservative GC16 full-screen request rather than growing memory without a bound.

## Rendering order

The producer only appends to a mutex-protected queue and nudges the existing wakeup pipe. It does not open a framebuffer or call an ioctl.

On the event-loop thread, application closures, touch events, and Slint timers run first. The backend snapshots the pending requests, renders the current scene, and writes all dirty pixels through its existing framebuffer object. Only after `draw_if_needed()` returns does it resolve pending regions and call the existing detected Legacy, Rex, Zelda, or MTK driver implementation. Requests arriving during rendering stay queued for the next awakened iteration.

The submitted region includes both valid requested rectangles and any dirty pixels written in the same frame. Conflicting requests choose the strongest requested waveform, and a flashing request uses GC16. A rotation change retains the original full-screen GC16 path. Without any explicit request, rendering retains the original AUTO partial update for the dirty region.

## ABI constants

The baseline already defines GC16 as 2, AUTO as 257, and the four update ioctl structures. The additional DU and A2 values are reviewed against [`koreader-base` at `6e4bc81af4e04f78d81677a323a780a71b29702a`, `ffi/mxcfb_kindle_h.lua`](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/mxcfb_kindle_h.lua):

| Waveform | Legacy | Rex / Zelda / MTK |
| --- | --- | --- |
| DU | 1 | 1 |
| GC16 | 2 | 2 |
| A2 | 4 | 6 |

DU requests set the reviewed `EPDC_FLAG_FORCE_MONOCHROME` value of 2. A2 falls back to AUTO when the driver family is not yet known or is unsupported. This refresh extension adds no pen support. Renderer-owned inversion uses the separate standard framebuffer mode API described below.

The application adapter maps UI and reading partial requests to AUTO, press feedback to DU, and flashing/full requests to GC16. A2 is exposed for a future measured policy but is not selected by the application's initial adapter.

## Changed files

- `src/refresh.rs`: producer handle, bounded queue, region resolution, and controlled tests.
- `src/clipboard.rs`: two application-local clipboard slots and pure standard-library tests.
- `src/touch.rs`: convert normalized panel pixels to Slint logical coordinates using the current window scale after undoing rotation.
- `src/lib.rs`: public refresh, render-mode, and startup APIs, with handle preservation across backend typestates.
- `src/platform.rs`: queue wiring, render-mode changes, suspend eligibility, post-render execution, bounded startup orientation queries, and termination state.
- `src/render_control.rs`: night-mode configuration and requests, native-baseline observation, canonical-buffer conversion, and controlled rendering tests.
- `src/sleep_presentation.rs`: token-bound completion of the application-owned sleep frame.
- `src/splash.rs`: an exclusive temporary framebuffer owner for startup text, stages, and low-frequency loading feedback.
- `src/wakeup.rs`: make the existing pipe nudge usable inside this crate and retry interrupted writes.
- `src/framebuffer/mod.rs`: execute resolved plans through the existing driver, map driver-specific waveforms, and perform checked framebuffer mode I/O.
- `src/framebuffer/night_mode.rs`: hardware opt-in, software compensation, native-sleep baseline, startup restoration, and mocked mode-I/O regressions.
- `src/framebuffer/ffi.rs`: reviewed DU/A2, monochrome constants, and the standard Linux framebuffer mode ABI.
- `Cargo.toml`: omit the unavailable example workspace and align font dependencies with Slint 1.17.1's actual resolved versions and dlopen requirements.

## Resident managed power handoff

The project adds a process-local managed-sleep request and acknowledgement API
to the existing render handle. Before suspension is acknowledged, the renderer
restores the observed native grayscale baseline and rewrites retained canonical
RGB pixels with the matching software compensation. The raw framebuffer and
native mode then describe the same physical app frame for the launcher's frame
guard. This preparation submits no extra panel refresh. Suspension then cancels
the Slint pointer, drops the touch descriptor to release `EVIOCGRAB`, and gates
every further framebuffer write and refresh. Event-loop timers remain available for the application/supervisor
handshake; the supervisor subsequently stops the entire application process.
This does not enable `WakeSchedule` or write a new kernel suspend policy.

Wake restores the existing process and its retained canonical RGB frame. The
backend reopens touch input, observes the current native framebuffer baseline,
reapplies the retained app night request, resets its gesture state, repaints with a new
buffer, writes every panel row even when the previous dirty region was empty,
and submits a checked full GC16 update before acknowledging `Running`.
Acknowledgement establishes successful update submission, not unbounded
waiting for physical panel completion. Rapid request reversals retain their
desired state, and failures release input before reporting termination.

These are project-owned additions. `UPSTREAM_REVISION` and the dependency
licenses remain unchanged. Controlled regressions and physical Kindle results
must be recorded separately in the application's verification documents.

## Checked sleep presentation

The application-owned sleep cover uses a separate token/acknowledgement API on
the render handle. The event loop forces a complete Slint render, submits a
checked full update, and checks panel completion before acknowledging the token
captured before that draw. Newer requests cannot be acknowledged by an older
frame. Native handoff cancels unfinished presentation requests, and event-loop
termination invalidates all tokens. This stage does not change `powerd`, native
lock settings, or the existing durable managed-sleep acknowledgement.

Each accepted EPDC submission has its own nonzero update marker. A failed
submission consumes its attempted marker without replacing the last successful
one; waits identify that successful submission and cache only confirmed
completion. Sleep-cover presentation also waits for the preceding update before
rewriting mapped pixels. This follows the reviewed
[`koreader-base` marker ordering](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L61-L69).

The application hides its cover before requesting managed resume, so the first
full wake render uses the retained Reader state. The original native-grayscale
normalization and input-drain boundaries remain in force. See the application's
[`docs/SLEEP_COVER.md`](../../../docs/SLEEP_COVER.md) for the pinned KOReader
comparison and the separate software/device acceptance boundaries.

## Device-gated image Y4 processing

The project adapts the reviewed behavior of BiliComics
[`7241a85fac8aa032b79f4e7f2f6eb29aacdbebd3`](https://github.com/moooyo/bilicomics/commit/7241a85fac8aa032b79f4e7f2f6eb29aacdbebd3)
to mixed text/image pages. The existing MIT/Apache notices and upstream baseline
remain unchanged; no application license or new runtime dependency is selected.

`RenderHandle::set_hardware_y4_for_frame` publishes bounded raw-image regions
with the matching UI scene. The backend snapshots actual rendered ownership,
keeps canonical RGB, and maps crop-local picture phase through render rotation.
CPU fallback converts only picture spans. A native Y4 flag applies to its whole
update rectangle; independent local updates outside pictures retain the normal
refresh path. Sleep/native snapshots software-complete retained raw pictures
without replacing pending or retained scene identity.

Eligibility uses a fixed-width, bounded hardware identity read and actual MTK
eight-bit metadata, never a UI profile or resolution. The MTK Y4 flag is `0x4000`
with zero dither/quant fields; native inversion uses the conservative software
path. Typed errno results distinguish definite rejection from unknown delivery.
Only definite rejection permits old-marker completion, software rewriting, and
new-marker submission. An uncertain update or wait latches all later mapped
writes and submissions, including across mode revisions, and ends the loop.

See [`docs/HARDWARE_Y4_DITHERING.md`](../../../docs/HARDWARE_Y4_DITHERING.md) for
the fixed primary-source references, current storage boundary, software results,
and the separate physical acceptance gate.

## Application-local clipboard

The native platform implements [Slint 1.17.1's public platform trait](https://github.com/slint-ui/slint/blob/v1.17.1/internal/core/platform.rs): `Platform::set_clipboard_text(&self, text: &str, clipboard: Clipboard)` and `Platform::clipboard_text(&self, clipboard: Clipboard) -> Option<String>`. `DefaultClipboard` and `SelectionClipboard` use separate owned text slots. Unknown future clipboard kinds are ignored or return `None`.

The store lives only in this application process's RAM and is dropped with the platform. It does not read or write files, call a Kindle service, or provide a system-wide clipboard across applications. Public Slint `TextInput.copy()` and `TextInput.paste()` use these platform methods without a Rust private API or a firmware-specific contract.

## Native touch coordinates

Touch controller axes remain normalized to the physical framebuffer dimensions. Press, move, and release events use one conversion that first undoes the existing panel rotation and then divides both coordinates by the current Slint window scale factor. This preserves the upstream rotation mapping and contact-slot tracking while aligning native touch events with a scaled logical window. Logical pointer events dispatched directly by the application already use logical coordinates and do not pass through this conversion.

The added regression vectors cover a noncentral point and the logical origin at scale 2 for rotations 0, 90, 180, and 270 degrees. They exercise the pure coordinate conversion without opening an input device or framebuffer. Controlled-test and firmware-emulator results are recorded below; these vectors alone do not establish physical-device support.

## Renderer-owned night mode

`KindleBackend::render_handle()` returns a thread-safe `RenderHandle`.
`set_night_mode(bool)` requests the application's physical night appearance;
`set_dark_mode(bool)` remains a compatibility alias with the same desired-mode
semantics. A successful call acknowledges the queued preference, not a physical
panel update. The event loop applies it without releasing touch, entering managed
sleep, stopping the process, or writing LIPC/dynconf preferences. The application
retains its own persisted preference independently of the native owner's theme.

`configure_night_mode(NightModeConfig)` must precede the event loop. The default
is `SoftwareOnly`. `HardwareIfSupported` is an explicit model/firmware opt-in and
still requires actual startup metadata with 8 bits per pixel and grayscale 1 or
2. The hardware path follows [`koreader-base` at
`1c9682506757d5fd30cfa01c5698929dcc35340f`,
`ffi/framebuffer_linux.lua`, lines 157-200](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_linux.lua#L157-L200):
change only `grayscale` between 1 and 2 through Linux `FBIOPUT_VSCREENINFO`
(`0x4601`). The complete fresh `fb_var_screeninfo` is copied before that single
field is changed. The 160-byte layout and grayscale/rotation offsets have compile
time assertions. GETVAR (`0x4600`) and GETFIX (`0x4602`) verify the mapped layout
before and after each write, and the readback must confirm the requested grayscale
without changing rotation. A write or readback failure ends the event loop with
an error; an uncertain hardware write is never reported as applied.

Unsupported or unapproved framebuffers use software inversion. All paths retain
canonical light RGB and convert using `desired_dark XOR observed_kernel_inverse`.
Text, images, overlays, and later dirty regions therefore invert exactly once,
including when native UI had already enabled kernel inversion at launch. A mode
transition rewrites every visible pixel and submits one checked full GC16 update,
even when the required software transform is unchanged or Slint reports no dirty
region. Ordinary night requests do not create an input/presentation transition.

For the MTK update ABI, final request packing maps a full GC16 request to GCK16
(`8`) only when hardware inversion was explicitly enabled and the current
observed framebuffer state is inverted. The native baseline does not select this
waveform. Packing applies this mapping even to the first uncached MTK ioctl
probe; its ioctl number, probe order, and flags remain unchanged. Global
`grayscale = 2` already provides inversion, so no EPDC inversion flag is added.
Day mode, software fallback, other waveforms, partial updates, and Legacy/Rex/Zelda
requests keep their existing values. The reviewed MTK definitions are GC16 `2`,
GCK16 `8`, and GLKW16 `9` in [`koreader-base` at
`1c9682506757d5fd30cfa01c5698929dcc35340f`,
`ffi/mxcfb_kindle_h.lua`, lines 134-153](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/mxcfb_kindle_h.lua#L134-L153).
The full-refresh mapping follows [`ffi/framebuffer_mxcfb.lua`, lines
433-455](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L433-L455),
[lines 978-981](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L978-L981),
and [lines 1072-1100](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L1072-L1100).
GLKW16 partial policy remains future work. This source review and the controlled
packing regressions do not establish a physical Kindle pass.

`native_output_inverted()` returns the latest observed native baseline, or `None`
before the framebuffer is initialized. That baseline is separate from the
renderer-owned temporary hardware mode. A real resident wake reobserves native
metadata before reapplying app night mode. The original startup grayscale is
retained separately for normal process exit and unwinding panic cleanup, matching
the restoration approach reviewed in [`koreader` at
`9040116`, `frontend/device/generic/device.lua`](https://github.com/koreader/koreader/blob/9040116/frontend/device/generic/device.lua).
Framebuffer destruction best-effort restores only that original grayscale using
fresh metadata while the app owns display mode, then unmaps memory. A successful
native-baseline and compensated-raw rewrite commits a yield before Suspended is
acknowledged. Exit after that yield issues no mode write, preserving any later
native grayscale change. A failed baseline restoration or unwinding raw rewrite
retains app cleanup authority until a later successful yield. Wake observation
retains native ownership; reapplying app night mode reacquires it. The original
startup value survives this ownership handoff for a later active app exit.
Uncatchable signals, `SIGKILL`, and aborting
panics cannot execute Rust cleanup; this patch does not add a global recovery
mechanism or claim those exits restore native state.

## Startup framebuffer ownership

`StartupSplash` displays application-supplied raster text and stage images before the main font initialization completes. Its worker updates loading dots at 0.8 Hz within a local region. Startup and the normal renderer never write the framebuffer concurrently: `pause()` stops and joins the worker before backend installation changes the process environment, `resume()` resumes startup feedback, and `finish()` joins it before the application event loop takes over. The splash uses the same software display mode as the application.

Startup images also compensate for the actual kernel inverse observed by their exclusive framebuffer owner. `StartupSplash::show()` receives app desired mode and never enables hardware inversion. Startup orientation lookup has a 500 ms polling budget, validates the supported orientation values, and reaps its child process on success, failure, or timeout. These changes do not alter framebuffer geometry or claim a physical-panel completion deadline.

### Orientation test process ownership

The inherited-stdout regression now joins its real `sleep 5 & printf D` reply
helper to a private group held by a separate 20-second anchor, matching the
application's native-command fixture pattern. Group cleanup requires the
anchor's original birth, direct parent, group membership, and live state;
the reaped reply shell and its finite descendant do not authorize a signal.
An expired or mismatched anchor cannot authorize cleanup of a reused group.
The anchor also verifies its own birth and group before finite TTL cleanup.

The test retains its `None` result and under-one-second assertion, and checks
successful direct-shell exit, reaping, and parent reply-pipe release. One small
identity regression covers wrong birth, parent/group mismatch, and an actually
exited owned anchor. This is a test-fixture safety correction, with no production
orientation or deadline change. These cases are included in the 2026-11-12
complete 95-test remote software run recorded below.

## Atomic descriptor inheritance

Touch-device discovery opens its temporary event descriptor with `O_CLOEXEC`,
matching the final grabbed descriptor. This prevents a concurrent native helper
exec from retaining a discovery handle after its scanner closes it.

The Linux wakeup pipe uses `pipe2(O_CLOEXEC | O_NONBLOCK)` so there is no creation
window before the existing flag checks. Both descriptors still immediately enter
`OwnedFd` ownership and close on every subsequent error path. This patch makes
no additional legacy-kernel or physical-device support claim.

## Verification boundary

On 2026-11-12, the complete standalone backend suite passed 95 tests on
`test-env`, including the checked sleep-presentation API, independent EPDC
markers, preceding-update ordering, failed submissions/completion, and existing
input, orientation, and night-mode cases. The root lock seeded the disposable
standalone lock; offline resolution and explicit local patch overrides kept the
reviewed sources. The log is
`test-env:/tmp/weread-cover-sleep-20261112-r1/backend-tests.log`.
These are controlled software results; no Kindle framebuffer or physical panel
was exercised. Hardware acceptance remains in the application documents.

The renderer-owned night-mode patch adds mocked framebuffer regressions for
hardware 1/2 transitions, unchanged nonmode metadata, software fallback, physical
pixel equivalence without double inversion, startup restoration, failed writes
and readbacks, changed layouts/rotation, coherent native-sleep snapshots, fresh
wake baselines, retained app requests, configuration sealing, and preserved input
ownership during ordinary night requests. These additions are included in the
complete remote software run above. No physical model/firmware acceptance
follows from that run.

Controlled tests exercise empty/outside rectangles, integer bounds, rotation, region merging, full requests, default AUTO behavior, queue saturation, cross-thread producers, termination, and driver-specific waveform mapping without opening a Kindle framebuffer. Remote ARM cross-compilation checks the code and ABI layout assertions for the intended build target. Neither test class establishes a physical model/firmware support claim, ghosting quality, or panel timing.

On 2026-11-03, `cargo test --lib --manifest-path vendor/slint-backend-kindle/Cargo.toml` passed all 11 dependency tests on `test-env`, including nine refresh/driver tests and two retained upstream touch tests. The queue tests used the actual nonblocking wakeup pipe. No framebuffer was opened by these tests.

After adding the clipboard extension, five additional pure standard-library tests passed remotely with `rustc --test src/clipboard.rs`. They cover initial emptiness, separation of selection and explicit copy, owned copy/paste values, replacing old text with empty text, and preserving Unicode and paragraph boundaries. These tests use no display or firmware service.

After the native touch-scale fix, the complete standalone dependency suite passed 17 tests on `test-env`, including the new four-rotation/scale-2 coordinate vectors. The pinned PW6 firmware emulator independently reproduced the old physical-coordinate mismatch and exercised corrected QEMU-injected taps through the guest input driver. These are separate controlled-test and emulator observations; neither establishes physical-device input acceptance.

The current standalone dependency suite passed 31 tests on `test-env`, including software display-mode conversion, complete rewrites on mode changes, startup rendering, and bounded orientation queries. The earlier counts above retain their historical scope. Current physical and firmware-emulator observations, with their separate executable identities and outstanding gates, are recorded in [Physical testing](../../docs/PHYSICAL_TESTING.md).
