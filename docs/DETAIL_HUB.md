# Reader detail hub

Accepted: 2026-11-14 (Asia/Shanghai).

Status: source frozen; the two focused host Rust regressions passed on the
current source snapshot. The ARM/package build passed. Bounded paired main
native PNG checks passed for Scribe and PW6, and both ordinary UI restorations
passed. The focused gallery passed its DOM checks. Independent long-catalog,
populated-note, and cleanup coverage is tracked separately: Scribe's strict
public-fixture report passed, the PW6 long-catalog attempt remains
Blocked/Unverified by font-startup failures, and task cleanup passed. Core UI
delivery is complete within its recorded scope; not all native conditions
passed. No earlier candidate's results establish acceptance of this change.

## Accepted entry and navigation

The Reader's directory action uses `reader.detail("toc")` to open the existing
BookDetails view for the same book. That entry forces the contents tab,
regardless of the last Detail tab, and locates the current chapter using its
actual ordinal in the sorted catalog. A source chapter index must not be used
as a substitute for that ordinal when choosing the catalog page.

When entered from Reader, `detail.back` restores that Reader's character
anchor and its `menu`, `tool`, and `subtool` context. It must not return to
Library instead of the originating Reader. Normal navigation may finish the
old reading session and start a new one on return. The contract preserves the
character anchor and visible Reader context, not a particular paginator
allocation or reporting-session identity.

The return handling is integrated and has passed its focused host regressions.
Both recorded main Reader-menu returns passed bounded pixel comparisons.
Scribe's broader fixtures passed separately; PW6 long-catalog coverage remains
Blocked/Unverified.

## Accepted layouts

| Profile | Book context | Main content |
| --- | --- | --- |
| Scribe / KS | Fixed 280-logical-pixel left pane for cover, title, author, rating, reading progress, and book actions | Right pane with Introduction, Contents, Reviews, and My Notes tabs |
| Paperwhite | Book context remains above the main content | Existing stacked Detail arrangement adapted to the same entry/return contract |

The Scribe information pane remains fixed while the right tab content is
browsed. Page navigation and chapter-selection actions have a fixed footer.
Long chapter titles wrap within their available row width; only the bounded
application-supplied catalog page scrolls, so the final row remains reachable.
The footer must not move or disappear when wrapped rows exceed the visible
content area. Native observations are qualified by the profile-specific
evidence below; PW6 long-catalog layout is not qualified by its failed starts.

The frozen Scribe source uses a 280-logical-pixel pane with 256 logical pixels
of actual information content. The right content spans logical x=336..898.
The 52-logical-pixel Detail header spans the full width; the ordinary footer
is 48 logical pixels, including My Notes. These are logical source values;
bounded native pixel regions are checked separately below.

Detail lists use vertical-only Flickables and row TouchAreas. Fixed PageBars
handle page changes rather than horizontal row swipes, avoiding contention
with Slint's built-in diagonal gesture handling. Library's existing horizontal
swiping is unchanged. No vendor patch is introduced. My Notes now has both a
PageBar and the numeric page-jump backend. Scribe's populated-notes PageBar
coverage passed; numeric jump interaction is not inferred from that fixture.

More retains its button-anchored rectangular popup. The accepted font slider
and straight rectangular control styling remain in effect. This request reuses
WeRead's Detail and Reader use cases; it does not introduce a separate book
source, new font system, or new acquisition contract.

The AidokuForKindle chapter-hub proposal is a design reference for the shared
Detail entry and layout. Its design-only status is preserved; it is not a
runtime dependency or verification performed by WeRead.

## Frozen source and focused host evidence

The task-owned remote root is
`test-env:/root/weread-detail-hub-20261114-r1/`. The current runtime source
snapshot contains 568 files:

| Source identity | SHA-256 |
| --- | --- |
| Runtime source manifest | `799e061d4df0a0d7ca02d41d098ce5747fe8b4c5ba6ccc1a621f7d65851047c2` |
| `ui/bookstore.slint` | `cbad15c8bcc450ba1e146393f45d0c66ae78eb866b7e28b2e1b8c98dd37038e5` |
| `src/app/library.rs` | `0f17904c18a58dde875d4f43aa4cd51f1f7397521ce988f2f77ee06ab179ad94` |

