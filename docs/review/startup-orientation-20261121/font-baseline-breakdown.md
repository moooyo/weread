# Where the qualified font build spent startup time

This November 21 **offline replay** uses the [November 20 indexed font A/B](../library-startup-latency-20261119/indexed-font-ab-20261120.md) on one Kindle Scribe 2022, firmware 5.19.5. Both arms used the same temporary indexed entry, one boot, warm OS caches, and a restored public 15-cover offline shelf; each launch started a new process. The default [font analyzer](../../../tools/kindle-font-trial/analyze.py) reproduced all 12 measured samples, both excluded warmups and the four picture qualifications. The six measured B launches below **include font preflight but not orientation reuse**.

All times are milliseconds, measured from device `CLOCK_MONOTONIC` timestamps within each launch. `Request` is the programmatic native file open; `SEND` is the first accepted Library frame submission. Neither is a physical tap, a completed panel refresh, or evidence that all covers are visible.

| Case | Request → SEND | Request → main | Main → software ready | Software ready → SEND | App attach | Both orientation waits |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| B1 | 6940.448 | 4254.212 | 1362.878 | 1323.358 | 619.310 | 1017.831 |
| B2 | 6917.545 | 4310.978 | 1255.937 | 1350.630 | 518.168 | 1012.215 |
| B3 | 6772.975 | 4188.897 | 1279.006 | 1305.071 | 535.945 | 1014.466 |
| B4 | 6773.051 | 4163.891 | 1272.887 | 1336.274 | 529.206 | 1015.791 |
| B5 | 6948.590 | 4305.221 | 1318.761 | 1324.608 | 553.360 | 1019.188 |
| B6 | 6775.470 | 4195.298 | 1282.382 | 1297.789 | 546.500 | 1012.664 |
| Median | **6846.507** | **4224.755** | **1280.694** | **1323.983** | **541.223** | **1015.128** |

The three adjacent stages sum to the primary endpoint **within each unrounded run**. App attach is already inside `main → software_ready`. The splash orientation wait is also inside that stage; the event-loop orientation wait is inside `software_ready → SEND`. Both queries timed out in all 12 measured A/B runs. In B, their combined time was **14.807%** of request-to-SEND at the median (14.633–14.998%), calculated per launch. This identifies a candidate for optimization, not a measured saving. Do not add independent medians or count the diagnostic columns again. The largest stage is before `main` (4.164–4.311 s); this replay does not have same-clock shell spans to apportion it to dispatch, the fixed wait or display takeover.

**To reproduce:** use `samples.B1` through `samples.B6` in the font analyzer's output. In each `t_from_request_ns`, use zero for the request, `main_enter`, `software_ready` and `first_application_frame_send` for the adjacent boundaries, and `application_attach_begin` and `application_attached` for attach. Sum the two within-site `orientation_timeout − orientation_begin` intervals (`splash`, `event_loop`). Convert to milliseconds only after subtraction; compute the median across the six results. The [published samples](../library-startup-latency-20261119/indexed-font-ab-20261120.json) provide the primary, font and individual orientation checks. The verification workspace retains `logs/previous-indexed-font-replay.json` with the six unrounded interval sets. This replay collected **no new hardware timings**.
