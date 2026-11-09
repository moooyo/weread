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