`build/artifacts/unit-lib-26ca452525-receipt.json` records the current
`detail_navigation_tests` filter: two parent cases passed, zero failed or
ignored, command exit 0, in 51.99 seconds. Nested isolated-child summaries are
not counted again. This is host Rust evidence on source `799e061d4df0`; it
does not establish native UI geometry or interactions. Earlier 38-pass runs
retain their own historical scope and are not the current snapshot's count.

## Implementation and acceptance status

| Area | Current status | Acceptance scope |
| --- | --- | --- |
| Reader directory entry and return handling | Source frozen; focused host regressions passed | Same book, forced Contents tab, true catalog ordinal, character anchor, and Reader menu context |
| BookDetails layout | Paired main checks passed; Scribe stress passed; PW6 stress Blocked | Fixed Scribe sidebar/four tabs/right footer and Paperwhite stacking |
| Focused Rust regressions | Passed: two parent cases, zero failed or ignored | Current source `799e061d4df0`, entry/ordinal selection and return-context/anchor behavior |
| Remote ARM and development bundle | Passed: exit 0, 206.65 seconds | Matching source, application, helper, and bundle identities below |
| Main native snapshots and interactions | Passed: 57 bounded checks | Both ordinary main profiles, exact current identities, and recorded return/restoration; independent fixtures excluded |
| New focused gallery | Passed: nine JS/DOM/metadata checks | 13 original PNGs, exact current candidate and fragment bytes; no CSS/host-render claim |
| Scribe long-catalog stress | Passed within Scribe-only strict report | Independent current-candidate fixture, not the historical `4bc` report |
| PW6 long-catalog stress | Blocked / Unverified | Two font-startup failures; zero accepted long-catalog PNGs |
| Populated Scribe notes | Passed within the same Scribe-only strict report | Five-note quotes/thoughts, scroll, PageBar, and Reader return; no double counting |
| Task cleanup | Passed | Two task-owned QEMU processes exited, firmware restored, one canonical task tmpfs unmounted |

Verification runs on `test-env` under the task's authorization. Both prepared
guests report firmware 5.20.1. Both applications ran and exited through their
ordinary UI, with native restoration recorded. PW6 ran the same ELF from its
userstore after the tmpfs startup failures below. Paired main acceptance is
bounded to the captures and receipts described here; no physical-device result
is claimed.

## Built candidate and main native evidence

`build/artifacts/arm-build-receipt.json` records exit 0 and 206.65 seconds.
Its source identity is the frozen `799e061d4df0` snapshot above.

| Artifact | SHA-256 |
| --- | --- |
| ARM application | `f2c0f429540ba45870c4de500b97ff1dd6e7472000f039dc11643a447c99689b` |
| Native frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Development bundle | `638b178bf27cd4778897ed46c98275ccbbf39e6e7eb4ae951f550de143c14d1e` |

The read-only checker `artifacts/detail-hub-20261114/remote/verify-detail-hub.py`
wrote `native-verification/report-scribe.json`: 32 checks passed, zero failed
or pending within the requested Scribe set. Its status is `passed_partial`,
`checked_profiles` contains only `scribe`, and `final_report_complete` is
false. It neither contacts a VM nor changes application source or input PNGs.

The eight Scribe captures have matching new ARM/helper identities, immutable
PNG hashes, original-scanout sidecars, and 1860 x 2480 dimensions. The sidebar
crop `(64,208)-(600,2300)` is unchanged across Contents, Introduction, Reviews,
and Notes. Corresponding right tab underlines and content changes were
checked, with the divider near x=622/624 retained to the bottom. Ordinary
Contents and Notes footer controls are visible only in the right-side
`(672,2384)-(1796,2480)` band. This checks visible control pixels, not an
invisible element's bounds.

More's four edges at `(1156,176)-(1844,696)` and outside-dismissal return to
the same Detail image passed. ReaderMenu and its returned image match outside
the clock region. The original menu visibly reads `2 / 2; 20%`; its compared
footer pixels are unchanged after return. That is a page/progress observation,
not a model callback or reading-session count.

