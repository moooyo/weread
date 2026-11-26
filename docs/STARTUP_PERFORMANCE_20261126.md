# Kindle Library first-screen startup

Date: 2026-11-26 (Asia/Shanghai). Baseline: `f23940c`, with matched
measurement-only instrumentation. The candidate passed the scoped Linux
correctness checks and four physical pairs on a Kindle Scribe 2022 running
firmware 5.19.5. Both optimized stages were faster in all four pairs; the
complete Library frame finished its driver wait sooner in three of four.
The median within-pair first-screen reduction was **531 ms**.

## Physical diagnostic baseline

The first-generation Kindle Scribe (2022), firmware 5.19.5, opened the
ordinary WeRead tile in the stock Kindle Library. A temporary single-file
selector routed that launch to a disposable current-layout shelf with 15 public
books and their cached thumbnails. The stock card, daily application, private
data, and the shared dispatcher were not replaced. This was a warm-device
process launch, not a reboot or an authenticated workload.

The native input tool bracketed its synthetic button release on the device's
`CLOCK_MONOTONIC`. The one-shot Library observer required a complete draw of
the populated shelf, with navigation and recognizable entries, and a checked
full-frame `SEND` from that launch. The measurement build also timestamped
successful completion of the existing `WAIT` for that same update. The splash,
blank frame, and a later frame could not qualify. Covers were not a completion
criterion. Driver completion is the hardware endpoint available here; optical
settling and physical finger latency were not measured.

The qualified diagnostic launch took **8.6846–8.6858 s to render completion**,
**8.7805–8.7817 s to driver submission**, and **9.1321–9.1332 s to
driver-confirmed completion**. The outward-rounded brackets reflect the
1.066 ms uncertainty in the synthetic release, not run-to-run variance. This
is **one diagnostic sample**, not a median or part of the planned paired series.
The [monotonic receipts](review/startup-first-screen-20261126-baseline.json)
contain the observation and the earlier admission refusals.

| Diagnostic stage | Time | What it contains |
| --- | ---: | --- |
| Card release to Rust `main` | 5,756 ms | Stock dispatch, temporary selector, launcher, power observation, native queries and display takeover |
| &nbsp;&nbsp; Power observer settle | 2,014 ms | Existing two-second wait, including measurement overhead |
| &nbsp;&nbsp; Settling complete to display takeover start | 125 ms | Read-only sleep-cover capability probe and adjacent work |
| &nbsp;&nbsp; Native display queries before frame guard | 1,079 ms | Live display, orientation and service state |
| &nbsp;&nbsp; Frame guard start to ready | 914 ms | Guard initialization, X11 calls, surface mapping and fullscreen confirmation |
| Rust `main` to software ready | 2,040 ms | Loading feedback, backend, fonts, recovery and initial application state |
| &nbsp;&nbsp; Backend installation | 546 ms | Includes default-font staging and shared font initialization; not an isolated I/O duration |
| &nbsp;&nbsp; Application attachment | 1,016 ms | Includes 240 ms for storage setup and 732 ms for first model projection |
| Software ready to first draw begin | 165 ms | Panel, input and portrait setup plus event-loop work |
| First Library draw | 724 ms | Slint software rendering |
| Draw complete to driver submission | 96 ms | Grayscale/framebuffer and submission work |
| Driver submission to matching `WAIT` | 352 ms | Includes the instrumented one-shot receipt before the existing wait |

Times use the midpoint of the release bracket only for intermediate stage
summaries. The indented rows are nested, not additional durations. A guard
span is not the measured cost of palette or pixmap work alone, and attachment
does not isolate thumbnail checks from image loading and model publication.

Three initial visits were refused by stale installation guards before any
navigation or launch. A separate read-only audit narrowed the mismatch; the
current package and closed November 25 deployment receipts provided the
reference, not device readback used as a new expectation. The successful visit
restored the original stock view and pointer, verified the original launcher
and document bytes, removed its stage and released the lease at **10:13:16
Asia/Shanghai**. It did not persistently update the device.

## Candidate changes

