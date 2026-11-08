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

DU requests set the reviewed `EPDC_FLAG_FORCE_MONOCHROME` value of 2. A2 falls back to AUTO when the driver family is not yet known or is unsupported. No new hardware inversion or pen support is introduced.

The application adapter maps UI and reading partial requests to AUTO, press feedback to DU, and flashing/full requests to GC16. A2 is exposed for a future measured policy but is not selected by the application's initial adapter.

## Changed files

- `src/refresh.rs`: producer handle, bounded queue, region resolution, and controlled tests.
- `src/clipboard.rs`: two application-local clipboard slots and pure standard-library tests.
- `src/touch.rs`: convert normalized panel pixels to Slint logical coordinates using the current window scale after undoing rotation.
- `src/lib.rs`: public refresh, render-mode, and startup APIs, with handle preservation across backend typestates.
- `src/platform.rs`: queue wiring, render-mode changes, suspend eligibility, post-render execution, bounded startup orientation queries, and termination state.
- `src/render_control.rs`: software dark-mode requests, canonical-buffer conversion, and controlled rendering tests.
- `src/splash.rs`: an exclusive temporary framebuffer owner for startup text, stages, and low-frequency loading feedback.
- `src/wakeup.rs`: make the existing pipe nudge usable inside this crate and retry interrupted writes.
- `src/framebuffer/mod.rs`: execute resolved plans through the existing driver and map driver-specific waveforms.
- `src/framebuffer/ffi.rs`: reviewed DU/A2 and monochrome constants.
- `Cargo.toml`: omit the unavailable example workspace and align font dependencies with Slint 1.17.1's actual resolved versions and dlopen requirements.

## Resident managed power handoff

The project adds a process-local managed-sleep request and acknowledgement API
to the existing render handle. Suspension cancels the Slint pointer, drops the
touch descriptor to release `EVIOCGRAB`, and gates every framebuffer write and
refresh. Event-loop timers remain available for the application/supervisor
handshake; the supervisor subsequently stops the entire application process.
This does not enable `WakeSchedule` or write a new kernel suspend policy.

Wake restores the existing process and its retained canonical RGB frame. The
backend reopens touch input, resets its gesture state, repaints with a new
buffer, writes every panel row even when the previous dirty region was empty,
and submits a checked full GC16 update before acknowledging `Running`.
Acknowledgement establishes successful update submission, not unbounded
waiting for physical panel completion. Rapid request reversals retain their
desired state, and failures release input before reporting termination.

These are project-owned additions. `UPSTREAM_REVISION` and the dependency
licenses remain unchanged. Controlled regressions and physical Kindle results
must be recorded separately in the application's verification documents.

## Application-local clipboard

