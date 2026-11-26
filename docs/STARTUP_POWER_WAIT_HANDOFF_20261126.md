# Handoff: Kindle startup power wait

**Status (2026-11-26, Asia/Shanghai): successor AB and BA complete.** The
zero-wait arm was faster in both orders, with a median within-pair saving of
**1.517 s** to same-update driver completion. Both warmups and all four
measured visits restored native state and returned their exact leases.
The earlier 2.107 s AB saving under a different contract is not included.
The daily wait remains in place; physical startup sleep and USB correctness
are still unverified.

## Last verified installation

The November 26 deployment on Scribe 2022 / firmware 5.19.5 installed
`767fd2c4e2aff3418608b137b919b4b20939a79e`, with the previously qualified
[frame-guard and first Library model changes](STARTUP_PERFORMANCE_20261126.md#four-pair-results).
It replaced only the application and frame guard, read back all 38 public
installation files, started the actual daily app from the native card,
preserved the existing data directory and configuration, and returned the
device at **15:40:21.143**. That was a launch smoke, not a speed measurement.
The reference launcher retains the [fixed two-second delay](../packaging/kindle/launch.sh#L2781-L2787).
The revised preparation did not recheck the live installation. No zero-wait
test replaced the daily files or used the private shelf.

## What the delay does

The timer starts after the launcher accepts a local observer stream record.
The [subscriber publishes that record](../packaging/kindle/launch.sh#L1617-L1621)
before executing `lipc-wait-event`. It does not acknowledge firmware
registration or wait for a particular power state. The interval is a local
buffer, not a firmware requirement and not a guarantee of event delivery.
In the [reviewed KOReader implementation](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/input/input.c#L108-L152),
the parent continues after opening the pipe and forking; the
[child listens asynchronously](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/input/input-kindle.h#L143-L175).
That is a source comparison, not a WeRead correctness test.

## Evidence to retain

- **Native queries, separate diagnosis:** one warmup and three measured visits
  on Scribe 2022 / 5.19.5. The fixed delay was **2.019 s** median. From its end
  until frame-guard startup, the six read-only queries and surrounding work
  took **1.675 s** median (1.497–1.979 s). The largest individual span was
  portrait calibration (**632 ms**, of which **386 ms** was measured within
  the command). The two direction requests together were only **20 ms**
  median. Sensor and framebuffer metadata checks together took **115 ms**
  median, with **417 ms** in the third sample. These instrumented durations
  include observer overhead and are not savings estimates. The diagnostic
  restored the device after every visit; the last release was **16:32:24.343**.
- **Isolated zero-wait A/B:** A retains the line; B removes only that line.
  The two launchers otherwise match byte for byte. Both use the same
  instrumented ARM program, native helpers, public 15-book cached shelf, and
  ordinary stock card. The start is the synthetic release on that card; the
  endpoint is the matching driver `WAIT` for the first complete, populated
  Library update, not the splash or optical settling.
- **Qualified measured pair (AB):** **7.976260 s → 5.869063 s**, using the
  release-bracket midpoints; the paired saving is **2.107197 s**. The measured
  interval around the fixed delay was 2,018.755 ms in A and 4.379 ms in B
  (marker overhead remains in B). The two warmups are excluded. Both visits
  independently restored native state and returned their leases.
- **Incomplete original BA:** B produced a valid but unpaired **6.402722 s**
  `WAIT` result, excluded from comparison. A was refused at stock-card
  activation with `tap:uncertain_or_refused_tap`, before a qualified launch
  receipt. Its first automatic close could not reconcile the supplemental
  `activeView` changing from unavailable to `KPP_HOME`. A close-only recovery
  checked the original Home tab and exact pointer, native state, original
  bytes and independent stage absence, then released the original owner/token
  at **18:14:38.093**. No output from that A was promoted.
- **Incomplete replacement BA:** the B visit produced a qualified in-app
  `SEND` witness, but the original offline analyzer rejected the `WAIT`
  association with `ValueError: Wrong first update/clock`. A later read-only
  replay on `test-env` found that only the time-gap check failed: the Library
  witness timestamp came **18.933846 ms** after the first application `SEND`,
  exceeding the strict **<10 ms** gate. The 0° rotation, timestamp ordering,
  and exact release-interval arithmetic passed. The gap alone neither proves
  nor disproves that the events came from the same update. Worker readback
  confirmed native restoration and file guards, independent stage absence,
  exact lease release, and no remaining private token for that B attempt.
  The matching A plan had no attempt. The B remains excluded; do not use
  `primary=qualified` as proof of `WAIT` or dispatch the unused A.
- **Linux checks:** 17 targeted launcher cases passed in each arm. They cover
  normal launch and exit, frame guard lifetime, USB and sleep events delivered
  before takeover, and delayed subscription with a later USB delivery. The
  original card transaction passed 17 cases; the revised return-only rule
  passed 18, including rejection of an incorrect tab or pointer. These mock
  cases do not establish that physical firmware delivers events generated
  before subscription. A real overlapping physical power or USB transition
  has not been validated. Neither the published duration nor KOReader evidence
  warrants removing the wait from the daily build by itself.

The [sanitized timing and status record](review/startup-power-wait-20261126-partial.json)
keeps the original brackets, exclusions and the offline replay outcome. The
initial pause followed a Bitwarden signing refusal on `test-env`; the later
SSH replay resolved the outstanding host-side readback without opening Kindle
SSH. A historical lease release does not establish current availability.

## Revised same-update trial

The trial-only build, based on `767fd2c`, records the EPDC update marker on
both the accepted first full application `SEND` and the successful `WAIT` for
the confirmed full Library draw. A failed ioctl, a different update or a
completion reused from the userspace cache cannot qualify. The analyzer
rechecks the original populated-Library witness and synthetic release
bracket, requires matching positive update markers and ordered timestamps on
`CLOCK_MONOTONIC`, and keeps exact release-to-`WAIT` bounds. The endpoint is
still driver completion, not optical settling. No extra refresh or driver
wait was added. The original analyzer and its rejected receipt were not
changed.

The two new arms use the same instrumented ARM executable, native helpers,
public 15-book fixture and reference files. Only B's trial launcher omits
`sleep 2`. The new instrumentation requires fresh measurements in both
orders; the previous AB is historical, not part of the revised comparison.

On `test-env`, 165 backend tests, seven analyzer tests, 49 combined stock
transaction and submission tests, 12 pinned-request and GUI identity tests,
and 17 launcher lifecycle tests per arm passed. The Cortex-A8 app version,
bind policy and observer clock/deadline probes also passed. The BusyBox
variant was not run because the tool was unavailable. All six plans for the
first same-update batch passed offline preflight, but that batch yielded no
physical timing sample. Physical sleep and USB transitions during startup
remain unverified.

## Stopped window and wake diagnosis

The `00-A` warmup in the 22:40 window stopped at `admit:ssh_transport_exit`
with SSH exit 255. No staging, navigation, tap or application launch was
attempted. The exact lease was returned at **22:42:40.976**; the worker
confirmed the token was absent and no recovery was required. The five
remaining slots were not dispatched. The window was yielded and the batch
was stopped, not paused between measured visits.

After the operator woke the Kindle and BiliComics returned its separate
deployment lease, a newly approved read-only check authenticated to the
Scribe. The unmodified trial `admit` returned `daily_install_guard`; the
following document/launcher byte audit was not run. The diagnostic did not
stage files, change native state or consume a measurement slot. Its exact
lease was returned at **22:49:57.662**, and the host readback found the
shared lock absent. All times are Asia/Shanghai.

The BiliComics team reported a closed installation of 0.1.25 at
`13f4434b45d8372c2b2a18e151793814a18b8d5d`, with successful public-file
readback. On `test-env`, the released development archive's executable and
launcher were 74,980,360 and 80,637 bytes, respectively; the old guard
expects 74,940,168 and 80,185. The JPEG worker remains 426,672 bytes. This
accounts for an expected peer-version refusal; the composite result does
not isolate other predicates or retrospectively diagnose SSH exit 255.

## Successor batch with refreshed peer references

On `test-env`, the peer's closed deployment summary confirmed exact lease
return, native restoration, stage removal and matching static-file readback.
The three public BiliComics archive members were independently byte-compared
with the verified staged package. Only the expected BiliComics executable
and launcher sizes changed in the generated device scripts; the JPEG worker
and every other guard stayed unchanged. The package comparison is not a new
readback from the live Kindle.

Both frozen A/B archives match the stopped batch byte for byte. The original
instrumented ARM application, 15-book seed, launch policies, native helpers,
and same-update analyzer were reused without recompiling. The 61 stock,
request and GUI tests, two updated guard tests, and seven analyzer tests
passed on `test-env`. All six fresh labels and corresponding scripts passed
offline verification and preflight before any attempt. No Kindle connection
or lease was made during preparation.

## Successor physical window

A separate read-only lease at the start of the 23:08–23:50 Asia/Shanghai
window verified SSH, the refreshed installation guards, and the original
stock document and launcher bytes against the frozen references. The device
was not staged or activated, and that lease was returned at **23:08:04.927**.
The successor then passed both scheduled warmups on Scribe 2022 / 5.19.5,
including exact same-update driver completion and independent handback.
The A and B leases were returned at **23:12:19.985** and **23:16:39.125**,
respectively. Warmups do not count as paired observations.

The four measured visits ran in `AB`, then `BA` order. Each used a fresh
lease and the same public fixture, with B removing only the startup
`sleep 2`. The endpoint is the first complete populated Library frame's
matching driver `WAIT`, timed from synthetic release on the stock card.

| Order | A: fixed wait (s) | B: zero wait (s) | Paired saving (s) |
| --- | ---: | ---: | ---: |
| AB | 8.242 | 6.487 | 1.754 |
| BA | 7.750 | 6.470 | 1.280 |

Values are release-bracket midpoints. The median within-pair saving was
**1.517 s** (exact endpoint bounds: 1.516092346–1.5183950775 s). Those bounds
reflect only synthetic release timestamp uncertainty, not sampling variance
or optical settling. Both orders improved, but two pairs on one Scribe
establish neither a hardware support matrix nor a general performance
estimate. Neither warmup nor the historical 2.107 s saving is pooled here.

All six launches rendered at 0°, had positive matching `SEND` and successful
`WAIT` update markers, independently verified original file identities and
bytes, restored native state, and removed their owned stage. The last lease
was returned at **23:32:42.059 Asia/Shanghai**; the readback showed no shared
lock and the remaining window was yielded. No daily package or private shelf
was changed. The [sanitized paired receipts](review/startup-power-wait-20261126-same-update-pairs.json)
retain exact bracket, render, `SEND` and `WAIT` timestamps and per-visit handback.

## Next acceptance gate

The normal-start comparison is closed. **Do not reopen any slot or remove
the daily wait from this result alone.** The observer's stream-ready record
still precedes firmware subscription, so separately verify naturally
occurring sleep and USB transitions that overlap startup. Prepare that
validation off-device, with a way to distinguish event generation, delivery
and native recovery. Before any new physical work, review the live
installation against approved references, coordinate a fresh window, obtain
approval for Kindle SSH, and follow the
[single-visit safety procedure](../tools/startup-observer/STOCK-AB-CLI.md#transport-timing-and-recovery-for-each-visit).
The lighter portrait probe is an independent candidate.

The completed evidence, preparation receipt and analyzer remain on `test-env`
in `/root/weread-zero-settle-peer-refreeze-20261126.gnnz0rdb`. The frozen
batch at `/root/weread-zero-settle-same-update-20261126.b0fnyadp` stopped at
its first warmup; its unopened slots are not a continuation. Sync `main`
and use approved worker SSH before any replay. Do not copy credentials,
tokens, private data or raw device logs into Git.

For historical replay only, the original and rejected replacement trials
remain at `test-env:/root/weread-zero-settle-20261126.RrJ1JNza` and
`test-env:/root/weread-zero-settle-completion-20261126.SCq941Gr`. The original
refused A's close receipt is under `stock-library-ab/visits/05-A/attempt` in
its own workspace. The separate native-query breakdown remains in
`test-env:/root/weread-native-timings-20261126.2b9ptyi7/native-query-profile.json`.
Retain the sealed inputs needed for resumption and clean up disposable
artifacts after final verification.
