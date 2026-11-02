# Architecture Agreement

Recorded: 2026-11-03 (Asia/Shanghai).

## Decision

Build an independent Rust and Slint client for jailbroken Kindle devices. Use a modular monolith: one application process with clear internal responsibilities, background work, and persistent local state.

WeChat Read cover images are retrieved from the upstream and cached on the Kindle. The architecture does not include S3 or a hosted cover library.

The application has its own package identity, executable, data directory, UI, and launcher. Reusing a GUI framework or a device adapter does not turn it into a plugin of another application.

## Stack

| Area | Choice | Status |
| --- | --- | --- |
| Application language | Rust | Accepted |
| UI | Slint with project-owned components and design tokens | Accepted; exact design system and dependency version pending |
| Asynchronous work | Tokio | Accepted |
| HTTP and TLS | reqwest with rustls | Accepted; upstream behavior and device TLS operation pending |
| Local structured state | SQLite | Accepted; schema and Rust binding pending implementation |
| Local resources | File-based cover and thumbnail cache | Accepted |
| Kindle integration | Project-maintained platform module | Accepted; initial implementation revision is a candidate |
| Build direction | Linux cross-compilation; Rust musl and cargo-zigbuild are the candidate path | Pending target prototype |
| Distribution | Independent KPM package and launcher | Accepted |
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

## Version policy

Freeze language, library, backend, and toolchain revisions after the target prototype establishes a working combination. The researched Jobman snapshot is a reference baseline, not an automatically accepted dependency set.

The existing research noted Slint 1.17.1 and backend fork revision `03aa48d3531e9eae7b360bdc24e90a72e9f307a4` in that snapshot. Confirm the combination for this application before adopting it.
