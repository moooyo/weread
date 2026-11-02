# weread

Planning and research for an independent bookstore application on jailbroken Kindle devices.

This repository records the agreements reached on 2026-11-03 (Asia/Shanghai). It currently contains documentation only: no application implementation, build environment, or verified Kindle binary.

## Product direction

- Build our own application, with its own executable, interface, data directory, launcher, and KPM package identity.
- Target Paperwhite 5 / 11th generation, Paperwhite 6 / 12th generation, and Kindle Scribe. The exact Scribe generation and supported firmware matrix remain to be selected.
- Support Chinese text and search input, book browsing, cover display, search, details, and local caching.
- Add content download and reader handoff only after the actual upstream capabilities and formats have been confirmed.
- Use WeChat Read as the confirmed source of cover images. Cover availability is a user-provided requirement, not evidence that the upstream exposes downloadable book files.

## Current architecture

Use a client-first, locally cached architecture:

```text
Upstream interfaces
        |
        v
Source adapters <--> Application use cases <--> Slint UI
                              |
                  +-----------+-----------+
                  |                       |
             SQLite metadata         Local file cache
                              |
                      Kindle platform layer
```

The client uses Rust, Slint, Tokio, reqwest with rustls, SQLite, and local files. The Kindle platform layer is isolated behind interfaces maintained by this project. KPM is the initial distribution mechanism.

Cover images flow from the upstream URL to the client, then to a local thumbnail/cache pipeline. S3 is not part of this architecture.

A custom server is conditional. The first implementation should aim to use upstream interfaces directly where the verified authentication and API contracts allow it. Add a thin service only for a demonstrated requirement such as a required callback, proxy, conversion, or cross-device synchronization. Axum and PostgreSQL are not baseline dependencies.

## Documentation

| Document | Purpose |
| --- | --- |
| [Architecture](docs/ARCHITECTURE.md) | Components, data flows, ownership boundaries, and superseded choices |
| [Research](docs/RESEARCH.md) | Primary sources, relevant community projects, and evidence limitations |
| [UI guidelines](docs/UI_GUIDELINES.md) | E-ink design direction, references, and the design system still to be specified |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Discovery gates, prototype scope, and implementation sequence |
| [Open questions](docs/OPEN_QUESTIONS.md) | Decisions and capabilities that are not yet confirmed |
| [Contributor instructions](AGENTS.md) | Language, scope, and verification rules |

## Evidence and status

Research so far consists of source code, project documentation, release metadata, and static inspection of upstream archives. No application was built or executed, no upstream authentication or book-content interface was exercised, and no target Kindle was tested for this project.

Community applications such as Jobman are implementation references. Our application does not require Jobman or a KOReader plugin to run. Specific dependencies, device support, and release suitability must be established through our own implementation and target-device acceptance work.

No project license has been selected yet. Dependency licenses and the intended open-source or closed-source release model remain an explicit decision before implementation and distribution choices are finalized.
