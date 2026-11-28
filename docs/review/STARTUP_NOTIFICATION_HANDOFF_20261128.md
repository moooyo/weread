# Handoff: Kindle startup notification comparison

November 28, 2026 (Asia/Shanghai). **Formal v6 stopped after two warmups
and one measured pair because measured B received no ready hint.** All four
visits returned the device. There are zero eligible formal pairs. Do not
resume v6 or reuse a slot from an earlier roster. This document is not a
reservation or authorization to contact a device.

## Current receiver and shared operation lock

The worker now uses the [shared process-held file lock](../../tools/kindle-physical/README.md).
The single-visit process takes it after staging the local stream and holds
it through checked owner/token acquisition, device operations, native
restoration, closed transports, and verified return. The durable owner/token
record remains a separate recovery barrier. Do not use any earlier frozen
operator that bypasses the common lock.

Original 5.19.5 Scribe firmware BusyBox rejects `timeout -k`. The replacement
receiver uses one `timeout -s KILL` policy with a 150-second limit, a pinned
archive lock, and name revocation on bounded failure. The unattempted v2 load
roster and the lock-only refresh are superseded. The final replacement
A/B archives match the retained archives byte for byte; the device programs
change only the visit labels and receive/retirement code. No application or
production launcher source changed.

On `test-env`, 74 host regressions (including independent process contention
and kernel release on exit), 37 isolated BusyBox transaction tests, and three
GNU `timeout` controls passed. QEMU user-mode replay with public ARM BusyBox
applets from Scribe 5.19.5, Scribe 5.20.1, and Paperwhite 6 5.20.1 confirmed the
fast and blocked self-tests, exact/short/extra-byte reads, and revocation when
an exact-length sender kept the pipe open. The applet replay is not a full
firmware guest or physical verification. The final isolated worker compared
both entire staged streams, checked the real kernel lock on all 20 simulated
device operations, verified two separate handbacks, and rejected missing
retirement evidence. Look for its private `load-v4-verification.json` on
`test-env`.

## Physical load-only gate: passed

Two fresh visits, A then B, passed on a 2022 Scribe running 5.19.5. Both
provided the exact `admit → stage → install → stop → restore → close → absent`
sequence, visit-bound retirement on normally closed transports, and verified
original installation identities and before/after document and supervisor
bytes. The visits did not navigate, activate a selector, or run the application.
The original installation identities matched across visits. No load or recovery
remained pending and no timing sample was collected.

| Visit | Original lease returned (Asia/Shanghai) |
| --- | --- |
| A | 2026-11-28 03:40:39.525109 |
| B | 2026-11-28 03:43:31.761872 |

An independent worker readback confirmed both returns, the absence of the
owner record and any held operation lock at that moment, and an empty local
wire staging directory. No daily package or private shelf was changed.
Keep the exact originals and private receipts on `test-env`; only the above
redacted results belong in the repository. This is not a reservation for the
next device window.

## Original handoff state

- The ordinary Scribe 2022 / firmware 5.19.5 installation was last
  independently verified after deploying [`a48d030`](../STARTUP_CRITICAL_PATH_20261127.md).
  This is historical evidence, not proof of the current installed files or
  device availability.
- The first display-takeover notification candidate is not on `main` and
  has not been deployed as the ordinary package. The prior comparison had
  two returned warmups and one returned measured A. B lost SSH during
  installation, before navigation or startup. Its separate recovery removed
  only the proven original stage, independently confirmed the original files
  and native control, closed all its device connections, and released the
  original lease. **There are zero complete formal pairs.** Neither this
  recovery nor any load-only check is a timing sample.
- The revised [load-retirement tooling](../../tools/startup-observer/stock-load-retirement.md)
  is separate from application code. It persists a pending `stage` or
  `install` before dispatch, requires the original terminal retirement
  record and closed SSH, and keeps the lease pending on uncertainty.
  The device receive limit is 150 seconds with a one-second escalation grace.
  The reader checks and locks the pinned archive inode before writing;
  the installer rechecks that lock before validation or retirement. This
  is not a bound on the entire install, including header and validation.
- On the designated Linux worker, the hardened tooling passed 81 scoped
  regressions with the pinned BusyBox applets and two additional GNU
  `timeout` controls. The latter coverage checked a held-open sender and
  the refusal to advertise retirement while a simulated reader still held
  the archive lock. The integrated load-only worker independently compared
  both complete staged input streams and validated both synthetic handbacks;
  missing retirement evidence was rejected. Those tests are not physical
  qualification of the revised transport.
