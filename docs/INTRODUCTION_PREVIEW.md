# Scribe introduction preview

Accepted: 2026-11-14 (Asia/Shanghai).

Status: implementation is source-frozen and the focused host checks passed.
The corrected candidate's ARM/package, current short/long/empty Scribe native
scope, raw-capture package, ordinary UI restoration, and task cleanup passed.
This authorized preview iteration is complete within those bounds. Paperwhite
retains host-only coverage, and physical-device acceptance is not claimed.
Initial-candidate failures and earlier Detail hub results keep their own scope.

## Behavior

The fixed Scribe book-information column shows an introduction below the
unchanged reading/download actions. Short text uses its natural height; long
text wraps within at most six lines and is elided. The preview does not move
the existing actions. View full introduction is a left-aligned text link with
a chevron, an underline, and a 48-logical-pixel touch area. It selects the
existing right Introduction tab, where the full text scrolls vertically. The
link is not a centered ActionButton.

Repeated empty paragraphs, CRLF boundaries, and Unicode line/paragraph
separators are compacted only in the transient preview. Full Introduction
retains its raw text, and stored introductions are not rewritten. The full
tab's Column/Text use natural content height to create a scrollable extent.

An empty introduction displays No introduction and has no full-text action.
A rejected preview displays Introduction unavailable without replacing the
book context or altering stored data. Switching books replaces the preview;
Paperwhite retains its stacked layout and does not request the sidebar prefix.

## Data boundary

The sidebar reads at most 160 Unicode scalar values plus one lookahead scalar,
with at most 644 encoded bytes accessed through read-only incremental reads.
UTF-8, UTF-16LE, and UTF-16BE are supported within that fixed read boundary.
It does not load the complete introduction or publisher/ISBN metadata just to
populate a preview. Full Introduction payload admission remains on demand.
SQLite incremental reads use rusqlite 0.37's safe blob API. Its existing
dependency declaration adds `blob` and `limits` features; the version and
lockfile do not change. The integration uses the safe API's `main` database
name and handles the Result returned by `set_limit`.

The database layout, private introductions, annotations, and reading anchors
remain unchanged. No migration or historical-payload alias is introduced.

## Frozen source and environment

The current source is based on main commit
`ab6664992500108bc6c67e7369e557ead4244dd8`. Its Git archive contains 614 files;
the two new test files bring the frozen snapshot to 616 files. The current
corrected source manifest is
`9b1ffbe0ea8596d4d62d94206b82b7cb1a15f558ae6fdcfb6ea366bdddc18228`.
Task-owned sources and evidence are under
`test-env:/root/weread-intro-implementation-20261114-r1/`.

The 616-file manifest binds the verification snapshot. Later documentation-only
closeout is outside that snapshot and does not require recompilation. Initial
installation identities retained a stale `source_commit_base=6142bc16` tool
label; the authoritative baseline is `ab666499` with the candidate's manifest.
Original identities and PNGs are preserved. The current installer reads the
baseline from the candidate rather than retaining that old label.
`provenance-correction.json` records that bookkeeping clarification and confirms
that no original identity, PNG, or sidecar was rewritten. It is not a new
application run or device acceptance result.

The initial four-lower cache mount failed before any test executed. The worker
then used the previously proved slider/rectangular/review three-lower cache
arrangement, with writes confined to this task's upper/work layers. This was
an environment repair, not a passing product check or a zero-test success.
Earlier integration compile failures also remain failed attempts; correcting
the rusqlite API calls does not relabel those attempts as passing builds or
executed tests. The completed receipts below belong to corrected source
`9b1ffbe0ea85`.

## Retained initial-candidate failure

Initial source
`0829cdd101065a6c1eb067538b737bd981bda6f28d56db1c7dd1dd2f2ebca2dd`
and ARM `123258e7a7d8` passed their initial host/build and bounded short-text
observations. Those receipts are preserved under
`checkpoints/initial-123258e7a7d8/`, with their own artifact identities.

`public-intro/intro-observation-123258e7-report.json` remains
`requires_ui_fix`. Five original PNGs showed a long preview with four ink
lines separated by blank lines and no visible ellipsis. The full tab opened,
but its drag did not move the viewport and the END marker was not visible.
An attempted-end filename does not establish a successful END observation.

At that checkpoint only the ordinary runtime `report.next_check.v1` setting
changed; positions, preferences, and chapter bodies/status/ownership matched.
This is not whole-database byte invariance. The original report and image
labels are not rewritten. The minimal correction compacts preview-only blank
paragraphs and gives the raw full-text Column/Text natural height; it does not
change stored text or infer a new hardware result.

## Verification

The `introduction_preview_tests` filter passed ten tests on the current source:
eight Store cases and two real App parent scenarios for Scribe and Paperwhite.
The Store scope covers bounded encoding/preview reads and invalid data. The
App scope covers the bounded sidebar, inactive full-payload counters,
full-text shortcut, stale-state replacement, field-local error degradation,
preview-only empty-paragraph/CRLF/Unicode-separator compaction, raw full-text
and database preservation, and the unchanged Paperwhite path. The ten-case receipt is
`build/artifacts/unit-lib-2db2d3ace5-receipt.json`.

Two existing exact introduction regressions also passed independently, one
case each: visible-tab payload projection and quota/private-data preservation.
Their receipts are `unit-lib-571c270d3e-receipt.json` and
`unit-lib-9443b4afeb-receipt.json` in the same artifacts directory. The current
host total is twelve within those separate scopes, all rerun on source
`9b1ffbe0ea85`; isolated-child summaries are not counted again. This is not a
complete application-suite result.

