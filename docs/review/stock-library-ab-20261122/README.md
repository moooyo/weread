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

## Original file-exchange series

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

## First device attempt: transfer not confirmed

The first A warmup (`dcb6fdc5f079ab22`) acquired the Scribe at 03:33:20
Asia/Shanghai on November 22. Admission and stage creation succeeded. The
install step returned `unconfirmed` **276 ms after the stage acknowledgment**.
There was no successful install receipt, navigation, launcher activation,
card click or submission. Stop, restore, close and independent absence all
succeeded; the original owner's lease was released at **03:33:30**. This
attempt contributes **zero measured pairs** and is never reused as a warmup.

The old install path sent a 47,225-byte SSH command before the 102,851,217-byte
payload. It did not retain a transport exit code or a device-side refusal
from which to identify why it stopped. A command-length limit is a plausible
explanation, not a verified fact about the device; the result does not show
a broken card, renderer or launcher, or a successful transfer.

The revised sender uses a 217-byte SSH command. The remote shell reads the
exact script length from stdin; the untouched archive follows. In an isolated
Dropbear/BusyBox test on `test-env`, the old command failed immediately in
both lanes. The short command received all 102,851,217 A bytes and 102,853,044
B bytes respectively, with the original archive content and EOF verified.
This check used no Kindle. The failed device visit and its receipt remain
separate from any new series; a transfer fix cannot supply a timing sample.

## Second device attempt: exchange guard refused

The fresh A warmup `0796ac63322c9ceb` ran from **03:46:39 to 03:48:54
Asia/Shanghai**. This time the device acknowledged the complete script header
and got past both archives, the installed-file reference checks and public
shelf preparation. The private scratch-file check then returned
`selector_exchange_probe` (SSH status 1). This is not a transport failure:
the stage's own test of the exchange operation did not pass. Its original
error name covers several filesystem checks; it is not itself an errno or
proof that the exchange syscall was unavailable.

The production launcher was never exchanged, the native card was not clicked,
and there is no render timestamp. Stop, restore, close and independent absence
all passed; the worker verified the owner and token on release. The two
warmups belong to separate series. **Neither yields a performance sample.**

## Physical verdict: the measured volume rejects both rollback strategies

At **04:02:01–04:02:03 Asia/Shanghai on November 22**, a separate 51,740-byte
ARM diagnostic tested two disposable 19-byte markers on this Scribe 2022 /
5.19.5. It did not run WeRead, tap the screen, or open the production launcher
or private data. The device reported a **FUSE** user volume. Directory and
mount identity checks passed. The first cross-directory `RENAME_EXCHANGE`
syscall returned **`EINVAL` (22)**, before any post-exchange `fsync` or file
check. The independent hard-link strategy failed at **`linkat`, `EPERM` (1)**,
before replacing even a test file. These results apply to the tested
operations on this volume, not all FUSE filesystems, ordinary renames or
all Kindle models.

The diagnostic itself completed successfully: scratch cleanup, close and an
independent absence check passed. The owner and token were verified on
release. Its [public device receipt](filesystem-probe.json) is distinct from
both failed warmups.

**Decision: stop this series with zero formal launches, zero pairs and no
stock-card speedup claim.** Do not retry the same rejected operations, write
the launcher in place, or restore a look-alike copy instead of the original
file. The normal Kindle package default is unchanged. A different rollback
design, such as a temporary mount of *only* WeRead's own launcher, needs a
separate decision and rehearsal. No such mount was attempted. An explicitly
labeled temporary card would measure that entry, not the ordinary stock card.

## Approved retry: a single-file bind

The operator subsequently approved a disposable-file bind probe and, only
if that passes, a temporary mount at `/mnt/us/weread/launch.sh`. The
original launcher stays in place beneath the mount. The card, shared MIME
handler, production executable and private shelf are not replacement
targets. Unmounting must uncover the same original file, not a copy. This
is a new series; neither failed exchange warmup counts toward it.

The probe checks visibility using disposable files only. Even a successful
probe cannot show that a future card activation went through the selector.
Each warmup must establish that separately through the actual stock card,
the visit's selector and the same GUI's submission. A helper test or a
returned probe does not contribute a performance sample.

The handback assumes the exclusive device lease is respected. On a kernel
that supplies a nonreusable mount ID the selector checks it as well. On an
older kernel, a privileged process that deliberately unmounts the test and
rebinds its exact private source to the same target can, if the ordinary ID
is reused, produce an indistinguishable mount. The test does not claim to
defend against such a concurrent administrator. Observable foreign mounts
are never forcibly or lazily detached.

Before the device probe, `test-env` exercised actual Linux single-file
mounts in isolated namespaces, including independent visibility, normal
unmount, an interrupted caller and refusal to remove a foreign layer.
The selector passed 9 native and 9 ARM/QEMU cases. The scratch wrapper
passed 4 Linux integration cases; the complete stock transaction fixture
later passed 21 cases under Ubuntu shell and 21 under BusyBox. These are
worker recovery and integration results, not physical card timings.

