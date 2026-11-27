# Handoff: Kindle startup notification comparison

Paused November 28, 2026 (Asia/Shanghai). **Resume with the two fresh
load-only checks, not with a timed comparison.** This document is a record
and checklist, not a reservation or authorization to contact a device.

## State at handoff

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

## Resume the untimed gate

1. Sync `main`, preserve unrelated working changes, and locate the retained
   private load-only worker and its Windows transport through the previous
   session's handoff evidence. Keep private plans, tokens, receipts and
   material archives off Git. The redacted verification summary is named
   `load-v2-verification.json` in that worker's logs; it is not a physical
   pass. If the private inputs are unavailable or changed, rebuild and
   reverify a fresh batch rather than reopening an old slot. The production
   notification source remains in the separate local
   `perf/kindle-portrait-completion` worktree. Its uncommitted comparison
   report is `docs/PORTRAIT_COMPLETION_AB_20261127.md`; do not copy private
   recovery receipts into this repository.
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

## Formal comparison and delivery are still gated

Only after both new load-only checks pass, freeze a new formal roster with
no reused labels: A and B warmups, then `AB / BA / AB / BA`. Do not pair the
historical measured A with a new B. Both arms share the same trial materials;
only B enables the notification on the initial display takeover. Each visit
must qualify the same populated Library frame and independently return
native control. Each pair requires the same automatic orientation path
and rotation, no notification wait in A, and at least one valid ready hint
with no waiter error in B.

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
