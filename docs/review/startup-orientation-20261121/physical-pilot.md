# Physical orientation pilot: two Library launches

On November 21, 2026, the indexed pilot `f297b8b406c0e5a5` ran on one
Kindle Scribe 2022 with firmware 5.19.5. Both arms used the frozen launcher
and the same public 15-cover shelf. A ran once before B; there were no
warmup exclusions or counterbalanced timing pairs. The device transaction
started at 11:35:22 Asia/Shanghai. Admission, staging, both launches,
native restore, collection and close each returned 0; release was recorded
at 11:39:10. The trial did not replace the daily application.

| Launch | Splash terminal marker | Elapsed (ms) | Event-loop terminal marker | Elapsed (ms) | Indexed request to first Library SEND (ms) |
| --- | --- | ---: | --- | ---: | ---: |
| A-library (control) | `splash_orientation_timeout` | 511.500847 | `event_loop_orientation_timeout` | 504.910923 | 7933.489924 |
| B-library (candidate) | `splash_orientation_reused` | 35.526846 | `event_loop_orientation_reused` | 3.238462 | 5433.003923 |

Site intervals start at `splash_orientation_begin` and
`event_loop_orientation_begin`. The final column starts at the programmatic
indexed file-open `REQUEST` and ends at `first_application_frame_send`, on
the same device `CLOCK_MONOTONIC`. Across these two launches A minus B was
2,500.486 ms, of which 977.646 ms fell within the two orientation intervals.
The rest of the difference is not attributed. With one fixed-order pair,
this is an engineering observation, not a performance result for the daily
entry, a stock Library tile release, or visible ink.

The native idle, launch and restored portrait readings agreed in both arms:
`down`, raw framebuffer rotation `1`. A-library established a 0-degree
digital scanout alignment; B-library matched it. The compared Library bodies
had mean absolute pixel differences of 0.0 for software capture and 0.0
for framebuffer scanout. This does not verify physical grip, the opposite
portrait, touch mapping, sleep/wake, or panel settling.

**Offline replay:** on `test-env`, the frozen `orientation-pilot/plan.json`
and `batch-5mtnu4ml/{operations.json,results.tgz,pilot-reference.json}` under
that plan identify the inputs. The [pilot gate](../../../tools/kindle-font-trial/orientation_gate.py)
reproduced the released reference. The [trace parser](../../../tools/kindle-font-trial/analyze_orientation.py)
validated both cases' marker order; the table uses nanosecond differences
converted to milliseconds. No credentials, account data or raw images are
included here.
