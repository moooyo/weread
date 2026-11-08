# Design fidelity review

Recorded: 2026-11-03 (Asia/Shanghai).

Correction update: the user subsequently authorized implementation of these fixes. This report preserves the pre-correction findings; current changes and acceptance evidence are tracked in [Design fidelity corrections](DESIGN_FIXES.md).

## Conclusion

The implementation does not yet fully reproduce the v0.2 design handoff. The main page families, navigation, grayscale direction and several geometry foundations are present, but important interaction states, component structures and Scribe-specific arrangements still differ. One primary-action hit-testing defect was reproduced. The previous 133-check suite verifies substantial functionality and persistence; it is not acceptance against every design screenshot.

This task was a read-only review of the application. No implementation files were changed. New application captures ran only through `ssh test-env`, using the existing desktop binary and isolated demonstration data. No authenticated account, private content or physical Kindle was used.

## Method and source precedence

- Design source: the operator-supplied `weread-kindle-design-v0.2/weread-kindle-design` handoff folder, maintained outside this repository.
- Implementation source: this checkout, including its current uncommitted implementation.
- Reference PNGs are two physical pixels per dp; desktop captures are one pixel per dp. Comparisons resize the design by exactly 0.5 and do not rescale the implementation to hide differences.
- Appearance was checked against PNGs, interaction against the matching prototypes, and declared values against tokens/SPEC. Data, dates, source text, current chapter, record counts, QR patterns and missing real credentials were excluded as fidelity errors.
- The handoff has a real precedence conflict: `README.md:14` and `SPEC.md:14` give numeric precedence to tokens, while `design-tokens.json:2` says the prototype wins. Conflicting numeric values are listed separately below; they are not automatically classified as undisputed implementation bugs.
- Some review-only SQLite fixtures populate cached annotations, article images and an example original URL. These expose actual view layouts; they do not prove authentication, service access or a phone scan.

Evidence root: `artifacts/design-review-20261103/`. The `captures/` directory contains 36 new catalog/auth captures and their actions/metadata. `reading/` contains 41 new Reader/Article captures, fixed-scale paired comparisons and a detailed evidence README. `auxiliary/` contains the publication, statistics and settings evidence indices and captures. Generated evidence is ignored by Git.

No numerical restoration percentage is asserted: different real/fixture content and incomplete access to authenticated states would make such a number misleading.

## Highest-priority defect

### DF-01 - P1 - Scribe detail action buttons are covered by the tabs

The fixed 196dp detail header cannot contain its cover, metadata, progress and action row. The next tab row covers approximately the lower 8dp of the declared 48dp buttons, reducing the effective action height to about 40dp.

This affects input, not just appearance. Independent remote pointer captures show:

| Input on the read button | Observed result |
| --- | --- |
| `(400,260)` near the center | Reader opens. |
| `(400,280)` in the declared lower strip | Detail remains visible. |
| `(200,280)` in that strip | The Introduction tab is activated instead. |

Source: [bookstore.slint](../ui/bookstore.slint:278), especially the metadata/action layout at line 290. Evidence: `artifacts/design-review-library-detail/online/scribe/detail-read-center.png`, `detail-read-lower-edge.png`, `detail-read-lower-edge-intro.png`, plus their action/geometry JSON. [Side-by-side](../artifacts/design-review-20261103/comparisons/01-scribe-detail.png).

The conflicting design header dimensions do not excuse overlap or a target smaller than the accepted minimum.

## Confirmed interaction and component differences

All source references below are one-based. P2 means a clear behavior/component discrepancy to address before calling the design fully reproduced. P3 covers smaller styling/asset discrepancies.

