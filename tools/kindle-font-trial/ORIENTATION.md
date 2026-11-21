# Indexed orientation timing on the shared Scribe

Compare two builds of the **same source** on a Kindle Scribe 2022 running
5.19.5. Both use `kindle,bundled-font-preflight`; only B also enables
`startup-orientation-reuse`. Keep the same indexed temporary entry, launcher
(including `sleep 2`), helpers, one boot, restored public 15-cover offline
shelf, instrumentation and restoration policy. Do not edit the installed
daily entry, shared handler or private data. Preparing the experiment does
not reserve a device; a frozen plan is not a measurement or an installation.

**Do not run the font `freeze.py`/`run.py` for this experiment.** They
select different font arms and the font runner invokes its own analyzer.
The original `device.sh` also requires `/mnt/us/weread/launch.sh` to be
absent. It is now the installed daily supervisor. Never remove it, weaken
the old admission guard, or reuse that collector to force a new trial. The
[orientation collector](orientation-device.sh) protects the November 21
WeRead, Aidoku R3 and lowercase BiliComics installation expectations. If a
peer installation has since changed, stop and get a new handoff rather than
restoring the old expectation. The collector's temporary document,
stage and native request helper use their own orientation namespace. Keep
failures and interrupted attempts separately; never fill a missing row from
a retry or the font trial.

## Prepare without holding the device

The November 21 worker workspace is
`/root/weread-orientation-20261121.WR7ofkGL` on `test-env`. Its
`orientation-inputs/` contains the public 15-cover seed, production helpers
and a separate measurement helper built with `WEREAD_ORIENTATION_TRIAL`.
Both application binaries come from the retained `arm-pair` build receipts.
On the **worker**, freeze a fresh pilot only once; use `verify` on an existing
pilot instead of recreating it:

```sh
W=/root/weread-orientation-20261121.WR7ofkGL
tool=/work/src/tools/kindle-font-trial/prepare_orientation.py
/opt/kindle-toolchain/ubuntu-run "$W" python3 -B "$tool" freeze --work /work \
  --run /work/orientation-pilot --seed /work/orientation-inputs/seed.tar.gz \
  --helpers /work/orientation-inputs/helpers --pilot
python3 -B "$W/src/tools/kindle-font-trial/prepare_orientation.py" verify \
  --work "$W" --run "$W/orientation-pilot"
```

The pilot contains only `A-library, B-library`, not timed pairs. It requires
no assumed holding or alignment. When the current owner has confirmed safe
restoration and release, and the payload, authentication and recovery
command are ready, run the following on the **worker host**, not inside
`ubuntu-run`. `OWNER` is this session's identifier and `LEASE_TOOL` is the
reviewed worker lease utility. Supply authentication via private terminal
input, not a command-line argument, file in this repository or chat message.

```sh
runner="$W/src/tools/kindle-font-trial/run_orientation.py"
python3 -B "$runner" --work "$W" --plan "$W/orientation-pilot" \
  --owner "$OWNER" --lease-tool "$LEASE_TOOL"
```

The November 21 operator shortcut supplies those same frozen paths and the
WeRead coordinator identity. From a private PowerShell terminal, run
`ssh -t test-env /var/tmp/kindle-build-coordination/weread-orientation-pilot-20261121`.
It does not replace the daily application, wait for a lease, or run the six-pair
comparison. Do not start it during the ordinary-entry baseline. A close-only
recovery uses the same shortcut with `--recover` and the original attempt path.

There is no reserve/wait mode. A pre-existing lease is not permission to
connect, regardless of its age. A rejected read-only admission releases
immediately. A started trial first restores its own changes and verifies
stage removal and native state, then releases before offline analysis or
code changes. A failed transfer does not excuse leaving a recoverable stage
behind. An unconfirmed physical close is **not** a release: use the same
command and owner with `--recover "$ATTEMPT"` for cleanup only, never a new
run or deletion of another owner's lock. No other session takes over recovery.

