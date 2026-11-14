# UI Guidelines: Findings and Design Proposals

Status: historical research and design proposals, with separately identified accepted amendments linked below. The historical proposals do not establish an accepted visual specification or claim device validation.

## Scope

Implementation update: the user subsequently supplied and authorized design handoff v0.2. Its accepted numeric tokens are stored in `assets/design-tokens.json`; the implementation uses those grayscale, typography, geometry, touch-target, pagination, and refresh rules. The original research and proposals below remain historical context. See [Implementation status](IMPLEMENTATION_STATUS.md) for the current design coverage and evidence boundaries.

The accepted 2026-11-14 [control-shape amendment](CONTROL_SHAPES.md) and direct follow-up require straight rectangular controls and surfaces, full rectangular icon/label tabs, and binary switches with square thumbs. The [rectangular implementation and capture record](RECTANGULAR_UI.md) supersedes earlier corner choices for these rectangular surfaces. Genuine circular slider handles, avatars, and radio/status marks retain their roles. That record's earlier native results remain evidence for its own artifact; intermediate candidate provenance, completed current gates, and separate follow-up boundaries are listed below.

The intended application is an independent Kindle bookstore application using Rust, Slint, and a project-owned Kindle platform layer. The intended device families are Paperwhite PW5, Paperwhite PW6, and Scribe. These are target devices, not a statement of verified application compatibility.

This document records the available references and proposed UI direction. It does not contain application code or authorize assumptions about book-content retrieval capabilities.

## Accepted Slider, Contents, and More Amendment

The 2026-11-14 follow-up is implemented in source. The contents origin is now
explicitly anchored at `x: 0px` and `y: 0px`. Both main firmware-guest profiles
and the separate 37-chapter public synthetic catalog verification have passed
their recorded gates. The authorized software, emulator, artifact, and cleanup
work is complete; physical acceptance remains a separate untested boundary.
See the
[font slider, contents page, and anchored More record](SLIDER_CONTENTS_MENU.md)
for the accepted behavior, artifact identities, and acceptance gates. These
accepted interactions are separate from the historical proposals below.

- The real stepped font-size slider uses the existing supported value list.
  `Down` and `Move` update only the local thumb, fill, and numeric preview;
  `Up` commits one supported value once through the existing font-size use
  case. `Cancel`, a late `Up` without an active drag, disabling, value-list
  changes, and component destruction do not commit. Existing decrement and
  increment controls remain available. The thin straight track retains a
  circular thumb, distinct from a binary switch's rectangular track and
  square thumb.
- Reader contents is an opaque full-window page. Opening it hides the Reader
  menu visually while preserving `menu`, `tool`, `subtool`, and the Reader
  page. Back or selecting the current chapter restores that context; another
  chapter opens at its normal start. Chapter titles wrap at the available
  width. Only the current bounded application page scrolls in a `Flickable`:
  11 chapters on Paperwhite 6 and 16 on Scribe. The header and pagination
  footer remain fixed, and Locate uses the current row's actual position.
- More actions use an opaque rectangular popup beside the detail-header
  trigger, aligned to its right edge. The popup is anchored to the trigger;
  the accepted presentation is no longer a bottom sheet.

The contents design draws on AidokuForKindle's chapter-hub design proposal
reviewed for this amendment, and the More presentation draws on its existing
detail menu. These are design references only. They are not a runtime
dependency or verification performed by this project. The accepted source
geometry for the full native guest viewports is:

| Target guest | Bounded chapter page | Fixed header, logical px | Fixed footer, logical px | Visible list, logical px | Footer origin Y, framebuffer px |
| --- | --- | --- | --- | --- | --- |
| Scribe 2022/DVT, firmware 5.20.1 | 16 | 200 | 64 | 976 | 2352 |
| Paperwhite 6/production, firmware 5.20.1 | 11 | 200 | 56 | 592 | 1584 |

