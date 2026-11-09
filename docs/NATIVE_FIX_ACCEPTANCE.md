# Native reading corrections and acceptance

Recorded: 2026-11-09 (Asia/Shanghai).
Updated with the KOReader alignment decision: 2026-11-10 (Asia/Shanghai).

This record preserves the historical TOC status alignment, immediate SYSTEM
night-mode implementation, display-release correction, and bounded
[KOReader rotation comparison](ROTATION_ALIGNMENT.md), followed by their v8/v10
physical evidence and normal-mode deployment. The latest accepted night-mode
decision supersedes the SYSTEM path; it does not reopen the completed finite
resource closeout or certify other devices.

## Current KOReader-alignment candidate

The user explicitly chose the reviewed KOReader framebuffer approach on
2026-11-10 (Asia/Shanghai). Application night mode now has an independently saved
preference. The implemented source candidate uses renderer-owned grayscale 1/2
hardware inversion only for the explicit Scribe 2022 / firmware 5.19.5 target
with actual 8bpp and checked layout; it uses software compensation otherwise.
Ordinary toggles retain touch ownership and a checked full redraw without SYSTEM
IPC, native WM continuation, application suspension, work quiescence, or explicit
light setters. Native day/night settings remain unchanged. Real stock sleep
still uses coherent native-baseline frame-guard handoff; active ordinary exit
restores startup hardware state, and an already-yielded exit performs no further
mode write. See [KOReader night-mode alignment](KOREADER_NIGHT_ALIGNMENT.md).

On `test-env`, this candidate passed 1,492 library cases with three manual
benchmarks ignored, 52 Main cases, 82 backend cases, four package-contract cases,
and checks covering 463 translation keys. The first launcher execution passed
194 of 195 cases, retaining one failure involving exit 143. After the minimal
grace correction, all 10 targeted and all 198 full launcher cases passed.
The shell correction left the Rust/UI/assets/vendor/lock inputs unchanged,
preserving the scope of their recorded results. No second Rust execution is
claimed.

