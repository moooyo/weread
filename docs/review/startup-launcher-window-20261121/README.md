# Remove an unused window-tree scan from ordinary startup

**Scope:** the shell supervisor, not the application ELF or a change to
orientation, fonts, power subscription, cover-sleep setup or display takeover.
The normal Library document ultimately starts this same detached supervisor.

The old launcher invoked a bounded `xwininfo -root -tree` on every approved
Scribe launch with the relevant tools installed. Its result only advertised
the historical Quick and Wi-Fi handoffs. Neither is part of the current UI
([product scope](../../PLATFORM.md#user-removed-wi-fi-and-quick-experiments)).
Even for an explicit handoff, the supervisor did not use this first snapshot
to decide which native windows were open: after the application exited and
the display was handed back, it read the foreground, view and window tree
again before opening anything.

The candidate does not publish the two availability records or spawn that
startup command. A generation-matched explicit request still needs a clean
application exit, state-open and checkpoint acknowledgments; it checks the
profile and tools on demand and observes current native visibility before
opening. Closing a native panel still requires independent fresh observations
before and after display takeover. Screen-saver policy and the two-second
power observer wait are unchanged.

## Measurement boundary

The [historical Scribe trace](../library-startup-latency-20261119/README.md#where-the-time-goes)
measured 555 ms median between settling and the start of takeover. That span
contains **both** this probe and sleep-cover configuration; it is not the
cost of `xwininfo`. The November 20 font A/B has no same-clock shell spans
for this split. Neither result assigns a Kindle speedup to the new launcher.

The [Linux benchmark](bench.py) exercises the actual two shell sources in
fresh instances of the launcher's Scribe fixture. It measures the worker's
monotonic clock from invoking its detached supervisor to a witness inside
the mock application. It retains the real `sleep 2`, uses a mock X service,
and checks the sleep-cover query, one app launch, native restoration and
guard cleanup each run. Each cohort excludes one warmup per arm and keeps
all six counterbalanced pairs, including any slower candidates. A second
cohort injects 200 ms into the mock X query alone; it tests causality, not
firmware latency. Neither cohort is a device boot, first Library frame or
tap-to-ink measurement.

For reproduction on `test-env`, use the dedicated worker directory
`/root/weread-launcher-window-20261121.F0uWjv1q`. `control.sh` is the
pre-change `17a2659` supervisor; `src/packaging/kindle/launch.sh` is the
candidate. In its Ubuntu userspace run `python3 -B /work/bench.py /work/src
/work/control.sh /work/src/packaging/kindle/launch.sh /work/repeat.json`
with a fresh output filename. The original host capture is
`window-bench-1.json` in that workspace.

## Worker results

| Mock X service | Control median | Candidate median | Median paired saving | Faster pairs |
| --- | ---: | ---: | ---: | ---: |
| No added delay | 2,393.599 ms | 2,350.121 ms | **46.071 ms** | 6/6 |
| 200 ms added to the X query | 2,615.685 ms | 2,378.747 ms | **240.927 ms** | 6/6 |

All 28 launches (including four excluded warmups) completed. The old launcher
ran one X query in all 14 starts; the candidate ran none. Both ran one blanket
query and one application, stopped and restarted the statusbar, resumed the
fixture window manager, retired the frame guard and removed the owned lock.
See the [individual samples](samples.json). The comparison includes the
mock application's entry witness. Medians of within-pair differences are
not computed by subtracting the two column medians.

## Lifecycle verification

Seven targeted launcher cases passed in the Ubuntu worker. They cover the
ordinary Library document, absent settings records, unavailable tools and
firmware, a fresh explicit request with no usable window visibility, command
timeout, output limits and process retirement on interruption. Separately,
the package contracts (10), native query helper (6), Xvfb frame guard (36)
and frame-guard watcher (5) passed. The two guard suites used host-target
`zig cc` with warnings as errors, not an ARM executable on a real panel.
The worker's unmodified GCC 13 build of the pre-existing guard rejects a
discarded `write` result under its fortified `-Werror` configuration. No
guard source or warning policy was changed for this launcher experiment.

The first full launcher run passed 241 of 242 tests. The failing passcode
test observed an application stop and cover acknowledgment, but those did
not establish that the supervisor had selected the custom cover. The same
controlled event order on both control and candidate reproduced the two
branches: revocation before the decision returns control to the stock path;
revocation after it exits with the specific permission-change error without
waking the app. A mock stock service reporting `active` is not a real unlock.
The corrected fixture waits for the supervisor to process the preceding
sleep event before changing the condition. No sleep code, timeouts, passcode
checks or restoration criteria were changed. The corrected test passed four
alternating control/candidate runs. A new complete run of the final launcher
suite then passed **242/242** in three independent shards (81 + 81 + 80);
the initial failure and subsequent passes are retained separately in the
worker's `logs/` and `passcode-fixture/` directories.

**Decision:** keep the on-demand window check in the working launcher. Its
normal startup no longer issues the unused query, and the Linux evidence
supports the change without relaxing the required handoff checks. Do not
replace the daily installation or claim a Kindle duration from this result.

The unchanged orientation pilot and its source stay in their own worker
directory. Do not copy this launcher into that frozen experiment. A later
physical comparison of this launcher must use a separate transaction, an
unchanged application binary and the same orientation policy in both arms.