- Both new load-only material sets were frozen and verified byte for byte
  against the earlier A and B archives. They use the unchanged shared ARM
  application, notification bridge, frame guard, public shelf, launcher
  variants and timing instruments. Neither new slot has been attempted.
  No updated tool has been used on Kindle, no new lease was acquired, and
  no comparison roster was created. The current session withdrew its
  window request. Another session may be in the queue or using the device.

## Procedure used for the now-closed untimed gate

Do not replay these visits. These are the constraints for any separately
approved replacement if the tested host or device transport changes.

1. Sync `main`, preserve unrelated working changes, and locate the retained
   private worker by its `load-v4-verification.json`. Keep private plans,
   tokens, receipts and material archives off Git. Use the private live stdin
   transport with the pinned worker SSH configuration. If the original
   inputs are unavailable or changed, rebuild and reverify a fresh batch
   rather than reopening an old slot. The initial candidate was authored in
   `perf/kindle-portrait-completion` with an uncommitted comparison report;
   that worktree was not present in the resumed checkout. The original
   notification worker retains the candidate supervisor, native bridge,
   bridge tests and binary provenance. Do not copy private recovery receipts
   into this repository.
2. Offline, run the load-only readiness check for **both** slots, confirm
   the frozen archive and tool byte identity, and verify adequate private
   staged-wire space. Verify the pinned SSH configuration and arrange an
   explicit handoff through the active coordination queue before acquiring
   an exclusive lease. A missing shared lock alone is not a handoff. Use
   a separate authorized window; the first admission limit is at most
   three minutes after the start, and the no-new-visit limit is at most
   ten minutes after the start. The latter must not interrupt an active
   visit's cleanup.
3. Run **A, then B** on the representative Scribe, each under a separately
   verified original lease. Do not select or tap the card, activate the
   selector, launch the application, or time anything. Require the exact
   successful `admit → stage → install → stop → restore → close → absent`
   receipts, stage/install retirement on normally closed SSH, and no
   pending load. Independently confirm the three original installation
   identities, document and launcher byte readback before and after,
   native control, stage absence, and original owner-scoped release. Do
   not start B before A's confirmed return. A recovery-only close, a
   refused install, or `already_absent` is not a passing load-only slot.
   The aggregate result also requires the same original installation
   identities in both slots and no overlap between their leases.
4. On disconnect, missing evidence, drift or timeout, stop the batch.
   Do not resend a load or repeat an uncertain removal or release. Client
   exit, installer header acknowledgement, and temporary stage absence
   do not prove remote retirement. The normal runner deliberately refuses
   recovery while a load remains unconfirmed. Keep the original journal
   and token for separate, scoped reconciliation; never clear a pending
   entry by hand or generalize the historical one-off B cleanup. Notify
   the coordinator only after a verified return. Another project's later
   lease is not evidence that the earlier visit did or did not return.

### Known limits in the prepared tool

A successful SSH exit with a valid retirement marker but a malformed
`stage` or `install` stdout receipt still leaves the load pending. This is
conservative: it blocks ordinary cleanup and release, even though the
terminal marker was observed. Treat that case as unresolved and preserve
both inputs for reconciliation. A future change can retain authenticated
retirement evidence separately from rejecting the operation result.

The current busy-reader control returns supervisor status 124, which is
otherwise an admissible timeout status, to isolate the lock check. The
separate delayed-reader control verifies that revoking the name prevents a
reader that opened the old inode from writing or recreating the archive.
If the transport changes again, do not treat these closed verification
slots as evidence for the changed transport.

## Formal comparison: stopped, no timing sample

The retired formal worker is
`test-env:/root/weread-display-handoff-20261128.formal-v4.t97vc2fk`. All ten
slots passed offline preflight: two warmups followed by `AB / BA / AB / BA`.
The frozen archives were byte-identical to the load-only inputs, and the
device program for each arm matched the corresponding load-only program
apart from its visit label. The transport loader needed no changes.

