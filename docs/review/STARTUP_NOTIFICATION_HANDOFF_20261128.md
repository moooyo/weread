# Handoff: Kindle startup notification comparison

November 28, 2026 (Asia/Shanghai). **The replacement formal roster passed
offline verification and has not started.** The previous formal comparison
stopped before its first launch with zero timing samples. Its failed A warmup
was independently cleared and returned; never reopen that roster. The separate
physical load-only checks passed. This document is not a reservation or
authorization to contact a device.

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

## Replacement formal comparison: offline-ready, unattempted

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

To recheck before seeking a new device handoff (from PowerShell):

```powershell
$Worker = '/root/weread-display-handoff-20261128.formal-v5._5v31u14'
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_ready.py' offline --series comparison"
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_report.py' --public"
```

Before live use, reconfirm the current daily and peer references, coordinate
conflict-free access and use the exact shared lock and private stdin
transport. No approval or reservation carries over from the cancelled
roster. Use the `gate` action with a fresh start and bounded admission limit,
then the `handoff.py range --series comparison` controller for the authorized
fresh slots. Its cutoff limits new admissions, not active cleanup. Only B
enables the notification on initial display takeover. Each pair must have
the same automatic orientation path and rotation, no notification wait in A,
and at least one valid ready hint with no waiter error in B. Stop the batch
and retain the original evidence on any refusal; do not reuse its slots.

Use the existing synthetic stock-card release bracket and the same-launch
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