- The frame guard uses one X11 query for the entire immutable StaticGray
  palette instead of up to 256 sequential color requests. It still requires a
  complete one-to-one mapping for all 256 exact neutral shades. The solid
  initial guard surface uses the corresponding white X11 background pixel,
  without a per-pixel conversion or pixmap upload. The geometry and image
  layout checks, fullscreen and topmost confirmation, retained snapshot,
  and nonwhite capture and alignment path remain.
- The Library looks up and verifies only its bounded visible thumbnail set as
  one operation. It commits successful recency updates in display order in a
  single SQLite transaction rather than one transaction per cover hit. It
  reuses that page's cover metadata and lookup results for immediate download
  scheduling. A failed batch still checks covers individually, so one bad
  entry does not hide other usable covers. Later refills re-read current state.
  The first Library frame still projects the same checked covers; this change
  does not hide them to reach the timing endpoint sooner.

No background writer or new startup thread is introduced. The two-second
power wait is retained: stream readiness does not prove that the firmware has
registered the subscriptions. Native orientation and display ownership still
use fresh checks. The join that hands the framebuffer from the splash to the
application, the first refresh waveform and completion wait, and the input
barrier are unchanged. Font fallback and persistence layouts are unchanged.

## Software verification and helper-only measurement

The first candidate run was interrupted by memory pressure on `test-env` and
an SSH outage. An Xvfb attempt also skipped its tests because `xrefresh` was
missing. After the worker recovered, builds used at most two Cargo jobs and
the helper suite used a privately extracted Ubuntu `xrefresh`. The final
product changes passed these checks, all on the designated worker:

| Check | Result |
| --- | --- |
| Library tests | 1,725 passed; seven existing cases ignored |
| Application entry tests | 57 passed |
| Xvfb frame guard | 36 passed; includes initial white pixels, all 256 gray values, capture, rotation, and native restoration |
| Launcher guard lifecycle | 11 passed |
| Stock-card transaction fixtures | 15 passed; includes same-GUI trace collection and handback |
| Visit and matching-submission fixtures | 24 passed |
| Desktop Library and Detail application checks | 31 passed |
| Normal offline public shelf with 15 cached covers | Scribe and PW6 layouts showed the grid, switched to list view, and retained all 15 cache entries |
| Standard Kindle feature selection | Passed; exact-pose reuse and bundled-font preflight remain enabled, with trial and reconcile features excluded |
| Translation inventory | All 486 interface keys available |
| Instrumented ARM builds | Both passed the Cortex-A8 version probe; their shared launcher bytes matched |
| Standard uninstrumented ARM build | Development bundle staged, Cortex-A8 version probe passed, and the guard matched the measurement candidate byte for byte |

The existing diagnostic baseline also replayed through the strict analyzer.
The cache tests check corruption retirement, display-order eviction, rollback
of a failed recency batch, and reuse of the inspected visible thumbnails in
scheduling. Desktop rendering and Xvfb do not establish device compatibility.
No local application verification was performed.

A separate [six-pair Xvfb benchmark](review/startup-first-screen-20261126-guard-xvfb.json)
used the pristine and candidate C helpers compiled with the same `-O2` settings.
It measured process spawn to the existing ready record on a 1860 × 2480 8-bit
StaticGray screen, excluding one warmup per arm. Direct pixel comparison confirmed a solid white background in every run.
The median went from **25.92 ms to 3.59 ms**; all six candidates were faster,
with a **22.39 ms** median within-pair reduction. This is the guard alone on an
x86-64 worker, not a Linux estimate of the Kindle's guard span or of first-screen
latency.

## Physical comparison

The series used one warmup per arm, then four balanced pairs (`AB`, `BA`,
`AB`, `BA`) on the same Scribe, with the same stock card, public seed, build
options, orientation policy, launcher bytes, and one-shot instrumentation.
Arm A used the original application and guard behavior; arm B changed only
the frame guard and visible cover cache path. All ten launches used disposable
data and separate exclusive leases. Every launch
rendered at `0°`; none installed a daily package or touched private shelf data.
The two warmups and earlier diagnostic are excluded from all paired medians.

