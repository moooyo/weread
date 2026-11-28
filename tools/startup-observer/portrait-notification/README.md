# Experimental portrait notification diagnostics

This tool adds observability to a private copy of the retired v6 trial
sources. It does not modify the ordinary Kindle package, the v6 inputs or
its acceptance rules. It does not prepare a physical roster or contact a
Kindle. The generated builds are **not startup performance candidates**:
clock reads, bounded appends and a dedicated `renameat` publisher can change
scheduling. Do not mix their timings with v6.

## What the receipt establishes

The existing numeric `native-theme-done` record remains the completion
authority. The launcher still checks the exact owned command and reads its
status itself. A watcher hint is not a successful command result. The
watcher's 100 ms slot begins before setup; the original watchdog remains
independent. Exit `1` means deadline expiry. Setup, poll, read, queue loss,
lookup, and clock errors have separate reasons and a non-timeout exit. The
launcher counts those as errors and uses its existing bounded polling
fallback. No diagnostic readiness or trace flush is required to proceed.

All recorded times are on the same Linux `CLOCK_MONOTONIC` clock:

- `publish_begin` and `publish_end` bracket the actual atomic rename. The
  begin is appended before the rename. If the launcher reaps the producer
  first or the trace write fails, the end may still be missing. Do not
  infer an exact publication time from a missing or failed bracket.
- A `wait` row records the slot, start, sample after watch registration,
  deadline, first userspace `poll` wake, first read of the matching named
  event, successful final-name lookup, terminal sample and outcome. The
  first wake might be unrelated. No userspace sample is the kernel's event
  enqueue time. Zero in an optional field means the sample is unavailable.
- `analyze.py` distinguishes a known late publication, a publication wholly
  inside the slot but observed after expiry, an in-window publication with
  no timely observation, an ambiguous boundary, and a watcher failure. It
  does not infer a scheduling cause or a lost kernel event. Pass the exact
  same-launch wait count if available to expose missing rows explicitly.

The diagnostic launcher reserves a small append-only trace inside its private
lock directory and removes that exact file with its own lifetime cleanup.
The parent does not wait for the trace to become readable or complete.

## Single-visit collection

`diagnostic_visit.py` connects `collect.sh` to a separate, one-shot stock-card
visit. It has no live CLI or roster generator. A future controller must
supply a verifier for fresh, byte-checked diagnostic materials and a plan
with purpose `portrait-notification-diagnostic`. Its verifier returns the
plan metadata, its underlying stock device program with concrete archive
length, and the fresh static ARM receive helper bytes. The adapter verifies
that the sealed device program is exactly `device_program(base, reader)`.
Use a trial renderer that separately checks the original installation references
and permits the verified diagnostic launcher/helper bytes. The ordinary
stock A/B freezer is not a diagnostic material verifier. For recovery use
the original plan, token, device program, and its frozen worker tools, not
another launch.

- The ordinary `claim`, `witness` and `library_submit` path must prove a
  same-launch submission for the disposable public shelf. The device binds
  the snapshot to that session, its submitted and observed samples, and the
  visit label. The host checks the same release/request window before
  requesting the read and rejects records outside the release-to-submit
  window. Neither the session identity nor unrestricted device output is
  exported.
- The read runs once before `stop`, while the host holds the shared operation
  lock and verifies the visit owner. It rechecks the GUI around a bounded,
  pinned private file snapshot. The read-only command has its own deadline
  (12 seconds on the device, 25 on the host). It rechecks the stage inode
  without retaining a stage descriptor or flock, so a stalled reader does
  not own any stage resource needed by `stop`.
- The normal stop, restoration, closed transports, absence check and byte
  audit still govern handback. In the private attempt directory,
  `portrait-notification.json` records `collected`, `unavailable`, or
  `not_attempted`, and whether the original attempt completed handback. The
  worker writes it outside the physical operation; a write error is reported
  separately. A zero visit exit status means the ordinary guards and handback
  passed, not necessarily that the snapshot was collected. Recovery never
  reads or overwrites the diagnostic receipt.
- These visits stay `not_timed` even when collection succeeds. The parsed
  result and validated trace rows are separate from v6's `startup_trace`.
  The collector does not request the launcher's optional wait count; missing
  slots remain unknown unless independently established for this launch.