The native platform implements [Slint 1.17.1's public platform trait](https://github.com/slint-ui/slint/blob/v1.17.1/internal/core/platform.rs): `Platform::set_clipboard_text(&self, text: &str, clipboard: Clipboard)` and `Platform::clipboard_text(&self, clipboard: Clipboard) -> Option<String>`. `DefaultClipboard` and `SelectionClipboard` use separate owned text slots. Unknown future clipboard kinds are ignored or return `None`.

The store lives only in this application process's RAM and is dropped with the platform. It does not read or write files, call a Kindle service, or provide a system-wide clipboard across applications. Public Slint `TextInput.copy()` and `TextInput.paste()` use these platform methods without a Rust private API or a firmware-specific contract.

## Native touch coordinates

Touch controller axes remain normalized to the physical framebuffer dimensions. Press, move, and release events use one conversion that first undoes the existing panel rotation and then divides both coordinates by the current Slint window scale factor. This preserves the upstream rotation mapping and contact-slot tracking while aligning native touch events with a scaled logical window. Logical pointer events dispatched directly by the application already use logical coordinates and do not pass through this conversion.

The added regression vectors cover a noncentral point and the logical origin at scale 2 for rotations 0, 90, 180, and 270 degrees. They exercise the pure coordinate conversion without opening an input device or framebuffer. Controlled-test and firmware-emulator results are recorded below; these vectors alone do not establish physical-device support.

## Software display mode

`KindleBackend::render_handle()` returns a thread-safe `RenderHandle`. Its `set_dark_mode(bool)` method wakes the event loop after a mode change and rejects requests after termination. The renderer retains a canonical light RGB buffer and applies inversion only while converting pixels for grayscale framebuffer output. Text, images, overlays, and later dirty regions therefore use the same mode without repeatedly inverting stored pixels.

A mode change rewrites the complete framebuffer and requests a full GC16 refresh, even when Slint has no dirty region or reports only a partial region. The application persists its appearance preference through its own platform boundary. This extension does not implement a hardware inversion ioctl or modify the stock Kindle dark-mode preference.

## Startup framebuffer ownership

`StartupSplash` displays application-supplied raster text and stage images before the main font initialization completes. Its worker updates loading dots at 0.8 Hz within a local region. Startup and the normal renderer never write the framebuffer concurrently: `pause()` stops and joins the worker before backend installation changes the process environment, `resume()` resumes startup feedback, and `finish()` joins it before the application event loop takes over. The splash uses the same software display mode as the application.

Startup orientation lookup has a 500 ms polling budget, validates the supported orientation values, and reaps its child process on success, failure, or timeout. These changes do not alter framebuffer geometry or claim a physical-panel completion deadline.

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
orientation or deadline change. These changed tests have not been executed;
verification belongs to the next frozen compilation and complete-suite result.

## Atomic descriptor inheritance

Touch-device discovery opens its temporary event descriptor with `O_CLOEXEC`,
matching the final grabbed descriptor. This prevents a concurrent native helper
exec from retaining a discovery handle after its scanner closes it.

The Linux wakeup pipe uses `pipe2(O_CLOEXEC | O_NONBLOCK)` so there is no creation
window before the existing flag checks. Both descriptors still immediately enter
`OwnedFd` ownership and close on every subsequent error path. This patch makes
no additional legacy-kernel or physical-device support claim.

## Verification boundary

Controlled tests exercise empty/outside rectangles, integer bounds, rotation, region merging, full requests, default AUTO behavior, queue saturation, cross-thread producers, termination, and driver-specific waveform mapping without opening a Kindle framebuffer. Remote ARM cross-compilation checks the code and ABI layout assertions for the intended build target. Neither test class establishes a physical model/firmware support claim, ghosting quality, or panel timing.

On 2026-11-03, `cargo test --lib --manifest-path vendor/slint-backend-kindle/Cargo.toml` passed all 11 dependency tests on `test-env`, including nine refresh/driver tests and two retained upstream touch tests. The queue tests used the actual nonblocking wakeup pipe. No framebuffer was opened by these tests.

After adding the clipboard extension, five additional pure standard-library tests passed remotely with `rustc --test src/clipboard.rs`. They cover initial emptiness, separation of selection and explicit copy, owned copy/paste values, replacing old text with empty text, and preserving Unicode and paragraph boundaries. These tests use no display or firmware service.

After the native touch-scale fix, the complete standalone dependency suite passed 17 tests on `test-env`, including the new four-rotation/scale-2 coordinate vectors. The pinned PW6 firmware emulator independently reproduced the old physical-coordinate mismatch and exercised corrected QEMU-injected taps through the guest input driver. These are separate controlled-test and emulator observations; neither establishes physical-device input acceptance.

The current standalone dependency suite passed 31 tests on `test-env`, including software display-mode conversion, complete rewrites on mode changes, startup rendering, and bounded orientation queries. The earlier counts above retain their historical scope. Current physical and firmware-emulator observations, with their separate executable identities and outstanding gates, are recorded in [Physical testing](../../docs/PHYSICAL_TESTING.md).
