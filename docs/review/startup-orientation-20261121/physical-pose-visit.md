# Physical pose qualification: fixed native down

**Result, November 21, 2026:** the isolated orientation-reuse candidate drew
the startup lettering and the public Reader in the expected digital orientation
on a Kindle Scribe 2022 / 5.19.5. The native reference was `D:1`, **the same
portrait as the six-pair experiment**. The requested screen-saver cycle was
**not run**. A [separate shelf retest](#confirmed-fixed-down-retest) qualified
one physical tap in `D:1`: the operator's report and independent same-visit
shelf and final-route evidence agreed, with both orientation queries reused.
Keep the feature off in the normal package. These visits did not measure
ordinary-entry startup latency or optical settling.

| Actual observation | Result | What it establishes |
| --- | --- | --- |
| Independent native reference | `D:1` before and after; 10/10 read-only `EVIOCGABS(ABS_PRESSURE)` snapshots returned `16` (native down) across 456.923 ms. | The independent reader and native calibration agree at these instants. This was a separate lease, not an observation during application startup or sleep. |
| Candidate public `B-reader` visit | Native `D:1` before, in the launch record and on return. Both `splash_orientation_reused` and `event_loop_orientation_reused` were logged. | The new pictures exercised the actual reuse path, not just its fallback. |
| Startup artwork | The 0° reference scored **0** error; the 180° reference scored **248**. The pixel reads lay 3.667–6.922 ms after `startup_feedback_ready`, before `startup_feedback_finished`, in the same GUI process. | The fixed title and startup lettering in the live framebuffer matched their predeclared orientation. No native screen image was recorded. The interval locates the read, not how long startup took. |
| Public Reader | The body of the 1860×2480 digital scanout matched the software frame at **0°**: mean absolute error **0.00437**, vs **17.14113** at 180° (gray levels, 0–255). | Digital orientation of the Reader in this visit, not the shelf, physical touch coordinates, or optical ink. |
| Screen saver and wake | `not_run wake_command_unavailable`; no sleep request and no resumed framebuffer. | While the device was already active, the guardian did not confirm `lipc-set-prop -i com.lab126.powerd wakeUp 1`. This is **not** a failed wake after sleep or proof that the firmware lacks the property. The receipt contains neither a detailed native exit code nor stderr. |

## Handoff and receipts

All times are Asia/Shanghai on November 21. A first acquisition attempt during
BiliComics' work was refused with `lease_exists`; it made no Kindle connection.
After the peer confirmed return, the two runs each acquired a fresh exclusive
lease and released it before analysis:

- **13:26:05–13:26:11**, public label `c1a8578172cfdc03`: native admission,
  sensor observations, protected-state comparison, close and independent
  absence succeeded. The lease release verified its owner/token.
- **13:26:11–13:28:38**, public label `8d45db78a4d56080`: admit, stage,
  `B-reader`, restore, collect, close and independent absence all succeeded.
  Native state and the protected project entries were checked, the temporary
  indexed item and stage were removed, SSH closed, and owner/token release
  succeeded. Neither visit replaced the daily app or opened private books.

On `test-env`, `/root/weread-physical-pose-20261121.Qb79xqcC` retains the
native reference in `runs/probe-7hgl0d8x/operations.json` and the Reader
visit in `visit/batch-qg73dxh4/{operations.json,results.tgz,pose-visit.json}`.
The raw, allowlisted public archive was independently re-read after release.
`operations.json` covers the transaction; successful cleanup does not mark
an unobserved wake as passed. The worker's Ubuntu suite passed **90/90**
tests. The splash recognizer separately passed 10/10 examples on host and
10/10 on ARM user mode, including an explicit upside-down result. The closed
visit's 69,889,077-byte upload was removed; its plan and public result archive
remain available. Freeze a new label for another visit.

## Assisted opposite-portrait attempt

The operator-assisted public Library visit (`6e3bda685cbce23d`) did **not**
launch the candidate. The stock reference was `D:1` before the turn cue. After
the upload, the prelaunch calibration was still `D:1` and all ten independent
pressure reads were `16` over 460.801 ms. The collector therefore did not arm
the temporary entry or send an open request. There is no shelf, final route or
tap sample. The operator reported that the app did not open, consistent with
the gate. These readings cannot establish whether the operator moved the
device or what it reported outside the sampled interval.

Native return was `D:1`; restore, close and independent absence passed and
the lease was released at **15:36:49 Asia/Shanghai**. See
`test-env:/root/weread-assisted-shelf-20261121.BcM2UfxL/plan/batch-j10ef22z`
for the operation receipt, raw public records and `assisted-shelf.json`. For
the next attempt, use the short standalone native probe to confirm the target
portrait **before uploading** another candidate; keep the same-launch
prelaunch gate as well.

### Native-only follow-up

- **15:40:32–15:40:38**, standalone stock probe `f9fdca5180b8ede2`:
  both native calibrations were `D:1`; all ten independent evdev reads were
  `16`. Protected state was unchanged; close, independent absence and exact
  release passed. The operator reported that the stock Library text had
  turned upright. That observation does not identify a numeric rotation or
  establish when the physical movement occurred relative to this short probe.
- **15:46:43–15:47:11**, live native reference: the first `D:1` triggered a
  single turn cue. All **18/18** completed reads were `D:1`; none were `U:3`.
  This window read the window manager's normalized direction and the
  framebuffer's scan rotation, **not** the evdev axis. Its UTC stamps are
  receipt times on the worker, not sample times on the Kindle. Do not combine
  the two windows to claim a sensor reading during the turn. The read-only
  lease was released with owner/token verified. There was **no candidate upload
  or app launch** and no shelf or touch result. Do not repeat the same cue to
  diagnose this.

Worker receipts: `weread-physical-pose-20261121.Qb79xqcC/runs/probe-foiydu15/operations.json`
and `weread-assisted-shelf-20261121.BcM2UfxL/live-reference-tnuztiw7/report.json`,
both under `/root` on `test-env`. Neither an orientation lock nor a failed
sensor has been established by these visits.

The [native sensor and display trace](../../../tools/kindle-font-trial/ORIENTATION.md#native-sensor-and-display-trace)
is prepared on the worker for a future motion window. It reads the dedicated
sensor and scan rotation on both sides of the raw stock orientation query,
using the Kindle clock, without starting WeRead. Its worker preflight and
the 134-test Linux collector suite passed; the ARM helper was also exercised
under QEMU with four valid rotations and eleven rejection cases.

### Physical three-source trace

At **15:59:13–15:59:18**, the new baseline (`dc5d07a77a204457`) reported
native `D:1` at both ends and ten axis reads of `16`. It released before the
next visit. At **15:59:41–16:00:06**, with the operator present, a separate
trace (`938e194177e7f8df`) recorded **eight groups spanning 17.901 s** on the
Kindle monotonic clock. All 16 axis reads were `16`, all 8 intervening raw
window-manager responses were `D`, and all 16 scan-rotation reads were `1`.
The time budget ended the trace before the maximum of 12 groups. No direction
change was observed at those sample points. The readings do not timestamp
the operator's movement, rule out changes in the gaps, or diagnose a lock or
sensor fault. After the visit the operator reported missing the chat cue and
not turning in time. **This is not a measurement during a confirmed turn.**
Future assisted actions require a desktop notification, not a chat-only cue.

Both visits checked protected state, closed, independently verified absence
and released with the matching owner/token. Neither uploaded a candidate nor
launched WeRead. Their `operations.json` receipts are in `pose-trace-runs/`
under `/root/weread-assisted-shelf-20261121.BcM2UfxL` on `test-env`:
`probe-egxo6o_z` (baseline) and `probe-_0s_6mdb` (trace).

### Fixed-down Library touch

**Human observation:** after acknowledging a Windows readiness dialog, the
operator was notified again before the single tap. In a separate optional
feedback dialog for this visit they confirmed all four items: held the pose,
saw an upright populated shelf, tapped the bottom-right profile tab exactly
once, and saw the profile page. The [operator record](fixed-down-operator.json)
does not rely on an inferred route. This is an observed successful tap in
the **fixed down** posture, not evidence for the opposite posture.

**Machine observation:** label `b46b1217463d5290` ran from **16:04:43 to
16:07:15**. The initial reference was `D:1`. In the frozen program, reaching
`refused public_owner` required passing the two down calibration reads, ten
down sensor checks, open request, first application frame submission, and
the GUI identity and launch calibration checks. The error did not distinguish
the checks before and after shelf sampling from a failure to publish that
sample. The resulting archive contained only the batch label: **no digital
shelf observation, final route or per-site reuse records**. No timing result
or combined human-and-machine acceptance follows from this visit. The error
does not contradict what the operator saw.

Restore, collection, close and independent absence succeeded, followed by
owner/token-verified release. The public receipts and archive are in
`fixed-down-160145-0a4faa/batch-9ybcmwk_` under the same worker as the trace.
The later `operator-analysis.json` includes the four explicit answers and
retains both automated qualifications as `not_established`. The initial
machine-only report was not overwritten.

### Confirmed fixed-down retest

The operator opted in separately to a new run. Both the readiness and action
prompts were visible Windows dialogs. Label `2a937e8d13e142f5` ran at
**16:17:47–16:20:35** on the same Scribe 2022 / 5.19.5. The candidate and
public seed were unchanged; the observer separated pre-sample, post-sample
and result-file errors. The new run does not identify which check rejected
the first visit.

| Evidence in this visit | Result |
| --- | --- |
| Native direction | `D:1` before, at launch and on return; all ten prelaunch sensor checks were `16`. |
| Candidate startup | Both the splash and event loop reused the launch calibration; neither took the fallback path. |
| Digital shelf and destination | The live framebuffer classifier recognized a **populated grid**. The candidate's final route was `settings` (the bottom-right **Profile** tab). |
| Independent human account | In a new, label-bound [four-item report](fixed-down-repeat-operator.json), the operator confirmed holding still, seeing an upright populated shelf, tapping Profile once and seeing the Profile page. Dismissing either earlier notification was not counted as feedback. |

`operator-analysis.json` reports `operator_and_route_confirmed` and
`confirmed_in_scope`. This qualifies **only this fixed-portrait Library tap**,
not the opposite grip, another touch target, wake, or a startup time. No
screen recording or private books were used. All eight operations succeeded,
including restore, close and independent absence, followed by exact release.
`fixed-down-refined-161659-39f51b/batch-zgzkz1az` under the same worker
retains the allowlisted archive, operations, original machine-only result,
and separate operator and combined reports. The Ubuntu collector suite passed
**166/166** before the visit.

## Remaining gates

The [software qualification](pose-qualification.md) does not replace a real
turn. These visits did **not** qualify the opposite stable portrait, a turn
between splash and the first business frame, a turn while asleep, or the
subsequent physical touch mapping in those states. Neither peer knew of a verified remotely
controlled turn/touch fixture. Do not book the device to wait for one, or
replace it with an orientation override. Before another sleep experiment,
check which wake property and type this firmware advertises and distinguish
a refusal while already active from a missing utility, timeout or unsupported
operation. Do not substitute a blind power toggle. A physical pass would
apply only to the tested model/firmware, not to all Kindle packages.