| Current host run | Passed | Duration | Log under `build/logs/` |
| --- | --- | --- | --- |
| `introduction_preview_tests` | 10: eight Store/two App | 56.16 seconds | `unit-lib-2db2d3ace5-20261114T151955Z.log` |
| Existing exact visible-tab regression | 1 | 59.74 seconds | `unit-lib-571c270d3e-20261114T152602Z.log` |
| Existing exact quota/private-data regression | 1 | 54.42 seconds | `unit-lib-9443b4afeb-20261114T152802Z.log` |

| Area | Status |
| --- | --- |
| Bounded UTF-8/UTF-16 preview and invalid-prefix handling | Passed: eight Store tests |
| Scribe projection, full-text shortcut, error and stale-state handling | Passed within the two App parent scenarios |
| Paperwhite inactive-payload and stacked-path behavior | Passed within the App scenarios; no new PW6 firmware claim |
| Existing exact introduction regressions | Passed: two separate one-case receipts |
| ARM and development package | Passed: exit 0 on the corrected source, 194.94 seconds |
| Current short native preview/full tab/return | Passed: three new original PNGs on the corrected ARM |
| Current long and empty native cases | Passed: five original PNGs, 12 visual/pixel and 12 itemized data checks |
| Current ordinary UI exit/restoration | Passed: normal confirmation, lock removed, status bar/window manager restored |
| Task cleanup and raw-capture package | Passed: task-owned Scribe QEMU closed and eight-source-PNG archive CRC passed |
| Physical-device acceptance | Not performed |

## Built artifacts and bounded native scope

`build/artifacts/arm-build-receipt.json` records build/package exit 0 in
194.943097 seconds, with log `build/logs/arm-20261114T152114Z.log`. Its
candidate receipt matches the corrected source manifest
`9b1ffbe0ea8596d4d62d94206b82b7cb1a15f558ae6fdcfb6ea366bdddc18228`.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM application | 84,746,296 | `64e66a8a243c66de5ab291cf4661168c27094afe6586978b8f627f78219eb5d7` |
| Native frame guard | 25,792 | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Development ZIP | 68,260,856 | `18a646b2f1e7532101192f4dda6663d291c8aa77d353b79278a9f86e50e116fa` |

## Completed current native scope

`public-intro/final/intro-final-64e66a8a-report.json` has status
`passed_scribe_intro_scope`, SHA-256
`2a223dfacd13f25b97ff7a6ca0d35bb174f4c8f000ac606812246d79a222f704`.
It accepts five original PNG/sidecar pairs from the independent public long
and empty variants. All twelve visual/pixel checks and twelve itemized data
checks passed for the exact current ARM/helper and Scribe 2022/DVT firmware
5.20.1 guest.

The long preview has five ink lines and a visible ellipsis. Its full-text
entry selects the right Introduction tab; one real vertical drag moves that
viewport and reveals `END-INTRO`. Cover, reading/download actions, and the
preview remain pixel-identical on the left while the right text scrolls.
Returning to Contents retains the original preview. The empty variant shows
No introduction without a full-text entry.

Both fixture introductions match their expected raw stored content after
native use. Positions, preferences, and every chapter field are unchanged.
Seed-stage changes are introduction-only. The ordinary runtime
`report.next_check.v1` scheduling timestamp changes during native execution;
seed-only invariance must not be extended to a zero-write or byte-identical
whole-database claim.

`final-short-report.json` separately records three new current-ARM PNGs:
natural three-line preview, full entry to the right tab, and Contents return.
Its visual observations and four pixel/state booleans are true. No invented
aggregate check count or callback count is assigned to that record.

| Current short capture | SHA-256 |
| --- | --- |
| Preview | `44b5905317554852d40c8656fca35aea688c1238677f330831d8ba86f9f79298` |
| Full Introduction | `bb5ef5e2df5b51317896ad013a0e09363eb8f231484e961265adbd5a9c6f486b` |
| Contents return | `44b5905317554852d40c8656fca35aea688c1238677f330831d8ba86f9f79298` |

The short left pane and existing actions are unchanged across those states;
the Contents-return PNG exactly matches the new preview PNG. These are new
`64e66a8a243c` observations, independent of the preserved initial `123258`
short result.

## Restoration, cleanup, and original capture archive

`restoration/scribe/final-ui-exit-64e/receipt.json` records ordinary UI
confirmation exit, an absent launcher lock, running status bar, and a
non-stopped window manager. The collector did not kill the application.
No numeric application/launcher exit code is inferred from this UI receipt.

`cleanup-receipt.json` passed after matching the task disk, PID, executable,
machine, and QMP identity. Only this task's Scribe QEMU was closed, and ports
4372, 4373, and 5927 were released. Original source and capture artifacts were
preserved. No Paperwhite was booted for this preview task and no physical
device was accessed.

The unified `scribe-introduction-preview-64e66a8a-raw.zip` preserves all eight
current original PNGs: the three short observations and five public long/empty
observations. Its size is 1,262,519 bytes, SHA-256
`35eeb1e9931da5f5dbebd734a5c69d5c8dd84f57b212f49c5cec0256649f1394`.
`raw-capture-package-receipt.json` records passed archive CRC and the current
ARM/source identities. The earlier independent five-image archive remains
separate; no original PNG or failure label was retouched.

Application verification runs only on `test-env`. Native work for this request
targets Scribe only; Paperwhite retains host-only unchanged-path coverage.
Firmware-emulator screenshots are separate from physical Kindle observations.
The prior PW6 font-startup CRC failures retain their recorded scope and are
not investigated by this task. No cause or resolution is claimed.