## Install progress and EOF

The diagnostic script embeds the verified ARM helper before the first archive
byte; it does not fetch or install a receiver separately. Build
`receive-progress.c` with the worker's Zig, targeting static ARM hard float:

```sh
zig cc -target arm-linux-musleabihf -mcpu=cortex_a8 -mfpu=vfpv3-d16 \
    -mfloat-abi=hard -static -std=c11 -Oz -Wall -Wextra -Werror -s \
    receive-progress.c -o receive-progress
```

The sealed script must fit the transport's 120 KiB shell header limit. The
worker's full wire must contain that exact header followed by the exact
archive. No change is made to ordinary A/B packaging or to retired programs.

The private install operation now has `install_receive` in `operations.json`.
It contains the expected archive size, bytes read from stdin, bytes accepted
by the file write calls (not an fsync guarantee), the latest sample and last
successful data read, and a distinct EOF observation. Timestamps use device
`CLOCK_MONOTONIC`, in nanoseconds. EOF means `read` returned zero, not that
the file was full or that the SSH session returned normally. On a timeout
these are the last completed samples, not an exact deadline snapshot;
`eof_observed: false` means no EOF was recorded in that sample. No sample,
or a malformed or unretired reply, is represented by `null`, not by zero
bytes or a negative EOF claim.

An exact archive with no EOF cannot be promoted. The 150-second device receive
and 240-second host install deadlines, the length-plus-one fence, and the
original retirement and handback proof still apply. The snapshot is emitted
only after the waiting shell retakes the original writer lock, and the host
retains it only with the original closed transport and retirement record. The
snapshot neither authorizes cleanup nor certifies archive identity. A failure
in the optional progress record does not become an install or cleanup verdict.
For offline controls, run `test_receive_progress.py` under `unshare --net`
with the native compiler and again with `--reader "$QEMU_ARM" "$ARM_READER"`.

## Offline generation and verification

Use the designated Linux worker. Start with the two byte-bound v6 launcher
variants and their experiment-only native helper source, not the daily
helper on `main`. The generator requires that the launchers differ only in
whether initial display takeover uses the notification. It writes one new
private output directory and leaves the original files unchanged.

```sh
python3 -I -B prepare.py "$FROZEN_HELPER" "$ARM_A" "$ARM_B" "$NEW_DIR"
```

Run `test_diagnostics.py` with the same three source arguments in an isolated
Linux process/mount/network namespace, as root with a private `/tmp`. The
suite compiles the helper, exercises the actual generated publisher and
watcher, forces late processing and injected faults, verifies the launcher's
normal fallback and command validation, and checks collection and analysis.
Keep the namespace shell as Python's parent; the helper deliberately rejects
PID 1 as its parent.

For the end-to-end host/device fixture, run on the designated Linux worker
from outside any retained input directory. Set `TOOLS` to this tool directory
and `RETIRED_V6_DEVICE_SH` to a retained v6 device program. The latter is
read only for source and syntax checks, not run as a visit.

```sh
cd /
unshare --mount --pid --fork --mount-proc --net --propagation private sh -eu -c '
    mount -t tmpfs -o mode=1777 tmpfs /tmp
    TMPDIR=/tmp python3 -I -B "$@"
' fixture "$TOOLS/test_collection.py" "$RETIRED_V6_DEVICE_SH"
```

For BusyBox coverage, repeat with `WEREAD_TEST_BUSYBOX=1` and the reviewed
BusyBox applets on `PATH`. The fixture uses a private kernel lock, local
shell transport, simulated GUI/Library data, and no Kindle connections.

Use the firmware-ABI Zig flags from the ordinary build for a separate ARM
helper build of the **generated** source; do not stage it with the ordinary
package. Using the public Scribe libc on QEMU user-mode is an ABI and helper
check, not verification of the full firmware kernel or a physical run.

For an already collected private trace, on the worker:

```sh
python3 -I -B analyze.py "$TRACE" --wait-count "$SAME_LAUNCH_WAIT_COUNT"
```

Omit `--wait-count` if the original launcher count was not independently
captured. The analyzer never produces a startup sample or changes formal
admission. Any new physical diagnostic visit needs a fresh verified plan,
an installation baseline, and its own authorization.
