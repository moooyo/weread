# Skipping the second orientation query after a startup turn

**Decision:** keep the [shared exact-pose default](README.md#package-decision).
Compare it against a candidate that can also accept a confirmed opposite
grip at event-loop entry. The candidate changes neither the splash nor wake.
It is a separate testable change, not a model-specific exception.

## What prompted the change

In the [first-draw Scribe 2022 / 5.19.5 visit](first-draw-boundary.md#physical-result),
the splash reused calibration, the event loop's fallback query timed out in
**556.837 ms**, and the first submitted frame used the correct **180°**
software rotation. The independent sampler saw the new grip before the
query, but it was not observing the application's sensor descriptor. That
archive cannot identify which check rejected reuse or establish whether
the new branch would have run. It remains a fallback result.

## What the two arms do

| Event-loop input | A: current Kindle package policy | B: add `startup-orientation-reconcile` |
| --- | --- | --- |
| Live grip matches the initial calibration; raw scanout still matches | Reuse | Reuse; no additional pose read |
| Opposite grip, confirmed on the same quiet sensor; raw scanout unchanged | Run the existing query | Apply the launch calibration, then the new grip, and retain the sensor |
| Scanout moved, calibration or confirmation is missing, or the request changed | Run the existing query | Run the existing query |

B admits the new path only for the initial request (`sequence == 1`) with a
complete native portrait calibration. It brackets two consistent pose reads
with three framebuffer rotation reads matching that calibration. Before and
after the second pose read, a zero-timeout poll must find the original sensor
descriptor quiet. The read does not consume events. B rechecks the entire
request, then applies the original anchor and updates it with the observed
grip. Render rotation uses grip relative to scanout; touch uses grip. This
distinction matters if the native scanout also turns: the candidate does not
assume that every grip change needs a 180° render rotation. After admission,
later motion still belongs to the normal observer. This is not time-based
debouncing or a cached orientation across launches.

Only a successful application of that mapping emits
`event_loop_orientation_reconciled`. `reused` still means the original grip
matched. On rejection B discards the attempted observer and takes the usual
bounded query exactly once. Splash, wake and the launcher's two-second power
observer wait are not part of this optimization.

## Results on `test-env`

The [controlled Linux A/B and paired samples](reconcile-host.md) measure the
event-loop decision alone. They use synthetic orientation readings, an actual
child process, and the product's unchanged timeout and cleanup. A and B below
were built from the same source with only the candidate feature switched.

| Input and transport | Pairs | Observed result |
| --- | ---: | --- |
| Confirmed opposite grip; silent child | 6 | A timed out once, B never queried; median within-pair saving **500.363 ms** |
| Confirmed opposite grip; immediately responding child | 6 | A queried once, B never queried; median within-pair saving **20.221 ms** |
| Unchanged grip | 4 | Both reused the initial calibration without a query or a second sensor read |
| Confirmation lost or request advanced | 4 | Both issued the existing query once |

For a separate cross-check against actual `074a4d2` source, all six
silent-child pairs also skipped one query (median saving **500.374 ms**).
The two exact-pose arms agreed on all 18 input cases; the three-arm matrix
passed **54/54** branch checks. This demonstrates removal of an unnecessary
query **when the witness is available**, not the frequency of this situation,
the cost of real sensor reads, or a half-second improvement on a Kindle.

The full Linux backend suites passed **163/163** with exact reuse, **169/169**
with the candidate, and **158/158** without orientation reuse. An ARMv7
Cortex-A8 QEMU user-mode run passed **11/11** targeted cases, including
direction reversal, rejection paths, and touch mapping. Across five synthetic
Slint scenarios, all 15 contacts hit the expected controls. These are not
physical taps, firmware wake tests or timings. The [historical two-site
benchmark](host-timeout-bench.md) also passed with its original exact-only
expectations; it does not exercise the new path.

The complete [trial-tool suite](../../../tools/kindle-font-trial/ORIENTATION.md)
passed **315/315** in one Ubuntu 24.04 run with no skips. Only a validated new
B plan may record `reconciled`, and only at the event loop. The same marker
in an old plan is an error even in a warmup; it cannot become a timing sample.
Replaying the original pilot, six-pair batch and first-draw archives returned
the same reports and recovery programs. The old startup turn still reports
`splash: reused`, `event_loop: timeout`.

## ARM package and preflight

Two static ARMv7 hard-float release binaries were built from the same source
on `test-env`: **95,228,264 bytes** for A and **95,229,160 bytes** for B.
Cargo reported a fresh compilation of both the app and backend in each lane.
The effective feature lists differ only by reconciliation. Both applications
passed version and draw-policy checks under Cortex-A8 QEMU. A is a recompile
of the current default **policy**, not the exact binary from `074a4d2`.

The worker `test-env:/root/weread-reconcile-pair-20261121.z2TTbdxT` retains
both executables, their Cargo records, `build-reconcile-lane.py`, and
`pair-verification.json`. Its `reconciled-draw-acceptance` plan (label
`7ce33bf9c723db91`) passed offline preflight. The archived B application
matches the actual compiler output; the draw-policy response is the same in
both arms and is **not** evidence of a branch hit. Preflight did not access
the device. The plan stages only B for a correctness visit, not a device A/B.

## November 22 physical result: turn after draw

**The operator turned the device once. The application had already selected
the correct exact-pose path for that startup; the new branch was not hit.**
On Scribe 2022 / 5.19.5, the matching launch, native and initial references
were `U:3`. Across 401 independent sensor/scanout samples, raw scanout stayed
at `3`. The sole up-to-down transition was between samples 110 and 111,
**43.895–94.206 ms after the first submitted frame began drawing** and
before its submission. Both samples bracketing draw still indicated up. This
is a time bracket on the device's reported state, not on the hand motion.

| Same-launch observation | Recorded result |
| --- | --- |
| Splash orientation | `reused`, 40.853 ms |
| Event-loop orientation | `reused`, 4.156 ms; no fallback query and no `reconciled` marker |
| First application frame | Software angle 0°; driver-accepted submission logged 394.958 ms after draw began |
| Final state | Application route `library`; the independent public shelf classifier returned `none`; native returned `D:1` |

The result was independently recomputed from the original sensor and GUI
markers in `reconciled-draw-acceptance/batch-pa0l86n5/results.tgz` on the
worker above. The indexed open request to first application submission was
5.530 s; it neither starts at a stock-card release nor identifies a complete
Library frame. **There is no ordinary-startup A/B result and no measured
reconciliation saving.** The eight transaction steps succeeded, including
restore, collect, close and the independent absence check. The owner and
token were verified at release at **02:37:19 Asia/Shanghai on November 22**.
The installed daily application was not replaced; no second turn was requested.

## Promotion criterion

The next useful physical observation is one same-launch `reconciled` outcome
with a first submitted frame whose draw angle agrees with independently
sampled grip and scanout. A fallback or an unchanged grip at the decision
point is a valid observation, not a reason to repeat the gesture until it
passes. The earlier gestures already
qualify the behavior of the **earlier** build. A test of the new branch is
needed before changing its default, not a new test of each Kindle model.

A branch hit is not a measurement of ordinary startup. For that, compare
identical entries and fixtures from [release of the normal stock tile to
the first complete Library render or submission](../../PHYSICAL_TESTING.md#library-first-frame-startup)
on one device clock. Cover placeholders are allowed; submission is not
optical completion. No such A/B or deployment is claimed here.
