# From the native Library to the WeRead shelf

Investigation date: 2026-11-19 (Asia/Shanghai). WeRead source: `c810ea5`.
No WeRead runtime change or deployment is part of this investigation.
The [subsequent controlled font experiment](font-preflight-experiment.md)
tracks the opt-in implementation separately from these original measurements.

## What is actually timed

The closest qualified WeRead measurement is the November 17 [indexed-entry
experiment](../native-library-entry-20261117/README.md#formal-entry-measurements)
on one **Kindle Scribe 2022, firmware 5.19.5**. Its six measured candidate
launches used the real indexed document, the waiting supervisor, ARM
`d3a6b663d9fb`, and an isolated offline shelf. The app process was started
anew each time, on the same boot after warmup. These are not measurements of
the currently installed daily app, a reboot, a user's finger, or the optical panel.

| Endpoint from the native file-open request | Median | Six-run range |
| --- | ---: | ---: |
| Entry into Rust `main` | 5.030 s | 4.850-5.350 s |
| App startup feedback initialized (not the shelf) | 5.610 s | 5.430-5.950 s |
| App window shown and state attached (`software_ready`) | 8.980 s | 8.750-9.350 s |
| Driver accepted the first Library frame (`SEND`) | **9.820 s** | **9.610-10.230 s** |

The six SEND intervals, in round order, are **9.610, 10.040, 10.230,
9.810, 9.830, 9.740 seconds**. Warmups and two separate framebuffer
qualifications are excluded. The latter confirmed native/software pixel
agreement; they did not time the panel. The separate 9.090 s result was
obtained with a different temporary entry, not this formal-entry baseline.
The [daily deployment record](../../PHYSICAL_TESTING.md#skills-guide-preserved-data-deployment)
and the [subsequent source integration](../../CATALOG_ENTRY_ADAPTER.md) do not
prove that this candidate is currently installed.

## Where the time goes

New analysis on `test-env` reads `shell.tsv` and `native.tsv` from
`weread-library-entry-20261117-r1/private/library-entry-8cbce1e6/outputs/out`,
cases `candidate-01` through `candidate-06` only. Each final metadata record
said `Library`; all six traces had both `portrait_transport_1` and
`portrait_valid`. External endpoints use the device's `/proc/uptime` at
request and receipt (10 ms resolution); internal spans use differences on
`CLOCK_MONOTONIC` only. Each median is calculated from six within-run
differences. Rows overlap where indicated; do not sum independent medians.

| Interval (start -> end) | Median | Range | Interpretation |
| --- | ---: | ---: | --- |
| File open -> document entry | 0.400 s | 0.370-0.470 | Native dispatch; part of the 5.030 s before `main`. |
| Power observer ready -> settling finished | 2.020 s | 2.010-2.020 | A fixed `sleep 2`, including scheduling and instrumentation. |
| Settling finished -> display takeover begins | 0.555 s | 0.480-0.700 | Surrounds the native settings and sleep-cover configuration calls. |
| Display takeover begins -> takeover done | 1.515 s | 1.370-1.850 | Includes the 0.385 s median portrait calibration, plus native service checks, frame guard and transfer of ownership. |
| Platform ready -> startup feedback ready | 0.555 s | 0.552-0.564 | Includes early preferences, direction lookup and drawing the splash, not just the drawing time. |
| Startup feedback ready -> bundled font bytes available | 0.0014 s | 0.0011-0.0020 | Distinct from validating/registering the fonts. |
| Window created -> fonts registered | **3.094 s** | **3.069-3.132** | Window configuration and the full font initialization path, including seven bundled faces. |
| App attach begins -> app attached | 0.139 s | 0.131-0.152 | Public offline fixture only; not a bound for a private or much larger shelf. |
| Software ready -> first SEND | 0.860 s | 0.825-0.879 | Includes the following two intervals. |
| Touch opened -> portrait opened | 0.524 s | 0.517-0.537 | Night-mode setup, buffers, direction lookup and active portrait observation. |
| Portrait opened -> first SEND | 0.307 s | 0.301-0.337 | Includes sizing, event-loop work, first draw, conversion and submission. |

The observed settle interval plus the font interval occupied **50.4-52.9%**
of the same run's end-to-end time. Between the fastest and slowest runs,
500 ms of their 620 ms difference had already accrued before Rust `main`.
Those are descriptions of this sample, not guarantees about tail latency.

## Verification-host experiments

These are new Linux x86_64 measurements on `ssh test-env`, **not** new
Kindle measurements. Neither benchmark opens the physical device.

| Isolated experiment | Result | What it establishes |
| --- | ---: | --- |
| Seven bundled OTFs, subset of startup work around admission/registration | Median 314.401 ms (313.566-318.479), six runs after two warmups | Bytes were already resident. All seven files matched the font manifest; 228,824 CFF glyphs and 64,104,120 font bytes were processed in production order. |
| `admit_font_glyphs`, within those runs | Median 301.291 ms; 95.81-95.85% of each run | Checking the outlines dominates this **host harness**. Temporary and shared Fontique registration together took under 1 ms. The three non-bold Serif weights accounted for 52.90-53.19% of host glyph-check time. |
| Two serial production orientation queries, silent synthetic child | Median 1,000.897 ms per pair (1,000.674-1,001.103); six pairs | Two independent 500 ms deadlines can cost about 1 s when there is no response. |
| Same two serial queries, immediate valid `U` response | Median 41.148 ms per pair (40.648-42.449); six pairs | A responding service has a different cost. The previous physical markers do not reveal which outcome occurred. |

The optimized font harness reused the production glyph admission and budget
code, x86_64 release dependencies, and a stand-alone Fontique collection, not
the entire Slint startup. The raw per-face record on `test-env` was
`/tmp/weread-font-host-BFrVCwM6/raw.csv`, SHA-256
`4590d2e54ffb4c8700bfc92ea04f511bf8b9d5eaf67fe48875fac8e1786bc6ed`.
The seven-font totals, in measured round order (ms), were `316.879, 314.702,
313.566, 313.637, 314.100, 318.479`; the glyph checks alone were `303.661,
301.530, 300.471, 300.566, 301.051, 305.191`.
The orientation harness used the unchanged production parser, deadline,
polling and child retirement code. In microseconds, the six silent pairs were
`1000676, 1000674, 1000968, 1000881, 1000912, 1001103`; the responding pairs
were `41436, 42449, 41102, 40672, 41194, 40648`. Do not multiply the host
percentages by the device font interval or subtract 1 s from the Kindle result.

## Experiments worth doing

1. **Look for a repeated orientation timeout.** Both the [startup splash](../../../vendor/slint-backend-kindle/src/splash.rs)
   and [initial business frame](../../../vendor/slint-backend-kindle/src/platform.rs)
   call `launch_render_offset`. On an accelerometer-equipped device it asks
   `winmgr` for the orientation, with a 500 ms polling deadline. The launcher
   already queries the native direction before taking the display and may
   have paused the window manager by the time these calls run. The two
   surrounding intervals (555 and 524 ms) fit two deadlines, but do **not**
   measure the calls in isolation. Record per-call elapsed time, success vs.
   timeout, and accepted calibration before changing anything. Test reuse of
   fresh, validated launch orientation on normal and inverted portrait, with
   the original fallback for missing or changed calibration and after wake.
   Do not assume a `Down` holding direction means flipping the displayed
   pixels: the [mapping](../../../vendor/slint-backend-kindle/src/orientation.rs)
   depends on holding **relative to scanout** and differs from touch rotation.
2. **Profile the seven fixed fonts.** [Font initialization](../../../src/main.rs)
   validates glyphs, registers the faces and prepares metrics synchronously.
   Test equivalent build-time admission for the exact bundled bytes, keeping
   the runtime work budget and full dynamic checks for custom fonts. Check
   book title, missing cover fallback, Reader layout, selection and the first
   interaction. The shelf itself needs Serif Bold; postponing all Serif
   registration is not equivalent.
3. **Test a condition instead of an unconditional two-second wait.** The
   [launcher](../../../packaging/kindle/launch.sh) waits after subscribing to
   power events. Establish an observable ready condition with a bounded
   fallback; compare sleep during startup, ordinary exit, and restoration
   of native input/display, not just the fastest successful launch.

For scale, reclaiming **1.0 s** of settle time and **half** the font interval
would put a 9.82 s run at approximately **7.27 s** (26% less). This is a
sensitivity calculation, not a prediction, a tested speedup, or evidence
that the two changes can be combined without cost. Neither private database
work nor downloading covers has been demonstrated to dominate this fixture.

## November 19 measurement scope

For new reports, use [ordinary stock tile release to the first complete
Library frame](../../PHYSICAL_TESTING.md#library-first-frame-startup). Keep
the recorded file-open-request-to-first-Library-`SEND` baseline unchanged:
it lacks that release timestamp. A later interaction or optical observation
is not part of the current primary interval.

- No fresh daily-build timing was obtained in this investigation. The same
  device was in another project's live measurement workflow and an explicit
  WeRead handoff was outstanding. No device operation or lease acquisition
  was performed by this study.
- Confirm the current **WeRead** device window with the higher-priority Aidoku
  owner and acquire a fresh exclusive lease on `test-env`. An absent lock or
  a prior release does not reserve the next window. Do not edit the installed
  entry, shared registration, private shelf, or peers to measure the daily
  build. The November 17 runner replaces the document and is not appropriate.
- The November 18 `--aidoku-handoff` protection profile is **not current** for
  Aidoku's [November 19 R3 installation receipt](https://github.com/moooyo/aidokuForKindle/blob/3d8f0d7d5d2981d56f9df08c092fbd35fd33f8d2/docs/review/startup-r3-qualification-20261119-8baf57f/release/installation.json).
  The owner records a changed app and launcher and a new portrait helper.
  Freeze the latest joint inventory from the owners before observing it;
  reject drift, never use device readback to invent trusted expectations.
- The study distinguished native file-open requests, touch releases, digital
  shelf pixels and optical ink. It did not time an ordinary tile release to
  a qualifying Library frame, an effective tap or optical settling. In a new
  warm trial, retain the entry and cache conditions, label automated touch
  input as synthetic, and record release and the qualifying render/submission
  event on the same device clock. Do not mix endpoints or clear private caches.

### A passive endpoint for the installed build

The research-only [framebuffer observer](../../../tools/startup-observer/scribe-library.c)
does not require a timing-enabled WeRead binary. It opens `/dev/fb0` read-only
and samples masked ink/paper points at the fixed shelf heading and selected
bottom navigation icon; it does not export pixels or inspect the book grid.
Only the recorded Scribe geometry (1860 x 2480, Gray8, stride 1872, rotation
3, zero pan) is admitted. Starting on an existing match is rejected. Two
positive samples about 50 ms apart confirm the digital signature. Keep the
last negative sample's beginning and first positive sample's end as a bracket;
the confirmation time is later and must not replace the first match. Record
the trigger on the **same `CLOCK_BOOTTIME` domain**. A default evdev timestamp
is not sufficient without choosing or correlating the clock. A mismatch,
changed mode, unexpected initial screen or timeout is not a timing sample.

On `test-env` the [public replay](../../../tools/startup-observer/test_scribe_library.py)
matched two public frames from physical batches and two firmware-emulator
shelf views. It rejected 14 actual non-shelf frames (including startup and
native Home), five synthetic negatives and a truncated input. A static ARMv7
observer was built but not installed, launched or qualified on this Kindle.
The template does not generalize to another theme, resolution, rotation or
layout without new qualification. Matching means **shelf chrome in display
memory**. It does not by itself establish the required entries or empty state,
the render/submission timestamp, cover completion or optical panel state.

To reproduce the expanded replay, run the following in a checkout **on
`test-env`**, where the public Scribe fixture gallery is retained. Omitting
the optional inputs runs the smaller repo-only set. No Kindle is accessed.

```sh
python3 tools/startup-observer/test_scribe_library.py \
  --gallery /root/weread-ui-review-20261114-r1/delivery/captures/scribe \
  --negative /root/weread-emulator/artifacts/ux-extended-final/startup-06.png \
  --negative /root/weread-emulator/artifacts/ux-extended-final/after-explicit-exit.png
```
