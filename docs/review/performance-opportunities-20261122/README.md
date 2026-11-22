# Where to optimize Kindle startup and page turns next

November 22, 2026. Research against `f8194f3`. No product code, package,
device or private library changed. The new experiments ran on `test-env`;
neither used a Kindle or the firmware emulator.

**Implementation follow-up:** the [ordinary Reader tap comparison](reader-pointer.md)
subsequently confirmed fewer catalog reads and a faster release-to-software-draw
interval in both public workloads. The original research and device boundaries
below are retained for comparison.

**Recommendation:** start with ordinary text-page taps. Measure the actual
pointer path, remove unchanged old-page publications, then share the catalog
work used to display and save the new position. For startup, measure the split
before Rust `main` and the cost of the first shelf separately. Do not shorten
the power wait or change the display waveform on the strength of host results.

## What the device has established

The [November 22 stock-card experiment](../stock-library-ab-20261122/README.md#final-result-and-package-decision)
is the relevant startup baseline: **8.103–8.104 s** median with exact-pose
reuse, versus **9.426–9.427 s** without it. B was faster in six of six
balanced pairs, with a median paired saving of 1.394–1.397 s. It was a warm
Scribe 2022 / firmware 5.19.5, a synthetic release of the ordinary card, a
public 15-book shelf and a same-launch witness immediately after driver
acceptance of the complete shelf. Book titles and navigation are required;
cover placeholders are valid. The trial selector and witness were common to
both arms. The later uninstrumented daily installation with existing data was
a functional check, not another timing. Bundled font preflight and exact-pose
reuse are already in the standard build.

We also replayed the six B timing records from the [November 21 exact-pose
experiment](../startup-orientation-20261121/physical-balanced.md). This was a
**different entry**: a programmatic request to a temporary indexed document,
on the same model/firmware, fixed pose, warm caches and public 15-cover shelf.
It is useful for deciding where to instrument, not for estimating a substage
of the 8.10 s ordinary-card interval.

| Within each indexed B launch | Median |
| --- | ---: |
| Request to Rust `main` | 4.065 s |
| `main` to software ready | 0.842 s |
| Of which: application attachment | 0.555 s |
| Software ready to first Library `SEND` | 0.414 s |
| Request to first Library `SEND` | 5.356 s |

For each B1–B6, subtracting the device-monotonic `request` and `markers` in
the closed `orientation-six-pair` batch reproduced the [published endpoint](../startup-orientation-20261121/physical-balanced.json)
at nanosecond precision before rounding. The three adjacent intervals sum
within a run; do not add their independent medians or count attachment twice.
There was no guard, settling or draw-start marker. Neither the pre-`main`
interval nor the entire attachment interval is a saving available to one fix.

There is no corresponding physical distribution in the reviewed evidence for
**release to the next ordinary text page visibly settling**. The existing
[image experiment](../../IMAGE_PREPARATION_PERFORMANCE_20261116.md#paired-performance-results)
measured desktop handlers and model readiness. Its roughly 60 ms for new
images is neither ordinary page-turn latency nor a panel measurement.

## 1. Remove work that does not change the page

For a short tap in the next-page zone, with no menu, selection or mark hit,
the [Reader surface](../../../ui/reader.slint#L1322) emits `press` on down and
`release` followed by `tap` on up. [Dispatch](../../../src/app/mod.rs#L887)
only exempts `move` from `render()`. This gives two projections of the old
page and one of the new page, **not necessarily three screen updates**. Only
the latter two projections start on release. The [same-chapter paginator](../../../src/reader.rs#L1108)
already advances by index; it does not lay out the chapter again.

On each projection, the [closed contents view](../../../src/app/reading.rs#L2237)
still loads every local chapter header before selecting a bounded page. The
[footer progress calculation](../../../src/app/reading.rs#L4232) independently
loads the headers, counts scalars in the chapter body, and reads the stored
remote catalog if present. [Saving the new position](../../../src/app/reading.rs#L4300)
calculates progress again for an unfinished book. On a successful turn between
interior pages of a nonempty, unfinished book this accounts for **seven full
header projections per gesture** (`3 × 2 + 1`; five on the up/tap path), and
four remote-catalog decodes when that setting exists. These are source-path
counts, not runtime observations. The projections do not return chapter bodies.

| Synthetic chapters: current book / other books | One header projection | Seven projections | Header rows returned by seven |
| --- | ---: | ---: | ---: |
| 24 / 0 | 0.025 ms | 0.173 ms | 168 |
| 1,863 / 0 | 1.448 ms | 10.123 ms | 13,041 |
| 24 / 1,839 | 0.093 ms | 0.441 ms | 168 |

New x86_64 Linux medians: Python 3.12.16 and SQLite 3.46.1, synthetic databases
using the production schema, WAL/NORMAL, warm caches. Each projection performed
`BEGIN`, the exact [admission and ordered header queries](../../../src/store.rs#L9383)
with titles, Python `fetchall`, and `COMMIT`. Two warmups and twelve samples per
cell alternated forward/reverse case order. The planner scanned `chapters` for
admission but used the book index for the returned rows. The seven-read large
case ranged from 10.064 to 10.276 ms. This measures SQLite and Python row
extraction, not Rust allocations, remote-catalog decoding, touch, drawing,
storage writes, or an achievable Kindle saving. See [the samples](samples.json).

1. Give `press` and `release` a way to report that **nothing visible changed**.
   Target one model publication for the plain turn. Keep hold timing, selection
   previews, mark hits, cancellation, protected edges and menu behavior intact.
2. Reuse the new page's fraction for presentation and persistence within the
   same operation. Have the closed TOC load the bounded current/access rows it
   publishes, not every title. Keep the [current/access contract](../../ARCHITECTURE.md#L126)
   and resource admission. If reusing chapter ranks and word-count prefixes
   across pages, invalidate on catalog/reindex, account, and content/access
   changes. Ordinary position writes must not invalidate an otherwise unchanged
   catalog. Do not delay position durability to make a timing look better.

## 2. Keep the first shelf proportional to what is visible

The [Library projection](../../../src/app/library.rs#L694) materializes all book
metadata, filters or sorts, then takes the visible page. The store prepares a
[text lookup per row](../../../src/store.rs#L7361). The Scribe cover grid uses
15 books; PW6 uses six. Each [visible tile](../../../src/app/library.rs#L658)
synchronously checks the local thumbnail, updates its LRU on a hit and asks
Slint to load it. With the online, non-demo cover scheduler active, the [same
publication](../../../src/app/backend.rs#L1330) can re-read the book metadata
and check the thumbnail a second time.

First reuse the current page's metadata, source URL and operation-local cache
result. Compare shelves of 15, 300 and 1,000 public books with all/missing
thumbnails. If materialization dominates, test count plus bounded visible
rows for the default order; preserve ordering, offline hidden/cached books,
accurate totals and admission errors. If thumbnail work dominates, present
real titles and navigation, then fill the *still-visible* covers in bounded
batches. Compare first usable shelf, first/last visible cover, and extra
refreshes; do not trade a small first-frame gain for prolonged flashing.

## 3. Measure display takeover before changing it

The [frame guard](../../../packaging/kindle/frame_guard.c#L829) starts with an
all-white Scribe image (1,860 × 2,480 = 4,612,800 pixels). The current
[generic pixmap path](../../../packaging/kindle/frame_guard.c#L528) converts
each pixel and uploads it to X11 in bands. The [supervisor](../../../packaging/kindle/launch.sh#L1341)
waits for guard readiness before stopping the status bar and starting WeRead.
The experimental arm used [Xlib's background pixel](https://www.x.org/releases/X11R7.6/doc/libX11/specs/libX11/libX11.html)
with the already validated `gray_pixels[255]` for this initial solid window
only. It kept the white snapshot initialization, pixel-layout and allocation
checks, ownership, and the normal capture/realignment path for nonwhite frames.

| Xvfb at Scribe resolution | Current guard | Solid-background guard | Median within-pair saving |
| --- | ---: | ---: | ---: |
| 8-bit StaticGray | 19.086 ms | 13.367 ms | 6.626 ms (6/6) |
| 24-bit TrueColor | 15.407 ms | 2.928 ms | 12.494 ms (6/6) |

Both arms used host GCC 13.3.0 at `-O2`. The same parent clock timed process
spawn to the matching ready record; the table is **not** process CPU or app
startup. Each visual had one excluded warmup per arm and six alternating AB/BA
pairs. Direct pixel comparisons after timing checked white, a nonwhite retained
frame, 180-degree realignment, exit and native-background restoration. A fixture
restoration failure with a non-retained background preceded this series; both
arms restarted from warmup with the same reproducible background. Neither
visual or savings figure establishes Kindle behavior. Measure the guard's
share on the ordinary-card path before pursuing this over the shelf changes.

The existing [two-second power wait](../startup-orientation-20261121/power-wait.md)
is a separate question: local `observer-stream-ready` precedes subscriber
`exec` and is not firmware registration acknowledgment. Keep the wait and
native recovery safeguards intact. Instrument card release, selector/launcher
entry, observer startup/settle, guard readiness, display takeover, `main`,
attach, draw and matching first submission. The [stock witness](../../../tools/startup-observer/LIBRARY-FRAME.md#L26)
does not accept `WEREAD_STARTUP_TIMING`: use a separate diagnostic series or
equivalent trial-only instrumentation in both arms, not unlike endpoints.

## Which experiment comes next

- On `test-env`, use [Slint pointer down/up](../../../tools/offline-ui-fixture/src/main.rs#L1309)
  rather than the existing [direct `turn` script](../../../scripts/verify-reader-image-performance.py#L126).
  Count application publications, header rows, remote-catalog decodes and
  position writes for 24 and 1,863 chapters. Verify the same target page,
  footer, saved anchor, long press, mark tap, swipe and menu. Keep close/sleep
  position restoration intact. Compare Scribe/PW6 layouts in the emulator.
- After software gates, use a representative Scribe in an exclusive prepared
  visit. Bind each release to the **same target page**, not just the next
  generic draw. Record model publication, grayscale write, driver acceptance
  and (when already available) that update's completion. Do not add a blocking
  wait or reuse the Library witness, which forces a full repaint. Report
  ordinary partial turns separately from the default sixth-turn full refresh,
  cached/uncached chapter crossings, and image readiness. Report sample counts,
  medians and, with enough turns, tail latency. Only synchronized optical
  observation establishes visible ink settling; otherwise label it unmeasured.
- If illustrated books drive the tail, [image preparation](../../../src/app/book_images.rs#L166)
  verifies the chapter image snapshot before it knows whether the current page
  has any pictures. Test skipping that work for text-only pages while clearing
  previous pixels and stale jobs. If drawing dominates instead, measure the
  actual dirty area versus the [bounding rectangle](../../../vendor/slint-backend-kindle/src/platform.rs#L701)
  before optimizing grayscale writes. Neither warrants changing waveforms,
  compromising source validation, or pre-rendering every page by default.