`restoration/scribe/final-ui-exit/receipt.json` records ordinary confirmation
exit, an absent launcher lock, a running status bar, and a non-stopped window
manager. The collector did not kill the application. No numeric launcher exit
code is inferred from that receipt.

The initial optional selection tap did not activate selection. Those captures
are explicitly excluded; identical ordinary screenshots are not selection
acceptance. Populated-note pagination/jump and long-title fixtures are outside
this bounded ordinary-state report unless separately recorded.

The completed `native-verification/report.json` passed 57 checks with zero
failed or pending checks. Its `checked_profiles` contains `scribe` and `pw6`,
and `final_report_complete` is true for the bounded main scope. The preceding
32-check Scribe partial remains historical and is not added to 57.

The PW6 six-state main set has matching f2 ARM/helper identities, immutable
PNG/sidecar hashes, and 1272 x 1696 dimensions. Its upper book-context crop
`(40,168)-(1232,576)` is fixed across Contents, Introduction, and Reviews.
Corresponding tabs and lower content change, the chapter list occupies the
full lower width, and ordinary footer controls remain in the full-width
`(40,1600)-(1232,1696)` band. It does not retain Scribe's full-height divider.

PW6 More's edges were checked at `(728,176)-(1256,664)`. Its main set has no
separate outside-dismissal restoration PNG, so that specific pixel assertion
is Scribe-only. This exclusion is explicit in the aggregate report.

PW6's Reader-menu footer visibly reads `2 / 2; 30%`, and returned body/menu and
footer pixels match outside the clock region. Current identities and the
recorded non-first-page return are also bound by
`pw6-operator/main-handoff.json` and `pw6-operator/navigation-receipt.json`.
Its ordinary UI exit/restoration is recorded in
`restoration/pw6/detail-hub-main/receipt.json`. A restored application is not
evidence that its VM has already been closed.

The read-only aggregate command is:

```sh
python3 /root/weread-detail-hub-20261114-r1/verify-detail-hub.py \
  --root /root/weread-detail-hub-20261114-r1 --profiles scribe pw6 \
  --output /root/weread-detail-hub-20261114-r1/native-verification/report.json
```

## Focused gallery evidence

`delivery/kindle-native-ui.html` uses 13 current source PNGs: six main states
for each profile and ordinary Scribe Notes. One full-Scribe historical
comparison is identified separately. Original PNGs and sidecars remain
unchanged. The fragment is 963,005 bytes, SHA-256
`5845c134fbed6aa8d431b4bcce6455df4415b385f5b7066c837f1ad504ed6809`.

`gallery-validation/report.json` passed all nine focused JavaScript syntax,
DOM interaction, runtime-metadata, and immutable-byte checks. This is the new
focused suite, not a rerun of an earlier broader gallery suite. CSS layout,
image decoding, and actual Codex-host rendering are excluded. Selection is
not part of the accepted main capture set. Neither the gallery nor the main
native report closes the independent long-catalog or populated-notes gates.

## Current PW6 startup limitation

The main workflow encountered two Serif Bold gzip CRC failures when executed
from guest tmpfs. The same-hash userstore fallback then started and completed
the six-state PW6 main capture/return/ordinary-exit workflow. Its success is
recorded in the bounded 57-check main report and is not withdrawn by the
independent fixture failure below. The cause remains unconfirmed and
integrity checks remain enabled. Neither changing execution location nor a
successful start establishes that the checksum issue is resolved or excludes
a source/runtime defect. These observations apply to the firmware emulator;
no physical-device behavior is inferred. Earlier `4bc` startup observations
retain their own scope in the historical record.

## Independent PW6 long-catalog limitation

The 37-entry PW6 fixture completed a coherent backup, seed, and same-f2-ELF
userstore deployment. Its data remained conserved: 37 catalog entries,
three original cached chapters, and 34 metadata-only entries without bodies
or content authority. This preparation is not long-catalog UI acceptance.

