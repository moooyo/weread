# Physical orientation comparison: six balanced pairs

**Result:** on one Kindle Scribe 2022 / 5.19.5 in a fixed native portrait,
the candidate reduced **programmatic indexed request to first Library frame
submission** in every timed pair. Baseline median: **6.604 s**; candidate
median: **5.356 s**. The median of the six paired savings was **1.323 s**
(range **0.911–1.697 s**). This is an engineering result, **not** release
of the daily stock Library tile to its first complete frame. Keep the feature
opt-in; do not change the power wait or deploy based on this batch alone.

Both arms used the same public 15-cover offline shelf, bundled font preflight,
launcher (including `sleep 2`) and isolated temporary document. Only B enabled
`startup-orientation-reuse`. The origin was the device `CLOCK_MONOTONIC` log
immediately before the native file-open request; the endpoint was
`first_application_frame_send`, after the driver accepted the first application
frame. It is not update completion or optical settling. Four separate Library
and Reader picture qualifications and two warmups preceded the timed pairs;
none contributed to the medians.

| Pair | Order | A (ms) | B (ms) | A minus B (ms) |
| ---: | :---: | ---: | ---: | ---: |
| 1 | AB | 6,653.620 | 5,317.586 | 1,336.034 |
| 2 | BA | 6,548.798 | 5,638.291 | 910.507 |
| 3 | AB | 6,554.558 | 5,244.676 | 1,309.883 |
| 4 | BA | 6,533.009 | 5,614.666 | 918.343 |
| 5 | AB | 6,969.026 | 5,393.622 | 1,575.404 |
| 6 | BA | 6,870.450 | 5,173.077 | 1,697.373 |

All six pairs are retained, including the two gains below one second. The
difference between the arm medians is 1.248 s; it is not the median paired
saving. Splash and event-loop queries each timed out in **6/6** A runs
(site medians 507.945 ms and 507.390 ms). B reused both in **6/6** runs
(40.721 ms and 5.468 ms). Within each pair, the two measured orientation
intervals together fell by a median **964.852 ms**. Do not attribute the
entire endpoint saving to time inside those intervals.

The [independent pilot](physical-pilot.md) fixed native `down`, raw rotation
`1` and software-to-scanout alignment `0°` before this run. All 18 launches
matched the native, launch and return readings; this measures the device
state, not someone's grip. All four route pictures matched their digital
scanouts at `0°`. Between A and B the Library body matched exactly; Reader
body mean absolute difference was `0.0043` on the 0–255 grayscale scale in
both software and scanout. Every timed thumbnail matched the Library rather
than Reader. No private shelf image or screen recording was used.

**Close and scope:** batch `36402ebb5e6aeeb1` started at **11:42:43** and was
released at **11:53:25 Asia/Shanghai on November 21, 2026**. All 24 transaction
steps succeeded, including native restoration, collection and close; the
temporary document and staging were removed. The daily installation was not
replaced. Physical correctness in the other portrait, transitions, touch
mapping, changed-pose wake/return, KPW6 behavior and the [daily first-frame
metric](../../PHYSICAL_TESTING.md#library-first-frame-startup) remain separate.

The [machine-readable summary](physical-balanced.json) contains the six
unrounded endpoint intervals. Offline on `test-env`, each duration was
recomputed by subtracting `out/<case>/request` from the submitted frame marker
in `results.tgz` and compared with the [frozen analyzer](../../../tools/kindle-font-trial/analyze_orientation.py)
report. The worker plan is `orientation-six-pair/` and its closed attempt is
`batch-8yk236y7/`, under the [recorded worker workspace](README.md#verification-gates).