Only the A warmup was dispatched. On Scribe 2022 / 5.19.5, it passed
`admit`, `stage`, `install`, and the independent original-file audit. Both
load operations had normally closed retirement receipts. `prepare` then
refused with `native_view_changed`, before it saved the stock origin or sent
any input. The device's recorded `activeView` changed from `KPP_LIBRARY`
to `KPP_HOME`; the cause of the transition was not established. The normal
`stop` passed, but the normal restoration refused the changed view. There
was no selector activation, card release, app launch, render or driver
sample. The B warmup and all eight measured slots were never dispatched.

The operation lock was released on worker exit and the original owner/token
record was kept as a recovery barrier. An initial close-only attempt
refused before deletion. A separate read-only check, under the same owner
and shared lock, confirmed the original stopped stage, no input/request or
binding evidence, unchanged native process projection and installed files,
and stable native Home with a clear pointer. The stage-only cleanup then
removed only that visit's untouched directory, without navigating to Library.
Independent absence, original identities and document/supervisor byte
readback passed. The original owner/token was returned at **13:32:10.215302
Asia/Shanghai** after device SSH closed. Worker readback confirmed no
WeRead stage, owner record, held operation lock, or staged stream remained.
The ordinary package and private shelf were not changed.

Keep the original `operations.json`, the refused `recovery.json` and the
successful `reconciliation-preinput.json` private. The last is the terminal
handback evidence; the older formal report does not consume that separate
receipt, and its original refusal must remain intact. Do not rerun any
operator or recovery script from the retired roster. A read-only resume
check on the worker can confirm the terminal state without opening Kindle SSH:

```powershell
$Worker = '/root/weread-display-handoff-20261128.formal-v4.t97vc2fk'
$Readback = @'
import json
with open("reconciliation-preinput.json", encoding="ascii") as source:
    record = json.load(source)
print(record["released"], record["recovery_required"], record["released_at_utc"])
'@
$Readback | ssh -T test-env "cd '$Worker/native-portrait-notify-comparison/visits/00-A/attempt' && python3 -I -B -"
```

## Replacement formal comparison: offline qualification and first seal

The new worker is
`test-env:/root/weread-display-handoff-20261128.formal-v5._5v31u14`.
The two preflights for warmups and eight for measured slots all passed. Its
fresh labels exclude the ten retired formal slots, the load-only visits,
and the earlier pilot labels. The frozen order is A, B warmups, then
`AB / BA / AB / BA`. Both archives are byte-identical to their retained
counterparts. The application, launch policy, instrumentation, receiver,
and measurement endpoints are unchanged. Only the test controller's
pre-input refusal and handback changed. The frozen BiliComics public-size
reference is the last reported verified `13f4434` installation; this is not
a new live readback and must be reconciled if that installation changes.

