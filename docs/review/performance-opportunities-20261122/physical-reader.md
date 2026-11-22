# Scribe 2022: ordinary text page turns on firmware 5.19.5

**Decision (November 22, 2026): accept the Reader optimization for `main`.** On this physical
Scribe, the next text page reached the display driver's completion acknowledgment
about **71 ms earlier on ordinary partial refreshes** and **79 ms earlier on
periodic full refreshes**, comparing pooled medians. Both paired visits favored
the candidate, in opposite run orders. No daily package or private shelf was
replaced. These are real device/driver measurements, not finger-to-visible-ink
timings.

## Results

Each cell is a median in milliseconds from the **actual dispatch of a synthetic
Slint pointer release** on the Kindle. A used the Reader code at `f8194f3`; B
used the optimization later committed as `0da4cb9`. Both measurement builds
contained the same trial instrumentation; the ordinary product build does not.

| Refresh and sample count per arm | Target model, A → B | Driver accepted frame (`SEND`), A → B | Matching driver completion (`WAIT` return), A → B |
| --- | ---: | ---: | ---: |
| Ordinary partial, 34 | 155.7 → 84.7 | 260.4 → 188.9 | **555.6 → 484.3** (−12.8%) |
| Periodic full, 8 | 157.7 → 84.9 | 266.4 → 188.2 | **632.6 → 553.7** (−12.5%) |

The effect is already present when the release handler returns. For ordinary
turns, drawing itself took 67.9 → 67.6 ms, and the interval after successful
submission until the matching WAIT returned was 295.4 → 295.3 ms. For full
refreshes that latter interval was 365.8 → 366.1 ms. The change gets the target
page to the driver sooner; it does not make the panel waveform faster. Stage
medians are descriptive; they do not add exactly to the median of the total.

The fixed visit order was **A, B, B, A**. Within each pair, compare the same
target pages individually and then take the median saving. Partial-refresh
completion improved by **71.6 ms** in the AB pair and **69.5 ms** in the BA
pair; full-refresh completion improved by **76.4 ms** and **75.9 ms**. Every
one of the 42 same-page comparisons favored B on completion. Four visits on
one device are not a population study; no P95 or cross-device claim is made.
See the [per-visit and matched-pair statistics](physical-reader-pairs.json) and
[all 96 turn records, including warmups](physical-reader-samples.json).

## What was exercised

- The benchmark used the normal Kindle application, screen, Slint hit testing,
  SQLite persistence, native framebuffer and display driver. Each visit used a
  fresh copy of the same public, offline 1,863-chapter book. Chapter zero had
  1,600 numbered Unicode paragraphs. No user action, account or network book
  was required.
- Each visit opened page 1 and turned forward 24 times to page 25. The first
  three turns were excluded from timing; of the remaining 21, 17 used a
  nonflashing partial update and 4 used the normal sixth-turn full refresh.
  All 96 submissions had an acknowledgment for that exact driver marker. No
  waveform, full-repaint policy or display mode was changed to get a timestamp.
- The application checked the current book/chapter, source and target page,
  rendered row identity and durable scalar position. The 25 page anchors in
  all four visits matched the [ARM software rehearsal](physical-reader-qemu-seed.json):
  67 pages in total, first turn 0 → 1,113; last turn 29,097 → 30,369. An old
  page, unrelated redraw or missing sample could not replace a target.
- The device sampled `CLOCK_MONOTONIC` immediately before the release callback,
  after the target model, around the natural draw/write and at successful SEND.
  A separate worker waited on a duplicated framebuffer descriptor for the same
  update marker; its return includes scheduling, not an optical observation.
  The input bypassed the physical sensor and evdev. No camera or photodiode was
  used; completion of visible ink is unmeasured. The scope excludes startup,
  chapter boundaries, images, heavily marked pages, night mode and PW6.

## Preparation and return

Before any Kindle lease, `test-env` ran the worker-side runner/cleanup,
public-seed and report tests (19 passed), exercised the submission observer on
Linux and under ARM QEMU (7 each), verified the ARM candidate's pointer path,
and rehearsed the exact physical seed at 1,860 × 2,480 / scale 2. Both native
release builds identified their compiled A/B arms in user-mode QEMU and passed
the staged payload checks. This is not a Kindle firmware-emulator result: the
historical firmware guest was unavailable. The [trial tools](../../../tools/reader-turn-trial/README.md)
apply their instrumentation to disposable source exports only.

Each visit had a separate exclusive lease, used only its own temporary program
and public data, stopped its supervisor, confirmed native-state restoration,
removed its stage, independently checked absence, closed transports and released
the matching lease. The four releases were **13:43:57, 13:46:20, 13:48:12 and
13:50:07 Asia/Shanghai**. The final shared handoff notified both peer projects.
The offscreen CPU-only fallback was not needed on the device.