| ID / priority | Difference and effect | Implementation | Evidence |
| --- | --- | --- | --- |
| DF-02 / P2 | Library horizontal swipe opens a book. A 300dp drag from `(500,300)` to `(200,300)` ends on Detail on both profiles; the design requires page turning after 64dp. | `ui/bookstore.slint:264`; `src/app/library.rs:443` | `captures/{pw6,scribe}/library-list-swipe.png` and metadata; SPEC line 99. |
| DF-03 / P2 | Chapter selection survives switching to Introduction. The download-selection bar remains on unrelated tabs; the prototype clears selection on tab change. | `src/app/library.rs:650`; `ui/bookstore.slint:377` | `captures/pw6/detail-selected-intro.png`; `prototype/PW-Detail.dc.html:291`. |
| DF-04 / P2 | The selection action-bar divider runs through text/buttons. Its 2dp Rectangle has no explicit top position. The unconditional pager also occupies space in Introduction and selection states where the prototype hides it. | `ui/bookstore.slint:376`, `:379` | `captures/pw6/detail-selected.png`; [paired image](../artifacts/design-review-20261103/comparisons/02-detail-selection.png). |
| DF-05 / P2 | List view drops the cached/downloaded icon although the same book has it in cover mode. Users lose the local-readiness cue. | `ui/bookstore.slint:254` | `captures/pw6/library-list.png`; design `PW-Library--list.png`; [paired image](../artifacts/design-review-20261103/comparisons/10-library-list.png). |
| DF-06 / P2 | Uncommitted pinyin is placed at the far right without the design underline. Typing `shi` also removes recent searches and shows zero/no-result feedback before selecting a candidate or submitting. | `ui/auth.slint:684`, `:716`, `:764`, `:787` | `captures/{pw6,scribe}/search-composing.png`; [paired image](../artifacts/design-review-20261103/comparisons/03-composing-search.png). |
| DF-07 / P2 | The keyboard's intended top border is centered into the key area. Its Rectangle declares height but not `y:0`, leaving the top boundary missing and a line in key gaps. | `ui/auth.slint:114` | Search captures on both profiles; `PW-Search.dc.html:89`, `SC-Search.dc.html:92`. |
| DF-08 / P2 | Login onboarding is reduced to one path hint instead of the supplied three-step structure. Scribe verification also retains the PW-sized code cells and full-width keypad instead of its centered narrow composition. The verification layout is source-confirmed only. | `ui/auth.slint:359`, `:385`, `:403`; `assets/i18n/zh-CN.json:173` | `captures/{pw6,scribe}/login-default.png`; design `SC-Login--code.png`; [guide comparison](../artifacts/design-review-20261103/comparisons/12-login-guide.png). |
| DF-09 / P2 | Detail Introduction loses the structured metadata grid, note-count entry and update stamp. Scribe notes reuse review rows, losing chapter/date, serif quoted text and underline hierarchy. Filled-note typography is source-confirmed. | `ui/bookstore.slint:320`, `:368`; `src/app/library.rs:536`, `:603` | `captures/pw6/detail-intro.png`; `PW-Detail--intro.png`, `SC-Detail--notes.png`. |
| DF-10 / P2 | More is a centered generic dialog instead of a bottom sheet. Download choices are rectangular inverted buttons rather than radio rows, and omit the specified chapter/cache/size information structure. No download was started during review. | `ui/app.slint:133`, `:142`, `:153`; `src/app/library.rs:963` | `captures/{pw6,scribe}/detail-more.png`, `detail-download-choose.png`; matching prototype interaction definitions. |
| DF-11 / P2 | Scribe Reader typography loses its 2x2 main layout, direct font choice, horizontal font cards, three-column indent panel and descriptive turn cards. Generic stacked PW rows replace those structures. | `ui/reader.slint:274`, `:403`, `:428`, `:434` | `reading/scribe-reader-{type,font,indent,turn}.png`; [type comparison](../artifacts/design-review-20261103/comparisons/04-scribe-type.png), [font comparison](../artifacts/design-review-20261103/comparisons/05-scribe-font.png). |
| DF-12 / P2 | Reader/Article menu header actions expand horizontally instead of keeping compact touch widths. Selected bookmark/favorite backgrounds occupy large strips and shift the title/return composition. | `ui/reader.slint:337`, `:345`, `:704`; `ui/widgets.slint:76` | `reading/{pw6,scribe}-reader-menu.png`, `pw6-article-menu.png`; matching paired images. |
| DF-13 / P2 | Scribe annotation cards lose dashed/solid quote semantics and the popular-review preview. A review-populated fixture still shows blank preview text because the controller creates an empty body. Expanded threads also lose the dedicated return-to-list header. TOC notes/bookmarks omit chapter/date/page/icon/count details. | `ui/reader.slint:247`, `:484`, `:511`, `:555`; `src/app/reading.rs:503`, `:521`, `:644`, `:655` | `reading/scribe-annotations.png`, `scribe-thread.png`, `*-reader-toc-{marks,notes}.png`; [sidebar comparison](../artifacts/design-review-20261103/comparisons/06-sidebar.png). |
| DF-14 / P2 | PW bottom-position thought popup retains the centered 506dp width and 63dp margins instead of reaching both screen edges. Child cards also interrupt the outer border. Bottom geometry is defined by the prototype; there is no delivered bottom-state PNG. | `ui/reader.slint:543`, `:545`, `:550`, `:577` | `reading/pw6-thought-bottom.png`; `PW-Reader.dc.html:614`. |
| DF-15 / P2 | Reader/article size and light scales become numeric steppers, removing direct scale selection. Article paragraphs are not justified; quoted blocks follow the sans body family instead of the design's independent serif quote. | `ui/reader.slint:209`, `:396`, `:405`, `:735`, `:1101`, `:1112`; `src/app/auxiliary.rs:1450` | `reading/*-reader-{type,light}.png`, `*-article-type.png`, `*-article-quote-image.png`. |
| DF-16 / P2 | Article pictures always use a 132dp total slot, leaving 104dp for the bitmap/placeholder. The design has larger device-specific frames. The placeholder also loses its dashed boundary and centered picture icon. This was checked with equal-aspect-ratio fixture pictures. | `src/article.rs:497`; `ui/reader.slint:1120`-`:1127` | `reading/*-article-quote-image.png`, `*-article-loaded-image.png`; see the reading evidence report for the exact target states. |
| DF-17 / P2 | The article phone QR card is left-biased and fixed at 648dp high, producing excessive blank space rather than the centered content-fitting composition. QR contents were excluded. | `ui/reader.slint:794`, `:797`, `:804`, `:809` | `reading/{pw6,scribe}-article-qr.png`; paired images. |
| DF-18 / P2 | Settings segmented choices are replaced by a current-value row that cycles on tap. Available choices and selected-segment inversion are absent. | `ui/auxiliary.slint:602`; `src/app/auxiliary.rs:1096` | `auxiliary/settings/*-{display,notes}.png`; [comparison](../artifacts/design-review-20261103/comparisons/08-settings-options.png). |
| DF-19 / P2 | Publication search places the icon after the input, floating inside the field rather than as its leading prefix. | `ui/auxiliary.slint:217`, `:235` | `auxiliary/publications/{pw6,scribe}-accounts.png`. |
| DF-20 / P2 | Statistics uses four separately rounded buttons rather than one segmented frame. Scribe controls/summary lose their horizontal relationships; the calendar intensity/read-day legend and category percentage legend are missing. This excludes the disputed ranking/calendar column arrangement. | `ui/auxiliary.slint:371`, `:377`, `:393`, `:490`, `:530` | `auxiliary/stats/actual/`; [comparison](../artifacts/design-review-20261103/comparisons/09-statistics.png). |

