# Reader image and sleep-cover preparation

Recorded: 2026-11-16 (Asia/Shanghai).

## Scope and baseline

The operator requested an independent WeRead investigation of the Reader
publication and optional-cover contention identified by Aidoku's Perf05 work.
The WeRead baseline is `d17ef46e7b252e443ba140c71a83cb9e7798633f`, including
the later login, detail-page, and Skills changes. Aidoku's measurements are
reference evidence, not measurements performed by WeRead.

WeRead reads text chapters with inline pictures. It has no comic Fast mode or
prepared-next-page bitmap queue. Its existing chapter prefetch acquires source
content near a chapter boundary. The controlled workload therefore exercises
real chapter opens, ordinary text-page turns, repeated Reader controls, rapid
replacement requests, and large cached covers. It does not label chapter opens
as single comic-page turns or claim bitmap prefetch hits.

## Confirmed causes

`render_reader` called `prepare_reader_image_rows` on every publication. The
latter synchronously read and decoded canonical PNGs, performed Triangle
sizing, allocated final RGB, and quantized or dithered each pixel. Hardware
selection also prepared a complete software replacement. Reopening controls
could repeat this work for the same visible image. Article publication used
the same synchronous picture path.

`prepare_sleep_screen_for_book` ran before the Reader's pictures. A changed
book synchronously decoded a cached full cover or upscaled its thumbnail and
allocated up to 1536 by 2048 RGB pixels. A downloaded cover was synchronously
decoded for its thumbnail, decoded for its full derivative, and decoded again
for display. Article image downloads similarly performed canonical Gray8
conversion on the UI thread.

The contention was the UI thread itself. WeRead does not have Aidoku's shared
serial image-decoder gate. Cover and article HTTP requests share two admitted
public-image slots; download preparation has its own admission.

## Implementation

One bounded display-scene worker prepares final shared RGB buffers. UI
publication constructs Slint images from those existing allocations. Repeated
publication of the same complete scene reuses its prepared pixels. A new scene
can show geometrically correct unloaded placeholders while its pictures are
being prepared. The previous book's pixels are never attached to replacement
text. Source identity, account epoch, layout, physical scale, crop/fit, and
processing choice participate in ownership.

Preparing pictures keep ordinary Reader and Article page gestures available.
They do not advertise a new download or capture page taps as image-load
actions. Uncached pictures retain their explicit load control. This is
temporary view state, not a new persistent image format or reading mode.

Reader source and cache checks remain operation-local. Workers independently
check content-addressed PNG bytes, and publication resolves the current source
again. Old verified paths are not promoted to a persistent integrity cache.
Cancellation keeps its active slot and metadata admission until completion is
drained and joined. A replacement retains no unbounded backlog of page payloads.
Ready scenes survive a sleep pause, and incomplete work has a one-shot restart
after wake. Pixel-row iteration removes the runtime width division per pixel
without changing Bayer phase, rounding, source crop, or Triangle filtering.

Completed foreground preparation posts a weak-window notification to the UI
event loop, using the existing Slint/Kindle proxy. The dedicated event drains
and publishes the current scene without counting as reading activity. A reply
that narrowly precedes thread exit gets at most one one-millisecond deferred
retry; slots and admission still wait for the real join. The normal 100 ms
tick remains a fallback. Hidden cover completions do not use this foreground
notification or repaint the Reader.

Cover preparation uses a separate bounded worker. Full-resolution preparation
and Reader cover acquisition wait for four seconds of reading inactivity and
for the display-scene worker to be idle. Reading activity postpones that
deadline and cancels optional work. Existing ready covers remain available;
cancellation cannot clear the deadline and immediately restart preparation.
Finishing a hidden cover does not request another Reader publication or panel
refresh. The power-event path still only publishes already prepared pixels and
bounded text, with the established title/author fallback when no image is ready.

Downloaded thumbnail and article canonical conversion also run off the UI
thread. Pure preparation returns a sealed PNG value; the UI retains the single
FileCache connection and commits only prepared bytes. No worker opens another
cache connection or participates in cache recovery. Transport credits travel
with retained input and output through completion processing. They are not
returned while an optional derivative still owns the original response.

Existing output palettes, Hardware fallback, manual refresh, and image-quality
choices remain. Display scenes retain the eight-megapixel viewport limit and
64 MiB combined primary/fallback RGB allowance. There is one display worker,
one cover worker with at most three retained work items, and one article
canonical worker with at most two retained requests. Original response storage
is bounded to 16 MiB per conversion controller and remains subject to shared
transport admission. Decoder limits remain separate from RGB, encoded-input,
cache, and metadata allowances; no individual allowance is a total-process RSS
limit. Source code, builds, and analysis do not reserve the physical device.

## Verification status

