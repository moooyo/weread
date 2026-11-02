# UI Guidelines: Findings and Design Proposals

Status: research summary and design proposal. This document does not establish an accepted visual specification or claim device validation.

## Scope

The intended application is an independent Kindle bookstore application using Rust, Slint, and a project-owned Kindle platform layer. The intended device families are Paperwhite PW5, Paperwhite PW6, and Scribe. These are target devices, not a statement of verified application compatibility.

This document records the available references and proposed UI direction. It does not contain application code or authorize assumptions about book-content retrieval capabilities.

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

The project has not frozen the following design artifacts:

- Design tokens: typography roles, grayscale palette, spacing, borders, cover dimensions, icon dimensions, and touch regions.
- Components: controls, book rows and cards, dialogs, search input, candidate presentation, navigation, and conditional download task presentation.
- Component and page states: default, selected, pressed, disabled, focused where applicable, loading, empty, offline, and error states.
- Page templates: bookstore, search results, book details, library, account, and any confirmed download task view.
- Device rules: physical-size mapping, orientation, layout changes, supported refresh behavior, and content-specific update policies.
- Acceptance criteria and device evaluation procedures.

These artifacts should be designed and reviewed explicitly. The proposals above must not be converted into invented, supposedly approved font sizes, colors, dimensions, or component behavior.

## Verification Status

This document does not claim that a build, runtime probe, UI test, or device acceptance check has been completed. Future verification must follow the authorized verification environment and task permissions.
