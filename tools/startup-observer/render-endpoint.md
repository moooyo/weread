# Daily Scribe render observer

`render-observer` is an external read-only witness for the already installed
WeRead app on the Scribe 2022 / 5.19.5. It does not enable logging in that app,
replace the normal entry, install tracing, issue panel updates, grab input, or
export pixels. The surrounding sampler must use the ordinary native card and
the same device `CLOCK_MONOTONIC` clock for the release interval.

## What the receipt means

- The first `ARMED` is a negative initial screen, not a launch timestamp.
  The observer admits at most 90 seconds to the matching dispatch intent and
  then 30 seconds to a result. A matching cancellation record ends either
  phase. There is no retry or second input. A late attach to an already
  recognizable Library is rejected, not assigned a zero startup time.
- `last_negative_ns` and `first_complete_scene_ns` bracket the first supported
  digital scene. `confirmed_scene_ns` is a later corroborating sample; it is
  not the first frame timestamp. Samples are taken roughly 10 ms apart, with
  monotonic bounds around each read, not a promise of 10 ms resolution.
  The observer pins the app's executable identity and start time, ordinary
  arguments and launcher PID record, and binds its result to the same
  generation and process as the case's private session receipt.
- On the qualified upright light pixel layout, the match requires the fixed
  Library heading, selected bottom navigation and default sort control, **and**
  a structured first grid cover plus caption, a list row with thumbnail,
  text and separator, or the literal empty-shelf label plus its two distinct
  buttons. Placeholder art is valid. Only threshold/shape decisions are
  made in memory; no book titles, image bytes, counts or coordinates are
  returned. Geometry checks bracket every read of the active Gray8 page.
  Raw rotations 1 and 3 are admitted at the reviewed geometry, but pixels
  are never rotated or inverted to make a mismatch pass. An unsupported
  theme, flat white cover, different layout, or switch between matching
  configurations can result in no measurement.
- `evidence=submit_upper_bound` requires the same WeRead main thread to be
  sampled in `MXCFB_WAIT_FOR_UPDATE_COMPLETE` on its `/dev/fb0` before and
  after reading the qualifying content, with the same descriptor and ioctl
  argument. A later three-descriptor event-loop `poll` or `ppoll` must also
  bracket the same complete scene on the same active page. The timestamp is
  **after that later observation**. The ordinary initial frame requires a
  successful submission before this next loop; the startup artwork does not
  wait there. The second observation distinguishes a fallback path that may
  wait for a preceding image update before resubmission. This is a conservative
  **submission-by observation** for this build; no ioctl return, update
  marker value or application timestamp was captured. It cannot establish
  the exact time or identity of the first accepted update, successful
  completion of a wait, or optical response. Subtract the earliest plausible
  release from the observed upper bound; do not convert it into an exact
  dispatch-to-SEND measurement.
- `memory_only` means only the digital scene was recognized. A short wait,
  denied `/proc/<own-app-pid>/syscall` read, or another kernel ABI may produce
  this outcome. The zero submission field is a sentinel, never latency.
  Neither branch establishes finger input, input readiness, or stable ink.

## Replay on the verification worker

`render_replay.py FRAMES NotoSerifSC-Bold.otf.gz` compiles against the same
selector and consumes the two public historical `physical-a.png` and
`physical-b.png` Scribe Library frames, the public firmware gallery's
`library.png`, `library-list.png`, `settings.png`, `detail.png`, and the four
qualified pilot images `pilot-{A,B}.{png,pgm}` (software and active scanout).
The empty-state positive uses the known immutable face and public layout as
a source-model fixture, not a previously qualified physical empty shelf.
Half-turn, wrong polarity, missing navigation, controls, caption or content,
the empty label/buttons separately and malformed input are negative controls.
`render_observer_test.c` exercises supported page geometry and drift, private
session attribution and discrimination of the exact WAIT request and fd.
Build the field binary with `zig cc -target arm-linux-musleabihf -O2 -static
-s -std=c11 -Wall -Wextra -Werror render-observer.c -o render-observer`.
Require static ARM hard-float with no `INTERP` or `DYNAMIC` program header.
