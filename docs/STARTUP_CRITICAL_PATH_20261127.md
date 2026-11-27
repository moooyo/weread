# Kindle Library startup critical path

Date: 2026-11-27 (Asia/Shanghai). Production baseline: `16af2c9`.

The preceding production-only phase diagnostic on a 2022 Scribe (5.19.5)
observed three device-clock intervals to Library submission. Median time was
1.682 s from the state-open record to `SEND`, 1.054 s from the first observed
input descriptor to `SEND`, and 1.022 s from the power observer record to frame
guard readiness. Observing a descriptor is not input readiness; those spans
identified where to instrument, not specific bottlenecks. The earlier runs do
not contribute samples to this comparison.

## Changes under comparison

- The launcher avoids shell subprocesses for readiness budgets, the native
  command's permission-record read, and window-manager identity reads. It
  retains the original native query order, timeout budgets, process-group
  cleanup, and restoration obligations. The frame guard batches three X11
  atom requests without changing its fullscreen or topmost confirmation.
- The Kindle backend registers its embedded default font from static bytes.
  It no longer stages a temporary font or changes the process environment.
  The splash worker no longer needs a middle stop and restart; its final
  framebuffer release and join still precede the event loop.
- The Library reuses SQL statements for bounded metadata and cache reads.
  Frontlight replies publish only the corresponding controls instead of
  repopulating the shelf. Fixed cover-title slots avoid unnecessary layout
  measurement while preserving the downloaded indicator and title elision.
- Grayscale frame writes select the pixel conversion once per frame. The
  luminance, inversion, monochrome threshold, raw-image dithering, and
  dirty-region rules are unchanged.

The native input discovery optimization is not included: directory listing
introduces different permission and mount behavior, and a separate speedup
was not established. The original qualified input discovery remains in use.
A dedicated lightweight portrait query is separate follow-up work.

The candidate does not defer the first Library draw or hide cached covers to
reach the endpoint sooner. It keeps the existing native ownership checks,
orientation observations, framebuffer waveform, and driver wait.

## Measurement contract

