# Preflighting bundled glyphs: controlled experiment

## Question and design

Can WeRead move outline admission for the **seven immutable bundled font
files** out of application startup without changing what is admitted or
rendered? The [earlier investigation](README.md) measured the surrounding
font stage at 3.094 s on one Scribe; it did not measure the effect of this
change. The workspace baseline is `c810ea5`. This is a development experiment,
not an installation or a new estimate of physical tap-to-ink latency.

- **A:** interpret each bundled font's outlines during ordinary initialization.
- **B:** opt into the `bundled-font-preflight` Cargo feature. Generate receipts
  for those same embedded bytes using the project's admission code at build
  time; retain the shared runtime limits for the whole startup. Unrecognized
  or user-supplied fonts must not be trusted as bundled assets. Do not change
  the payloads, weights, Fontique registration, text metrics, shelf logic,
  launcher settling interval, or the orientation decisions.
- Use one source snapshot per matched comparison, one release profile,
  one display configuration, and the same restored public offline data. The
  two arms must use identical timing instrumentation. Warm up each arm once,
  then run six pairs in fixed `AB, BA, AB, BA, AB, BA` order. Keep individual
  times and slower pairs. Run captures separately from timing trials.
- Primary host endpoint: `window_created` to `fonts_registered`. Secondary:
  `main_enter` to `software_ready`. Use the same monotonic clock within each
  process. Neither endpoint proves that ink changed, the first display
  submission completed, the book covers arrived, or touch was accepted.
- Correctness gates: exact embedded font identities, equivalent per-face and
  cumulative outline budgets (including target-dependent state charges),
  unchanged dynamic admission/failure accounting, Library/Reader pixel
  comparisons and all 15 expected public Library cover images. Examine the
  32-bit target separately from the build host.
- A separate observation-only change tags the two existing native orientation
  queries when `WEREAD_STARTUP_TIMING=1`. Keep it common between comparable
  binaries. Do not shorten a timeout, assume a failed query was a timeout,
  or credit the font candidate with orientation savings.

The evaluation environments and results below are independent. In particular,
a Linux desktop result or ARM user-mode emulator run must not be reported as
the physical Kindle's speedup. Leave the feature off by default pending a
matched, isolated on-device comparison and ordinary display/input recovery.

## Measured results

**Complete Linux desktop application, not a Kindle.** On `test-env`, the
x86_64 release builds used Winit's software renderer at 930 x 1240, scale 1.
Each process opened its own restored copy of the same offline Library seed.
The [six pairs in their original order](desktop-ab.json) include every warm
timing trial. Captures and the two independent path checks are not timing
samples. Before timing, an actual baseline process interpreted seven faces
and used zero receipts; an actual candidate process interpreted zero and used
all seven receipts.

| Interval | Baseline median | Candidate median | Difference of medians |
| --- | ---: | ---: | ---: |
| Window created -> fonts registered | 325.337 ms | 0.721 ms | **324.616 ms (99.78%)** |
| Rust main -> software ready | 456.981 ms | 115.500 ms | **341.482 ms (74.73%)** |
| Process launch request -> software ready | 459.365 ms | 117.651 ms | 341.714 ms (74.39%) |

All six pairs favored the candidate at all three endpoints. Font-stage
savings per pair were 322.860-326.348 ms. Software-ready includes work other
than fonts; do not attribute every millisecond of its difference to the font
loop. In separate runs, the warm shelf (15/15 cover matches), a shelf with
one missing cover (14/15 plus the exact fallback), and the Reader each had
**zero differing pixels over the full frame** between A and B. These are
desktop software pixels, not Kindle display submissions.

**Correctness on the verification host.** The default Main suite passed
56/56; the opt-in Main suite passed 58/58, including identical accounting
and partial charges on rejection. A same-length one-byte change in an
immutable font failed the build against the trusted manifest. The complete
Kindle backend unit suite, including the opt-in orientation timing markers,
passed 150/150 on Linux x86_64.

A separate incremental check ran the real build script under a small Cargo
driver. Same-length changes to the generated OTF and to the charge declaration
were each detected and repaired on the next build. That fixture normalized
an unrelated generated Slint file's timestamp; it does not claim that the
entire application always has a no-op rebuild. Temporary storage exhaustion
interrupted one initial default-suite build, not a test assertion; the
completed 56-test rerun used a private cache without incremental compilation.

