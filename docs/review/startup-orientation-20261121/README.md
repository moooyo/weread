# Startup orientation reuse on Kindle

**Status, November 21:** both release arms and a verified two-launch pilot are
ready on the verification worker. The opt-in candidate has not been run on a Kindle or
deployed. The November 20 [font A/B](../library-startup-latency-20261119/indexed-font-ab-20261120.md)
remains the latest physical measurement: 6.847 s median to first Library
SEND on the specified Scribe 2022 / 5.19.5 fixture. In that batch the splash
and event-loop queries timed out on all 12 timed launches (507.523 and
506.161 ms median, respectively). [Replaying the original captures and timestamps](font-baseline-breakdown.md)
puts the two waits at 1.015 s median within the six optimized-font launches;
4.225 s elapsed before Rust `main`. Neither is a demonstrated saving.

**Decision:** retain the reuse candidate for a controlled device comparison;
do not enable it in the package or replace the installed app yet. Keep the
two-second power-observer wait in both arms. Promote only after the fixed-pose
request-to-SEND comparison shows a positive median paired saving with at least
five of six pairs faster, the candidate actually takes both fast paths, and
the separate orientation, touch, wake and native restoration gates pass.
A faster host test or a correct framebuffer image alone cannot promote it.

## Candidate and fallback

- Both arms keep `kindle,bundled-font-preflight`, the indexed entry, power
  observer delay, splash artwork and first business-frame/input barrier. Only
  the candidate adds `startup-orientation-reuse`; the normal build script does not.
- The splash may read the same private launch-generation, nonce and ancestry
  checked portrait record used by the app. It requires a recognized live
  sensor pose matching that anchor, bracketed by matching GETVAR reads on
  its own framebuffer. It never passes its sampled pose to the event loop.
- The event loop independently checks the current initial calibration, opens
  the unique validated non-grabbing sensor, takes a synchronized snapshot,
  rechecks the framebuffer and verifies the request is still current. Only
  an applied mapping can skip the old query. It retains the same descriptor
  for subsequent events, without a gap from reopening it. Render offset is
  holding relative to scanout; raw touch uses holding. A stable inverted
  device does not imply a 180-degree pixel flip.
