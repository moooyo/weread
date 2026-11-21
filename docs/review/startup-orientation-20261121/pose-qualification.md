# Startup orientation correctness, November 21

**Decision during this round:** the normal package kept `startup-orientation-reuse`
off. The later [package decision](README.md#package-decision) applies to all
Kindle models. The [six-pair timing result](physical-balanced.md) applies to one Scribe 2022 on
5.19.5 in fixed native `down`, raw rotation `1`. It does not establish that
the screen and touch agree in the opposite portrait or after a change. The
checks below use public fixtures on `test-env`. No Kindle was claimed,
accessed or updated in this round; there is no new startup time.

A [subsequent physical visit](physical-pose-visit.md) verified the digital
splash and Reader at native `D:1` on the Scribe. It did not enter sleep,
change pose or exercise physical touch. The software results below are unchanged.

| Qualification | What can be checked off-device | Physical evidence sought at the time on Scribe 2022 / 5.19.5 |
| --- | --- | --- |
| Opposite stable portrait | Fixed `U:3` and `D:1` fixture inputs; splash mapping, independently resolved event-loop mapping, oriented software pixels and asymmetric touch targets. | Establish the opposite native portrait independently of the app. Check the actual splash and shelf frame against the expected panel orientation, the intended physical tap target and native return. |
| Turn during startup | Change the simulated sensor between splash and event-loop decisions, or after attaching the observer and before the first application draw. A stale calibration must not skip the fallback. | Observe an actual change during the same startup window, then check the first shelf frame and touch. A turn entirely before startup or after submission is not a pass. |
| Turn while asleep | Suspend and wake the same launcher generation, supply a new native portrait, restore Reader state and exercise the input barrier. | Observe real suspension, pose change and wake in the same process, the resumed page and correct panel/touch orientation. Verify native control after exiting in the changed pose. |

`up` and `down` here are sensor labels, not an assertion about a person's
grip. Software-to-scanout rotation and raw-touch rotation are distinct; a
valid static `down` does not require the software pixels to rotate 180°.
The asymmetric touch is a correctness check, **not** an extension of the
[Library first-frame startup metric](../../PHYSICAL_TESTING.md#library-first-frame-startup).

## Results on test-env

| Check | Result |
| --- | --- |
| Full standalone backend library, Linux x86-64 | Feature off: **153/153**; on: **158/158**. The two focused candidate tests cover the five startup/wake scenarios and a separate active turn with a held control. |
| Same two tests, static ARMv7 hard-float binary on `qemu-arm -cpu cortex-a8` | **2/2**. All 15 decoded target/coordinate records matched the Linux x86-64 run. ARM user mode exercises the 32-bit event layout, not a Kindle kernel. |
| Backend coordinates replayed through the real Scribe UI | **5/5** Search → Library → Settings → Library trips, also replayed from the ARM output. Each verifies the selected tabs in the software render. Removing the trace failed as required; moving the inverted Settings contact to its opposite, **without changing its target label**, opened Search and failed the assertion. |
| Fixed logical targets without a backend trace | **3/3**: Scribe `1860×2480`, project KPW6 `1264×1680`, and `1272×1696` (experimental KPW6 guest geometry only). All use a scale of two. |
| Wake | **8/8** native portrait launcher fixtures; **38/38** selected app managed-sleep/Reader tests with the candidate features. Two successive launcher cycles (`U:3 → D:1 → U:3`) retained the child PID, birth and generation. |
| Orientation trial tools | **58/58** on the worker's Ubuntu userspace, including six new tests for startup-window coverage. |

The app tests used `--no-default-features --features kindle,bundled-font-preflight,startup-orientation-reuse`.
Only the selected app tests ran, not the full root library suite. The worker
retains small receipts in `/root/weread-pose-acceptance-20261121.p0Q4K9EZ`:
`backend-full-{off,on}.log`, `backend-focused.log`, `backend-arm-focused.log`,
`backend-touch.tsv`, `backend-arm-touch.tsv`, `app-geometry.log`, `app-replay.log`, `app-arm-trace.log`,
`app-negative-{missing,mirrored}.log`, `app-managed-suite.log`,
`wake-launcher-portrait-suite.log`, `orientation-tools.log`, and
`flip-window-legacy.json`.

## Evidence boundary

- The backend reads Linux input-event packets from pipes through its gyro and
  touch decoders, hit-tests a Slint surface and inspects rendered pixels.
  Pressure values, touch axis limits and framebuffer rotation are fixtures.
  The changed-before-loop case requires one legacy query, then drives the
  production controller with a fresh fixture observation; it does not run
  real sensor discovery or `EVIOCGABS`. The changed-after-attach case turns
  before the first draw. A contact queued across that draw cannot click; a
  held contact is cancelled on sleep and its later motion/release without a
  fresh tracking ID cannot click either. No display-driver `SEND` or optical
  ink is measured.
- The app replays the backend's emitted logical coordinates into the actual
  `AppWindow` with a public offline shelf. The point opposite Settings opens
  Search; its Back control returns to Library; the other two points open
  Settings and return to Library. These are control actions, not navigation
  injected into the app. The backend and application are separate processes:
  replay connects their observations, not two halves of a physical launch.
- The shell tests run the production launcher with real Linux process
  suspension/resumption and fixture native commands. The Reader test runs the
  app and Slint model with a substitute display sink. Its checks include the
  route, page, visible lines and draft, not a physical panel snapshot.
- The [window classifier](../../../tools/kindle-font-trial/pose_window.py)
  requires independently acquired pose snapshots bound to the same visit,
  boot, GUI process and device monotonic clock as its application markers.
  It credits a change only when the outer bounds of the two opposite readings
  are strictly after `startup_feedback_ready` and before `portrait_opened`
  (before the event loop draws the first frame). A change around the draw or
  `first_application_frame_send` is inconclusive rather than a false hit.
  A hit does not identify which orientation query saw the change, establish
  frame contents or verify the source of a supplied receipt. All 18 historical
  indexed launches have stage logs but none has this timestamped sensor
  stream; **zero** are credited as turns.

## Reproduce the two-layer check

Use the Linux worker and its normal Ubuntu build wrapper. In a private scratch
copy of `vendor/slint-backend-kindle`, seed the lock with the root `Cargo.lock`
and point Cargo's `[patch.crates-io]` entries for `i-slint-core`, `swash` and
`zeno` at the project's vendored copies. Run `cargo metadata --offline
--format-version 1 --filter-platform x86_64-unknown-linux-gnu` there to prune
the standalone lock before running `--locked` tests. The recorded standalone
lock contained only package identities from the root lock. Use an absolute,
writable `$RESULTS` directory visible from both test invocations:

```sh
set -eu
cargo test --locked --offline --lib --features startup-orientation-reuse \
  pose_acceptance_tests -- --nocapture --test-threads=1 > "$RESULTS/backend.log" 2>&1
awk 'index($0,"POSE_TOUCH ") {print substr($0,index($0,"POSE_TOUCH "))}' \
  "$RESULTS/backend.log" > "$RESULTS/backend-touch.tsv"

cd "$PROJECT_ROOT"
WEREAD_POSE_TOUCH_TRACE="$RESULTS/backend-touch.tsv" cargo test --locked --offline \
  --no-default-features --features kindle,bundled-font-preflight,startup-orientation-reuse \
  --lib app::pose_hit_tests::scribe_replays_backend_touch_trace -- --exact --ignored --nocapture
```

The replay is explicitly opt-in: the regular geometry suite cannot substitute
for it. It requires the five ordered groups of three records and prints one
`POSE_UI` result per successful group. A missing trace is not a pass.
Run the Python trial suite in the worker's Ubuntu userspace with the full
project fixtures: `python3 -B -m unittest discover -s tools/kindle-font-trial`.
The bare host lacked dependencies and the initial slim source copy omitted
tracked fixture inputs; neither was used for the reported 58-test result.

## Physical follow-up

The rebuilt worker has ARM user-mode QEMU, not a running Scribe or KPW6
firmware guest or its documented guest/QMP tooling. KPW6 layout tests are not
firmware compatibility tests. No controllable physical turning and touch
fixture was identified in the project or the worker tools inspected. An SSH
orientation override, X11 click, or a Slint event does not supply that input.

An operator subsequently supplied real input in the [fixed-down](physical-pose-visit.md#confirmed-fixed-down-retest)
and [opposite-grip, startup-turn and screen-saver visits](turn-and-wake-device.md).
Those reports distinguish physical input, software submission, raw frames and
operator observations; see them for the outstanding gates. Prepare the complete
isolated candidate, observation and restoration path before claiming the device.
Return it before investigating missing evidence. Scope physical results to
the tested model and firmware; they are not per-model admission requirements
for the shared package default.
