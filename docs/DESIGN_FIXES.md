# Design fidelity corrections

Recorded: 2026-11-03 (Asia/Shanghai).

This implementation pass follows the user authorization to repair the differences in [Design review](DESIGN_REVIEW.md). The earlier report is retained as a record of the defects and evidence. This document records the correction baseline and the resulting acceptance evidence separately.

## Numeric baseline

The handoff has conflicting precedence notes. This pass follows the shared accepted token constraints where the token has an explicit common value: 28dp status bars, 52dp page headers, 20/32dp page margins, 48dp minimum targets/general buttons, and at least 8dp between independent targets. Component-specific shapes and layouts without a token entry follow their matching prototype/PNG. Non-reader pages retain the agreed single-column layout; the Scribe reader retains its side notes panel.

Consequently, a component may match the specified structure while retaining a common token value that differs from one prototype. This is recorded rather than disguised by resizing reference images. In particular:

- The Scribe detail header is 292dp, with a 168x236dp cover and 28dp vertical padding. Its complete action targets remain 48dp high, with tabs below the header rather than overlapping it.
- Scribe code entry uses a centered 520dp column and 104x124dp cells; ordinary buttons retain the accepted target rules.
- Reader/Article typography uses the device-specific card/grid structures and real direct scales. Settings numerical adjustments remain steppers where specified.
- Scribe ranking and calendar stay vertically stacked, following the accepted non-reader single-column boundary rather than the conflicting side-by-side illustration.
- Article picture frames use the specific prototype values because no image-size token exists: PW placeholder/loaded frames 180/220dp plus 20dp caption and 8dp gap; Scribe 220/300dp plus 24dp caption and 10dp gap. Full unshrunk block heights are therefore 208/248dp and 254/334dp. Pagination may shrink a frame only when a regular page cannot contain it.

## Implemented corrections

| Review group | Correction |
| --- | --- |
| DF-01 | The Scribe detail header reserves complete button bounds and separates them from tabs. |
| DF-02 | Shared swipe-aware touch handling recognizes 64dp horizontal navigation and suppresses accidental book activation; vertical drags are not taps. |
| DF-03/04 | Tab transitions clear chapter selection. Selection-bar dividers are explicitly top-aligned. Pager visibility follows the actual content/selection state. |
| DF-05/09 | List cache indicators, rich detail metadata/notes, known counts and content-width filter capsules are restored. Unknown server values remain explicitly unknown. |
| DF-06/07/08 | Login onboarding/code layout, inline underlined pinyin, recent-query capsules, composing state, results summary, scope tabs and keyboard boundaries are restored. Real authentication and clipboard guards remain in place. |
| DF-10 | More is a bottom sheet. Acquisition choices use radio rows, real chapter/cache counts, explicit unknown size, and current-chapter progress information. |
| DF-11/12 | Scribe typography cards/grids and explanatory turn controls are restored. Header controls have fixed bounds, with symmetric Scribe title flanks. |
| DF-13/14 | Annotation preview/underline semantics, thread return hierarchy, rich TOC entries/counts and position-specific thought popup geometry are restored. |
| DF-15/16/17 | Article text is justified without stretching paragraph endings; quote metrics/family are separate; image frames/captions, list hierarchy, phone QR and ending structure are restored. |
| DF-18/19/20 | Settings show direct segmented choices. Publication prefixes/actions/indicators and statistics segmented controls, horizontal summary relationships and legends are restored. |
| Shared styling | Actual disabled outlines/switch states, disabled icon color, bounded icon actions, semantic picture/phone/star icons and fixed separators replace simplified or displaced treatments. |
| Font resources | Actual Sans Medium and Serif Light/Medium faces were added from the same reviewed source versions. Original four font hashes and licenses remain unchanged; per-weight layout uses the corresponding real metrics. |

## Verification policy

All builds, tests, fixture execution and screenshot generation run through `ssh test-env`. The production binary retains its real account and capability checks. A separate disposable offline fixture runner may set public UI models to inspect QR/code/reviews/notes without claiming authentication or adding a production bypass.

`scripts/verify-design-fidelity.py` adds pointer, database and pixel regressions for the reviewed defects. A passing page route alone is insufficient for swipe, selection, clipboard, picture or direct-choice acceptance. Original reference PNGs are compared at a fixed 0.5 scale; fixture text/date/count differences are not a restoration score.

The separate fixture runner is reproducible from `tools/offline-ui-fixture/`. It is an independent development crate, always uses disposable offline demonstration data, and is not included as a production entry point or Kindle package. `scripts/verify-remote.ps1 -Design` builds it on the remote worker and runs the design checks. An optional `-DesignReferenceDirectory` names an already copied remote design `screens/` directory for paired evidence; no original design archive is required in source control.

Final results and artifact links are recorded in [Verification](VERIFICATION.md) after the correction run. No desktop result establishes a physical Kindle model/firmware, live-account access, EMR, multi-touch, hardware inversion or release licensing acceptance.

## Correction acceptance results

- The complete application suite passed 200 tests with zero failures.
- The final frozen design suite passed 251 checks with zero failures in 128.042 seconds: 135 captures and 116 assertions, with 41 paired reference images. Its main binary SHA-256 is `1ac4c11db6735f476862778cf842bfb3dacb41f07da358f54837c0e08a8a06c7`.
- The existing functional/persistence suite passed 133 checks with zero failures on the immediately preceding functionally identical candidate. The final increment only rounds the outside segment backgrounds and corrects the radio-dot diameter; it changes no event, bounds, data or persistence behavior. The final 251-check run accepts that paint increment separately.
- The full Scribe action target is 48dp high, including lower-edge input. Long horizontal swipes turn exactly one page, vertical drags are not taps, and ordinary taps still open a book. Real selected chapter state is cleared on tab transitions.
- Article bitmap comparisons use an independently reconstructed sampler from the pinned Slint renderer. Both device frames matched exactly; original strict comparison limits were not relaxed. Four artifact-only negative controls rejected the original missing-cache-icon and stretched-header captures.
- The updated ARM release candidate built successfully and is a stripped, statically linked ELF32 ARM EABI5 executable. It has no dynamic section. Its development archive is `dist/weread-kindle-0.2.0-design-fix.zip`.

Generated reports are retained under `artifacts/design-fix-20261103/frozen-1ac4c11d/`, `functional-12e58de2/`, and `RESULTS.md`. These checks do not claim identical-data pixel acceptance of every one of the 104 handoff images.