**32-bit target semantics.** A separate [static ARMv7 probe and evidence](arm-admission/README.md)
ran on `test-env` in QEMU user mode, not on Kindle firmware. Recomputing the
seven receipts using the target ABI produced the same per-face and cumulative
resource use. Seven budget failure cases produced the same errors and partial
charges. Copied and truncated slices could not select a receipt. No QEMU
duration is used to infer hardware latency.

Both [full ARM release builds](arm-build.json) are stripped, static ELF32 ARM
hard-float executables from the same source, without the desktop measurement
patch. The baseline is 95,120,368 bytes and the candidate 95,120,920 bytes
(+552). Building these executables does not qualify a Library launch. The
normal application and launcher were not replaced.

## Physical follow-up

No physical lease was acquired, no file was sent to the Kindle, and no new
hardware timing was collected in this experiment. The previously measured
9.820 s formal-entry SEND is a reference for that older candidate, **not**
the result of enabling this feature. In particular, do not subtract the
desktop saving from it or apply the desktop percentage to it.

The seven-font CPU-only probe is about 61.5 MiB. On this Scribe `/tmp` was
previously verified to share the [64 MiB `/var` tmpfs](../../PHYSICAL_TESTING.md#temporary-memory-cleanup);
do not upload it there or evict user caches. The verification host became
unreachable while preparing an alternative user-partition staging plan. That
plan was not verified or run. The already-completed desktop and target ABI
results above are separate from this blocked physical step.

First re-establish `test-env` and confirm a fresh exclusive WeRead device
window. Validate isolated staging and cleanup **before** device writes. An
on-device CPU probe can isolate the glyph-loop effect, but cannot time the
Library. For the decision to enable the feature, use the two matching ARM
application builds with the same public offline shelf and launcher. Compare
warm pairs from native request to first Library SEND or a qualified shelf
pixel bracket; include the orientation outcomes, Library and Reader pixels,
and native recovery. Record an actual tap or optical display endpoint only
when it is independently observed.

The [November 20 worker recovery](rebuilt-worker.md) supersedes the
connectivity and retained-input assumptions, not the November 19 measurements.
The [later physical CPU run](physical-20261120.md) measures the seven-face
admission loop (2,709.496 vs. 3.557 ms median). Its independent installed-entry
audit did not launch either application or qualify first Library SEND.

## Implementation and next decisions

The build checks the decompressed bytes against the SHA-256 in the approved
font manifest, then runs the actual CFF interpreter. Its receipt covers just
the glyph loop: work, visits, points, and peak outline memory. Header checks
and descriptor scratch costs still run on the target, where pointer width
can differ from the build host. The lookup is by full pointer and length of
a named static byte array, not by filename, length alone, or a cache file.
Remaining aggregate budgets are checked before any charge is applied. Stricter
per-glyph limits or insufficient remaining budget run the normal interpreter,
so the exact error and work already consumed on failure remain observable.

An initial feature-enabled test run passed 57 of 58 Main tests. Content was
unchanged, but an anonymous `include_bytes!` allocation could be duplicated
by constant promotion: one lookup did not select its receipt. A safety fallback
that silently misses would invalidate a performance experiment. The final
design gives each embedded array its own static backing. Do not mix the
rejected build or its early measurements with the matched sample.

Two other experiments stay independent of this candidate:

1. `splash_orientation_*` and `event_loop_orientation_*` distinguish success,
   timeout, unavailable and failure per call. Collect these on the native
   path; only consider deduplicating a query when its cost and the current
   launch's usable orientation calibration are both established. Preserve
   holding-vs-scanout mapping, reversed portrait, and wake fallback.
2. Keep the two-second launcher wait unchanged. The launcher's
   `observer-stream-ready` record is published **before** `exec lipc-wait-event`;
   it does not demonstrate that the firmware subscriber has already attached
   or that native display takeover is safe. A shorter wait needs its own
   matched startup, interrupt-during-startup, sleep/wake, and exit cases.
