# Handoff: Kindle startup power wait

**Status (2026-11-26, Asia/Shanghai): paused.** One valid physical AB pair
showed a 2.107 s saving after removing `sleep 2`. The reverse-order pair is
not complete. Do not treat this as final acceptance or deploy the zero-wait
launcher. This handoff changes documentation only; there is no queued WeRead
visit or reserved device window.

## Installed baseline

The daily build is `767fd2c4e2aff3418608b137b919b4b20939a79e`, with the
previously qualified [frame-guard and first Library model changes](STARTUP_PERFORMANCE_20261126.md#four-pair-results).
The November 26 deployment on Scribe 2022 / firmware 5.19.5 replaced only the
application and frame guard. It read back all 38 public installation files,
started the actual daily app from the native card, preserved the existing data
directory and configuration, and returned the device at **15:40:21.143**. That
was a launch smoke, not a new speed measurement. The installed launcher still
has the [fixed two-second delay](../packaging/kindle/launch.sh#L2781-L2787).
No zero-wait test replaced the daily files or used the private shelf.

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
- **Incomplete replacement BA:** a new B produced the qualified in-app
  `SEND` witness and the controller returned the exact lease after its
  independent restore and absence checks. The next offline check rejected the
  `WAIT` association with `ValueError: Wrong first update/clock`. This is a
  compound check; the exact failed predicate is not known. The corresponding
  A was never dispatched. Do not use the controller's `primary=qualified`
  field alone to count a `WAIT` measurement. No complete BA pair exists.
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
contains the retained brackets and the exclusions. The trial was paused before
fetching the last raw timestamps: new `test-env` SSH connections had been
accepted up to public-key selection but the configured Bitwarden agent refused
signing. The replacement B controller emitted `released=true` and
`recovery_required=false`; an additional host-side readback could not run.
There was no subsequent Kindle connection, new lease request, or success
marker for the replacement pair. Other sessions may have used the device
since. No remaining visit is authorized by an old release receipt.

## Resume from another computer

1. Pull the repository's `main` and read this handoff. Review the current SSH
   configuration and verify `ssh test-env` works with the configured broker.
   Keep credentials, keys, lease tokens, user data and raw device logs out of
   the handoff. Do not fall back to local verification if the worker is unavailable.
2. Start **read-only on the worker**, without opening Kindle SSH. The retained
   replacement BA workspace is
   `test-env:/root/weread-zero-settle-completion-20261126.SCq941Gr`. Read only
   the bounded `qualified` and `startup_trace` fields from
   `stock-library-ab/visits/00-B/attempt/operations.json` there. Check the
   release and native-guard fields, the absence of that attempt's token, and
   that `01-A` still has no attempt. The original bundle and closed AB receipts
   remain in `test-env:/root/weread-zero-settle-20261126.RrJ1JNza`; the failed
   original A's separate close receipt is under `stock-library-ab/visits/05-A/attempt`.
   The earlier native-query breakdown remains in
   `test-env:/root/weread-native-timings-20261126.2b9ptyi7/native-query-profile.json`.
3. Replay the replacement B receipt with its private `trial/analyze.py`.
   Inspect the rotation, release bracket, `first_application_frame_send`,
   qualified SEND timestamp and `qualified_library_wait_done`. The analyzer
   checks ordering, a SEND-to-witness gap under 10 ms, and exact interval
   arithmetic. Diagnose the actual difference; do not simply widen the gate
   or relabel the result. If the time gap alone is the issue, consider an
   explicit same-update marker at the SEND/WAIT call site, with regression
   coverage for rejecting unrelated refreshes. Do not run only the unused A
   and pair it with the older B. Freeze and verify a new complete BA before
   any measuring. If the endpoint or instrumented build changes, run both
   orders under the new contract rather than combining unlike samples.
4. Use the existing [single-visit procedure](../tools/startup-observer/STOCK-AB-CLI.md).
   Review the installation reference again, complete the `test-env` gates,
   coordinate all current device users, and only then acquire a fresh
   exclusive lease for each visit. Restore native tab, pointer, light and
   original public files, prove independent absence and close SSH, then
   release the exact owner/token immediately. Keep any rejected slots and
   do not reopen them. No current lease, marker or scheduled action should
   be carried over as permission to connect.
5. Report both wait policies at the same endpoint after the reverse pair is
   complete. Keep warmups, refused activations and unpaired timings separate.
   Before recommending a daily change, distinguish the normal-start benefit
   from the remaining physical startup sleep/USB correctness question. The
   next independent optimization candidate is a lighter portrait probe; do
   not combine it with the wait experiment.

The worker retains private frozen bundles, scripts and receipts so this can
resume without the previous workstation's temporary files. Remove disposable
artifacts after the final verification and preserve only sanitized evidence.