The first formal series stopped after its A warmup. The complete Library draw
and its driver completion were observed, but immediate handback refused the
supplemental firmware view: it had reported `KPP_HOME` before the visit and
returned no recognized value afterward. Independent native observations matched
the original Home tab and exact pointer position; the native state snapshot
and original file identities matched as well. A close-only recovery checked
these observations, the original launcher and document bytes, and independent
stage absence. It released the original owner/token at **13:51:03
Asia/Shanghai**. It did not launch another app. The abandoned slot remains
`not_timed` and is not included in any A/B result.

The replacement series keeps the same executable pair and measurement
markers. At handback only, an unavailable `activeView` now requires direct
verification of the original stock tab and pointer together with native-idle
and the unchanged state snapshot. A recognized conflicting value still prevents
handback. All 17 transaction fixtures passed, including the missing-value
recovery and rejection of the wrong tab/pointer or a recognized conflict.

### Four-pair results

| Pair | Order | Guard A → B (ms) | First Library model A → B (ms) | Card release to `WAIT` A → B (s) | Paired `WAIT` saving (ms) |
| ---: | :---: | ---: | ---: | ---: | ---: |
| 1 | AB | 785.1 → 341.2 | 606.4 → 375.1 | 8.911 → 7.933 | +978.1 |
| 2 | BA | 817.1 → 302.2 | 612.4 → 558.5 | 8.327 → 8.431 | **−103.6** |
| 3 | AB | 830.5 → 490.1 | 727.8 → 371.4 | 9.014 → 8.459 | +554.5 |
| 4 | BA | 866.0 → 393.8 | 586.7 → 378.9 | 8.329 → 7.821 | +507.8 |

| Physical interval | A median | B median | Median paired saving |
| --- | ---: | ---: | ---: |
| Frame guard start to ready | 823.79 ms | 367.54 ms | **458.01 ms** (4/4 faster) |
| Startup state to first Library model | 609.37 ms | 377.01 ms | **219.54 ms** (4/4 faster) |
| Stock card release to Library rendering complete | 8.210 s | 7.766 s | — |
| Stock card release to Library `SEND` | 8.256 s | 7.820 s | — |
| Stock card release to full Library driver `WAIT` | 8.620 s | 8.182 s | **531.17 ms** (3/4 faster) |

The guard and model medians are respectively 55.4% and 38.1% lower. First-screen
medians differ by **438 ms (5.1%)**; the median of within-pair differences is
**531 ms** (median within-pair relative reduction **6.1%**). Those summaries use
different aggregations. The rounded end-to-end values use release-bracket
midpoints; the median paired `WAIT` saving is bounded by **530.0–532.4 ms** from
the release brackets alone, not a population confidence interval.

The two targeted intervals enclose adjacent work, not individual operations.
The guard interval includes the window map and readiness check; the Library
interval includes model preparation, cache work, and visible-row publication.
The separate C-only benchmark does not substitute for either physical interval.
In pair 2 both target intervals improved, but after Rust `main` the candidate
spent about 151 ms longer reaching software readiness and then about 159 ms
more in the actual first software draw. Other stages varied, and that visit
was about 104 ms slower end to end. It remains in the result. The sample does
not isolate the cause of the variation in these surrounding stages.

After the changes, release to Rust `main` still took a median **5.23 s**,
including the retained **2.01 s** power settle and about **1.15 s** of native
pre-guard queries. Software readiness took another **1.85 s** from `main`,
and the panel driver's post-submission wait stayed near **0.36 s**. These
independent medians describe the remaining work; do not sum them as a single
launch or attribute the whole pre-`main` span to the application. Neither the
power wait nor the first display waveform was shortened.

Both warmups completed, at approximately 8.46 s and 7.74 s. All ten visits
restored the original native view, pointer, and installation, confirmed their
owned stage was gone, and released the corresponding lease. The final release
was **14:57:32.971 Asia/Shanghai**. The [full paired receipts](review/startup-first-screen-20261126-pairs.json)
include unrounded stage timestamps, synthetic release brackets, all render,
`SEND` and `WAIT` endpoints, and each handback time. The separately published
[first-pair receipt](review/startup-first-screen-20261126-first-pair.json)
remains an interim snapshot of the same series. No cold-boot, authenticated,
optical, or other physical hardware claim follows from these samples.