The native view is checked before and after the read-only stock scene probe,
before saving the navigation origin. A changed view still refuses startup.
For that tagged refusal alone, the [return contract](../../tools/startup-observer/STOCK-AB-CLI.md#transport-timing-and-recovery-for-each-visit)
allows removing the original untouched, stopped stage without returning to
the earlier tab. It requires no request, input or binding; unchanged original
files and native services; and a stable, coherent current stock tab and
pointer. Restore and close must observe the same scene. A conflicting scene,
foreign tag, input evidence, archive, or bind authority blocks the special
return. Normal post-navigation restoration, independent byte/identity audit,
and exact lease release are unchanged. A returned refusal still stops the
range and is not timed.

On `test-env`, 75 host tests and 40 isolated BusyBox transaction tests passed.
The BusyBox fixtures reproduced Library-to-Home before input and a change
during inspection. They also rejected a conflicting live view, foreign tags,
input or binding evidence, and a changed pointer before removal. The sealed
worker rehearsed both full staged streams under an isolated file lock and
synthetic transport (22 simulated device commands); both pre-input refusals
returned with zero samples. The fresh report accepted no unattempted slot as
measurement evidence. No physical device connection, lease or application
launch was made during this preparation. The original physical load-only
qualification applies to the unchanged archive receiver, not to the new
view-handback behavior.

## November 28 afternoon: A returned; historical identity gate stopped the range

After BiliComics returned its `ec9726e` static deployment, the unused first
seal was superseded. The refreshed worker is
`test-env:/root/weread-display-handoff-20261128.formal-v5.ooc6db2k`.
The peer reported its public executable at 75,052,096 bytes, supervisor at
89,581 bytes, and unchanged JPEG helper at 426,672 bytes. Only the two
changed size guards and fresh visit labels differed; both trial archives
were compared byte for byte and two isolated full-wire refusal fixtures and
ten new preflights passed. Physical admission matched the refreshed guards.

On Scribe 2022 / 5.19.5, the A warmup completed ordinary stock-card input,
Library submission, and the original handback. `stop`, `restore`, `close`,
independent stage absence and the before/after document and supervisor byte
audits all passed. Original identities agreed within that visit. The exact
owner/token returned at **13:59:48.146247 Asia/Shanghai**. Independent host
readback found no owned token, owner record, held operation lock, or staged
stream. There was no pending recovery and the daily package was not changed
by this trial.

The range controller then rejected the warmup because its installation
receipt did not exactly match the earlier same-day load-only reference. A
read-only worker comparison found that only the three inode fields differed:
original document, supervisor, and application. Other recorded identity
fields matched; the reason for the inode changes was not established. The
within-launch diagnostic replay independently accepted the populated frame,
matching `SEND` and `WAIT`, the automatic portrait path, and zero
notification waits in A. This is diagnostic only: it does not override the
formal rejection. The independent formal report has zero accepted rows,
zero pairs, and no outstanding recovery. The B warmup and eight measured
visits were not attempted. Neither v5 seal may be reopened.

To inspect the retired public report without Kindle SSH (from PowerShell):

```powershell
$Worker = '/root/weread-display-handoff-20261128.formal-v5.ooc6db2k'
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_report.py' --public"
```

## Pre-navigation installation gate: implemented and verified offline

The formal child visit now receives the exact `original` receipt from the
independently verified A/B load-only baseline. After recording the live
receipt and passing the original byte audit, it compares all three
identities before marking navigation possible or sending `prepare`. A
mismatch refuses the visit as `installation_baseline_mismatch`; the
controller stops, and the live original remains the target of the usual
cleanup, absence and after-byte checks. No inode check is weakened. Final
report qualification still checks the exact baseline, and close-only
recovery does not require the later baseline.

On `test-env`, 81 host regressions passed. The network-isolated formal
controller fixture replayed the actual retired inode-only discrepancy in
both lanes: both refused without navigation and returned under their live
original identities. A matching baseline reached preparation. Missing
baseline evidence prevented dispatch; a failed after-byte audit prevented
release. The overlay contains the integrated formal driver and an offline
verification receipt at
`test-env:/root/weread-notification-installation-gate-20261128.db_umre0`.
It deliberately contains no live roster or archives and cannot dispatch a
real device visit. From PowerShell, its isolated fixture can be rerun with:

```powershell
$Worker = '/root/weread-notification-installation-gate-20261128.db_umre0'
ssh test-env "cd / && unshare --net python3 -I -B '$Worker/verify-installation-gate.py'"
```

No Kindle connection or lease was attempted for that offline fix. The new
physical baseline and its separately sealed formal successor are below.

## Fresh physical installation baseline: passed

The new load-only worker is
`test-env:/root/weread-display-handoff-20261128.baseline-v6.ik6l82bb`.
Both new A and B visits passed on Scribe 2022 / 5.19.5, with the most recently
verified BiliComics `ec9726e` public reference. Their input archives matched
the previous verified arms byte for byte. Preparation verified both complete
wire streams under a private operation lock, before the physical attempt.

Both visits received successful retired `stage` and `install` operations on
closed transports, performed the independent document/supervisor byte audits,
and completed `stop`, `restore`, `close`, and independent owned-stage absence.
The original document, supervisor, and application identities matched exactly
before and after each visit and across both visits. They never prepared the
stock card, navigated, activated a selector or launched the application. No
timing sample or daily installation change resulted.

A returned at **14:18:21.911183**, and B at **14:21:05.172553** Asia/Shanghai on
November 28. A's verified return preceded B's independent acquisition. Final
worker readback confirmed no outstanding load, token, owner record, held
operation lock, or staged stream. These are historical returns, not a
reservation or an availability claim. Keep the original load-only receipts
sealed on `test-env`; only their verification status belongs here.

## Formal v6: seal and pre-navigation verification

The formal worker is
`test-env:/root/weread-display-handoff-20261128.formal-v6.593httjh`.
Its controller and report both use the fresh returned v6 baseline above.
A and B warmups and four measured pairs (`AB / BA / AB / BA`) have ten fresh
labels. The archives match the baseline exactly, and every device script
matches its corresponding physical load-only arm apart from that label.
The startup candidate, device receivers, and measurement endpoints did not
change. On `test-env`, all ten preflights passed. A network-isolated replay
used the actual new baseline and the older installation identity to exercise
both formal child paths: two complete wire compares and two verified synthetic
returns, without dispatching navigation or admitting a sample. The initial
formal report had zero accepted rows and ten untouched slots. No Kindle
connection or lease was made for the formal seal.

## Formal v6 physical result: stopped at measured B

All four visits matched the fresh physical installation baseline before
navigation. Both warmups and the first `AB` pair produced a populated Library
frame with matching device-clock `SEND` and `WAIT`, the automatic direction
reads path, and 0° rotation. All four passed native restoration and
before/after original-file identity and byte audits. The last lease was
independently verified returned at **14:46:12.013691 Asia/Shanghai**.
No owned stage, pending load, private token, or live transport remained;
worker readback also confirmed an empty staged-stream directory and an
unheld shared operation lock. The ordinary install was not changed.

The B warmup reported two waits, one ready hint, and no waiter error. The
first measured B reported two waits, **zero ready hints**, and no waiter
error. The controller returned status 6 (`portrait_notice_unproved`) after
handback, independently of the already requested safe yield. Six measured
slots were untouched. Neither warmup contributes to the measured pairs.

The raw first pair is diagnostic only. At the matching driver `WAIT`, A
was **5.962743–5.963623 s** and B was **6.574365–6.575819 s**. B was
**610.7–613.1 ms slower** for this pair; its owned portrait command was
**40–80 ms longer**. The optional publication bracket was not captured in
measured B. These are endpoint bounds from the synthetic release, not optical
settling or an estimate of the notification effect. The reporter exposes one
raw pair with its mechanism flag false; the number of eligible formal pairs
is **zero**. Do not retain only notification hits to claim a benefit.

Inspect the retired data on the worker without connecting to Kindle:

```powershell
$Worker = '/root/weread-display-handoff-20261128.formal-v6.593httjh'
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_report.py' --public"
```

The predeclared notification gate did not pass. Do not clear the stop marker,
resume v6, change the gate after seeing these data, or deploy the candidate.

## Offline diagnosis: a zero-hint result does not identify a lost event

The raw v6 records confirm `2 / 1 / 0` (waits / ready hints / waiter errors)
for the B warmup and `2 / 0 / 0` for measured B. The analyzer did not discard
an observed hint. The parent still read the authoritative completion for both
launches. Each watcher has a 100 ms deadline starting before setup, and opens
the directory watch before looking for the final file name. A rename between
the parent's check and the watcher's registration is therefore covered by the
post-registration lookup if it is processed within the slot. There is no
per-wait registration, publication, or wake timestamp in the retained trace.

Exit `1` is ambiguous. It can mean the slot expired before publication, or
that a queued event was not acted on before the deadline. The current helper
also disables an already-armed watch after certain poll/read failures, waits
the remainder, and can exit `1`; the launcher does not count that as a waiter
error. The parent checks the completion record again so none of those cases
implies that it lost the command result. The physical run cannot distinguish
them. Measured B's optional publication sidecar was absent from the trace.
The producer writes that sidecar only after the authoritative rename, and the
parent is allowed to reap the producer immediately after reading completion.
The missing sidecar therefore cannot locate the rename or identify the cause.

The B warmup spent 310–330 ms from launcher permission to status acceptance;
measured B spent 350–370 ms. The inner Rust probe was **231.877 ms** in
measured B, not slower than the B warmup's **239.194 ms**. The parent also
spent 60–80 ms after wrapper retirement validating output in measured B
versus 0–20 ms in the B warmup. These separate clocks do not place a rename
relative to a watch deadline. The larger end-to-end difference cannot be
assigned to notifications from this evidence.

On `test-env`, an isolated Linux build of the frozen watcher reproduced
successful hints before expiry, late publication after two expired waits,
and a timeout even when the file was renamed during an armed wait if the
process resumed after the deadline. A synthetic error on an armed watch
also returned `1`. The frozen producer fragment retained the authoritative
file when reaped between rename and the sidecar write. QEMU user-mode replay
of the byte-identical frozen ARM helper with public Scribe 5.19.5 and 5.20.1
libraries confirmed the ordinary hint, late publication and delayed wake
cases with QEMU's default CPU; an explicit Cortex-A8 run stopped at `SIGILL`
and supplied no watcher result. This is not a full firmware guest, physical
kernel trace, or identification of measured B's cause. No device connection,
lease, candidate change, or new roster was made. Worker fixtures and summaries
are retained at `test-env:/root/weread-notify-diagnosis-20261128.fdvwe2iA`.

## New diagnostic build: verified offline only

The [diagnostic generator](../../tools/startup-observer/portrait-notification/README.md)
creates private A/B launcher and helper sources from the retained v6 trial.
The ordinary package does not compile or stage them. The diagnostic watcher
reports actual deadline expiry separately from setup, poll, read, queue,
lookup, and clock failures. The generated launcher retains the bounded
polling fallback and still validates its own completion status; it never
makes a hint or the trace an authority. Recorded times cover registration,
rename bounds, userspace poll wake, named-event read, completed-file lookup,
and terminal outcome on one `CLOCK_MONOTONIC` clock. A missing rename end
remains an incomplete bracket even if the authoritative file survived.

On `test-env`, 14 network-isolated Linux tests passed, including forced
wake after expiry, injected poll/read/queue failures, producer interruption
after the atomic rename, the generated launcher's actual fallback, and
owned bounded trace collection. The separate ARM build requires only
`GLIBC_2.4`; QEMU user-mode checks with public Scribe 5.19.5 and 5.20.1
libraries passed timeout separation, already-published and armed-rename
cases on the shared clock. This is not a full firmware guest, physical
result, or resolution of the historical v6 miss. Verification summaries
and generated diagnostic sources are retained at
`test-env:/var/tmp/weread-notice-offline-20261128.nyeL7gaM`.

No new roster or production package was built, and Kindle was not contacted.
The clock reads, appends, and renamed-file publisher may perturb scheduling;
do not treat these builds as performance candidates. Do not change the v6
acceptance rule retroactively or restart its slots.

### Diagnostic collection: integrated offline

The [one-visit adapter](../../tools/startup-observer/portrait-notification/README.md#single-visit-collection)
collects after the verified `claim`, `witness`, and public Library submission
and before `stop`. The device rechecks the original GUI and session around
a bounded private snapshot; the host checks the visit label and exact
submission receipt and saves only validated rows and analysis. Missing or
refused evidence is independent of the original stop, restoration, audit
and owner-token handback. Recovery does not collect. All diagnostic visits
remain `not_timed`; no new path writes the formal v6 trace.

On `test-env`, 81 ordinary host and shared-lock regressions and 14 diagnostic
producer regressions passed in network-isolated Linux fixtures. All 16
collection tests passed with GNU tools and again with BusyBox. The fixture
uses the actual stock device shell and host transport, with synthetic input,
a simulated GUI, and a private kernel lock. It covers stale and replaced
records, an unbound GUI, absent or invalid output, a stalled reader separate
from the host, output storage failure, and exact-token recovery. Keeping an
open stage descriptor in an early implementation pinned the fixture mount
after host timeout; the final reader keeps neither a stage descriptor nor
its flock. Final logs are under
`test-env:/var/tmp/weread-portrait-collection-20261128.p34jGO1t`.
No local application tests or physical operation were run. The collector
does not infer a total wait count from the rows it receives.

There is no physical diagnostic plan or live controller ready to run. Before
an authorized diagnostic visit, verify fresh launcher/helper and public seed
bytes, an installation baseline, and a sealed diagnostic plan. Do not use a
retired timing slot as that plan. No ordinary package change or deployment
was made.

For any separately authorized, non-diagnostic comparison, use the existing
synthetic stock-card release bracket and the same-launch
render, `SEND` and matching positive, uncached `WAIT` endpoints. Require
at least three of four positive lower bounds and a positive paired median
lower bound for the entire owned portrait command. The matching Library
`WAIT` paired median saving lower bound must be nonnegative. Report every
pair and its bounds; a missing optional publication bracket is unmeasured,
not zero. An EPDC `WAIT` is not a visible or optically stable first screen;
visible loading dwell and optical first-screen timing remain unmeasured.

After an engineering pass, verify the ordinary non-diagnostic package and
its handback separately before promoting the notification change. Only
then may that change be merged and pushed, followed by building the formal
development package from the pushed `main` and deployment in a separately
coordinated window. The host trial tooling in this handoff is not a Kindle
package change and does not by itself warrant replacing the installed app.
