# Handoff: Kindle startup notification comparison

November 28, 2026 (Asia/Shanghai). **Both untimed load-only checks passed.**
Do not repeat either slot. The next step is a fresh formal comparison roster
and a separate device authorization. This document is not a reservation or
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

## Formal comparison: frozen, not started

The formal worker is `test-env:/root/weread-display-handoff-20261128.formal-v4.t97vc2fk`.
The closed load-only evidence remains in
`test-env:/root/weread-display-handoff-20261127.load-v4.5sx2ocqq`. All ten
fresh slots passed offline preflight: A and B warmups, then
`AB / BA / AB / BA`. The formal archives are byte-identical to the physical
load-only inputs, and every timed slot's device program matches its tested
arm apart from the fresh label. The independent starting report rejected all
unattempted slots as timing evidence. Do not pair the historical measured A
with a new B. No formal run has contacted the Kindle or obtained a lease.

From the operator's PowerShell, after approval to connect to the worker,
rerun the read-only preflight and inspect the redacted report:

```powershell
$Worker = '/root/weread-display-handoff-20261128.formal-v4.t97vc2fk'
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_ready.py' offline --series comparison"
ssh test-env "cd / && python3 -I -B '$Worker/driver/notify_report.py' --public"
```

A live comparison needs a separately approved device window, direct
cross-project clearance, and the private live stdin transport. Only the
actual visit process obtains the shared lock. Both arms share the same trial
materials; only B enables the notification on the initial display takeover.
Each visit must qualify the same populated Library frame and independently
return native control. Each pair requires the same automatic orientation
path and rotation, no notification wait in A, and at least one valid ready
hint with no waiter error in B. Stop at a failed visit; never reuse its label.

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
