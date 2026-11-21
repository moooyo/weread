# Physical pose qualification: fixed native down

**Result, November 21, 2026:** the isolated orientation-reuse candidate drew
the startup lettering and the public Reader in the expected digital orientation
on a Kindle Scribe 2022 / 5.19.5. The native reference was `D:1`, **the same
portrait as the six-pair experiment**. The requested screen-saver cycle was
**not run**. Keep the feature off in the normal package. There is no new
startup latency, physical touch, or optical display result.

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

## Remaining gates

The [software qualification](pose-qualification.md) does not replace a real
turn. This visit did **not** qualify the opposite stable portrait, a turn
between splash and the first business frame, a turn while asleep, or the
subsequent physical touch mapping. Neither peer knew of a verified remotely
controlled turn/touch fixture. Do not book the device to wait for one, or
replace it with an orientation override. Before another sleep experiment,
check which wake property and type this firmware advertises and distinguish
a refusal while already active from a missing utility, timeout or unsupported
operation. Do not substitute a blind power toggle. A physical pass would
apply only to the tested model/firmware, not to all Kindle packages.