The endpoints follow the [physical stock-card timing contract](PHYSICAL_TESTING.md#library-first-frame-startup).
The fixed power wait was already removed in the production baseline. This
comparison cannot count that earlier change as a saving.

Both arms use the ordinary stock Library card, the same disposable 15-book
public shelf with cached thumbnails, and identical diagnostic instrumentation.
The candidate combines the changes above; it is not a one-change-at-a-time
experiment. Both measurement payloads use the standard production build
features plus the existing trial Library-frame feature. The installed ELF is
not executed and the daily data is not used. The shared dispatcher is
unchanged. Each timed trial selects its disposable application and data
through a temporary single-file bind on WeRead's own launcher, with exact
original-file and native restoration checks before its individual lease can
be released.

The native synthetic release is bracketed on the device's `CLOCK_MONOTONIC`.
The first complete populated Library frame eligible for the trial witness
must emit the original one-shot receipt in that same launch. This does not
identify any earlier ineligible draws, prove that every cover is loaded, or
record when touch becomes usable. Additional markers distinguish renderer return,
framebuffer write completion, successful full-frame submission (`SEND`), and
a successful uncached existing `WAIT` for the same positive update marker.
The measurement builds share ordinary-update numbering between the loading
renderer and the application framebuffer so those two paths cannot reuse each
other's marker. A raw-image or managed-sleep presentation is not eligible.
Both arms keep the existing refresh and wait count. The diagnostic takes the
`SEND` time immediately after acceptance; the unchanged one-shot receipt
independently qualifies the Library scene. The interval from accepted `SEND`
to `WAIT` return includes receipt handling and scheduling, not just the ioctl.
The matched `WAIT` is a driver endpoint, not optical verification of the ink.
The splash is not the first screen. The splash worker's join marks a user-space
framebuffer handoff, not the end of its visible display. Visible splash dwell
was neither measured nor subtracted. These card-to-Library proxy intervals do
not measure the first-screen metric with that dwell excluded. Both arms have
the same instrumentation, but its absolute overhead was not measured. Stage
boundaries enclose adjacent work; they do not isolate individual changes.

The sequence uses one warmup per arm followed by four preset pairs (`AB`,
`BA`, `AB`, `BA`). It does not reset the device or system page cache. Each
pair requires the same observed rotation and verified restoration after both
visits. Pairwise reductions are calculated before aggregation; positive values
mean B was faster. The release brackets describe timestamp uncertainty, not
run-to-run variance or a confidence interval.

## Software verification

On the designated Linux worker, the final Kindle-feature candidate passed
1,731 library tests (seven existing cases ignored) and 59 entry tests. The
same final backend passed 164 tests with the standard package's orientation
feature and 170 with both optional orientation features enabled. All 244
launcher tests, 36 Xvfb guard tests, and 55 BusyBox stock-card transaction,
request, and qualification tests passed. Standard package feature selection,
the static ARM executable, and the scoped development-bundle publish and
rollback rehearsal also passed. The candidate and baseline have the same
pre-existing workspace-format discrepancies; the modified project Rust files
add none. No local application verification was run.

On the desktop software renderer, the two Library layouts at 930 × 1240 and
632 × 840 matched pixel-for-pixel below the changing status bar, for the
ordinary public fixture and a long Chinese title. The fixture has cached
covers and a downloaded book. Desktop screenshots do not qualify a physical
panel or native touch path.

The initial candidate passed the normal entry, populated shelf, QMP touch
navigation, and native handback on isolated Scribe 2022 and Paperwhite 6
firmware 5.20.1 guests. The early PW6 attempts reached the guard while the
firmware boot splash was still viewable. The same guest also reproduced that
fullscreen refusal with the production baseline before the candidate later
opened normally. This is an emulator readiness issue, not a timing comparison
or an argument to loosen the display guard. The rebuilt uninstrumented final
candidate restores the baseline evdev discovery source and passed the same
populated Library, touch navigation, and native-return check in the Scribe
5.20.1 guest. The PW6 observation belongs to the earlier candidate; there was
no physical PW6 run.

## Physical A/B results

On Kindle Scribe (2022), firmware 5.19.5, all ten visits qualified, ran at 0°,
verified the original installation and native handback, and returned their
leases. The last lease was released at 10:39:25 Asia/Shanghai. Two warmups are
shown but excluded from statistics. The [normalized device receipts](review/startup-critical-path-20261127.json)
retain the release brackets, observed stage timestamps, and pair calculations.
No daily installation was replaced or private shelf used in this experiment.

| Visit | Pair / order | Arm | Render complete (s) | Accepted `SEND` (s) | Matched `WAIT` return (s) |
| ---: | --- | :---: | ---: | ---: | ---: |
| 0 | Warmup | A | 6.243356–6.244354 | 6.284548–6.285546 | 6.650237–6.651236 |
| 1 | Warmup | B | 5.498277–5.500108 | 5.531655–5.533485 | 5.897143–5.898973 |
| 2 | 1 / AB | A | 6.353135–6.354541 | 6.397264–6.398669 | 6.762639–6.764044 |
| 3 | 1 / AB | B | 4.847141–4.848238 | 4.892800–4.893896 | 5.257702–5.258799 |
| 4 | 2 / BA | B | 5.448343–5.449370 | 5.486139–5.487166 | 5.851618–5.852644 |
| 5 | 2 / BA | A | 5.647662–5.649261 | 5.690674–5.692273 | 6.058702–6.060300 |
| 6 | 3 / AB | A | 6.319602–6.321810 | 6.367797–6.370006 | 6.733611–6.735820 |
| 7 | 3 / AB | B | 4.505106–4.506129 | 4.612298–4.613321 | 4.977497–4.978520 |
| 8 | 4 / BA | B | 5.032296–5.033677 | 5.081688–5.083069 | 5.446766–5.448146 |
| 9 | 4 / BA | A | 6.198583–6.200086 | 6.246999–6.248503 | 6.612468–6.613971 |

| Pair | Order | `SEND` saving bounds (ms) | `WAIT` saving bounds (ms) |
| ---: | :---: | ---: | ---: |
| 1 | AB | +1503.4 to +1505.9 | +1503.8 to +1506.3 |
| 2 | BA | +203.5 to +206.1 | +206.1 to +208.7 |
| 3 | AB | +1754.5 to +1757.7 | +1755.1 to +1758.3 |
| 4 | BA | +1163.9 to +1166.8 | +1164.3 to +1167.2 |

| Endpoint | A median (s) | B median (s) | A mean (s) | B mean (s) | Median paired saving (ms) | B faster |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Render complete | 6.260 | 4.940 | 6.131 | 4.959 | +1336.2 | 4/4 |
| Accepted `SEND` | 6.308 | 4.988 | 6.177 | 5.019 | +1335.0 | 4/4 |
| Matched `WAIT` return | 6.674 | 5.353 | 6.543 | 5.384 | +1335.4 | 4/4 |

Endpoint summaries use the midpoint of each release bracket. For `WAIT`, the
median paired saving lies between 1,334.1 and 1,336.8 ms given the recorded
release bounds; individual pairs saved about 207–1,757 ms. The arm medians
alone differ by 1.321 s (19.8%), distinct from the paired median saving.
These four pairs show improvement under this fixture, not a general speedup
estimate for authenticated shelves, cold boots, or another device.

### Where the time went

| Consecutive observed interval | A median (ms) | B median (ms) | Median paired saving (ms) |
| --- | ---: | ---: | ---: |
| Stock-card release to selector | 446.2 | 511.3 | −41.4 |
| Selector to display ready | 2390.0 | 2121.1 | +257.2 |
| Display ready to Rust `main` | 76.8 | 105.4 | −38.2 |
| Rust `main` to backend ready | 822.5 | 234.4 | +603.9 |
| Backend ready to application attach start | 260.6 | 293.0 | −20.7 |
| Application attach | 1039.2 | 585.7 | +406.1 |
| Attach end to Library draw start | 123.4 | 94.7 | +34.1 |
| Library software draw | 702.0 | 809.5 | −73.9 |
| Framebuffer write | 41.1 | 47.0 | −6.9 |
| Write completion to accepted `SEND` | 5.9 | 0.3 | +5.6 |
| Accepted `SEND` to matched `WAIT` return | 365.6 | 365.1 | +0.5 |

Backend initialization and application attach were both faster in all four
pairs. The selector-to-display span was faster in three; within that span,
frame-guard readiness to display readiness saved a paired median 143.1 ms.
That subspan is already included above. SQLite opening lies inside application
attach and cannot explain all of its reduction. Software draw and framebuffer
write were not faster overall in this batch. The driver's post-`SEND` span
stayed near 365 ms. Do not add independently aggregated stage medians or assign
an individual change the full reduction of its surrounding phase.

The launcher's selector-to-display path still occupies about 2.12 s of the
candidate median, including a 0.83 s median for the native pre-guard queries
and a 0.75 s median from selector entry to the power-observer record. The latter
is not subscription acknowledgement. The remaining software draw is about
0.81 s. More precise instrumentation of those spans would precede further
changes; no hardware refresh or readiness policy was relaxed.

This completes the scoped startup comparison. It does not close optical
first-screen timing (including visible splash exclusion), physical Paperwhite
6, native sleep/USB overlap, or page-turn performance acceptance. A normal
post-push installation and smoke check are separate from these timed trials.