Both permitted fixture starts failed while loading bundled Serif Light with
a gzip CRC error. Failure cleanup returned to stock Home and released the
launcher lock. This automatic failure restoration is distinct from the main
workflow's successful ordinary UI exit. There are zero accepted PW6
long-catalog PNGs. The operator
stopped at the retry limit; no further retry or cause investigation is claimed.
The fixture gate is Blocked/Unverified due to font-startup failure, not Passed.

The source/runtime/emulator cause is undetermined. This failure must not be
combined with the successful ordinary PW6 main screenshots or described as a
failure of the already recorded 57-check main flow. The separately passed
Scribe fixtures do not qualify this PW6 attempt. No statement that all native
conditions passed is made for this iteration. The fixture's original main
database and WAL bytes remained unchanged.

## Passed Scribe-only public fixtures

`public-fixtures/scribe-public-detail-final-report.json` has status
`passed_scribe_only`. It accepts 18 original PNG/sidecar pairs: eleven catalog
and seven notes captures. All 18 identities and dimensions match the f2 ARM,
the recorded helper, and Scribe's 1860 x 2480 viewport. All fourteen named
pixel checks and fourteen data checks passed. These counts are separate from
the 57-check main report and do not qualify PW6 long stress.

The catalog sequence verifies actual UI selection of original chapter 003,
its automatic reveal at native y=400, reselecting Contents, vertical scrolling
that remains settled without snapping back, fixed book column/footer, complete
`END-037` on page 4/4, and reading/menu return. The 37 metadata entries retain
three original cached chapters; added rows have no bodies or content authority.
The current original third chapter persisted at chapter index 2 / offset 0.

The separate five-note fixture verifies canonical quotes and the long thought
through `END-NOTE-003`, PageBar advancement to page 2/2 with `END-NOTE-005`,
fixed context/footer during scrolling, and return to Reader page 2/2 at 20%.
All five annotations match the public seed and valid scalar ranges. Original
titles, bodies, UIDs, and authority remain intact. The notes reading anchor
remained chapter 0 / offset 233, and ordinary main logical data was unchanged.
This accepted fixture covers PageBar navigation; numeric jump interaction is
not inferred from it.

The CRC-checked archive is
`public-fixtures/scribe-public-detail-f2c0f429-native.zip`, SHA-256
`5f3f6c6bbb0300dc566512283e4574005fabfccda292c6f06a49158b313b48bc`.
Its accepted set preserves original PNG/sidecar bytes. The local copies are
under `artifacts/detail-hub-20261114/public-fixtures/`. The report explicitly
records zero PW6 long-stress images in this archive and excludes PW6 from the
passed status. These synthetic fixtures do not prove account access, book
acquisition rights, or physical-device behavior.

## Task cleanup

`cleanup-receipt.json` passed for the exact f2 ARM. Both task-owned QEMU
processes exited after restoration of the firmware status bar and non-stopped
window managers. Original base machines were not modified; source and capture
artifacts were preserved. One task tmpfs was unmounted at the canonical
`/var/tmp/weread-detail-hub-20261114-r1-exec` path. This operation neither
changes the frozen product source nor resolves the unconfirmed checksum cause.
The cleanup receipt records no physical-device access.

## Separate introduction-preview proposal

The later six-line left-column introduction preview and its full-text shortcut
were illustrated as a conversation-only design proposal. They are not part of
the implementation or candidate evidence above. The implemented left column
still contains core book information and actions; the full introduction is
loaded and shown in the right Introduction tab. Generated proposal images,
conversation fragments, and development archives are not repository sources.

## Historical boundary

This accepted contract supersedes the default Reader directory entry and
layout in [the slider/contents/More record](SLIDER_CONTENTS_MENU.md). Its exact
`4bc076430a55` software, native, gallery, long-fixture, and cleanup results
remain historical evidence for that candidate. They must not be relabeled as
tests or screenshots of the new Detail hub. The former standalone contents
view's same-session observations do not impose a same-session requirement on
the new normal-navigation return path.

The unchanged font-slider, rectangular-control, and anchored-More decisions
retain their accepted roles. Current acceptance belongs to the new source and
profile-specific artifact receipts described above.