Only a successful, released pilot can produce `pilot-reference.json`. A's
independent Library picture sets the scanout alignment; both images and both
candidate reuse markers must agree. Each case records only the native
idle portrait, the current generation's launch calibration checked after
SEND, and the restored native portrait. A fixed `U:n` or `D:n` must agree
across these reads. This is native evidence, not proof of a user's physical
grip. If the branch misses or the picture differs, stop here; do not time or
promote a fallback as a successful optimization.

For a separate full comparison, `freeze` a fresh run without `--pilot`, then
`seal --work "$W" --run "$FULL_PLAN" --reference "$ATTEMPT/pilot-reference.json"`
using `prepare_orientation.py` on the host. The full plan must match the pilot
inputs. Use the same host runner with this new plan and a fresh lease only
after a new handoff. A failed gate does not change the daily installation.

## Recording contract

- The plan JSON must declare `purpose: "startup-orientation-reuse"`,
  `device: "Kindle Scribe 2022"`, `firmware: "5.19.5"`, `clock:
  "CLOCK_MONOTONIC"`, `launcher_wait_seconds: 2`, `control_features:
  ["kindle", "bundled-font-preflight"]` and `candidate_features:
  ["kindle", "bundled-font-preflight", "startup-orientation-reuse"]`.
  Freeze `holding` (`up` or `down`), `raw_rotation` (0 through 3) and
  `expected_panel_alignment_degrees` (0 or 180) **before** timing, from
  the independently closed control reference. Do not select whichever
  candidate orientation makes the comparison pass.
- Preserve the raw [`measure.c`](measure.c) request format and the application's
  `WEREAD_STARTUP` markers in `out/<case>/request` and `markers`. The request
  immediately preceding the native file-open operation and the first Library
  driver `SEND` must use the same device `CLOCK_MONOTONIC`. SSH, a screenshot
  timestamp, `state-open`, and an input descriptor are not endpoints.
- Both arms emit `splash_orientation_begin` and `event_loop_orientation_begin`,
  each followed by exactly one outcome for its site. Legacy queries produce
  `success`, `timeout`, `unavailable` or `failed`; only B can emit `reused`.
  Keep the original outcome when B falls back. Do not compare only its hits.
- Use one public trial `label` (16 lowercase hex characters, **not** a lease
  token or device ID) in both the frozen plan and the final operations receipt.
  The collector must place that label plus one newline in `out/orientation-batch`
  in the raw capture archive from its own stage. Use fresh output locations.
- For `schedule` use `A-library, B-library, A-reader, B-reader` for independent
  pictures, then `A0, B0` warmups, then `A1, B1, B2, A2, A3, B3, B4, A4,
  A5, B5, B6, A6`. The qualified software PNGs, Gray8 scanouts and per-run
  thumbnails use the [indexed protocol](README.md). A half-turn that differs
  from the predeclared alignment is a failure, even if rotating the image
  could make the content match. Keep the four independent picture runs'
  raw request and stage records too: both candidate pictures must exercise
  reuse at both sites. Those durations never enter the timing medians.
  The operations receipt must show admission,
  staging, the planned cases, collection after picture qualification, then
  restoration, final collection, close and release without failures.

On `test-env`, analyze the matching offline files. No credentials, device
commands or network access are used by the analyzer:

```sh
python3 -B tools/kindle-font-trial/test_orientation.py
python3 -B tools/kindle-font-trial/analyze_orientation.py plan.json operations.json results.tgz report.json
```

The report keeps the two warmups, raw request nanoseconds and every relative
marker, per-site outcomes/durations, six measured pairs including slower
ones, and both variant medians. This experiment's timing gate requires a positive median
paired request-to-SEND saving, at least five faster pairs, and both candidate
sites reusing calibration in all six **fixed-pose** measured runs. Authentic
fallbacks remain valid observations, not proof the fast path worked. A
completed batch missing the timing gate writes its report and exits nonzero;
invalid or incomplete inputs cannot produce a new successful report.