## Smaller confirmed discrepancies

- Disabled buttons and switches retain solid normal outlines instead of the specified dashed treatment. Disabled period-arrow icons also remain black. See `ui/widgets.slint:45`, `:89`, `:166` and the storage/total-period captures.
- Recent searches use large equal-width rectangular tiles instead of content-width wrapping capsules; scope tabs on Scribe stretch across half the page. See `ui/auth.slint:737`, `:774`.
- Publication unread dots are above the title baseline, uncached article actions omit their prescribed outline shapes, and Scribe account rows omit the trailing chevron. See `ui/auxiliary.slint:133`, `:158`, `:187`.
- Settings subheaders omit their separator and Scribe's visible parent return label. Article metadata/end-state/icon structures are simplified. Detailed evidence is in the subordinate reports.
- Known TOC/note/bookmark counts are absent from several tab labels. Sans Medium is not separately bundled; intermediate weights use matching/synthesis, so exact Medium fidelity is not established by declaring weight 500.

## Design-source conflicts requiring a separate decision

These are real visual differences from the PNGs, but not unambiguous failures against the written numeric rules:

1. Common status/header/search/tab values and Scribe page margins differ between tokens and Scribe prototypes. For example, the common tokens use 28dp status and 32dp Scribe margins while several SC prototypes use 32dp and 40dp.
2. Library list tokens specify 72dp rows; prototypes declare PW76/SC88dp, with additional content-box borders. The implementation uses 72dp. It must not be assigned a false exact pixel error without reconciling those sources.
3. Generic button/touch values differ from some SC keypad/action sizes, and some prototypes use targets below the common minimum. Structure findings above remain valid without selecting one disputed height.
4. The SC Stats PNG places ranking and calendar side by side, while SPEC/tokens require non-reader pages to remain single-column. The current single-column implementation follows that written boundary.