ARM application `d221d65d`, development ZIP `dae7dd52`, and production launcher
`a5fec72f` now have recorded identities in
[KOReader night-mode alignment](KOREADER_NIGHT_ALIGNMENT.md#remote-software-and-artifacts).
The isolated R1 physical continuation below has its own observations. Its isolated
preference-reopen gate is now closed; the normal-entry update and remaining
physical gates are **OPEN**. The
historical sections keep every earlier test count, hash, measurement, and device
result bound to its named artifact.
The installed normal trial remains the historical v10 SYSTEM implementation:
launcher `15aab68c`, application `8010a217`, and helper `a235b41c`. Its full
identities and normal-mode observations are retained at the end of this record.
Overall acceptance remains incomplete.

### R1 isolated physical continuation

On 2026-11-10 (Asia/Shanghai), the matching candidate ran in a separate home on
the identified Scribe 2022 / firmware 5.19.5. A short operator-terminated window
returned real exit 143 with owned-family, lock, and window cleanup and native
grayscale 2 restoration. In the later manual window, generation 1 retained the
same application while confirmed black and white transitions saved the matching
app preference and changed kernel grayscale 2/1 without changing native `Y8INV`.
The user estimated each confirmed transition at at most one second, saw no native
Home or startup detour, and reported stable illumination. The initial click was
inconclusive and is not counted as a successful transition.

A five-second stock power-button cycle retained the same application and matched
sleep/wake cycle 1; the user returned directly to the same white page with working
touch. An inverted-holding stage confirmed automatic orientation and a right-side
page turn. After returning upright, voluntary UI exit returned real status zero,
and the user confirmed dark native presentation and touch. The immediate operator
native-after guard reported `native-awesome-not-running` without a process state.
A later read-only sample found the original native manager running normally,
with native `Y8INV`, grayscale 2, restored status bar, active power, and native
foreground. All owned process-family identities, locks, and the guard window had
ended. The initial guard failure remains evidence with an unproven cause; it is
not reported as an immediate first-pass restoration result. Detailed chronology
and raw relative rotation values are retained in
[KOReader night-mode alignment](KOREADER_NIGHT_ALIGNMENT.md#r1-isolated-physical-continuation).

A subsequent isolated reopen confirmed the saved light startup and Library,
followed by real UI-exit status zero and dark native appearance with working
touch. Its first native-after check succeeded and all owned processes, locks,
and the guard window had ended. The earlier failed receipt is retained; the new
result closes only the isolated preference-reopen gate. The normal entry remains
the old v10 installation pending its separate update.

## Historical implemented SYSTEM behavior

The Reader TOC download icon and chapter status use the same full-height row,
with the icon and text vertically centered. The night-mode information subtitle
has been removed for the supported control.

On an identified supported Kindle, night mode requests a resident system-theme
transaction. The application suspends rendering and input; the owning launcher
briefly resumes its captured window manager, sets the system mode, verifies its
readback and available persisted preference, pauses that same manager, and
resumes the retained Reader. Confirmed native mode is authoritative at startup,
wake, and commit. Preference-mirror failure cannot undo a committed system
change, and an ordinary exit does not republish a stale target. See
[Native theme continuity](NATIVE_THEME.md) for the protocol.

The launcher defaults an absent `DISPLAY` to `:0.0` while preserving an explicit
value. Its power-event relay enters bounded child cleanup on a registered stop
and checks stopping before waiting. The fallback allows the original wrapper
to reap its proven child before retiring the wrapper. Original PID, birth,
ancestry, first-signal, and native-restoration guards remain enforced.

## Historical remote verification

All software verification in this record ran on `test-env`. Local operations
were source review, file transfer, documentation editing, and Git operations.

The original corrected Rust execution passed 1,484 library tests with zero
failures and three ignored manual benchmarks in 196.75 seconds, followed by all
52 Main tests in 5.46 seconds. The ARM release build exited zero in 4m 39s. Its
ELF SHA-256 is
`8010a21786b0b68dce67baeb00452e85f217daaef08293d6c25959d9f81fbc7b`.

After shell and Python-only corrections, 509 runtime source leaves exactly
matched the verified remote tree. Canonical sorted-leaf identity is
`634a4bf340481c796c933dcdb46dd8be7d066e997e9d25bba2db94a0285e0cb1`.
Ten vendored Rust leaves were copied back with the verified remote formatting;
their upstream revisions and rotation/resource behavior are unchanged.
The original Rust and ARM logs retain their original execution identities;
they are reused, rather than reported as a second execution. The operator's
jobs-only Cargo configuration is excluded; remote Cargo used one job.

The first integrated launcher run failed with 18 failures and four errors across
15 unique cases. It is retained as failed evidence. Its development ZIP is not
an accepted correction package. The corrected 30-case target, covering those
15 cases and the new theme/display cases, passed in 107.972 seconds. All 183
corrected launcher cases then passed in 674.411 seconds. The driver naturally
exited zero after checking that the frozen launcher sources and all runtime
leaves remained unchanged; packaging was gated on those passing results.

That correction's development ZIP has exactly the expected 35 members, passed CRC,
755/644 mode, and ELF/launcher identity checks, and contains no runtime data.
Its SHA-256 is
`058f36973794ae94fa81b6deaf4c011190b8277f07e9a9827cd9be4e04930ce9`;
the production launcher SHA-256 is
`7579ac370a1e564e2f8a477eab5a0c8cfb973fcb9d15f9fdd3fc6c2a5051a4cb`.
The [development ZIP](../dist/native-fixes-20261109/weread-kindle-0.2.0-native-fixes.zip)
and sanitized remote receipts are retained locally under
`dist/native-fixes-20261109/`.

## Native frame-guard v7 software candidate

The later visual-handoff candidate retains the verified Rust runtime source
and reuses the static ARM ELF identified above. It adds the independently owned
native frame guard and its matching launcher; it does not report a second Rust
test execution or ARM application build.

On `test-env`, all 195 launcher cases passed in 754.577 seconds. All 30
native-helper, pixel, and lifecycle-watch cases and four packaging-contract
cases also passed: 229 passing cases in total. Controlled X11 fixtures include exact
StaticGray pixel mapping and helper lifetime failures. These are software
results, not physical presentation acceptance.

The archive `weread-kindle-0.2.0-native-frame-v7-dev.zip` contains exactly the
expected 36 members and passed CRC, 755/644 mode, and executable-identity checks.
It contains no runtime state or private data. Its frozen artifact identities
are:

| Artifact | SHA-256 |
| --- | --- |
| Development ZIP | `057306376adabec34dea2e02dbf82a80e560f9d08d6506283d34a64b8a6af403` |
| Reused static ARM application | `8010a21786b0b68dce67baeb00452e85f217daaef08293d6c25959d9f81fbc7b` |
| Native frame guard | `a235b41c80a21a35c99f8798f366a4f2e921b971a606c83f3f8d69d576676806` |
| Production launcher | `d890fbfbe0c90c729f1bce95b7d77f46f1a683779eb015ccf947db2308128d84` |

The frozen manifest and execution receipts are retained under
`test-env:/root/weread-physical-native-fixes-20261109-r3/tools/` as
`frame-guard-v7-verification-manifest.json`,
`frame-guard-launcher-v7-full-run.log`,
`frame-guard-v7-native-final-run.log`, and
`frame-guard-v7-archive-verification.json`.

These v7 software results did not close the physical visual-handoff gate.
The R6 partial trial below retains its own incomplete acceptance scope; the
later v8/R9 observations are recorded separately. The R3 visual failures remain
recorded, and the isolated candidate did not replace the earlier normal-mode
private trial.

## Native frame-guard v8 software candidate

The v8 launcher reduces repeated external `cat` and `sed` work in the native
handoff's identity checks. Each check still reads current records. Process
birth, parent, state, group, and session come from one fresh `/proc` stat
record; private protocol records require one complete line and EOF from the
same open file. Every existing power, epoch, ownership, SYSTEM readback,
persistence, renderer, and commit checkpoint remains in place. The Rust
writer's newline-terminated atomic records are unchanged.

The complete 201-case launcher suite passed on `test-env` in 741.769 seconds,
including the six new identity/record/phase regressions. All four packaging
contract cases also passed. The archive's exact 36-member set, every CRC,
0755/0644 mode, and expected file digest were verified with no extra or private
input. These are new launcher and package executions. The main ELF, C helper,
and the 30 native/pixel/watch results above are reused evidence; no Rust or C
rebuild or repeated runtime test execution is claimed.

| Artifact | SHA-256 |
| --- | --- |
| v8 development ZIP | `eac8a0f6a2cdcf9b4ae817bbd19aee96c005bb16da7f014cb47f01ae2f88ace5` |
| Production launcher | `4192f7d45e4566612284ddab4b15f3e15285bcaac56fe45a31a15b64c3937dfa` |
| Launcher test source | `282e08e9113e5e6f8c601c7642641190c88ccdcaaca14e8ef5e4ca61e35c9bc0` |

The immutable source and software manifest is
`test-env:/root/weread-native-frame-v8-src/frozen/v8-verification-manifest.json`.
Its referenced full launcher log, package log, and archive inventory preserve
the individual execution and artifact identities.

One equivalent two-way theme fixture, including startup and cleanup, reduced
observed `cat`/`sed` invocations from 2024 to 98 (about 95.2%). This measures
external command count, not Kindle latency. The two ptrace-based timing
attempts were invalidated because tracing introduced process state `t`; they
provide no performance or product result. Software verification alone did not
close the v8 physical visual or responsiveness gates. The R9 trial below closes
only the scoped visual gate; the remaining interaction delay stays open.

## Native frame-guard v9 software candidate

The v9 change is confined to the launcher's first reap wait: ten 10 ms checks
consume one 100 ms tick, followed by nine 100 ms checks. The nominal one-second
budget and fresh PID, birth, PPid, PGRP, SID, and `wait` gates remain unchanged.
The launcher test source remains the v8 source identified above.

On `test-env`, nine targeted cases passed, the complete 201-case launcher suite
passed in 741.963 seconds, and all four packaging-contract cases passed. The
archive passed its exact 36-member set, CRC, 0755/0644 mode, and all expected
file-digest checks. The unchanged main application, native helper, Rust and C
sources, and 30 native/pixel/watch results are reused evidence; those builds
and tests were not repeated for this launcher-only change.

| Artifact | SHA-256 |
| --- | --- |
| v9 development ZIP | `74a92c47350cb714b2508cec80503c87a8493134182b4b8ab02d787705a63ebe` |
| Production launcher | `ff3b2951a66acea44faf98406542bdf796ab013e7fd7de25bb72a5697de73e43` |
| Unchanged launcher test source | `282e08e9113e5e6f8c601c7642641190c88ccdcaaca14e8ef5e4ca61e35c9bc0` |

The archive is retained as
`test-env:/root/weread-native-frame-v9-src/weread-kindle-0.2.0-native-frame-v9-dev.zip`.
Its frozen manifest is
`test-env:/root/weread-native-frame-v9-src/frozen/v9-verification-manifest.json`.
The v9 software result did not establish physical speed or presentation. Its
later R10 manual window below still showed an approximately five-second delay;
the v8/R9 result is not inherited as v9 acceptance.

## Native frame-guard v10 software candidate

The v10 production change adds a three-line native-ELF admission path using a
shell built-in same-inode comparison for the current executable. The existing
argument-vector fallback remains available. All fresh process identity,
power, acknowledgment, and `wc` checks remain unchanged. No Rust or C source
changed: the `8010a217` main ELF, `a235b41c` native helper, and 30 native/pixel/
watch results remain reused evidence, without another build or runtime suite.

The initial complete launcher run executed 206 cases: 205 passed and one new
shebang fixture failed because `/proc` command-line data was momentarily empty
before it reached the matcher. Only that fixture was corrected to publish
readiness and wait for its original PID to become ready; all five existing
assertions were retained. The other 205 methods and code outside the changed
fixture retained identical AST and source bytes. The initial `97f`-prefix
test source and failing receipt remain retained evidence.

All nine identity cases passed in isolation after the fixture correction.
The earlier nine identity and four native-group targeted cases also passed.
All four packaging-contract cases and the exact 36-member archive, CRC,
0755/0644 mode, and expected digest checks passed. The final software gate
combines the unchanged 205 successes with the corrected fixture checks; it is
not reported as one complete 206-case passing execution.

| Artifact | SHA-256 |
| --- | --- |
| v10 development ZIP | `e31dca0ebd3643521ac2b273fcc75a54bf9c51d1a46f5e6d0340347802695a7c` |
| Production launcher | `15aab68c25a6d9b31c2a1020f62001ab4e86827e4dcfd2926bbaf17ef6f3c9e9` |
| Final launcher test source | `7c9a9714e07ddc54258edb4ad89cbd1957e609dc22684dd5d5f1921426bb2433` |

The frozen manifest is
`test-env:/root/weread-native-frame-v10-src/frozen/v10-verification-manifest.json`.
The isolated v10 manual acceptance below closes its scoped visual, sleep, and
exit gates. It does not establish a speed improvement or a formal-trial update.

## Physical observations and remaining gates

The identified device is a first-generation Kindle Scribe (2022), firmware
5.19.5. The preceding isolated resource-review ELF `381a4b25` displayed startup
feedback, Library, Reader text and page turns, continuous selection, and active
Up-Down-Up rotation correctly according to the user. An inverted right-side
tap advanced the recorded position while the same process remained running.
Three sampled active stages retained 249 descriptors and four threads. These
are scoped observations, not a general absence-of-leaks finding.

A five-second stock power-button sleep stopped the original application
process, but wake did not retain that process. Native display diagnostics
confirmed that bare `xrefresh` failed with an absent `DISPLAY`, while
`DISPLAY=:0.0 xrefresh` succeeded. The unguarded display-release call propagated
that failure through `set -e` and exited the launcher. The default-display
correction addresses this observed failure; at that stage it had not yet
passed a new physical sleep cycle.

After the corrected full run, a fresh exclusive device lease was acquired.
The SSH attempt failed with `No route to host` before connecting, so the
baseline script did not execute and no candidate was deployed or started.
The unused lease was exactly released and the global lease directory was
independently confirmed absent. No new native appearance or touch result is
claimed for that attempt.

The later R3 isolated trial used the same ELF and launcher. Its first window
confirmed startup feedback, Library, reading touch, and a TOC framebuffer
capture with aligned download icons and chapter-status text. The initial native
portrait calibration was Down with raw GETVAR rotation 1. The framebuffer was
1860 x 2480, 8bpp, stride 1872, with zero visible offsets and scale 2. The raw
rotation code was treated relative to the observed native origin.

That first window ended when the operator's 900-second guard requested one
owned TERM; launcher exit was 143 with successful native restoration. The
subsequent user dark-mode report could not be reconciled with the expired
process, so it is not accepted as a system-theme result. Only the private
operator guard was extended to 3600 seconds for the second window; the product
ELF, launcher, and application arguments remained unchanged.

The second window confirmed the live system transaction: generation 1 retained
the same process and birth, theme sequence 1 returned `ok:Y8INV`, the matching
renderer acknowledgment was present, the request was removed, the persistent
native preference was `Y8INV`, and the application's dark preference was true.
The private framebuffer capture, decoded using the confirmed native inversion,
showed black background, white text, and removal of the old hint.
However, the user also observed the native Library for several seconds before
Reader returned. That visible handoff does not meet the required reading
experience, despite the successful resident system change.

A subsequent five-second stock power-button cycle retained the same process,
matching sleep/wake cycle 1, the same chapter/anchor, and dark mode. After the
completed wake acknowledgment, the process had four threads and 249 descriptors.
The user confirmed the original position, dark appearance, and working page
turns, but wake also showed native Library before Reader returned. This is
same-process recovery evidence with a failed visual handoff gate, rather than
acceptance of uninterrupted wake presentation.

Voluntary UI exit then finished with launcher exit zero. Native mode and its
persistent preference remained `Y8INV`; the original window manager was running,
the status bar was restored, no test GUI or launcher lock remained, and the user
confirmed native dark appearance and touch. The exact device lease was released
and its directory independently confirmed absent before BiliComics resumed.

At the end of R3, elimination of native-Library exposure during both theme
change and wake still required a new artifact-specific physical check.
Factory-source review confirms that the native `epdcMode`
setter explicitly requests root repaint and a full-screen refresh trigger;
merely retaining the application process does not suppress that drawing.
The earlier formal daily-trial installation has not been replaced by these
isolated runs. USB insertion remains deferred at the user's request. Manual
rotation is disabled, and no four-way automatic rotation or other model/firmware
support is claimed.

## R6 partial frame-guard trial

The isolated v7 candidate completed its supervised 15-second display window.
The operator requested one owned TERM, the launcher returned 143, the supervisor
finished, both display locks were absent, and the original native window
manager, status bar, active application, and `Y8INV` mode were restored. This is
managed-stop evidence, not a voluntary UI-exit result.

In the subsequent manual window, the user confirmed startup feedback, the
demonstration Library, Reader text, and working touch. The native guard was
viewable at 1860 x 2480 and depth 8. The first night-to-day transaction retained
the original application and guard identities, advanced the guard sequence to
2 for application generation 1, and completed theme sequence 1 with matching
`ok:Y8` result and renderer acknowledgment. The request was removed, native
persistence was `Y8`, and the application preference was false.

The user observed an in-place switch without native Library exposure, but
reported a four-to-five-second delay. That delay remains an open interaction
problem. The reverse transition, sleep/wake presentation, orientation, and
voluntary exit still require this candidate's own observations. A bounded
passive transaction sampler expired after 300 seconds without observing a new
request; it collected zero transaction samples and verified the original
identity again at completion. It supplies no handoff timing result. Any later
passive measurement must account for its own polling overhead.

The manual window ended with another owned managed TERM and launcher exit 143.
The supervisor finished; the original launcher, application, guard, lifecycle
watcher, and passive sampler had ended, and both display locks were absent.
An initial private cleanup assertion incorrectly required the numeric XID to
be absent: the X server had already reused that resource ID for its restarted
`JunoStatusBarDriver` title bar. The saved and current window title, class, and
1860 x 101 geometry distinguish it from the original full-screen guard. No
native window was destroyed to satisfy that assertion.

The original native window manager was running with restored status bar,
`KPPMainApp` foreground, active power state, and the user's committed `Y8`
setting. These machine observations do not constitute a new manual touch
confirmation. The exact R6 lease was released at 14:35:14 UTC, followed by an
independent absent-directory check, and BiliComics was notified. No reverse
theme, sleep/wake, orientation, or voluntary-exit acceptance is claimed for
this window; the visual and responsiveness gates remain open.

## R7 v8 bounded profiling trial

This separate automatic profiling run used the v8 candidate and a private
90-second application timer. It naturally exited zero when that timer expired;
this is not a voluntary UI-exit result. Three system-theme commits retained
the same application identity and passed their protocol checks.

The measured request-to-commit intervals were 6.14-6.28, 6.56-6.72, and
6.03-6.16 seconds. Sampler cadence had a 0.14-second median and a
0.11-0.22-second range; this was not strict 100 ms sampling. Grouped phase
observations were:

| Observed interval | Three durations (seconds) |
| --- | --- |
| Portrait group `go` to `done` | 0.93, 1.09, 0.95 |
| Theme result to renderer-ready acknowledgment | 2.32, 2.34, 2.34 |
| Last power-query completion before alignment to alignment-ready | 0.27, 0.13, 0.14 |

Within the result-to-renderer-ready interval, left-endpoint estimates placed
approximately 76-82% of the observed time in process state D. These observations
do not attribute that interval to framebuffer waiting or `fsync`. The short
tail before alignment-ready does not support a claim that native frame-guard
uploads caused most of the multi-second delay. Apart from its `usleep` helper,
the polling loop used built-in record and `/proc` reads rather than `tr` or
`grep`. A separate approximately 36 ms overhead observation is neither CPU
time nor a quantity that can simply be subtracted from the total duration.

The operator missed the 90-second window. A later stock-power start was refused
by the identity gate after the process ended, so this run contains no sleep
test. Run-owned cleanup restored native presentation with the committed
`Y8INV` setting. The exact
lease was released at 15:09:18 UTC and independently confirmed absent.

## R8 unused device window

R8 prepared a lighter RAM sampler and coordinator/wait-channel collection on
the host only. The device SSH connection timed out before installation or
startup; neither action executed. The unused lease was exactly released at
15:28:19 UTC and independently confirmed absent. R8 supplies no device or
performance evidence.

## R9 v8 physical visual-handoff acceptance

R9 used the ordinary R7 isolated home with private `--demo --offline` arguments
and the v8 archive and production launcher identified above. The main
application and native helper retained their `8010a217` and `a235b41c`
identities. The user confirmed the Library, Reader, dark appearance, and touch.

Both system-theme directions stayed in the application without exposing
native Library. The user still observed a four-to-five-second interaction
delay, so responsiveness remains **OPEN**. Machine observations retained the
same application and helper birth identities: theme sequence 1 completed
`ok:Y8`, sequence 2 completed `ok:Y8INV`, each had its matching renderer-ready
acknowledgment and no remaining request, and the guard sequence reached 4.

The operator then performed a real five-second stock power-button cycle,
observing active to screensaver to active and matched managed cycle 1. The
same application retained its reading position and dark mode. The user
confirmed working touch and page turns, with no native Library or startup
page. The user subsequently held the device upside down with USB at the top,
confirmed automatic rotation and a right-side page turn, and manually performed
another five-second sleep/wake. That observed inverted cycle also retained the
page, dark appearance, and touch without native Library or a startup page.

The final live sample retained application generation 1 and the original
application/helper identities. All four sleep/wake request and acknowledgment
records matched the latest managed cycle 3, and guard sequence was 10. The
intervening cycle 2 was not independently observed by the user and is not
counted as an additional accepted physical case.

The user returned USB to the bottom and voluntarily exited through the UI.
The launcher returned zero without a supervised TERM. All original launcher,
supervisor, application, runner, helper, watcher, and power-sampler identities
had ended, completion markers were present, and both display locks were
absent. The former guard XID had been reused by a native `JunoStatusBar`
window, distinguished by its native identity rather than treated as a leaked
guard or destroyed.

The original window manager birth identity was running, the status bar was
restored, and native mode remained `Y8INV`. The user confirmed a dark native
Library with working touch. The final read-only power observation was already
screensaver, not active. The exact R9 lease was released at 15:39:47 UTC and
its directory independently confirmed absent.

This closes the v8 visual-handoff gate only for the identified Scribe 2022,
firmware 5.19.5, and accepted automatic 0/180-degree path. It does not close
the remaining theme-change delay, establish other device support, or accept
v9 presentation or speed. USB remains deferred. The earlier `2fd3/799d`
normal-mode private trial has not been replaced by these isolated runs.

## R10 v9 manual observations

The first R10 manual window used v9. The user still observed approximately
five seconds per theme change and questioned whether illumination changed.
No controlled day-mode transition occurred during the separate 300-second
light trace: it timed out with only a `Y8INV` baseline, native brightness 8,
warmth 12, and raw light values 666/616. Constant minimum and maximum values
within that baseline do not show what a theme switch does to illumination.
Two other uncontrolled `Y8INV` snapshots showed brightness 12 then 8 with
warmth 12; their difference has no established cause.

The wait-channel investigation collected 450 samples, including 39 state-D
samples and 38 syscall observations, but syscall numbers were 0 or unknown
and wait channels were unknown. It supplies no valid `ioctl` or `fsync`
attribution. This manual window ended with an owned TERM and launcher exit
143, followed by complete cleanup and native `Y8INV` restoration. It is not a
voluntary UI-exit result.

## R11 automatic v10 observations under the R10 lease

R11 shared the still-held R10 lease. It used the final `15aab68c` production
source with private demo/offline arguments, scripted actions, a 90-second
application timer, and a 180-second operator guard. Its source was provisional
when this run was prepared; the combined software gate above was completed
afterward. Two valid system-theme commits retained the same application.
Observed transaction-duration intervals were 5,890-6,100 ms for sequence 1 and
5,890-6,130 ms for sequence 2, without a demonstrated speed improvement.

Read-only driver observations identified `fp9966-bl0` and `fp9966-bl1` and
confirmed that their actual values changed with system mode:

| Observed mode or phase | Channel 0 actual value | Channel 1 actual value |
| --- | --- | --- |
| Initial night baseline | 666 | 616 |
| During the day transition | Reached 0, then increased | Reached 0 |
| Stable day observation | 1,213 | 1,163 |
| Restored night observation | 666 | 616 |

The driver records showed `bl_power=0`. The channel numbers are not assigned
warm/cool meanings. Endpoint power-daemon reads both reported brightness 8
and warmth 12; there was no day-endpoint LIPC read, so day brightness is not
claimed to be 12. The observer completed its five-second post-second-commit
light-value observation window; the recorded values were also stable between
the two transitions.

The stock-power coordinator refused with `insufficient-private-timer-window`
and sent no power request. This run therefore supplies no sleep/wake result.
The application timer exited zero, all original observers and controllers
ended, and native `Y8INV`, locks, and owned windows were restored or cleared.
The overall operator completion returned 1 because the controller gate was
invalid; that failed operator gate remains recorded despite successful cleanup.

## Historical native illumination policy for the SYSTEM path

After reviewing the observed SYSTEM behavior, the user chose to retain the Kindle's
native day/night illumination presets. The application does not own two
illumination profiles or snapshot and restore light values around mode changes.
Automatic brightness remains outside product and acceptance scope. Manual
brightness and device-supported warmth remain available controls.

The later explicit KOReader-alignment decision supersedes this system-triggered
policy for ordinary application toggles. The new candidate does not change the
native system setting or select its illumination presets. Manual controls remain
unchanged, with no app-mode-driven profile copying, light snapshot/restore, or
ambient automatic-brightness action. No zero-transient claim is made for the
unverified direct framebuffer ioctl.

Source review found no light setter in the normal night-mode transaction;
light setters remain attached to manual UI controls. The existing native
validated 0-24 light-setter range is retained. Read-only observations preserve
the original unsigned values without clamping them.

## R10 v10 manual reopen and scoped acceptance

The user requested another launch, and the private v10 demo variant identified
by digest prefix `d31fd` was activated. The initial private activation script
used the shorthand active-application identity `KPPMainApp` and was safely
refused. The operator corrected that script to the actual
`com.lab126.KPPMainApp` identity. Product source and data were unchanged, and
the failed attempt remains in the receipts.

In `r10-manual-restart`, the user confirmed both night-mode directions and a
manual five-second sleep/wake without native Library or a startup page. Dark
appearance, reading position, and page turns remained correct. Generation 1
retained application PID 9585 and helper PID 9532 with their original births.
Theme sequence 1 completed `ok:Y8` and sequence 2 completed `ok:Y8INV`, each
with the matching renderer acknowledgment and no remaining request. Guard
sequence reached 6, and all four managed sleep/wake records matched cycle 1.

The user then voluntarily exited through the UI and confirmed normal dark
native presentation and touch. Launcher exit was zero without TERM. All
original process-family identities had ended, both display locks were absent,
and the actual guard XID no longer existed. The original window manager was
running in state S, the status bar was running as PID 12360,
`com.lab126.KPPMainApp` was foreground, native mode was `Y8INV`, and power
state was active.

The R10 private receipts include
`evidence/private/r10-v10-manual-after-user-wake.stdout`,
`evidence/private/r10-v10-manual-finished.stdout`, and
`evidence/private/r10-v10-manual-guard-gone.stdout`.
At this manual-reopen checkpoint, the R10 lease remained held and the earlier
`2fd3/799d` formal private trial had not been replaced. The subsequent matching
normal-mode deployment below records that replacement and the final lease release.

The v10 visual, sleep, and UI-exit gates are closed only for this Scribe 2022
and firmware 5.19.5. The separate inverted-holding observations remain v8/R9
evidence. The approximately five-second response remains a known **OPEN**
issue, without a claimed optimization. Overall product acceptance, USB,
other models, and other firmware combinations are unchanged.

## Matching v10 normal-mode deployment

The existing normal entry was subsequently updated on the same Scribe 2022 /
5.19.5 device. Read-only preflight independently matched the old `2fd3d6ab`
application and `799d5bba` launcher. The approved public inventory contained
18 existing files and 18 absent targets. Device and private host backups of
the existing public files passed the complete expected hash and size checks.
A separate byte-for-byte snapshot of the known `library.sqlite` file was
verified unchanged at its source and copied to private host storage; WAL and
SHM files were absent. No account or book rows were printed or published.

The update staged the 35 public App files and production document launcher,
then replaced files by same-directory rename with the document entry last.
It was a per-file transaction, not a globally atomic directory replacement.
The outer transport reached its 180-second timeout. Subsequent read-only
inspection found no remaining commit process and a complete commit journal,
including the final all-36-files marker. The commit was not blindly retried or
rolled back. An independent readback then verified every public hash and size
and all three executable targets:

| Installed component | SHA-256 |
| --- | --- |
| Normal document launcher | `15aab68c25a6d9b31c2a1020f62001ab4e86827e4dcfd2926bbaf17ef6f3c9e9` |
| Static ARM application | `8010a21786b0b68dce67baeb00452e85f217daaef08293d6c25959d9f81fbc7b` |
| Native frame guard | `a235b41c80a21a35c99f8798f366a4f2e921b971a606c83f3f8d69d576676806` |

The device's FSP filesystem reported mode 777 for all 36 files. This is recorded
separately from the verified ZIP's 0755/0644 attributes; no exact POSIX mode
pass is claimed for the mounted device storage. Required executability was
confirmed. Public replacement was restricted to the approved App and document
paths; account state, books, caches, progress, and private configuration remained
in the existing normal data directory.

The user opened `WeRead for Kindle` from native Library and confirmed the
original Library and retained books. The live process arguments were exactly
`--kindle --device scribe --data-dir /mnt/us/weread/data`, without demo, offline,
scripted actions, or a product exit timer. The production helper and original
process family were bound while running. The user then exited through the UI
and confirmed normal dark native presentation and touch. Independent cleanup
confirmed every original master, runner, application, helper, and watcher had
ended, both display locks were absent, and the actual guard XID no longer
existed. Native window manager and status bar were running with `KPPMainApp`,
active power, and `Y8INV`. No numeric launcher exit status was observed for
this user-started normal launch; the manual exit and cleanup observations are
not restamped as a supervised exit-code result.

Receipts remain private under
`test-env:/root/weread-manual-20261109-r10/evidence/private/`, including
`r10-formal-v10-readback36.stdout`, `r10-formal-normal-live.stdout`, and
`r10-formal-normal-user-gone.stdout`. The R10 lease was exactly released at
2026-11-09 18:09:47 UTC, its absence was independently checked, and BiliComics
was notified only after all device transports closed. The normal entry is
available for private daily trial. This deployment does not close the known
approximately five-second theme response or expand the device-support matrix.