A positive gate is not by itself a reason to enable the flag. Keep the six
paired differences visible alongside the within-arm timings; distinguish
repeat-to-repeat variation from clock resolution. If the apparent saving
cannot be distinguished from timing uncertainty, leave the flag off and
report the result as inconclusive. Do not subtract these request-to-SEND
times from a separately measured ordinary tile-release interval or from a
historical interaction upper bound.

## Separate physical acceptance

Use separate, explicitly authorized qualification runs for normal and inverted
holding, changes between splash and first frame, missing/stale calibration,
and changed pose across sleep/wake. Check that artwork and Library render
upright, mapped input selects the intended target in both poses, and native
input/display recover after exit. These are orientation correctness checks,
not extra actions required to stop the startup clock.

See the [pose qualification](../../docs/review/startup-orientation-20261121/pose-qualification.md)
for the completed Linux and ARM user-mode checks, the physical admission
criteria and the same-launch sensor-window classifier. The fixed portrait
samples from the six-pair batch cannot be reused as observed turns.

### Native reference and candidate Reader visit

These are separate, non-timing checks. On `test-env`, build and test
`pose_snapshot.c`, the public splash reference and observer, and the production
`pose_visit_alarm.c` before acquiring a lease. The snapshot helper is read-only
and non-grabbing. The wake helper's `--policy` must identify `CLOCK_BOOTTIME_ALARM`;
never package a simulation build. Do not query the Kindle just to reserve it.

1. In a private worker directory, run `run_pose_probe.py --preflight` with
   `--helper`, `--runs`, `--owner` and `--lease-tool`. After peer handoff, the
   same command without `--preflight` acquires once, samples the live sensor
   between two stock calibration reads, closes, verifies absence and releases.
   A matching result establishes only the current native reference. A busy
   lease or denied admission grants no right to proceed.
2. Freeze a **new** `pose_visit.py freeze` plan with `--retained` pointing to
   the original orientation worker, `--plan` in the new worker, and `--splash`
   and `--wake` pointing to the qualified helpers. No production installation
   or private library is used. Check `run_pose_visit.py --work WORK --plan PLAN
   --preflight` before the next independently acquired lease.
3. Use `run_pose_visit.py --work WORK --plan PLAN --owner OWNER --lease-tool LEASE_TOOL`
   for one public candidate Reader visit. The observer tests the static startup
   lettering against both orientations in the same process's splash window.
   Missing that window is not a pass. Sleep is optional: the guardian must
   confirm process ownership and a bounded wake path before asking stock
   `powerd` to enter the screen saver. A successful command alone does not
   establish sleep or wake.
4. The runner restores native state, collects the public results, closes the
   stage, independently checks absence, and releases **before** interpreting
   pictures or reporting wake. See `operations.json` for the transaction and
   `pose-visit.json` for the actual observations. If cleanup is interrupted,
   use the original owner and `--recover ORIGINAL_ATTEMPT` on the same runner
   and plan, never another launch or another owner's token.

True physical turning and touch mapping still need independent sensor and
target evidence. Neither a fixed-pose visit nor a synthetic tap satisfies them.

### Native sensor and display trace

If the operator sees the stock page reorient but repeated calibration reads
stay the same, do not repeat the shelf trial to guess why. The native-only
`run_pose_probe.py --display-trace` uses the small `pose_snapshot.c` helper,
not the candidate package. Supply `--helper`, `--runs`, `--owner`, and
`--lease-tool`; first add `--preflight` on `test-env`, without a lease. Only
run the live command under a fresh exclusive lease when a meaningful motion
window is available. It does not open any card, set an orientation, capture
pixels or collect touch input. It does not instruct an operator to turn.

