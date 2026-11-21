# First bound-launcher warmups: A timed, B not timed

These were the first two warmups of the November 22, 2026 stock-card series
that temporarily bound WeRead's own launcher. They ran on a Kindle Scribe
2022, firmware 5.19.5. Both arms launched an isolated measurement build
and the same disposable public shelf from the normal stock Library card.
A omitted startup orientation reuse; B enabled standard exact-pose reuse.
Neither enabled opposite-grip reconciliation. The installed daily executable
and private library were not the test inputs.

| Slot and public attempt label | Result | Lease held (Asia/Shanghai) |
| --- | --- | --- |
| `00-A` · `d0252953101885bb` | One qualifying Library submission with entries: **9.589599155–9.591219847 s** from card release | 04:52:35.992–04:56:41.801 |
| `01-B` · `28976d58145f5f1c` | The card was released and the trial GUI ran, but **no qualifying endpoint was collected** (`not_timed`) | 05:10:50.087–05:21:22.972, including recovery |

**What A measured.** The input helper bracketed the automated button release
on the recognized WeRead card. On the same device `CLOCK_MONOTONIC`, the
application recorded the first complete eligible Library frame after the
driver accepted its submission. Subtracting the later release bound gives
the lower elapsed bound; subtracting the earlier one gives the upper bound.
This is about **9.59 seconds** for one warmup, not a point estimate of optical
refresh, time to complete all covers, or a measurement of the untouched
daily installation. Normal stop, restore, close, independent absence and
lease release all passed for A.

**Why B has no time.** Release of the card was confirmed, but the host
collector refused the GUI ownership step (`claim:launch_identity`). The
trial GUI and its launcher generation were observed; the supervisor's
derived `application-owned` acknowledgement was missing. No same-attempt
Library submission receipt passed the collector. This does not establish
that the shelf failed to render, and a live process cannot substitute for
the missing endpoint.

The retained A and B launcher scripts were byte-identical. In a separate
Linux experiment using their actual ownership-check functions, the child
was held between fork and exec. Its parentage and birth already matched,
but its executable did not yet match when the supervisor consumed the
start notification. The check failed without publishing an acknowledgement;
after exec, the same guarded check succeeded. This demonstrates one way to
get the observed symptoms. **It does not establish the cause on B:** the
device recording cannot distinguish that timing from other rejected checks.

**How B was returned.** The original stop refused to act on incomplete proof
(`stop:unproven_close`). A recovery-only check independently validated the
original four-process group, sent one `TERM` to its verified supervisor, and
confirmed all four processes had exited and the native interface was idle.
It did not launch or tap again. Restore, close and independent absence then
passed; the installed launcher, stock document and application identities
matched their pre-visit records, the native guard audit passed, and the
original lease was released after its owner and token were checked. The
successful recovery is not a startup sample.

**Series boundary.** This series produced one timed A warmup, zero timed B
warmups and zero measured pairs. It supports no A/B speedup or default
decision. Neither visit, including A's valid interval, is an input to the
successor series: that series starts with fresh labels and its own warmups.