These dimensions are accepted native-capture geometry targets. Correct footer
placement in an intermediate candidate does not establish a correct catalog
origin. The explicit contents-origin correction is present in the fourth
built candidate. Paperwhite 6's short-catalog origin and footer have passed
their recorded native checks, as have Scribe's main-profile origin and
footer. The separate 37-chapter public synthetic catalog report also passed
within its fixture scope as recorded below.

## What Has and Has Not Been Established

The investigation has not confirmed a publicly available, unified Amazon Human Interface Guidelines document or design system for third-party native Kindle applications. This is a limited research finding, not proof that no such document exists.

Earlier references to "e-ink conventions" mean scattered KOReader development conventions, existing widget behavior, and device-related implementation guidance. They do not mean a complete official Kindle design specification. KOReader is a useful independent project reference; its conventions are not Amazon requirements.

Slint styling, project-defined design tokens, and design systems such as Material or Cupertino occupy different roles. They must not be presented as an official Kindle specification. Mentioning those systems also does not establish that a particular Slint style is available or suitable for this project.

## Reference Roles

| Reference | What it supports | Boundary |
| --- | --- | --- |
| [KOReader ui.size](https://koreader.rocks/doc/modules/ui.size.html) | A standardized set of widget sizes for borders, margins, padding, radii, and lines, with an explicit preference for UI consistency. | KOReader project conventions; not a Kindle HIG or a set of approved values for this application. |
| [KOReader Hacking: Developing UI widgets](https://koreader.rocks/doc/topics/Hacking.md.html) | Widget development guidance and an entry point to KOReader's UI tooling. | Developer guidance; not a complete visual or interaction design specification. |
| [KOReader Porting](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/doc/Porting.md) | Technical context for device adaptation. | Porting guidance; not a UI design standard and not a requirement to adopt KOReader's architecture. |
| [WCAG2ICT](https://www.w3.org/TR/wcag2ict-22/) | Informative guidance on applying WCAG to non-web information and communications technologies. | Informative guidance, not a normative Kindle standard or evidence of this application's conformance. |
| [Slint LogicalSize](https://docs.rs/slint/latest/slint/struct.LogicalSize.html) | The distinction between logical coordinates and physical pixels after applying a device scale factor. | A framework coordinate model; not a physical touch-target specification. |

The proposed reference order is: observe Kindle system interaction patterns for familiarity, consult KOReader's relevant UI conventions, and record the selected project decisions in explicit Slint tokens and component specifications. Observed system behavior must remain distinguishable from a published requirement.

## Proposed UI Direction

The following items are design proposals, not accepted standards or verified device behavior.

### Grayscale and Text Labels

Prefer clear grayscale hierarchy, readable text, and visible control boundaries. Important actions and states should have text labels. Selection and task status should not depend solely on subtle gray differences. Exact colors, font sizes, weights, line widths, and contrast criteria remain to be designed.

### Stable Cover Slots

Use a fixed cover slot within each chosen list or grid template. A placeholder and a loaded cover should occupy the same space so that image arrival does not move nearby controls or text. Keep the title, author, and relevant status readable independently of the cover. The slot dimensions and available presentation modes are undecided.

### Paging and Navigation Context

Consider paging as the default for bookstore and library browsing. Preserve the query, filters, sort order, page or scroll position, and selected item when returning from a detail view. Changing presentation mode should preserve a meaningful content anchor where practical. The exact restoration behavior must be specified per page template.

### Scrolling Where It Helps

Scrolling remains an available design choice for content such as long descriptions, tables of contents, and input candidates. Release-triggered scrolling, where the view updates after the gesture ends, is one candidate for reducing continuous display updates. It is not a universal rule or an accepted implementation choice. Choose the interaction per content type and device behavior.

### Device- and Content-Dependent Refresh

Choose refresh behavior according to the device's supported update mechanisms, changed content, and observed image quality. Consider combining nearby updates, using partial updates where supported, and providing a deliberate full-refresh or clear-screen action. Do not prescribe a universal refresh frequency, a fixed number of seconds, or the same full-refresh interval for all devices and content.

Provide a stable, visible acknowledgement of an action while asynchronous work proceeds. The final refresh policy, feedback states, and any user controls require device-specific design and evaluation.

### Physical Touch Targets

Use approximately 8-10 mm as an initial prototype range for primary touch targets, subject to adjustment. This is a design starting point, not an accepted minimum, an accessibility conformance claim, or a Kindle requirement.

Calibrate physical dimensions against device DPI and the platform's logical-to-physical coordinate mapping. Slint logical pixels, framebuffer pixels, and physical millimeters must remain distinct. Do not copy a web convention such as 44 or 48 CSS pixels directly into framebuffer dimensions. Visible icons may be smaller than their touch regions. Final target sizes and spacing need device evaluation.

### Chinese Input and Search

Design the whole input flow: text composition, candidate selection, character commitment, editing, search submission, and result presentation. Keep candidate placement and the input area stable. Support mixed Chinese and Latin queries in the proposed design, and preserve the query when navigating back from results.

Do not assume that a native Kindle input method or a usable Chinese keyboard is automatically available through Slint and the project-owned platform layer. The input implementation and its accepted interaction remain open decisions.

### Persistent Download Task State, If Supported

A confirmed, usable book-content interface is a prerequisite for implementing downloads. Metadata access or account access must not be treated as confirmation that full book downloads are available.

If the interface and feature scope are confirmed, provide task state that remains discoverable while navigating through the application. Candidate states include queued, downloading, processing, ready to read, failed, and cancelled. A completed network transfer alone should not be labeled ready to read. Consider cancellation, retry, and combined progress updates; these are proposals rather than promised capabilities. Persistence across application restarts and recovery behavior remain to be designed.

### Paperwhite and Scribe Layouts

Consider a single-column primary layout for Paperwhite and an optional two-pane list/detail layout for Scribe. Use additional space to expose useful context while maintaining appropriate physical text and touch sizes. A two-pane Scribe layout is optional, not a frozen requirement. Orientation behavior and layout breakpoints remain open.

## Provisional Navigation

An earlier proposal uses four destinations: **Bookstore**, **Library**, **Downloads**, and **Account**.

This remains a navigation proposal. In particular, the Downloads destination does not establish download availability. Its inclusion and behavior depend on confirmation of the book-content interface and acceptance of the download workflow. If those prerequisites are not met, revisit the navigation rather than presenting a functional download feature as established.

## Specification Work Still Required

At the time of this historical research, the project had not frozen the following design artifacts:

- Design tokens: typography roles, grayscale palette, spacing, borders, cover dimensions, icon dimensions, and touch regions.
- Components: controls, book rows and cards, dialogs, search input, candidate presentation, navigation, and conditional download task presentation.
- Component and page states: default, selected, pressed, disabled, focused where applicable, loading, empty, offline, and error states.
- Page templates: bookstore, search results, book details, library, account, and any confirmed download task view.
- Device rules: physical-size mapping, orientation, layout changes, supported refresh behavior, and content-specific update policies.
- Acceptance criteria and device evaluation procedures.

This list records the historical research state. Later accepted handoff tokens and amendments supersede the corresponding open proposals; current coverage is recorded in [Implementation status](IMPLEMENTATION_STATUS.md). The remaining proposals require explicit design and review and must not be converted into invented, supposedly approved font sizes, colors, dimensions, or component behavior.

## Verification Status

The historical research itself is not application verification. Completed
checks for later accepted implementations retain their own artifact and
environment boundaries. All gates authorized for this task are complete
within their recorded software, emulator, artifact, and cleanup scopes:

- Historical verified evidence: the earlier rectangular ARM candidate with
  SHA-256 prefix `60d643dacf6e` passed 26 native checks and produced 34 PNGs in
  the two firmware guests, as recorded in [the rectangular UI review](RECTANGULAR_UI.md).
  Those results must not be assigned to the newer slider, contents, and More
  artifact.
- Completed software gates: two Rust contents-navigation regressions retain
  their existing passed software scope. The 25 passed software slider checks
  cover the same widget with SHA-256 prefix `b738abd`, with matching fonts and
  lockfile. The whole final application was not retested by that software
  suite. These results are not native checks of the fourth ARM application
  and do not establish physical acceptance.
- Intermediate build provenance: the third ARM candidate with SHA-256 prefix
  `6c88111cb398` passed build and public-package checks. It remains an
  intermediate candidate whose contents origin has not passed native
  acceptance. Its Paperwhite footer was correct, but the first short-catalog
  row still began below the expected header boundary.
- Historical slider/standalone-contents candidate: the fourth ARM application has SHA-256
  `4bc076430a559eba01d7b1cc8e246d77c28e42776bfaf6f9f8d7bf63e5e60e5d`.
  Its build and public-package checks completed with exit code 0. The source
  change explicitly binds only `contents-content` to `x: 0px` and `y: 0px`.
  Build and package success do not establish native contents-origin acceptance.
- Historical partial native report: the independent
  `native-ui-verification/final/report-pw6.json` records 17/17 passed, with
  `checked_profiles = [pw6]` and `final_report_complete = false`. Preserve
  this partial report separately; its count is not added to the final report.
- Passed, both main profiles: `native-ui-verification/final/report.json`
  records 32/32 passed for the fourth candidate, with
  `checked_profiles = [scribe, pw6]` and `final_report_complete = true`.
  This result covers the main profiles, not the long-catalog aggregate.
  Blank contents clicks retain the contents view; Return restores the Reader
  menu context. An inside blank More click retains the popup and an outside
  click restores detail. Main-profile geometry is recorded in the table below.
- Passed, both native drag receipts:
  `gesture-verification/scribe/final-verified/report.json` and
  `gesture-verification/pw6/final-verified/report.json` passed their recorded
  held-preview and release-body checks. Changed-pixel counts are listed below.
  The native `native_model_commit_count` was not observed, and native
  cancellation was not tested.
- Passed, both exit restorations: ordinary UI exit was confirmed by
  `restoration/scribe/final-ui-exit/receipt.json` and
  `restoration/pw6/final-ui-exit/receipt.json`. Each receipt records an absent
  lock and restored status bar and `awesome`.
- Passed, final task cleanup: the task-owned `cleanup-receipt.json` records
  `status = passed`. Two main QEMU instances exited, the status bar and
  `awesome` were restored, and `original_machine_modified = false`.
  All three task-owned tmpfs mount layers were removed
  (`task_tmpfs_mounts_removed = 3`); source and capture artifacts were preserved.
- Passed, gallery structure and metadata: `gallery-validation/report.json`
  records 45/45 JavaScript, DOM, and metadata checks passed. The main gallery
  contains 36 current PNGs: 15 main and 3 optional captures for each profile.
  Six before comparisons remain independent historical evidence. This is not
  CSS or host-render verification.
- Passed, public synthetic long catalog:
  `public-catalog/final-public-synthetic-4bc076-report.json` records
  `status = passed`. It accepted 42 original PNGs and their original sidecars
  (22 Scribe, 20 Paperwhite), with all identities and dimensions passed;
  20/20 pixel-consistency checks and Paperwhite's 5/5 post-native data checks
  also passed. The separate `public-synthetic-native-4bc076.zip` archive
  passed 42/42 PNG/sidecar SHA and CRC checks. Its 91 entries exclude
  databases, logs, setup/startup files, and older `6c881` artifacts.
  Both fixture applications stopped and restored before their VMs were
  closed. This fixture retains three cached demo chapters and adds 34
  chapters with null bodies and unknown status; it preserves the original
  identities, body hashes, authority, and chapter 0 / offset 0 reading anchor.
  The results
  establish public catalog navigation, not 37 downloaded chapters or book
  rights. See [the amendment record](SLIDER_CONTENTS_MENU.md) for the strict
  report, receipts, archive, and fixture scope.
- Startup limitation: the fourth candidate's first tmpfs launch reported one
  `SerifRegular` gzip CRC error despite matching host and guest ELF hashes;
  the same-hash, same-path second launch succeeded. The cause is unresolved,
  CRC validation remained enabled, and a source or runtime failure has not
  been ruled out. See [the startup limitation record](SLIDER_CONTENTS_MENU.md#observed-emulator-startup-limitation).
- Not deployed: the current candidate has not been installed on a physical
  Kindle; physical installation and acceptance remain open. Firmware guest
  results are emulator evidence.

The passed main-profile geometry and native drag observations are:

| Main profile | Short-contents boundaries Y, framebuffer px | Full-width footer Y, framebuffer px | More bounds, framebuffer px | Held body changed pixels | Held slider changed pixels | Release body changed pixels |
| --- | --- | --- | --- | --- | --- | --- |
| Scribe | 400/544/688/832 | 2352 | (1156, 176)-(1844, 696) | 0 | 19,403 | 311,125 |
| Paperwhite 6 | 400/534/668/802 | 1584 | (728, 176)-(1256, 664) | 0 | 12,820 | 121,667 |

The intermediate native observations preserve the candidate sequence. Footer
placement and catalog origin are separate acceptance gates:

| Candidate | ARM SHA-256 prefix | Intermediate native observation |
| --- | --- | --- |
| First | `828179` | Footer around Y=832 framebuffer px; short rows centered. |
| Second | `14ccca` | Explicit full-height layout placed the footer at Scribe Y=2352 and Paperwhite Y=1584 framebuffer px; the short three-row catalog was still centered. |
| Third | `6c88111cb398` | After `alignment: start`, the Paperwhite footer remained at Y=1584 framebuffer px, but the first short-catalog row was at Y=791 instead of the expected header-end Y=400 framebuffer px. |

The fourth candidate's built artifacts and source provenance are:

| Item | SHA-256 |
| --- | --- |
| ARM application | `4bc076430a559eba01d7b1cc8e246d77c28e42776bfaf6f9f8d7bf63e5e60e5d` |
| Development ZIP | `9919688eb953c56413ae2f43df7bca590c0d74b778d17e7513b06a5e89840814` |
| Source digest | `672d2b7fe9495812df2b2eaafd726977aa32b8eb675dd5109b003c4b153d8bfa` |
| Raw `ui/reader.slint` | `f13d4800e23554d9284c367ad5342973344fa875133b4cf5dc48f55a9d7aa38b` |
| Main gallery HTML, 964,646 bytes | `64cee5d922a515796bfa28d8afecb10dd0d2e5f6073683cff008ae8c8df7840d` |
| Public synthetic native ZIP | `78905aec24c2369c140f1e90744eaf0f7e8717cb87b95a85b2129a4e3b20bb64` |

The same-version [Slint 1.17.1 default geometry pass](https://raw.githubusercontent.com/slint-ui/slint/v1.17.1/internal/compiler/passes/default_geometry.rs)
explains the automatic centering. In its non-layout-child, non-legacy,
non-Window path, a height that does not fill the parent can call
`maybe_center_in_parent` (lines 144-153). The helper does not inject `y` when
a binding already exists (lines 489-497). This compiler-source rationale
identifies default geometry as the cause of the catalog centering, rather
than native-driver placement. This source rationale is separate from native
evidence: both main profiles' short-catalog origins and the separate public
synthetic long-catalog fixture have passed their recorded checks.

Future verification must follow the authorized verification environment and
task permissions.
