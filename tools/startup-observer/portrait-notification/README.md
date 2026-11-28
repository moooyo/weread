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
The parent does not wait for the trace to become readable or complete. If
integrating a **new diagnostic visit**,
use the generated `collect.sh` in a separate read before the launcher exits.
The fragment requires the existing `startup_trace_identity` and
`stock_private_record` proofs and rechecks them around a bounded snapshot.
It emits only these numeric records and fixed outcome names. Do not append
it to v6's existing startup trace or accept raw device output in its place.

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
admission. Any new physical experiment needs its own baselines, tested
collector integration, frozen roster, and authorization.
