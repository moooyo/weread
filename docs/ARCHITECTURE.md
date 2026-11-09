# Architecture Agreement

Recorded: 2026-11-03 (Asia/Shanghai).

## Decision

Build an independent Rust and Slint client for jailbroken Kindle devices. Use a modular monolith: one application process with clear internal responsibilities, background work, and persistent local state.

WeChat Read cover images are retrieved from the upstream and cached on the Kindle. The architecture does not include S3 or a hosted cover library.

Book-resource authentication uses an explicit origin boundary: HTTPS port 443 on `weread.qq.com` and the individually reviewed `res.weread.qq.com` may receive web cookies. Other CDNs, unreviewed subdomains, suffix lookalikes, and nonstandard ports do not. A redirect leaving this set permanently drops cookies for that chain. Public covers remain anonymous, and the Skill Bearer key and reader authorization headers are never sent to resource origins. This corrects the former primary-only resource rule using the pinned source contract and the physical 401/302/200 comparison in [the API record](API_REFERENCE.md#book-chapter-image-resources).

Converted chapter resources may use the physically observed `CB_{book_id}_` root-member namespace. Plans provide the current book's exact prefix; only its unambiguous filename aliases can bind qualified HTML references. Exact paths remain authoritative, queries and case remain distinct, and raw path collisions are rejected before content deduplication can hide them. This is a scoped project contract established by private-device diagnosis, with synthetic fixtures and no private identifiers in source.

Accepted download-performance work keeps a serial chapter/checkpoint order while permitting two independent EPUB shards within a chapter. Requests share the existing authentication budget; Cookie changes cause a fresh serial read and disable further task parallelism. Reader/JSON gzip retains decoded-size limits, and verified TXT hints live only in their client/account/book/task scope. Image preparation uses an 8 MiB task PNG cache with cancellation closure and per-chapter bindings; worker-produced metadata is hash-bound before UI commit. Body/cursor/count updates are transactional, terminal writes compare the persistent task snapshot, and background UI updates avoid rebuilding covers. Cross-chapter prefetch and HTTP/2 remain later decisions. [Performance](DOWNLOAD_PERFORMANCE.md) distinguishes remote component measurements from device gates.

The application has its own package identity, executable, data directory, UI, and launcher. Reusing a GUI framework or a device adapter does not turn it into a plugin of another application.

Resource admission uses RAII credits retained through active work, queued results, and UI processing: eight metadata credits, two public-image credits within that budget, one login credit, one download-packet credit acquired before chapter-data acquisition or cloning, and one independent opportunistic progress credit. Download plans retain only bounded remaining-task metadata. A cached checkpoint sends identity and cursor data under the packet credit; persistence verifies the actual durable task, chapter, full-content ownership, and nonblank text before advancing the cursor. Only the current visible chapter is read when needed. Download preparation remains serial, with its permit held by the actual blocking worker. Busy and cancellation outcomes settle application state immediately without an unlimited control queue or receiver ownership cycle. Account and sleep fences discard actual stale packets rather than releasing credits while their buffers remain alive.

Fixed reviewed HTTPS/443 destinations retain capped first-party connection reuse. One shared arbitrary-resource client disables idle retention, with transport choice checked again for each redirect hop independently of cookie permission. Catalog admission prices new-row and inherited-owner metadata before copying; existing bodies remain in SQL during stable-UID reindexing. Cached underline and thought reads admit encoded bytes and record count in SQL before deserialization. The UI pages lightweight thought descriptors before loading bounded visible previews. Cache integrity proofs are operation-local, use streaming reads, and check original publication-destination identity before scoped cleanup can mutate it.

Local annotation views query scalar counts, anchors, and bounded headers before
loading selected quote/note fields in one read snapshot. Render and hit testing
share complete page geometry demand; drawing row tokens do not replace durable
annotation IDs. Closed TOCs still publish bounded current/access chapter
metadata without loading rich note entries. Bulk completeness checks share one
operation-local verification scope across selected chapters, while late thought
and fixed-review results use captured request/cache/view identities. New report
positions retain only the wire's 20-scalar summary; old titles and unresolved
delivery evidence remain intact. Complete verification of the latest resource
candidate is recorded separately from these implementation contracts.

Readonly Book and chapter consumers use distinct scalar or metadata capabilities;
these cannot be converted into a partial Book for persistence. Required Detail
descriptions, full Book merges, and canonical body proofs retain their original
data. The current native-input correction places admission on the actual
borrowed insertion and composition snapshot before text or Undo mutation, using
the already locked Slint source rather than clearing accepted editing history.
HTML style matching shares an immutable-DOM span proof and constant matcher
scratch. Local, remote, and selection fragments share one page output budget in
addition to caret admission. Report recovery keeps compact unallocated evidence
and admits consecutive bounded batches before atomic cursor persistence. These
contracts are integrated independently; their complete controlled validation
and explicit unchanged-Rust reuse chain are recorded in the resource record.

Inventory consumers first query numeric types, encoded sizes, and row counts in
one SQL snapshot. They admit the complete selected working set before owning
keys, manifests, or pending-task payloads; capacity refusal retains the FIFO,
cursor, and existing data. Bookmark rendering uses scalar geometry, and only a
selected action obtains its bounded canonical ID or label. Annotation range
proofs borrow the admitted canonical body and preserve Rust Unicode/NUL counting
instead of constructing another owned body.

Reader and Article layout share requested storage and allocation-work leases.
The defaults admit 64 MiB of live requested metadata and 128 MiB of cumulative
allocation work, including old/new buffer overlap. A completed fallible layout
is prepared before replacing a view or receiver. A known committed checkpoint
remains acknowledged if later layout is unavailable. These prices are not
allocator RSS or arbitrary font-execution deadlines.

The same-version Zeno dependency provides opt-in fallible fill storage and
access admission beneath Swash Outline/Alpha rendering. Its actual command,
subdivision, cell-chain, row, sample, and reserve operations can stop early;
only a complete successful staging result replaces the caller image. The
existing outline-mask target limits remain independent of this fill policy.

Stopping hardware workers remain owned until joined. Terminal cleanup independently stops producers, joins hardware workers, and checks native command quiescence even when persistence or a synchronous result handler fails. Cache cleanup identities survive publication and deletion failures; bounded diagnostics and compact confirmed-report evidence limit incidental operational history. [Resource lifetime review](RESOURCE_LEAK_REVIEW.md) separates integration, round results, and verification evidence. These changes do not authorize cross-chapter prefetch or a new download pipeline.

Operational session state also has explicit count and byte limits, applied atomically without evicting credentials or replaying elapsed-time writes. Local import checks both retained records and repeated decoding work. Its ZIP entry limit must apply before the dependency creates a central-directory index. The development source therefore retains the already resolved MIT-licensed `zip` 4.6.1 and a narrowly scoped bounded constructor under `vendor/zip`; the upstream source checksum and patch record justify this exact local revision. The bounded entry point reuses the parser's ZIP64 and candidate-fallback behavior. This is a reviewed resource boundary, with no upstream version upgrade or new project-license decision.

Image-anchor derivation uses a strict Result with early admission for 2,048 visible occurrences, 16,384 retained events, 262,144 retained edges, and 64 × 1024 × 1024 Unicode scalar work units. Quota errors stop preparation before body or asset publication; ordinary Unicode offsets, optional-caption disambiguation, and conservative unknown positions remain intact. These work/count limits do not define a total heap or wall-clock budget. The QR exchange separately admits a 1 KiB opaque UID and a 2 KiB complete confirmation URL from borrowed input before copying or percent encoding. Numeric identifiers remain supported; these are project resource policies rather than upstream identifier-format claims.

General caret positions also use fallible admission before vector allocation,
spacing, and reverse scans. A page preflights all actual local/remote/selection
demands, reuses one geometry per needed line, and withholds the complete derived
overlay on refusal while retaining the body, drafts, and existing selection.
Custom-font initialization separately admits collection faces, input work,
variation axes, selected name spans, and complete metric-copy costs before
temporary parsing and shared registration. Prepared font choices commit only
after the entire file succeeds; ordinary default-family and weight selection
remain unchanged. Verification and logical-budget limits are recorded in the
[resource review](RESOURCE_LEAK_REVIEW.md).

Content access is distinct from transport integrity and image readiness. A fresh Reader snapshot supplies request-bound, conservative entitlement evidence; unknown or contradictory metadata cannot grant full content. The domain and SQLite persistence retain explicit full, preview, locked, and unknown states with an opaque account binding. A preview is locally readable but cannot advance a full-download cursor, certify the entire book, or upload a fabricated complete reading position. Explicit revalidation after the user obtains rights may extend a preview only through a checked text-prefix transition that preserves personal anchors; incompatible changes preserve the existing cache and records. There is no automatic purchase or membership flow. See [Content access](CONTENT_ACCESS.md) for the correction and its evidence boundaries.

Managed sleep uses a resident application handoff by default. The application persists reading state, quiesces account work and hardware helpers, and asks the backend to cancel pointer input, release its touch descriptor, and stop drawing. Only a matching private generation/nonce/cycle acknowledgement permits the supervisor to stop all application threads and return control to the stock screensaver. After native dismissal, the same process reacquires touch and rewrites its complete retained frame before work resumes. Fonts, the paginator, drafts, and the current route remain in memory. Saved-session cold recovery remains a bounded fallback, and an ordinary manual launch still opens Library. The `2fd3/799d` Scribe 2022 / firmware 5.19.5 trial established its scoped same-process Reader/touch/theme return. The later v10 native-correction trial and normal-mode deployment have separate evidence in [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md). Earlier resource and cold-resume observations retain their original artifact scope.

The accepted 2026-11-10 (Asia/Shanghai) night-mode decision follows the reviewed
KOReader framebuffer implementation. The application owns an independently
persisted preference; native system settings do not replace it at startup or
wake. The Kindle backend owns the single output transform, using
`desired_dark XOR observed_kernel_inverse` for canonical RGB. Temporary hardware
inversion uses `FBIOPUT_VSCREENINFO` grayscale 1/2 only for the explicitly allowed
Scribe 2022 / firmware 5.19.5 target with actual 8bpp and checked layout; other
targets use software inversion. Ordinary toggles use the existing render
pipeline and a checked full redraw, without SYSTEM IPC, dynconf writes, native
window-manager continuation, process suspension, work quiescence, or touch
release. This candidate passed its scoped remote software gates and produced
ARM application `d221d65d` and development ZIP `dae7dd52`, with production
launcher `a5fec72f`. An isolated Scribe 2022 / 5.19.5 R1 trial observed both theme
directions, same-process stock sleep/wake, automatic portrait orientation, and UI
exit. The user estimated each confirmed theme transition at at most one second
with stable illumination. An inconclusive first click and the immediate native
restoration guard failure retain their separate evidence. An isolated reopen
confirmed the saved light startup and native-dark UI exit. The normal entry now
uses `a5fec72f` / `d221d65d` / `a235b41c`; the user confirmed the original Library
and books, display and touch, and native-dark UI exit. Database/WAL/SHM hashes
were unchanged across the public-file replacement. The scoped physical, reopen,
and deployment gates are closed. No launcher exit code was captured for the
unsupervised normal launch. A subsequent user-authorized clean installation
removed the previous normal configuration, account database, and cache before
reinstalling the same accepted runtime with fresh local device configuration.
Its new login page was confirmed; no previous account state was inherited. See
[KOReader night-mode alignment](KOREADER_NIGHT_ALIGNMENT.md) for counts, the
retained initial launcher failure, and build-time frozen fingerprints.

The project-owned native frame guard remains separate from the Rust/Slint main
process for real stock screensaver handoff. Before the backend acknowledges
Suspended, it restores the latest native grayscale baseline and rewrites the
retained canonical frame with matching software compensation, allowing the C
helper to capture coherent RGB. Wake observes actual kernel metadata before
reapplying the application preference, repainting, and acknowledging input.
Ordinary active exit restores the startup hardware state; exit after a completed
native yield performs no further mode write. Native day/night settings remain
unchanged. The small helper uses the existing device X11/glibc runtime, with no
firmware libraries redistributed.

The earlier R3 SYSTEM path exposed native Library. Later v8/R9 and v10 trials
closed their scoped visual-handoff gates. Their former normal installation used
launcher `15aab68c`, application `8010a217`, and helper `a235b41c` before the R1
renderer-owned update.
Their approximately five-second SYSTEM response and accepted native illumination
presets are historical evidence and policy for that superseded path. They do not
verify the new renderer-owned candidate. [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md)
preserves the full artifacts and acceptance boundaries.

Report settlement validates the measured run's account, book, and start time
before trusting its durable allocation cursor, including a zero-demand return.
Catalog SQL commit installs the corresponding active or retained Reader before
optional cache and task work can fail. The application's own Article request
is settled on every completion exit without retiring another receiver.

The later authorized [active portrait orientation](ACTIVE_ORIENTATION.md) keeps
a same-launch native scanout anchor and a dedicated event-driven sensor owner.
Rendering compensates holding versus scanout; raw touch combines holding with
the independent application rotation. A transition cancels stale input and
requires a complete rewritten frame before fresh mapped contacts. Calibration
and managed-wake records use the existing private generation/nonce/cycle fence.
These changes preserve Reader state and native display ownership during an
active flip. That orientation checkpoint passed its controlled execution and
ARM/private-bundle checks without replacing the private trial at that time.
Later native-correction deployments retain their own artifact-specific record.

## Stack

| Area | Choice | Status |
| --- | --- | --- |
| Application language | Rust | Accepted |
| UI | Slint with project-owned components and design tokens | Accepted; reviewed development versions are pinned; final design and production acceptance pending |
| Asynchronous work | Tokio | Accepted |
| HTTP and TLS | reqwest with rustls | Accepted; upstream behavior and device TLS operation pending |
| Local structured state | SQLite | Accepted; schema and Rust binding implemented in the development candidate |
| Local resources | File-based cover and thumbnail cache | Accepted |
| Kindle integration | Project-maintained platform module | Accepted; reviewed development adapter with model/firmware-specific acceptance still incomplete |
| Build direction | Linux cross-compilation; Rust musl and cargo-zigbuild are the candidate path | Development ARM artifacts exercised; production toolchain and support acceptance pending |
| Distribution | Independent KPM package and launcher | Accepted; distribution remains a development candidate |
| Custom API server | Thin service only when a verified requirement needs it | Conditional |
| Server database | Introduce only for an identified persistence requirement | Deferred |
| Server object storage | Not part of the current architecture | Removed |

## Client responsibilities

### Presentation

Slint renders the bookstore, search, book details, library, and capability-dependent task views. It receives application state and emits user intents. Keep networking, SQL, and device-specific operations outside UI components.

KOReader's visual and sizing conventions are references for our own design system, not a requirement to use its Lua widget implementation.

### Application and domain

Rust use cases coordinate catalog browsing, search, cover loading, account state where supported, local library state, and verified acquisition flows. Keep upstream response structures out of the core book and task models.

Use explicit states for asynchronous work and persist tasks that need to survive interruption. Update UI state through a mechanism supported by the selected Slint event loop and Kindle backend. Cross-thread event delivery is part of the prototype acceptance scope, not an assumed property of every backend revision.

### Source adapters

Place all WeChat Read-specific authentication, request construction, response parsing, and capability discovery in a replaceable source adapter.

The confirmed source agreement currently covers cover images. Login, catalog, search, content delivery, offline content, and reader integration require separate contracts and evidence. Do not invent endpoints, response schemas, export formats, or access capabilities.

The adapter exposes only verified capabilities. A cover URL does not imply that an EPUB or PDF acquisition endpoint exists.

### Local persistence

SQLite stores metadata and indices, such as book records, upstream identifiers, cover locations, cache status, and supported task state. This is a proposed responsibility boundary, not a frozen schema.

Files store cover images and generated thumbnails. Any book-content cache is conditional on the verified upstream delivery and reading model.

Cache operations should support bounded storage, eviction, stale-entry recovery, and temporary-file handling. Use atomic replacement where supported. Image dimensions are known to the UI before loading completes, so image arrivals do not change the layout.

### Kindle platform module

Own the interfaces for display updates, touch and input, geometry and scaling, rotation, lifecycle, and reader handoff.

A candidate implementation is the backend fork used by Jobman, which has author-reported tests on PW5, PW6, and first-generation Scribe combinations. Review and pin reused code. These reports do not establish this application's support matrix or support for every Scribe generation.

Musl-based builds may reduce user-space library dependencies, but still depend on device kernel interfaces, display and input protocols, permissions, and any selected Kindle services. Static linking does not solve lifecycle or device compatibility by itself.

#### Removed Quick and Wi-Fi experiments

The user removed the pull-down Quick panel and in-app Wi-Fi feature. Their UI/controller work and the project-owned C Wi-Fi helper are user-removed experiments, not current product capabilities or pending deliverables. The product build and package require the main Rust ELF, the independent native frame guard, launcher, and notices; they no longer require or package the Wi-Fi helper or firmware libraries.

The `582b` and `3ec60` candidate records, 17 synthetic helper checks, and probe/mock/VM records remain historical experiment evidence in [Verification](VERIFICATION.md). Their ABI findings and results are preserved without making the helper a Jobman or KOReader runtime dependency. The Rust/Slint application, conditional-server decision, and unselected project license remain unchanged.

### Reader bridge

Keep reading and bookstore responsibilities separate. Before choosing a handoff, confirm how the upstream delivers content and which reader can consume it.

Persist navigation state, release display and input resources as needed, and restore the stock UI on exit or handoff. Do not assume that a downloaded file can be opened by the system reader solely because its cover was available.

## Data flows

### Cover loading

1. Obtain a confirmed upstream cover reference.
2. Resolve a valid local cache entry or fetch the image.
3. Decode and generate a thumbnail appropriate for the display slot.
4. Store the file and update the SQLite cache index.
5. Notify the UI without changing the reserved cover geometry.

### Content acquisition

1. Establish that the authenticated upstream supports the requested acquisition or reading operation.
2. Determine its actual format and lifecycle.
3. Implement resumability, integrity handling, and persistence when the transport permits them.
4. Mark content as ready only when the selected reader can actually consume it.

This flow remains conditional until upstream research establishes the required capabilities.

## Optional server

Prefer direct client integration where the verified upstream authentication and API contract permit it. Add a thin service only for a concrete requirement, such as a required callback, necessary proxy, conversion, or synchronization.

Rust and Axum remain a candidate if such a service is needed. PostgreSQL may be selected when a server-side data model is actually required. Neither is a mandatory deployment dependency today.

Do not host cover images merely because they are displayed by the client. A future requirement to host project-owned resources must be evaluated and recorded separately.

## Superseded choices

| Earlier proposal | Current decision |
| --- | --- |
| Hosted books and covers with S3-compatible object storage | Removed; upstream covers are cached locally |
| Mandatory Rust/Axum server and PostgreSQL | Deferred; justify each server responsibility first |
| GTK2 as the default for a new application because tutorials exist | Not selected; retained as research context |
| KOReader plugin as the product form | Not selected; the product is an independent application |
| Treating Jobman as an SDK or runtime dependency | Corrected; it is an application reference |
| Treating a low-star backend or recent commit as proof of production readiness | Replaced by source review and model/firmware-specific evidence |
| Pull-down Quick panel, in-app Wi-Fi, and the project-owned Wi-Fi helper | Removed at the user's request; retained only as historical experiments |
| Immediate SYSTEM night-mode synchronization and native day/night illumination presets | Superseded on 2026-11-10 by independently saved application night mode and renderer-owned temporary inversion; manual light controls remain |

## Implementation note: 2026-11-03

The user authorized application implementation from the design handoff v0.2. The accepted independent Rust/Slint, local SQLite/cache, replaceable source adapter, and project-owned Kindle boundary remain in place. The handoff adds an in-process reader and publication views; `src/reader.rs` and `src/article.rs` own layout while application controllers own navigation and persistence. No Jobman runtime, KOReader plugin, custom server, S3 bucket, or server database is required.

The development candidate uses Slint 1.17.1 and a reviewed vendored copy of the previously identified backend revision, with small documented refresh and dependency-compatibility patches. `vendor/slint-backend-kindle/UPSTREAM_REVISION` and `PATCHES.md` identify the source and local differences. `Cargo.lock` makes this development candidate reproducible. These choices do not freeze a production support matrix or establish hardware acceptance.

The WeRead adapter follows the source revision recorded in `API_REFERENCE.md`. Its credential-dependent capabilities and local tests are source-derived implementation evidence; they do not establish live-account access or a portable upstream book-export format. Local imports support unencrypted EPUB text and UTF-8 TXT, with explicit archive limits and character anchors. Full EPUB/CSS rendering and a system-reader handoff are separate capabilities.

## Current implementation and acceptance

Automatic brightness was explicitly removed from the product, design-fidelity,
and acceptance scope. Manual brightness and device-supported warmth remain
accepted controls. Do not treat ambient-light automation as unfinished work or
an acceptance blocker.

Application night-mode changes do not copy, restore, or select light profiles
and do not set ambient automatic brightness. Manual brightness and warmth
controls retain their existing validated setter range. The earlier acceptance
of native day/night presets applied to the superseded SYSTEM path. No explicit
light setter exists in an ordinary candidate toggle. The R1 user reported stable
illumination during the two confirmed transitions; this does not establish an
absence of every hardware frontlight transient.

Implementation is still being iterated and has not completed acceptance. The accepted architecture above remains in force. Source behavior, remote checks, firmware-emulator observations, and physical-device observations are separate evidence classes; none establishes a released support matrix.

The native startup path presents the project name and loading dots before bundled font decompression. Dot updates are limited to 0.8 Hz and a local display region. A temporary framebuffer worker is joined before backend initialization mutates the process environment and before the normal event loop takes framebuffer ownership.

An ordinary start opens Library when a configured session, local books, or demo data exists; otherwise first use opens Login. A new launch after explicit exit must not implicitly reopen the previous book. Only a managed recovery requests Reader restoration. The physical experience-test wrapper uses Library on a fresh start and preserves the supervisor's sleep-resume intent.

The Reader opens with its notes sidebar hidden and updates continuous drag selection on an 80 ms cadence. Its TOC download icon and status text share a vertically centered row. Reader brightness and independently saved application night mode remain in scope. Startup feedback uses software compensation for actual native kernel inversion before renderer ownership begins. The renderer then applies the desired appearance exactly once, with software fallback on unapproved framebuffers. These are source behaviors with separate remote and physical gates; see [the Reader record](READER.md) and [Native theme continuity](NATIVE_THEME.md).

Sleep recovery belongs to the project-owned supervisor and application lifecycle. The default resident path requires saved local work and a matching private input/display acknowledgement before application threads are stopped for the native screen saver; native dismissal resumes the same process and retained frame. The saved-anchor restart-and-restore path is a bounded cold fallback: its checkpoint must be committed and the old application must exit successfully before a new generation can request that anchor. Explicit application exit and USB takeover take precedence over recovery. Physical `2fd3/799d` observations establish only their recorded resident paths; newer resource candidates, physical USB, full-device reboot, and other model/firmware combinations retain separate verification gates.

### Evidence boundaries

See [Physical testing](PHYSICAL_TESTING.md) and [Verification](VERIFICATION.md) for the current execution records, acceptance gates, counts, and report links. [The Reader record](READER.md) retains technical case details and measurement definitions.

- The physical Scribe 2022 firmware 5.19.5 offline demo candidate `61fa9afd` established the recorded Library, reading, saved-note, exit, and stock-UI restoration observations. The intermediate `9a13abe6` candidate did not meet the user's continuous-underlining expectation. Independent native Wi-Fi and Quick observations belong to the removed experiments.
- Desktop UI and Clipboard evidence for `e11d74c23e309f2aa4e1daa6c4e33fe3d611f747d7e5c904a40bd2537eb24b4c` predates the additional checkpoint regression fix; Reader source was unchanged by that fix. The central verification record keeps this artifact association explicit.
- The earlier firmware-emulator artifact `daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3` has its own selection and dark-mode observations and an unsuccessful native-return attempt. That historical failure is not the current candidate's result or a general finding that native QMP input is unsupported.
- ARM artifact `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` with launcher shell `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` passed the documented Scribe 2022 firmware 5.20.1 VM software cases for QMP selection, software dark mode, Quick and Wi-Fi returns, and same-anchor powerd sleep/wake. These runs used the existing XInput, input-grab, and driver configuration and have new evidence distinct from `daae495c`. The Quick and Wi-Fi cases are retained as removed-feature history.

The current VM cases use the documented [VM-only boot-recovery environment](SCRIBE_EMULATOR.md#current-environment-recovery-after-the-six-case-run). The earlier unpatched firmware-emulator run retains its original environment and results.

The earlier artifacts, including the preceding `095f` overlay candidate, retain their own execution records. They do not make the removed Quick/Wi-Fi work part of the current delivery. Reader brightness, night mode, startup feedback, continuous selection, and managed sleep remain in scope. Their physical gates stay scoped in the central records; USB was skipped in the current physical-device round, and overall acceptance remains incomplete.

## Production version policy

Freeze language, library, backend, and toolchain revisions after the target prototype establishes a working combination. The researched Jobman snapshot is a reference baseline, not an automatically accepted dependency set.

The existing research noted Slint 1.17.1 and backend fork revision `03aa48d3531e9eae7b360bdc24e90a72e9f307a4` in that snapshot. Confirm the combination for this application before adopting it.