- On missing or fixed calibration, changed pose/rotation, incomplete sensor
  data or an unsuccessful read, the site uses its original bounded winmgr
  query. Its timeout and zero-on-failure behavior remain intact. There is no
  cross-launch cache, wake shortcut or new input owner. The kernel distinguishes
  querying current input state from processing the event stream; keep existing
  dropped-event handling and input gating. See the [Linux input protocol](https://docs.kernel.org/input/event-codes.html#ev-syn).

## Verification gates

After authenticated SSH access was restored, the candidate was staged on
`test-env`. The public trial tools passed 7 orientation, 12 original indexed
trial and 3 rollout cases. The native query helper passed 6 cases and the
development package contracts passed 10. Rust formatting passed for the
root workspace and standalone backend. Both backend configurations passed
their full host library suites (150 control, 153 candidate); the candidate
application binary tests passed 58/58. See the [power wait notes](power-wait.md)
for the separate launcher fixtures. These are software results.

A [controlled two-site host experiment](host-timeout-bench.md) used the real
500 ms polling deadline and child retirement, but synthetic matching sensor
and framebuffer observations. A silent child timed out twice on the control
(1,000.902 ms median total). The matching candidate issued no queries in any
of the six pairs; a changed-pose case issued both queries and took 1,002.583
ms. This establishes which waits the candidate avoids, not its device I/O
cost or a one-second Kindle startup gain.

The first wider platform run failed two assertions about the process-wide
native-command registry. A smaller four-fixture comparison reproduced the
interference on **both** arms: each passed 1/4 at two test threads, and 4/4
at one. Their own helper processes had already been reaped. The fixtures
were asking whether the *whole process* was idle while another test owned a
command. The test-only fix gives all five users of this global idle
assertion one shared fixture guard, without removing PID or idle assertions,
serializing the production registry, or changing the orientation fallback.
With that isolation, two alternating eight-thread platform runs per arm and
the final diagnostic build passed 61/61 each. One earlier eight-thread
candidate run could not read the generated helper's PID file; its cause
was not established. The final fixture reports the mode and returned error
if this happens again. It still requires a PID, confirmed child exit and a
quiescent registry.

A complete candidate library run with the isolation passed 1,712 tests
with four existing benchmarks ignored. After the final fixture diagnostic
change, a four-thread and a one-thread run each passed 1,711, failed one
and ignored four. The first failed in the article image retry case and the
second in automatic image continuation, at the same three-second preparation
deadline. Neither was a launch orientation assertion: the isolated article
retry case both passed and failed on **each** arm and uses a non-Kindle test
platform. Do not treat the root library suite as uniformly green or change
its deadline to qualify this candidate.

The [ARM build driver](build-arm.py) compiled both applications and both
backends from the same working source. Each application has seven font
receipts; A is 95,202,712 bytes and B is 95,205,016 bytes. Both static
ARM32 hard-float executables returned the expected version with QEMU's
Cortex-A8 CPU. A separate ARM test executable passed all three targeted
orientation cases under QEMU user mode (150 unrelated cases filtered out).
The [build and test record](arm-worker.json) includes the actual features,
ABI and tool versions. No QEMU duration is a Kindle benchmark.

Worker directory: `test-env:/root/weread-orientation-20261121.WR7ofkGL`.
`arm-pair/baseline/weread` and `arm-pair/candidate/weread` are the two
retained applications for the next trial; neither is an installation.

### ARM feature-off versus feature-on branch comparison

While the physical device was unavailable, a separate `rustc`/QEMU Cortex-A8
comparison compiled the frozen startup entry and decision functions twice,
once with the feature disabled (A) and once enabled (B). Both binaries used
the same sensor, framebuffer, legacy response and marker fixtures. All 40
observations passed: ten inputs at two sites for each arm. A called the old
transport once at each site for every input. B behaved as follows:

| Fixture input | B: splash | B: event loop |
| --- | --- | --- |
| Stable upright; stable inverted | Reuse | Reuse |
| Pose changes between sites; initial request changes during the event-loop sample | Reuse | One legacy query |
| Missing anchor; changed or unknown holding; first or second GETVAR fails; GETVAR changes | One legacy query | One legacy query |

The inverted entry retained a zero render offset and a 180-degree touch
mapping in the fixture. The two binaries were **not** the complete app ELFs;
no physical input, firmware call, panel update or startup latency was measured.
The cases deliberately exercise failures; their counts are not a device hit
rate. The worker retains `arm-branch-probe/run.py`, `branch-fixture.rs` in
that directory, and `arm-branch-probe/evidence-01/{result.json,A.tsv,B.tsv}`.

## Frozen pilot and November 21 handoff

The worker's `orientation-pilot/` is the pilot prepared for this round. Its public label
is `f297b8b406c0e5a5`. The 139,047,790-byte archive has eleven regular members:
both recorded ARM builds, a common launcher and production helpers, the
public 15-cover seed, one temporary document and its orientation-specific
request helper. Both the worker host and its Ubuntu userspace verified the
plan. The preparation receipt is `orientation-inputs/pilot-worker.json`.
It contains no observed pose, measured alignment or physical run.

On `test-env`, all **52 tests** in `tools/kindle-font-trial` passed together
(`logs/orientation-collector-final.log`). They cover the frozen inputs,
pixel and pose gates, the simulated device transaction, ownership-checked
cleanup and interrupted close. In particular, a consumed document whose
supervisor has not yet acquired the display lock is not considered finished;
cleanup waits for its exit receipt. A failed transfer still attempts close,
but an unconfirmed close cannot release the lease. These fixtures did not
contact a Kindle or operate its real lease.

At **09:39 Asia/Shanghai on November 21** the Aidoku coordinator reconfirmed
its earlier handoff: the original interrupted batch still held the physical
lease, with no verified restore/release receipt. The worker's pilot directory
had no physical attempt. WeRead did not connect, acquire or reserve the Kindle.
Do not treat that status snapshot, its age or an empty lock directory as a
release. Wait for the original owner's verified recovery, then check live
availability and private authentication before using the frozen pilot.

The first device window is **two Library launches only**, then native restore,
owned stage removal, disconnect and release. Evaluate the pictures offline:
the control sets the reference alignment; both arms must agree on native
pose and scanout, and B must actually reuse both queries. A missed fast path
or mismatched image ends this experiment before timing. A passing pilot
permits preparing a **new**, separately leased six-pair experiment, not
enabling the build flag. The [runbook](../../../tools/kindle-font-trial/ORIENTATION.md)
also lists the separate physical orientation, touch, wake and native recovery
gates. Neither a digital capture nor a first-SEND timestamp measures real
tap-to-stable-ink latency.

The [power-observer readiness investigation](power-wait.md) is a separate
question. Do not change `sleep 2` in either orientation arm.