For each group the helper reads the dedicated evdev sensor and framebuffer
scan rotation, the script queries the unnormalized stock `winmgr.accelerometer`
property, and the helper reads the first two values again. Device
`CLOCK_MONOTONIC` bounds locate each pair and bracket the intervening property
query. These are **not simultaneous** readings: the evdev value is the last
reported axis state, not a timestamped physical turn. A missing property
value is recorded as `unknown`. If the required command timeout is unavailable,
the diagnostic refuses rather than running unbounded. It restores and releases
before analysis; a successful trace is **not** permission to launch or a pass
for a physical turn, rotation lock, optical orientation or touch mapping.

For an operator already waiting in the measured starting pose, the optional
`--turn-cue --operator-ready` on `run_pose_probe.py --display-trace` streams
**one** cue from the first complete group. Before claiming the device, use
the Windows readiness dialog described below and arm the local notification.
Keep the current pose until the action dialog appears. A fresh group matching
the prior stock baseline exactly (axis `16` and rotation `1` on both sides of
raw `D`) emits the standalone worker line `TURN_NOW`; use that line to show
one short topmost instruction to rotate 180° in the screen plane and hold.
`HOLD_STILL`, no cue, an expired notice, or a closed visit means **do not
turn**. Do not show an action dialog from the admission message, a final JSON
record or a prior attempt. The cue only records an instruction, not an
observed movement. Cleanup, final trace validation and the independent
opposite-portrait admission for a candidate are unchanged.

### Operator-assisted opposite portrait and shelf tap

This separate single-visit plan opens a **public offline Library** using the
orientation-reuse candidate. It does not use the installed daily card, put
the device to sleep or time startup. Agree on the two actions with a present
operator while off-device; do not ask them to touch a peer's running visit.
Use a visible Windows readiness dialog **before taking a lease**, and a second
desktop notification **before the actual action**. Chat output alone does not
notify this operator reliably. Use `show-operator-notice.ps1` with `-Title`,
`-Message` and a bounded `-TimeoutSeconds`; add `-Confirm` for readiness. It
reports an acknowledgment, cancellation or timeout and has no device access.
Dismissing a notification does not prove that
the operator performed the action. If readiness expires, stop without taking
the device. If the action window has ended, never request a retroactive tap.

1. Freeze a new plan with `assisted_shelf.py`. Preflight both worker commands:
   `live_pose_reference.py --work WORK --plan PLAN --lease-tool LEASE --preflight`
   and `run_assisted_shelf.py --work WORK --plan PLAN --preflight`. Use the
   live commands with `--owner OWNER --lease-tool LEASE --operator-ready` only
   after peer handoff and when the operator and both stages are ready.
2. Run `live_pose_reference.py` first. It uploads no candidate. On `TURN_NOW`,
   the operator holds the screen upright and facing them, turns it a half turn
   in its own plane and holds still. On `HOLD_STILL`, do not turn. Do not open
   a card. The 28-second window records the native direction and framebuffer
   rotation, not an independent sensor trace. Only two consecutive `U:3`
   readings **and a verified release** qualify for the next stage. Stop on any
   other result; do not infer that the operator failed to turn. Only then run
   `run_assisted_shelf.py` under a fresh lease. If its own reference is no
   longer `U:3`, it releases without uploading. After upload, the prelaunch
   native calibration and ten independent sensor samples must still agree.
3. When the complete shelf is visible, the operator waits about five seconds,
   taps the **rightmost bottom tab with the person icon** exactly once and
   leaves the resulting page visible. It is the profile tab, internally
   `settings`, not the settings list reached through a second tap. If the
   intended control is absent or unreadable, do not tap blindly. Keep the
   pose and let the trial close itself. Never ask for a photo of a private shelf.
4. Separate the native reference, prelaunch sample, digital shelf observation,
   application-reported final route, and operator's account. A `settings`
   final route alone cannot prove who tapped, where the finger landed or what
   the operator saw. Restore, verify absence, and release before discussing
   results. This does not qualify the other touch targets, a turn during
   startup, held-gesture cancellation or wake after turning in sleep.

