# First-frame orientation during a startup turn

**Result: the first submitted application frame used the new grip's 180°
software rotation.** This was one candidate correctness visit on the Kindle
Scribe 2022 / 5.19.5, not another latency A/B. The [previous turn](turn-and-wake-device.md#turning-during-startup)
happened before the first submission log, but its archive contains neither
the draw boundary nor the angle used by the renderer. Do not relabel it.

## Decision before device access

Use a new build and one public offline Library launch. Do not replace the
installed app or change the normal package as part of this visit. Before
acquisition, the worker must verify the ARM candidate, sensor, frozen
transaction and close path. A real
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

## Build and preflight

The candidate and sampler were frozen under
`test-env:/root/weread-turn-firstframe-20261121.pQkuTcbk/first-draw-acceptance`,
label `d1f8c818f6d0f0d3`. The worker retains `build-candidate.py`, the Cargo
artifact stream under `logs/final-arm-*/events.jsonl`, the build receipt
`candidate-final/result.json` and the frozen plan. The app and Kindle
backend were both freshly compiled (`fresh: false` in Cargo's stream). The
static ARM candidate is 95,228,264 bytes and the paired sampler 29,072 bytes.
The packed executables returned their expected policies under Cortex-A8 QEMU;
the device transaction preflight succeeded without a lease. The older daily
binary rejected the trace-policy CLI available in the reuse build; it was
not replaced or rebuilt during this visit.

On `test-env`, the complete Python trial suite passed **304/304**. In a
separate Rust backend suite, **158/158** tests passed with the default feature
set and **163/163** with orientation reuse. These are software and collector
results. They are not additional physical timing samples.

The first Windows readiness dialog was not confirmed and stopped before
acquisition. No physical observation came from that attempt.

## Physical result

On November 21, 2026, the operator returned, both peers confirmed handoff,
and the bridge received both the readiness acknowledgment and the separate
post-staging action acknowledgment. The isolated `B-library` visit ran from
**17:30:31 to 17:33:11 Asia/Shanghai**. The lease, GUI and sensor evidence
all belong to `d1f8c818f6d0f0d3`; no timing was borrowed from the earlier run.

| Event on the device clock | Time after splash-ready |
| --- | ---: |
| Only observed change from down to up (samples 106–107) | 805.756–855.984 ms |
| Event-loop orientation query began | 869.707 ms |
| Query timed out | 1,426.543 ms |
| Renderer began drawing the first submitted frame at **180°** | 1,899.625 ms |
| Driver-accepted submission logged for that frame | 2,248.509 ms |

All three initial references were `D:1`. Of 401 paired samples, the first
106 were down and the remaining 295 were up. The change was already visible
**1,044–1,094 ms before draw**. Samples 127 and 128 bracketed draw by 35 ms
before and 15 ms after (50.195 ms outer width, below the 100 ms limit).
Both reported up with raw scanout rotation `1`. With the launch anchor and
the observed zero app-base guard, the old mapping predicted 0° and the new
mapping predicted **180°**. The measured renderer angle matches only the new
mapping. The same frame's submission log followed draw by 348.884 ms.

The splash reused calibration. The event loop did **not** reuse it: its
fallback orientation query timed out after **556.837 ms**. This is evidence
of correct software mapping with fallback, not a speedup at both sites. If
investigating the remaining wait, preserve the check against stale launch
orientation. Do not infer ordinary tile-release latency from this visit.

The final route was `library`; the operator [separately confirmed upright
Library text after turning](first-draw-operator.json). This does not identify
the very first optical frame. The digital shelf classifier returned `none`
about 14.5 s after submission, not evidence of a blank or inverted shelf.
No touch or KPW6 result is claimed. This visit did not change the rotation
logic, daily installation or package default. The subsequent
[package decision](README.md#package-decision) is not another device result.

The device returned to native `U:3`. All eight operations succeeded, including
restore, collection, close and independent absence; owner/token release was
verified. `batch-bhb_8ixz` retains `operations.json`, `results.tgz`, the
original `startup-flip.json` and the separate operator report. Offline replay
reproduced the machine report. Only the duplicate 69,876,711-byte upload
`inputs.tgz` was removed; the candidate and raw results remain. The qualifying result is
`first_application_draw.software_mapping.status = matches_new_mapping`;
absolute first-frame pixels remain unmeasured. Do not repeat the gesture.
