# Shared physical Kindle operation lock

Use the single Linux worker lock at `/var/lib/kindle-physical/device.lock` for
all Scribe access from WeRead, Aidoku and BiliComics. The worker module is
`/usr/local/lib/kindle-physical/device_lock.py`; its source is
[`device_lock.py`](device_lock.py). The shared file is a stable, root-owned
`0600` regular file in a root-owned `0700` directory. Do not unlink, rename,
recreate, or replace it between jobs. Installation is a one-time host
administration step, not part of an operator run. Nothing is installed on the
Kindle by this tool.

## Operator contract

1. Before attempting the lock, finish the build, verify the frozen inputs,
   prepare the transport and read its credentials through the approved private
   channel. Obtain device authorization and confirm the explicit handoff with
   any active requester. Do not infer a handoff from filesystem absence.
2. In the actual Python process that runs the device operation, import the
   installed module from `/usr/local/lib/kindle-physical` and enter
   `operation_lock()`. A busy lock raises `DeviceBusy` immediately before the
   body runs. Stop or use a cancelable wait outside an active visit; do not
   busy-poll or route through a legacy operator when the lock is held. A
   missing or untrusted lock raises `DeviceLockError` and must block device
   access. Do not fall back to a per-project lock.
3. While holding the file lock, create the existing exact owner/token record
   under `/tmp/codex-kindle-physical-test.lock` for the visit. For a recovery,
   verify the original owner/token instead. This record blocks a fresh visit
   if a disconnected or interrupted operation has unresolved device state.
   Never take over, remove or reuse someone else's record.
4. Perform only the prepared device operations. Stop the job, restore native
   control, remove only owned temporary files and close all device transports
   before returning the original owner/token record. Release the file lock
   immediately after the verified return, before qualification, reporting or
   more preparation. If the device is unreachable, keep the exact private
   recovery record and exit the lock; another operator must not treat a free
   file lock as proof of clean device state.

The lock descriptor is close-on-exec and explicitly closed in Python fork
children. Never pass it to subprocesses or acquire it in a parent controller
that waits for unrelated work. The kernel releases it when the operation
process exits; a fork/exec transport cannot keep it held. An unconfirmed remote
operation still requires recovery under the original record before another
visit. The worker's separate build lock is unrelated.

The [stock A/B single-visit runner](../startup-observer/run_stock_ab_visit.py)
uses this entry point for both a fresh visit and close-only recovery, after
local staging and immediately before the owner/token step. It does not import
the module during offline preflight. Previously frozen or unadapted live
operators are not valid entry points. Give each visit its own lock scope;
never hold a single lock across a roster or timed comparison.

## Verification

Run `test_device_lock.py` on the designated Linux worker as root. The tests
use a new private fixture path for each case and independent processes for
contention and forced exit; they do not take the installed lock or connect to
Kindle. The bridge tests in `../startup-observer/test_stock_ab_visit.py` and
`../startup-observer/test_stock_ab_cli.py` use fake leases and local transport
fixtures. They cover ordering, contention with no device dispatch, and recovery
without carrying the process lock across offline work.