**Physical result: stop before mounting.** At **04:24:34–04:24:37 on
November 22 (Asia/Shanghai)**, the Scribe accepted admission and the two
disposable marker files. The `prepare` step returned `bind_mount_table`
**before calling `mount`**. The independent session had no bound file to
witness. The probe verified the marker identities and contents, removed
its scratch files, confirmed absence independently and released the exact
owner's lease. See the [public receipt](scratch-bind-probe.json).

This is a failure to validate the device's mount table, not a failed mount
syscall or proof that this Kindle does not support file binds. The receipt
does not identify the exact field or table condition that was rejected.
The production launcher was not accessed, the stock card was not pressed,
and no A/B warmup or formal pair began. No speedup or regression can be
reported for card release to frame submission. Do not rerun the same
probe, relax the mount guard, or substitute the earlier indexed results.

### Read-only diagnosis on the same Scribe

At **04:35:06–04:35:08 on November 22 (Asia/Shanghai)**, a separate visit
read the mount table without creating a file, binding a path or opening an
application. Two reads agreed: there was exactly one `/mnt/us` row, its
ID matched the opened volume directory, its filesystem root was `/`, and
its propagation was private. The first field rejected by the selector's
numeric rules was **a parent mount ID on a different row**. The diagnostic
did not record that row's path or numeric value, and did not exercise the
C parser's stream rewind. It does not establish that the value was zero or
that the other row was unrelated to the selector. The [read-only receipt](mount-table-observation.json)
also records the independent native-state check and exact lease release.
Bind support and ordinary-card performance remain unmeasured.

**Selector change after diagnosis:** the revised parser accepts the exact
parent token `0` only on an unrelated mount row or on a unique `/` row
whose filesystem root is also `/`. It does not skip those rows. The user
volume, protected paths, source aliases and the exact bind target still
require a positive parent, and malformed or out-of-range parent values
are refused. The new error categories distinguish a forbidden zero from
an invalid numeric value. This covers one possible cause; the physical
receipt does not establish that the actual rejected token was `0`.

On `test-env`, the revised native and ARM/QEMU helpers each passed four
parser test groups and the original nine real-mount and recovery cases.
The original 66,044-byte helper and the 14 unused visit plans were not
modified. At that point the revised 66,868-byte helper had not run on this
Kindle. The next step was the [fresh disposable-file probe](#0445-revised-helper-passes-the-disposable-file-gate),
which subsequently verified visibility and restoration. A worker preflight
alone did not clear that gate.

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
- Count which position within each pair finished sooner, independently of
  the A/B counts. If the first or second visit consistently wins, describe
  that order effect; do not attribute it to orientation reuse.

The [earlier stock-card observation](../library-startup-latency-20261119/daily-render-20261121.md)
was an upper bound from framebuffer memory. The [indexed comparison](../startup-orientation-20261121/physical-balanced.md)
and [startup turn](../startup-orientation-20261121/event-loop-reconcile.md#november-22-physical-result-turn-after-draw)
also have different endpoints. None supplies an A or B sample here.

## 04:45: revised helper passes the disposable-file gate

On November 22, **04:45:54.890–04:46:00.155 Asia/Shanghai**, the Scribe
2022 on firmware 5.19.5 ran the revised **66,868-byte** helper. A second
connection saw the bound marker. After unmounting, the original marker's
file identity and bytes matched. The independent absence check and exact
lease release both passed. The **5.27 seconds** span lease acquisition to
release; they are not an application startup time. See the [device
receipt](scratch-bind-parent-probe.json).

This visit touched only the disposable markers. It did not mount the
production launcher, press a stock card or start the on-demand handler or
WeRead. It contributes **zero card releases, warmups or measured pairs**.
The 14 unused plans frozen with the **66,044-byte** helper are a separate
series; they do not inherit this gate result. An ordinary-card measurement
requires a new series frozen with the verified helper and evidence from
the actual selector and GUI in each visit.

## Bound warmups and the successor series

The [first two bound warmups](first-bound-warmups.md) established the full
endpoint for the A arm. B reached its isolated application but lacked a
derived launcher acknowledgement. No timed pair resulted. The site of the
missing acknowledgement was reproduced on the worker, not established as
the cause of that physical visit. After ownership and handback were
verified, that series stopped; neither warmup carries forward.

The successor uses the same application binaries, helper, public shelf and
clock. Its collector uses two matching observations of the four-process
launch chain, the exact executable, public data path and original request.
A missing optional launcher acknowledgement is not the end timestamp: only
the application's own submission on the same request qualifies. An existing
wrong acknowledgement or a missing state-open record still prevents timing.
The full protocol and all 14 new visit slots were frozen before either arm
was run.
