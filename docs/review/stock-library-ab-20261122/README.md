# Stock Library card to complete WeRead Library submission

## Question and scope

How much does the standard exact-pose orientation reuse change startup time
when the user opens the ordinary WeRead card in the stock Kindle Library?
The operator authorized a temporary switch of **WeRead's own `launch.sh`
only**, with restoration after each visit. The card, shared dispatcher,
installed application and private library are not test fixtures and must not
be replaced. Both arms use the same disposable 15-book public shelf; this
measures the stock card launching an isolated measurement build, not the
absolute latency of the untouched daily installation.

| Arm | Common settings | Only performance variable |
| --- | --- | --- |
| A | Kindle backend, bundled font preflight, trial Library submission witness | No startup orientation reuse |
| B | Same source, launcher, fixture, build options and witness as A | Standard `startup-orientation-reuse` |

Neither arm enables the new opposite-grip reconciliation candidate. No
operator gesture, click, recording, library reset or change of grip is part
of this test. Existing device caches are not reset; this is a warm-device
comparison, not a cold-boot test.

## Before device access

`test-env:/root/weread-ordinary-isolated-pair-20261122.onNveKIX/pair.json`
records the same-source ARM builds. A is 95,242,368 bytes and B is
95,246,600 bytes. Both include `trial-library-frame`; neither changes the
default package. A test with the ordinary desktop application confirmed
that the public fixture reaches Library without an account or a forced
route; an empty directory with the same options reaches Login instead.

The device program checks the catalog entry and original installed launcher
against the retained, closed November 20 installation. Before touching the
live launcher it exchanges two of its own files and swaps them back on the
actual device volume. The test selector and original launcher then use the
same atomic operation; no copy-over or two-step replacement is permitted.
The submit witness binds the release, original request descriptor, GUI and
launcher generation. Restoring the original requires matching its bytes,
file identity, permissions and ownership. The document is byte-checked too;
the catalog can normally touch its timestamps, which are not rewritten.

Worker rehearsals covered both the ordinary button input with a live
observer lock and a disconnect after the launcher exchange but before any
click. Recovery restored the original without reopening the app. The ARM
QEMU software checks and these rehearsals are not device timing samples.

## Acceptance and stopping rules

- Start: the automatic button release on the uniquely recognized normal
  WeRead card. Keep the two device-clock timestamps that bracket release.
  File-open requests, shell launches and attempts on another card are not
  interchangeable with this event.
- End: the first full WeRead Library frame (layout, navigation, recognizable
  entries) submitted by that same GUI, recorded immediately after driver
  acceptance. Covers may be placeholders; `Ready`, a splash, a later
  framebuffer sample, and optical refresh are not this endpoint. The public
  fixture should produce `entries`, not an empty state.
- For release bounds `[early, late]` and submission `s`, report an elapsed
  interval `[s − late, s − early]` on `CLOCK_MONOTONIC`. Count a paired gain
  only when B's interval is wholly earlier than A's. For paired savings,
  use `[A.low − B.high, A.high − B.low]`; do not subtract independent
  medians or use the host clock.
- Run two unmeasured warmups (A, B), then six pairs in alternating order:
  AB, BA, AB, BA, AB, BA. Each visit must restore native state and the real
  launcher, close its connections and verify release before the next can
  begin. No lock is held for analysis, build work or another project's turn.
- An unsupported filesystem operation, changed protected entry, ambiguous
  card, missing witness or unconfirmed restoration stops the series. Record
  a failed or not-timed visit as such; never retry a possibly dispatched
  click, substitute a previous result or replace a missing pair. An
  interrupted close is recovered without starting the app again. A complete
  six-pair comparison requires the full fixed series.

The [earlier stock-card observation](../library-startup-latency-20261119/daily-render-20261121.md)
was an upper bound from framebuffer memory. The [indexed comparison](../startup-orientation-20261121/physical-balanced.md)
and [startup turn](../startup-orientation-20261121/event-loop-reconcile.md#november-22-physical-result-turn-after-draw)
also have different endpoints. None supplies an A or B sample here.
