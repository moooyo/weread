# Rotation alignment with KOReader

This bounded source comparison was completed on 2026-11-09. It compares the
current WeRead Kindle rotation path with KOReader's reviewed source revisions.
It records source facts and the resulting alignment assessment; it does not
replace executable provenance or physical acceptance records.

The accepted application scope is automatic upright/upside-down holding at
0/180 degrees, with the existing independent application-rotation composition
preserved. The Scribe `screen_orientation` settings row is currently disabled:
`App::setting_rows` in [`auxiliary.rs`](../src/app/auxiliary.rs) supplies `false` and
`pending_verification`. Retaining the backend's 0/90/180/270 setter does not make
manual rotation an available or physically accepted UI feature.

The upstream sources are pinned to KOReader
[`9040116701c888889bd35233b23b83283be66745`](https://github.com/koreader/koreader/tree/9040116701c888889bd35233b23b83283be66745)
and KOReader-base
[`1c9682506757d5fd30cfa01c5698929dcc35340f`](https://github.com/koreader/koreader-base/tree/1c9682506757d5fd30cfa01c5698929dcc35340f).
WeRead locators below use repository paths and stable function or type names,
so formatting and import ordering do not invalidate them. They describe the
rotation sources read in the current working tree. Concurrent
table-of-contents, theme, and sleep work has its own acceptance
status and is not certified by this document.

## Mechanism comparison

| Layer | KOReader source fact | Current WeRead source fact | Assessment |
| --- | --- | --- | --- |
| Sensor contract | `KindleGyroTransform` converts `EV_ABS/ABS_PRESSURE`: 15 to upright, 16 to upside-down, 17 to clockwise landscape, and 18 to counter-clockwise landscape. Its `rotation_fds` filter protects against unrelated pressure devices. [Kindle adapter](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/kindle/device.lua#L1541-L1575) | `decode_portrait` in [`gyro.rs`](../vendor/slint-backend-kindle/src/gyro.rs) folds 15/17 into Up and 16/18 into Down. `GyroObserver::open` and `inspect_sensor` accept one named pressure-only sensor and reject touch/key/relative capabilities. | The pressure values agree. Continuous two-pose folding is an explicit narrower policy, rather than KOReader's complete four-direction decoder. |
| Startup origin | Scribe reads LIPC `accelerometer`: U/L is upright and D/R is upside-down. A down startup sets `native_rotation_mode` before setting the current rotation. [Scribe initialization](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/kindle/device.lua#L1863-L1897) | `capture_native_portrait_calibration` in [`launch.sh`](../packaging/kindle/launch.sh) pairs repeated native direction and read-only GETVAR observations while the native owner and power state are active. `NativePortraitCalibration::scanout_at` in [`native_portrait.rs`](../src/platform/native_portrait.rs) resolves only a relative portrait delta. | Both retain a native origin instead of assuming that every device starts upright. WeRead additionally binds the observation to its owned launch and avoids a universal meaning for raw rotation codes. |
| Pixel rotation | Canonical physical modes increase clockwise; the software Blitbuffer uses `-90 * (mode - native_rotation_mode - blitbuffer_rotation_mode)`. Linux uses that software path unless forced hardware rotation is configured. [Canonical modes and hardware caveat](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L78-L86), [software rotation](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L454-L461), [Linux branch](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_linux.lua#L218-L239) | `PortraitMapping::render_offset` in [`orientation.rs`](../vendor/slint-backend-kindle/src/orientation.rs) derives holding minus scanout. `KindlePlatform`'s `Platform::run_event_loop` implementation in [`platform.rs`](../vendor/slint-backend-kindle/src/platform.rs) adds application rotation and passes the result to Slint's software renderer. GETVAR observation does not set native framebuffer rotation. | The native-relative software compensation agrees for the accepted 0/180 path. The opposite numeric signs belong to the respective renderer conventions; they are not evidence of a 180-degree mismatch. |
| Touch and gestures | `getTouchRotation()` defaults to the current canonical mode. The gesture detector applies the inverse coordinate transform after gesture recognition, including start/end positions and swipe direction. [Touch rotation](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L120-L125), [position transform](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/gesturedetector.lua#L1428-L1443), [gesture transform](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/gesturedetector.lua#L1514-L1535) | `KindlePlatform`'s `Platform::run_event_loop` implementation in [`platform.rs`](../vendor/slint-backend-kindle/src/platform.rs) supplies application rotation plus `PortraitController::touch_offset` to `TouchInput::poll`. `TouchInput::logical_position` and `panel_to_logical_position` in [`touch.rs`](../vendor/slint-backend-kindle/src/touch.rs) undo that rotation in panel coordinates before dividing by the Slint scale factor. | Both map input back into the application frame. WeRead performs the transform before Slint recognizes gestures, so a second post-gesture rotation would duplicate the correction. |
| Layout and viewport axes | Current-mode parity describes portrait versus landscape. Refresh viewport offsets follow the rotated native buffer; 180 degrees uses the opposite margins, while quarter turns exchange axes. [Orientation and screen mode](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L432-L449), [viewport offsets](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L319-L338) | `portrait_window_size` in [`platform.rs`](../vendor/slint-backend-kindle/src/platform.rs) swaps logical window dimensions only for quarter turns. Automatic 0/180 changes keep the full-panel dimensions. `Framebuffer::portrait_rotation` in [`framebuffer/mod.rs`](../vendor/slint-backend-kindle/src/framebuffer/mod.rs) rejects changed retained-buffer geometry. | WeRead has no cropped framebuffer viewport to translate. Its current full-panel portrait path does not need KOReader's viewport machinery. This does not certify manual landscape behavior. |

## Source-derived coordinate relationship

Let `A` be the independent application rotation, `H` the observed holding
direction, and `S` the calibrated current native scanout direction, in degrees.
The current controller and event loop implement:

```text
render_rotation = (A + H - S) mod 360
touch_rotation  = (A + H) mod 360
```

For `A = 0`, the source-derived mappings are:

| Holding H | Native scanout S | Software render rotation | Raw-touch inverse rotation |
| --- | --- | --- | --- |
| Up, 0 | Up, 0 | 0 | 0 |
| Down, 180 | Up, 0 | 180 | 180 |
| Down, 180 | Down, 180 | 0 | 180 |
| Up, 0 | Down, 180 | 180 | 0 |

These rows are reasoning from source, not four new device-test results. They
explain why rendering and touch must not blindly share the same native offset:
native scanout can change independently of the raw touch frame. Both projects'
180-degree inverse maps use `(width - x, height - y)` in their coordinate frame.

## Policy differences and required changes

KOReader's live gyro policy is broader than its Scribe startup normalization.
When the G-sensor is locked, it accepts only modes with the same portrait or
landscape parity as the existing mode; unlocked rotation can request UI layout
changes. [Input policy](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/input.lua#L1459-L1472)
WeRead instead continuously folds the reported poses into its accepted two
portrait states. Automatic 0/180 is consistent with the intended portrait
scope, but these two policies are not identical for sideways sensor reports.
This difference does not establish a requirement for four-way automatic UI.

WeRead owns an additional Slint input/presentation transition. Sensor reports
precede touch dispatch; a changed mapping cancels the old pointer, invalidates
the polled input generation, drains stale packets, redraws the complete scene,
and requires acceptance of a complete GC16 update before admitting fresh mapped
touch. See `KindlePlatform`'s `Platform::run_event_loop` implementation and
`begin_presentation_transition` in
[`platform.rs`](../vendor/slint-backend-kindle/src/platform.rs), plus
`TouchInput::cancel_and_drain` and `TouchInput::finish_presentation_transition`
in [`touch.rs`](../vendor/slint-backend-kindle/src/touch.rs). The
panel-completion wait remains best effort; it is not a measurement of visible
display completion. These are application/backend lifecycle requirements, not
upstream device-test claims.

Managed resume also differs in ownership protocol. WeRead keeps the initial
scanout anchor immutable, accepts only a matching private wake-cycle record,
queries current holding between matching GETVAR observations while input is
suspended, then reopens the observer and restores the complete frame. See
`KindleRenderSink::prepare_native_portrait_resume` in
[`mod.rs`](../src/platform/mod.rs) and `ManagedSleep::poll_inner` in
[`managed_sleep.rs`](../src/app/managed_sleep.rs).
This staging path is implemented; its presence does not prove that the native
sleep handoff succeeds.

Within this bounded comparison, no source-proven rotation-mapping defect
requires a further implementation change. Four-way automatic rotation, an
additional artificial debounce, and a new complete resource review do not
follow from the observed upstream differences. The identified launcher
display-release failure has a separate correction and acceptance gate.

## Current acceptance boundary

The user identified a first-generation Kindle Scribe (2022) running firmware
5.19.5. In the current physical trial the user confirmed fresh Library entry,
normal reading/touch, long-press dragging, and an Up-Down-Up transition without
sleep that retained the chapter and page without showing native Home. An
inverted right-side tap advanced the chapter while the same process remained
running. These observations establish the recorded active portrait path on
that device/firmware pair.

The latest actual five-second sleep attempt has not passed resident sleep/wake
acceptance. Investigation on that physical setup established that `xrefresh`
returned exit status 1 when `DISPLAY` was unset and succeeded with
`DISPLAY=:0.0`. An unguarded `release_display` call under shell `set -e` caused
the launcher to exit during the sleep display-release phase. This identifies
the observed launcher handoff failure; it is not evidence of a defect in the
holding, scanout, or touch rotation formulas.

The launcher now supplies a default `DISPLAY`. The coordinator's later
frozen-source rerun passed the corrected 30-case target and all 183 launcher
cases, with the original Rust and ARM results retained through an exact
runtime-source identity check. The earlier failed 183-case output remains
failed historical evidence. After an initially unreachable connection, the R3
physical trial confirmed a Down startup using raw GETVAR rotation 1, working
reading touch, a committed system night-mode change, and resident wake with the
same process and anchor. The user observed native Library before Reader returned
during both theme change and wake. [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md)
records these scoped results and the still-open visual handoff gate; this
exposure is distinct from a rotation-coordinate mapping failure.

This review ran no tests, builds, runtime probes, SSH verification, or device
actions. KOReader source behavior and the source-derived alignment are kept
separate from the user's WeRead physical observations. No broader device
compatibility or manual-rotation acceptance is claimed.