For a separate test at the known down posture, freeze a **new** plan using
`assisted_shelf.py freeze --fixed-down`. Only this plan accepts native `D:1`
and ten prelaunch samples in `16/18`. Do not turn for it. A successful tap
in this posture cannot be used to accept the opposite-portrait optimization.
If the fixed-down observer refuses the foreground before or after inspecting
the shelf, inspect the corresponding `pre_*` or `post_*` diagnostic after
release. `scene_publish` identifies a result-file failure, not a tap or route
failure. Before sampling, a temporarily busy GUI may return to the existing
allowed task state within a bounded wait; no picture is taken while it is
busy. The post-sample check never retries. Never replace a missing observation
with a screen from a later visit.

The frozen collector measures file-open request to first Library `SEND`.
The [current primary metric](../../docs/PHYSICAL_TESTING.md#library-first-frame-startup)
instead starts at release of the ordinary stock tile (label automated input
as synthetic) and uses the same launch's qualifying Library frame and
device-clock render-completion or submission log. Record which endpoint
was observed. Cover placeholders are valid; no second click or recording is
needed. Driver completion and optical settling are different events. Do not
combine evidence from separate launches or store private screens. Passing
the collector's timing gate never enables a feature or deploys a build.

### Assisted startup turn and screen-saver turn

These are two separate Scribe correctness visits, not startup benchmarks. On
`test-env`, freeze and preflight each plan under a fresh label before taking
its lease. At the Windows desktop, use `show-operator-notice.ps1 -Confirm`
with one short readiness question. Cancel, timeout or no desktop means no
acquisition. Start only after acknowledgment and peer handoff; a busy lease
is not a reservation. The operator never needs to run a command.

1. **Startup.** Build a fresh ARM candidate with the guarded first-draw trace
   and the paired pressure/scanout sensor. Freeze with `prepare_pose_flip.py
   freeze --retained RETAINED --plan PLAN --sensor SENSOR --scene SCENE
   --build-result WORK/candidate-final/result.json`; preflight with
   `run_pose_flip.py --work WORK --plan PLAN --preflight`. Use the Windows
   `Invoke-StartupTurnBridge.ps1` for the live visit. It asks for readiness
   off-device and obtains a second, short acknowledgment **after staging**.
   The gesture is triggered by the appearance of the WeRead splash, not by
   a host timestamp: turn once in the plane of the screen and hold. If the
   shelf has already appeared, do not turn. A missing or late acknowledgment
   ends the visit without launching the test case. The older pressure-only
   plan is an archived observation, not an executable first-draw plan.
2. **Screen saver.** Freeze with `assisted_sleep_visit.py freeze --retained
   RETAINED --plan PLAN --sensor SENSOR --witness-window --finish FINISH`;
   preflight with `run_assisted_sleep.py --work WORK --plan PLAN --preflight`.
   Use `Invoke-SleepTurnBridge.ps1 -OperatorReady` with the frozen label,
   worker, plan name, owner, lease tool, private authentication and one short
   message per phase. The bridge checks fresh, ordered permits. Ask for one
   power press only on `sleep` while the public Reader is visible. Ask for one
   half turn only on `turn`, with the same Reader process confirmed stopped
   and the stock screen saver actually visible; hold through wake. On `wake`
   or `recover-wake`, press once only if the saver is **still** visible. If the
   page has returned, the cue expires or the screen is unclear, do nothing.

Restore, close, verify absence independently and release by owner/token before
reading `startup-flip.json` or `assisted-sleep.json`. Report the same-launch
device-clock sensor bracket separately from what the operator saw. A startup
window hit is not the first frame's rendering angle. The [first-draw test](../../docs/review/startup-orientation-20261121/first-draw-boundary.md)
records the effective software angle independently; driver submission is not
optical completion. Different awake readings do not locate a turn inside the
screen saver. Compare sleep and wake frames from that cycle for relative
orientation and content: the exit-time Reader screenshot is **not** an absolute
0° reference. Record the operator's view separately. See the
[device observations](../../docs/review/startup-orientation-20261121/turn-and-wake-device.md)
before requesting another turn.