All execution is on the designated `ssh test-env` worker, using six Cargo jobs.
Local work consists of source editing, Git inspection, and inspection of copied
reports. The current native Kindle installation is outside this software run.

The source archive for the baseline is frozen before implementation changes.
The paired executable uses exactly the same standalone fixture source and
actions. The candidate alone enables its tool-local diagnostic feature; this
adds explicit counter readback without changing product rendering preferences.
Performance measurements use optimized fixture executables.

The exact Perf05 input archive has SHA-256
`0e11719838c8da38388be9f3f389fa59ce671625bb033b1707842bce520cf076`.
It supplies eight 2480 by 3508 PNGs, committed through the public BookAssetStore
packet API into isolated text chapters. The same source produces cached
thumbnail/full-cover cases. Generated content, data, archives, and screenshots
remain outside tracked source.

The fixture polls actions and model readiness every 20 ms while calling
`App::tick` every 100 ms, matching the product tick interval. It records
dispatch/tick self-time separately from model-ready latency, snapshots, and PNG
encoding. Desktop scale is one. OS file caches are not cleared. These times do
not include device SEND acceptance, touch-controller sampling, or optical panel
completion.

The final complete application run passed 1,704 library tests and 52 Main tests,
with four existing benchmarks ignored. This includes the real Slint pointer
regression for both preparing pictures and genuinely uncached load controls.
Translation completeness passed for 493 keys, and root/tool formatting passed.

The first complete library run passed 1,696 tests and failed four, with four
ignored. Three old fixtures assumed synchronous picture or cover completion.
Their text reflow, anchors, presentation acknowledgement, wake frame, resource
limits, and loop counts remain asserted; they now explicitly drain preparation
before sampling pixels or advance the test-only optional-work deadline. The
fourth new test made an overly specific assumption about the number of
placeholder rows after corrupting the canonical source. A valid text-bound
manifest can retain an unloaded image slot. The regression checks that every
retained slot has no pixels, is neither loaded nor preparing, and has the
current occurrence identity; no ready scene, active worker, or old scene may
survive. No timeout or resource allowance was increased.

Intermediate candidate performance runs were superseded while the no-source
idle guard and pending-picture interaction were completed. Their logs and
artifacts remain separate and do not supply the final performance figures.
The subsequent pre-wake comparison preserved all 66 picture/body checkpoints
but exposed a polling regression. For Software with a full cached cover,
forward-chapter median dispatch fell from 54.975 to 10.582 ms while model-ready
time rose from 54.996 to 128.280 ms. Those are intermediate desktop figures:
they identified the 100 ms scheduler wait and motivated the foreground
completion notification. They are not reported as the final result.
The baseline and final harnesses are identical after the same Rust formatter
and line-ending normalization; the original compiled baseline input hashes and
formatter-only diff are preserved.

A source-identity receipt compares 575 runtime, UI, asset, vendor, build,
package, test, and verification-fixture files between the operator checkout and
the worker. UTF-8 CRLF text is normalized to LF; binary bytes compare exactly.
There are zero mismatches; the measured source-manifest SHA-256 is
`ecb727b042a5db31373046882d40f99a81249f85390adf8283d0301b03915caf`.
Final documentation updates are outside that runtime-source receipt.

## Paired performance results

Both optimized executables passed all six cases. The paired comparison found
identical prepared-picture payloads and visible Reader bodies at all 66
checkpoints. Full cached covers recovered after the idle deadline, and cover
completion did not change the Reader body. Rapid replacement, an ordinary
forward/back page turn, and revisiting an image retained the correct source.

The following values are milliseconds. Same-page controls use three actions;
forward chapter operations use three distinct new pictures per case. Each cell
is baseline to final, and each action-family value is its small-sample median.
Hardware selection on this desktop is the real unsupported-device software
fallback, not simulated native Y4 eligibility.

| Selection | Cached cover | Same-page dispatch | Forward-chapter dispatch | Forward model ready |
| --- | --- | --- | --- | --- |
| Off | Full | 42.996 to 6.443 | 47.691 to 8.687 | 47.713 to 60.000 |
| Software | Full | 49.717 to 5.887 | 54.975 to 9.004 | 54.996 to 60.356 |
| Hardware fallback | Full | 44.885 to 6.301 | 47.527 to 9.241 | 47.548 to 60.698 |
| Off | Thumbnail | 45.772 to 5.860 | 54.661 to 11.373 | 54.683 to 60.330 |
| Software | Thumbnail | 43.679 to 7.024 | 54.529 to 10.783 | 54.549 to 60.409 |
| Hardware fallback | Thumbnail | 45.293 to 6.964 | 48.664 to 10.844 | 48.685 to 59.971 |

For the Software/full-cover first entry, App attachment took 130.979 ms before
and 28.229 ms after; the first Reader image model was ready at 152.601 ms before
and 71.243 ms after. This is one fresh-process observation per candidate, with
already seeded files and uncleared OS caches.