Choose and record one component-level numeric baseline before a pixel-fidelity acceptance pass. Fixing overlaps, state leakage and missing structures does not depend on resolving this conflict.

## Coverage and exclusions

| Area | Reviewed | Boundary |
| --- | --- | --- |
| Library/Detail | Both profiles, cover/list/select, Introduction, reviews, notes shell, download chooser, More; swipe and three detail-button pointer probes | Filled reviews/notes partly source-confirmed; no acquisition lifecycle or authenticated mutation. |
| Login/Search | Both base login views; empty/composing/results/shelf search; cached history; all corresponding source/prototype structures | Real QR/OTP, expiry and successful authentication were not exercised. Verification-code layout is source-confirmed. |
| Reader/Article | Both bases and typography tools, contents/bookmarks/notes, annotation/thread, no-panel, pictures, lists and phone QR; 41 fresh captures | Cloud conflict source only; some thought positions not separately captured; example QR URL was not opened. |
| Publications/Settings/Stats | Accounts/articles, main settings plus six subpages, all four periods on both profiles; 26 target states, with additional uncached/period fixtures | Authenticated account cards, real cloud data and unavailable platform controls excluded. |
| Foundation | Palette, font roles, base device geometry, main navigation, column counts, pagination direction, no-animation/no-scrim approach | No claim that all 104 design PNGs passed an identical-data pixel baseline. |

Already aligned foundations include the six grayscale colors, Sans/Serif role split, PW three-column and Scribe five-column cover grids, four primary bottom tabs, stacked detail/reading navigation, hidden primary navigation on pushed pages, current-chapter inversion and disabled cached-chapter selection logic.

Hardware/entitlement/release boundaries remain separate: EMR, pinch, hardware inversion, physical refresh, live source rights, update channel and project license decisions were not treated as visual defects. Desktop evidence does not establish a Kindle model/firmware result.

## Evidence index and next correction order

- Catalog/auth new captures: `artifacts/design-review-20261103/captures/capture-index.json`.
- Fixed-scale summary comparisons: `artifacts/design-review-20261103/comparisons/index.json`.
- Reader/Article full evidence: `artifacts/design-review-20261103/reading/README.md` and `review-captures.json`.
- Publications/settings: `artifacts/design-review-20261103/auxiliary/evidence.json`.
- Statistics: `artifacts/design-review-20261103/auxiliary/stats/evidence.json`.
- Detail pointer proof: `artifacts/design-review-library-detail/online/scribe/`.

Recommended order: repair primary hit areas and drag classification; repair state transitions and misplaced separators; restore missing component structures; then reconcile the design-source conflicts and perform targeted visual comparisons with matched fixture data. This review did not apply those corrections.
