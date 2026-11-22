# Ordinary Reader tap: implementation and measured result

November 22, 2026. **The host comparison supports keeping the change.** On `test-env`, a
real Slint pointer release reached the completed software drawing of the next
text page sooner in all six paired processes at both catalog sizes. The larger
catalog benefited more. This is an application result, not a measured change
in physical Kindle ink latency; no daily installation was changed.

| Public catalog | Release to draw P50, before → after | Release to draw P95, before → after | Paired median saving | Candidate faster |
| --- | ---: | ---: | ---: | ---: |
| 24 chapters | 3.315 → 2.859 ms | 3.624 → 3.132 ms | 0.450 ms | 6/6 |
| 1,863 chapters | 10.016 → 5.838 ms | 10.468 → 6.110 ms | 4.160 ms | 6/6 |

## Why it improved

- The [pointer handler](../../../src/app/reading.rs#L2782) still schedules the
  hold, checks mark hits on release, and can publish a selection, menu, thought
  or error immediately. An ordinary down or no-hit up does not republish the
  unchanged page. The subsequent `tap` publishes the new page. The scheduler
  and panel waveform are unchanged.
- One [admitted chapter-header snapshot](../../../src/app/reading.rs#L2008)
  now serves both the page footer and the open or closed contents view. It has
  the same titles, access metadata and order as the two old reads. It is not
  cached across operations. Position confirmation and durable progress writes
  are unchanged and still perform their own current lookup; two full catalog
  projections remain, not zero. A finished book's library progress remains
  distinct from its current page footer.
- In **every measured forward gesture**, page-line model publications went
  from **3 to 1** (down/up: 1/2 to 0/1), completed full header projections
  from **7 to 2**, and word-weight table builds from **4 to 2**. For 1,863
  chapters that is 7,452 versus 3,726 weight lookups. These are observed
  counts for both workload sizes, not inferred screen refreshes.

The smaller catalog's release-handler P50 fell from 1.030 to 0.610 ms; for
the larger one it fell from 7.645 to 3.531 ms. Drawing alone stayed near
2.3 ms in all four arms. This is a joint A/B; it does not separately assign
millisecond savings to unchanged-page suppression and catalog sharing.

## Method and correctness

The baseline is `f8194f3`; both arms received the same [test fixture](../../../src/app/reader_turn_pointer_tests.rs)
and test-only counters. Both host library-test binaries were compiled from
the same lockfile in the worker's Ubuntu x86_64 toolchain with test profile
optimization level 2 and incremental compilation off. Each process seeded
fresh local SQLite data and a synthetic word-count catalog, with only the
middle chapter populated. It used the real Reader, bundled font, Scribe
logical layout (930 × 1,240, scale 1), and a retained Slint software buffer.
There was no network, Kindle framebuffer, device touch, or driver in this run.

One qualification process per arm rendered the same public target page for
each size with a fixed clock: both PPM pairs compared equal **byte for byte**.
The timed runs did not write frames. After a separate excluded warmup process
per arm, six formal pairs ran in AB/BA/AB/BA/AB/BA order. Per process and size
there were three excluded forward turns and 24 measured ones. Each forward turn
used real `PointerPressed` and `PointerReleased` events in the next-page zone;
an untimed real back tap restored the identical source page. The ordinary
press was timed separately, and the previous page was allowed to draw before
release. The reported interval ends when drawing the **target** page returns;
assertions, reverse turns and setup are outside it. Each measured turn checked
the visible lines, footer progress and SQLite position: page 3, scalar anchor
2,548 in both arms, not merely a larger page number.

Each P50/P95 above uses nearest rank over 144 forward turns per arm and size;
the paired saving is the median of six differences between within-process
medians, not the difference between the pooled P50s. These are warm repeated
transitions between two pages, not independent books or a long uninterrupted
reading session. The [individual samples](reader-pointer-samples.json) include
press, release, draw, their combined interval, counters and position. To repeat
on `test-env`, run the following from each source tree with the same fixture
and instrumentation; the ignored measurement imposes no candidate work-count
threshold on the baseline:

```sh
CARGO_PROFILE_TEST_OPT_LEVEL=2 CARGO_PROFILE_TEST_INCREMENTAL=false \
  cargo test --locked --offline --lib app::reading::render_tests::measure_ordinary_pointer_turns \
  -- --ignored --exact --nocapture --test-threads=1
```

The candidate's full host library suite passed **1,718 tests, 0 failures, 7
ignored**. The ignored measurement and pixel qualification ran separately in
both arms. The ordinary work-count regression is not ignored. Existing cases
cover long-press selection, mark hits, sparse catalogs and access limits.

## Device boundary

The standard Kindle ARM release and its two native helpers built on `test-env`
with the reviewed X11 SDK headers. The first invocation stopped at its missing
X11-header prerequisite; the isolated retry used the existing project SDK. The
resulting application reported `weread 0.2.0` under user-mode Cortex-A8 QEMU.
This is a build/version check, not a firmware run. The disposable staging and
build outputs were not installed on the Kindle.

The rebuilt worker did not have the documented Scribe/PW6 firmware guest or
its QMP helpers available. The stock-card XTest observer is not an in-reader
evdev touch source and its forced full-screen witness is unsuitable for normal
turns. No Kindle lease was requested. After restoring the firmware environment,
exercise this gesture with guest input before a representative Scribe trial;
report ordinary partial and periodic full refreshes separately. Offer a
release-to-ink number only with a synchronized physical observation. Startup,
chapter boundaries and image-page latency were not measured by this change.

The [subsequent physical Scribe pilot](physical-reader.md) used a separate
ARM software rehearsal and then synthetic Slint pointer input on the actual
device. It measured natural submissions and matching driver completions, not
the missing firmware guest, evdev or optical ink settling.
