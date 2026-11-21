# First-frame orientation during a startup turn

This is a single candidate correctness visit on the Kindle Scribe 2022 /
5.19.5, not another latency A/B. The [previous turn](turn-and-wake-device.md#turning-during-startup)
happened before the first submission log, but its archive contains neither
the draw boundary nor the angle used by the renderer. Do not relabel it.

## Decision before device access

Use a new build and one public offline Library launch. Do not replace the
installed app or enable the optimization. Before acquisition, the worker must
verify the ARM candidate, sensor, frozen transaction and close path. A real
Windows readiness popup must be acknowledged. After staging, ask the operator
to turn once **when the WeRead splash appears**. Start only after a timely
acknowledgment; if the splash is missed, do not ask for a late turn. A popup
acknowledgment is not evidence of motion.

- **Which frame?** Use the one GUI process and device boot identified by the
  collector. The new draw record samples `CLOCK_MONOTONIC` immediately before
  the Slint render call, after its rendering rotation was selected. Accept the
  record only for that frame's successful full submission. `portrait_opened`
  is not the draw boundary. A claimed angle is the renderer's software angle,
  not an independent observation of pixels, the display glass or touch.
- **When did the sensor change?** Use the sampled interval from the start of
  the last old-state read to the end of the first new-state read. Require a
  single change strictly after splash-ready and wholly before draw to say
  the independent sensor had changed before draw. A gap, equality, crossing
  or a change after draw does not meet that claim. This samples the kernel's
  reported state, not the instant of hand motion or when the GUI consumed it.
- **Does the angle match?** Evaluate holding together with the framebuffer's
  scanout rotation, relative to the same launch's supported native anchor.
  For this candidate the app base rotation must be zero. The expected software
  turn is the holding angle minus the calibrated scanout angle, modulo 360.
  A changed grip alone need not require 180°: scanout may also have changed.
  Only compare when stable independent sample pairs bracket the actual draw
  within 100 ms and the reference is unambiguous. If both old and new states
  predict the same angle, they cannot be distinguished. Otherwise report a
  match only against the independently predicted new angle. If the sample
  or reference checks fail, report the recorded angle and
  the missing evidence, not a pass or a product defect.
- **What changes the decision?** A mismatch with a well-supported expectation
  warrants an offline review of how the app consumes orientation events. A
  match qualifies that sampled software case only. Neither proves optical
  orientation, KPW6 behavior, or a daily-entry performance gain. Return the
  device before interpretation. Do not repeat for a more favorable result.

## Build and current status

The actual candidate and sampler are frozen under
`test-env:/root/weread-turn-firstframe-20261121.pQkuTcbk/first-draw-acceptance`,
label `d1f8c818f6d0f0d3`. The worker retains `build-candidate.py`, the Cargo
artifact stream under `logs/final-arm-*/events.jsonl`, the build receipt
`candidate-final/result.json` and the frozen inputs. The app and Kindle
backend were both freshly compiled (`fresh: false` in Cargo's stream). The
static ARM candidate is 95,228,264 bytes and the paired sampler 29,072 bytes.
The packed executables returned their expected policies under Cortex-A8 QEMU;
the device transaction preflight succeeded without a lease. The older daily
binary rejected the candidate-only CLI; it was not replaced or rebuilt.

On `test-env`, the complete Python trial suite passed **304/304**. In a
separate Rust backend suite, **158/158** tests passed with the default feature
set and **163/163** with orientation reuse. These are software and collector
results. They are not additional physical timing samples.

The Windows readiness dialog did not receive a confirmation. The bridge
stopped **before acquisition**, the worker had no batch directory, and the
Kindle was not contacted. There is **no new physical draw angle or timing**.
The prepared plan remains available for one visit; it does not reserve the
device. Use a fresh peer handoff and readiness confirmation when an operator
is present. No change to the default package is justified by this preflight.
