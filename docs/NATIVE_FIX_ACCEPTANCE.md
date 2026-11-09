# Native reading corrections and acceptance

Recorded: 2026-11-09 (Asia/Shanghai).

This record covers the requested TOC status alignment, immediate system night
mode, the observed display-release failure during sleep, and the bounded
[KOReader rotation comparison](ROTATION_ALIGNMENT.md). It does not reopen the
completed finite resource closeout or certify other devices.

## Implemented behavior

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

## Remote verification

All execution in this correction was on `test-env`. Local operations were
source review, file transfer, documentation editing, and Git operations.

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

The final development ZIP has exactly the expected 35 members, passed CRC,
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

The R6 physical visual-handoff gate remains **OPEN**. In particular, these
results do not prove that native Library is absent during a system-theme
change or wake. The R3 visual failures below remain recorded, and the isolated
candidate has not replaced the earlier normal-mode private trial.

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
provide no performance or product result. The v8 physical visual and
responsiveness gates remain **OPEN**.

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
correction addresses this observed failure; it has not yet passed a new
physical sleep cycle.

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

The current correction therefore still requires elimination of native-Library
exposure during both theme change and wake, followed by a new artifact-specific
physical check. Factory-source review confirms that the native `epdcMode`
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
