# Testing the startup decision with real stalled child processes

This experiment isolates the two startup orientation decisions on `test-env`
(Linux x86_64). It is **not** a measurement of a complete application,
Kindle service, physical sensor, framebuffer, touch, wake or panel.

## November 21 results

In each of the six measured pairs the uncalibrated control launched **two**
silent children and reached both real timeouts. The candidate with matching
simulated readings launched **zero**. Median combined time across the two
decision sites was **1,000.902 ms vs. 0.005260 ms**; median within-pair
difference was **1,000.896 ms**. The sub-millisecond value is for in-process
decision logic supplied by the fixture, not device reads or user-visible work.

| Two-site interval | Samples | Legacy calls / timeouts per sample | Combined time |
| --- | ---: | --- | --- |
| A: control, stalled child | 6 | 2 / 2 | 1,000.653–1,002.094 ms |
| B: matching simulated evidence, same stalled-child configuration | 6 | 0 / 0 | 0.000390–0.012259 ms |
| B: changed pose, stalled child | 1 diagnostic | 2 / 2 | 1,002.583 ms |
| A: immediately responding `U` child | 1 diagnostic | 2 / 0 | 44.828 ms |
| B: matching evidence, same responding-child configuration | 1 diagnostic | 0 / 0 | 0.009340 ms |

The [raw nanosecond samples](host-timeout-samples.csv) contain all 17
observations in order, including both excluded warmups and the diagnostics.
Only A1–A6 and B1–B6 enter the medians. Cargo reported the unoptimized
`test` profile: the one selected experiment passed in 8.06 s, with 153
unrelated tests filtered out. No remote device lease was taken.

## Method

The [ignored research fixture](host-timeout-bench.rs) builds inside a
temporary copy of the experiment's actual backend, using Cargo's host test
profile, not either ARM release binary. The same test process runs both
arms. It calls `resolve_launch_orientation` first for the splash, then for
the event loop. Both use the product's `query_render_offset`, its real
500 ms polling deadline and its normal child retirement. The firmware
process is replaced by a direct, silent `/bin/sleep 5` child. The parent
does not inject a delay or skip retirement.

- **A:** no reuse. Each lookup must time out, kill and wait for its own
  child. **B:** use `confirmed_launch_mapping` and `confirmed_launch_controller`
  with simulated matching upright pose, two rotation reads of `3`, and an
  unchanged initial calibration. No file, sensor or framebuffer is opened
  for these observations; the fake observer is `()`. Both hot sites must
  return without invoking the legacy query.
- After a complete A/B warmup, six pairs alternate `AB, BA, AB, BA, AB, BA`.
  Use per-process `Instant` for the two adjacent decision intervals; the
  total also includes the clock read between them. The program checks the
  legacy and timeout counts and the returned offsets on every sample.
- Three independent diagnostic observations of the **hooks only** follow the
  measured pairs: (1) a changed simulated pose forces the candidate through
  both silent timeouts, (2) both control queries receive an immediate `U`
  from `/bin/echo U`, and (3) the same responding child would be available
  with matching calibration, but the candidate does not start it.

Real calibration-file validation, framebuffer reads, evdev discovery and
snapshot costs, and the rest of `ensure_active` and the event loop are
outside this experiment. A fast B value measures only the decision with
already-supplied test data. Do **not** subtract the observed difference
from the [physical font baseline](font-baseline-breakdown.md).

## Reproduction

On the designated worker, the following uses an isolated source copy,
the shared compiler lock and an optional **project-private** Cargo target.
`PROJECT` is the worker directory mounted as `/work`, containing `src/`.
The script removes its temporary crate on exit. Never use another project's
target or run this measurement on the operator's machine.

The first setup stopped at offline Cargo metadata resolution for an unused
Android dependency. It did not build or time the fixture. The successful
retry filtered metadata to the host target and did not download packages.
The worker retains both `logs/host-timeout-bench.log` (setup refusal) and
`logs/host-timeout-bench-host-only.log` (completed measurement).

```sh
/opt/kindle-toolchain/ubuntu-run --build "$PROJECT" env \
  CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=/work/host-target/cargo \
  sh /work/src/docs/review/startup-orientation-20261121/run-host-timeout.sh /work/src
```