Foreground completion publication is additional UI work outside the scripted
dispatch and periodic tick timings. Across the six cases it cost 32.228 to
58.078 ms in total over seven to ten rendering callbacks per case, averaging
4.195 to 5.808 ms per callback. No-op notifications are excluded. These totals
are explicitly included in the analysis rather than presented as zero-cost
publication. The latest-callback observations are samples, not exhaustive peaks.

Across this fixed six-case workload, measured dispatch plus tick work totals
3865.350 ms in the baseline. Candidate dispatch, tick, and the separately
recorded wake work total 1084.024 ms, including 250.297 ms over 50 wake renders.
That is a 71.96 percent reduction in these measured UI handlers. Attachment is
listed separately; background codec CPU, rendering outside the measured
handlers, device time, and total-process CPU are not included. No wake time is
added to model-ready latency, which already spans completion publication.
Rapid requests delivered fewer candidate targets because obsolete work was
superseded; their ready medians are not compared as equal delivered samples.

The change removes long uninterrupted image work from the input handler and
the additional 100 ms scheduler wait. It does not remove first-use image
decoding. New-picture readiness remains around 60 ms in a 20 ms readiness-poll
harness, compared with about 48 to 55 ms before; this is not evidence of a
corresponding reduction in total page latency. User-input handling and first
entry are the observed improvements. No native driver SEND or optical timing
is inferred from these desktop measurements.

| Measured executable | SHA-256 |
| --- | --- |
| Baseline release fixture | `95fac2d33f78877abc4e69ad10f2a9ae12df4367f4e47666722a54faf0910ace` |
| Final release fixture | `74b89c6d35b2200d95a39f304289a15fd3cf9987fb025429217bd2b8ad3a8711` |

Reports are under `test-env:/root/weread-perf05-20261116-r1/`:

- `artifacts/performance-baseline/run-20261116T102010Z-2221215/report.json`
- `artifacts/performance-wake-final/run-20261116T121556Z-2296009/report.json`
- `artifacts/performance-wake-comparison.json`
- `artifacts/performance-wake-summary.json` and `.csv`

## Additional UI and artifact checks

The first general desktop run passed 132 checks and failed its statistics
selection comparison. On Monday 2026-11-16, the seeded previous week's maximum
was Tuesday. The default selection was therefore identical to the endpoint of
the test's Monday pointer tap followed by one next-bar step. Both images were
correctly identical; this did not exercise the intended change of selection.
The verifier now explicitly selects the first bar for its baseline while
retaining the original pointer/step actions and pixel-difference threshold.
Only that statistics group is repeated against the same executable; results
must be combined by case identity rather than relabelling the failed full run.
The original report and identical PNGs are preserved.

This changes only `scripts/verify-ui.py`; the application, tested library,
performance fixture, and measured executable remain unchanged. The final
source receipt records this verifier-only difference separately from the
measured `ecb727` snapshot.

The final delivery receipt also compares all 575 files with zero mismatches;
its source-manifest SHA-256 is
`e43c67a6e0e168f20a3c4cbc2c355b7e7b935ec83de3f328cb29b7c21a7a612b`.
Its complete difference from the measured snapshot is the corrected UI
verification script; application and performance-fixture sources are identical.

The focused statistics group passed all 18 checks. Case-ID replacement produces
133 successful unique general UI checks: 115 unchanged original records plus
18 repeated statistics records. This is an aggregate over the retained full
run and focused correction, not a second complete 133-case execution. The same
already built desktop executable was used for both.

The final release fixture also passed all 22 image-processing checks and all
16 sleep-screen checks. No application source or binary changed for the
statistics fixture correction.

ARM release compilation completed in 3 minutes 57 seconds; the complete
build/package/check chain took 240 seconds. Four package contracts, all archive
CRCs, the exact 36-member public allowlist, and executable/data modes passed.
The main program is a stripped static ARM32 EABI5 executable. The native guard
remains its independent dynamic ARM32 glibc process.

| Development artifact | SHA-256 |
| --- | --- |
| ARM main | `13510bd63f87499f82ed15ec3ea4308f8b4f18366ff36bdb5d9add7197c1c3e0` |
| Development ZIP | `0b92dccefc76e5a0efe62d3f15922ff3c61806f1dc6b8de7c861a58c143f46e7` |
| Launcher | `713167110d4839f14505cd03c6b30753476772c729fe92890a86fbd0085fe6be` |
| Native guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |

The ARM outputs are in the remote run's `artifacts/kindle/`. Local generated
reports and copied packages are under `artifacts/weread-perf05-20261116/` and
are ignored by Git. No device connection, lease, installation, new physical
latency, or panel-quality result belongs to this task. The preceding deployed
Kindle application and private data were not accessed.
