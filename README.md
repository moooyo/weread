# weread

An independent Rust and Slint reading application for jailbroken Kindle devices.

Implementation started on 2026-11-03 (Asia/Shanghai), following the supplied WeRead for Kindle design handoff v0.2. The application shares its UI and application core between a desktop backend and an isolated Kindle backend. See [implementation status](docs/IMPLEMENTATION_STATUS.md) for completed features, verification evidence, and remaining acceptance gates.

Development and acceptance are still in progress. ARM candidate `2b39140b` and launcher `0226f613` ran on the physical first-generation Scribe / firmware 5.19.5 as well as in the firmware VM. The user confirmed startup feedback, the default body view, continuous finger selection and saving a highlight, and dark text/menu appearance. The user subsequently removed the pull-down Quick panel and in-app Wi-Fi feature. Their UI/controller and C helper work are historical experiments, with no current product delivery or acceptance gate assigned to them.

## Use the development build

The operator environment is Windows and PowerShell. Build and verification run on the designated Linux worker:

```powershell
.\scripts\verify-remote.ps1
```

Cargo builds default to 12 concurrent jobs, and development profiles enable
incremental compilation. Use `-CargoBuildJobs N` with this script, or
`CARGO_BUILD_JOBS=N` for direct worker commands, to select another limit. Reuse
the same `CARGO_TARGET_DIR` for incremental recompilation. Parallel projects
use separate source and target directories. Keep one verification driver for
this checkout and target; physical Kindle access remains independently exclusive.

Use `-Design` to include the review-defect pointer/pixel suite and its separate
offline fixture runner; `-Kindle` also stages the ARM development bundle.
The design correction baseline and results are tracked in
[Design fidelity corrections](docs/DESIGN_FIXES.md).

On that worker, an isolated offline demonstration can be rendered with:

```sh
xvfb-run -a -s '-screen 0 2200x3000x24' ./target/debug/weread \
  --demo --offline --device pw6 --screen library \
  --data-dir /tmp/weread-preview --capture /tmp/weread-preview.png
```

The actual Cargo target directory may be set through `CARGO_TARGET_DIR`. Device profiles are `pw5`, `pw6`, `scribe`, and `scribe2025`. Profiles select design geometry; they do not declare verified hardware support.

The normal data directory is `%APPDATA%/weread` on Windows and `$XDG_DATA_HOME/weread` or `~/.local/share/weread` on Linux. `--data-dir` overrides it. `--demo` uses a separate `demo/` child directory and makes no authenticated upstream writes. Put local TXT or EPUB files in the active data directory's `imports/` folder and choose **Import local books** or **Scan and associate local books** in the application. The interface itself is Chinese.

A fresh launch, including a launch after voluntary exit, opens Library when a session, local books, or demo data is available. First use without any of these opens Login. Managed sleep/wake resumes the previous Reader position instead. The physical experience-test entry follows the same Library entry rather than forcing Reader.

The operator's normal-mode private trial uses the **WeRead for Kindle** document entry and `/mnt/us/weread/data/`, with networking enabled and no automatic exit deadline. The former supervised demo installation is retired, with its progress and notes backed up outside the repository. See [normal-mode installation](packaging/kindle/README.md#normal-mode-private-trial) for the entry and data boundaries.

Authenticated book resources use the primary WeRead origin and the individually reviewed `res.weread.qq.com` resource origin. External CDN redirects drop account cookies. Ordinary web responses update the shared session, and changed credentials are persisted locally; a resource denial is not treated as global login expiry. The [API record](docs/API_REFERENCE.md#book-chapter-image-resources) explains the scoped origin decision and its physical evidence.

[Download performance](docs/DOWNLOAD_PERFORMANCE.md) records implemented priority-1/2 page compression, task TXT hints, two-shard reads with serial fallback, prepared-image reuse, and UI/checkpoint/cache improvements. That source passed 334 remote library tests and three explicit controlled benchmarks. Its physical continuation completed the transfer workflow but included 25 previews; those results do not prove full-book completeness or a Kindle speedup. See [Content access](docs/CONTENT_ACCESS.md) for the correction and separate acceptance gates.

[Further download research](docs/DOWNLOAD_PERFORMANCE_NEXT.md) records newly reviewed official Reader-state reuse, bounded pipeline design, and recovery-cache opportunities. These remain research proposals, with no verified EPUB/TXT bulk-download route or additional device speed claim.

[Content access and preview recovery](docs/CONTENT_ACCESS.md) separates verified chapter text from previews and unverified legacy caches. Limited content remains readable, but does not count as a full download or a completed reading position. After obtaining reading rights in the phone application, use **Refresh access and complete content** in book details or the Reader notice. This only revalidates existing rights; it does not purchase a book or a membership. Migration preserves cached text, positions, bookmarks, and notes, and incompatible text is retained for safe recovery. The correction's verification and live unlock acceptance are recorded separately from the earlier transport results.

The [Kindle development bundle instructions](packaging/kindle/README.md) describe cross-compilation, launcher configuration, and the explicit device/firmware acceptance gate. Do not copy a desktop executable to a Kindle.

## Product direction

- Build our own application, with its own executable, interface, data directory, launcher, and KPM package identity.
- Target Paperwhite 5 / 11th generation, Paperwhite 6 / 12th generation, and Kindle Scribe. The initial physical test target is the first-generation Scribe released in 2022 on firmware 5.19.5; the supported model/firmware matrix remains open.
- Support Chinese text and search input, book browsing, cover display, search, details, and local caching.
- Retain Reader brightness, system night-mode control on a supported Kindle, startup feedback, continuous selection, and managed sleep recovery in product scope. Desktop night mode remains local.
- Automatic brightness is outside the accepted product and acceptance scope. Retain manual brightness and device-supported warmth controls; ambient-light automation is not an outstanding delivery gate.
- Keep content acquisition dependent on inspected source contracts and configured credentials. The current implementation downloads validated chapter text and provides its own paginated reader; it does not infer a downloadable EPUB or PDF from a cover URL.
- Use WeChat Read as the confirmed source of cover images. Cover availability is a user-provided requirement, not evidence that the upstream exposes downloadable book files.

[Native theme continuity](docs/NATIVE_THEME.md) records the correction for retaining dark native UI after explicit exit, compensating the next startup and wake, and closing every login state. Executable `2fd3d6ab2d64` and launcher `799d5bba5eb9` passed these scoped physical checks on the identified Scribe 2022 / firmware 5.19.5. The 440 core and 86 launcher tests cover additional controlled states; they do not establish other device support or a full-device-reboot result.

The subsequent night-mode requirement uses a resident system-setting transaction
and treats confirmed native state as authoritative at startup and wake. The
isolated v10 trial passed scoped visual, sleep, and UI-exit checks on Scribe
2022 / firmware 5.19.5; its approximately five-second response remains open.
System mode changes retain the Kindle's native day/night illumination presets.
The earlier normal-mode private trial has not been replaced by these tests.
[Rotation alignment](docs/ROTATION_ALIGNMENT.md) compares the accepted automatic
0/180-degree display and touch paths with pinned KOReader sources.
[Current native corrections](docs/NATIVE_FIX_ACCEPTANCE.md) records the TOC,
system-theme, and display-release verification separately from earlier builds.

The [resource lifetime review](docs/RESOURCE_LEAK_REVIEW.md) records cancellation,
fault, input-admission, and ownership corrections with same-process Linux
resource sampling. The user requested finite closeout rather than further
full-domain discovery rounds. The original two consecutive zero-finding rounds
were not achieved and are not claimed. Historical failures retain their exact
source scope; final verification and package provenance are recorded separately
from the deployed private-trial executable and its physical evidence.

The later authorized [active portrait orientation](docs/ACTIVE_ORIENTATION.md)
change shares the final bounded candidate and its own controlled tests.
Final remote verification passed 1,481 library, 52 Main, 65 backend, 15 Swash,
and 168 launcher cases; three manual library benchmarks remain ignored.
The matching static ARM development bundle and source provenance are recorded
in [Verification](docs/VERIFICATION.md). This resource task has made no Kindle
deployment, and the existing `2fd3/799d` trial has not been replaced.

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

The client uses Rust, Slint, Tokio, reqwest with rustls, SQLite, and local files. The Kindle platform layer is isolated behind interfaces maintained by this project. KPM is the initial distribution mechanism. Product packaging contains the main Rust ELF, the independent native frame guard, launcher, and notices; the removed Wi-Fi helper is no longer required or packaged.

Cover images flow from the upstream URL to the client, then to a local thumbnail/cache pipeline. S3 is not part of this architecture.

A custom server is conditional. The first implementation should aim to use upstream interfaces directly where the verified authentication and API contracts allow it. Add a thin service only for a demonstrated requirement such as a required callback, proxy, conversion, or cross-device synchronization. Axum and PostgreSQL are not baseline dependencies.

## Documentation

| Document | Purpose |
| --- | --- |
| [Architecture](docs/ARCHITECTURE.md) | Components, data flows, ownership boundaries, and superseded choices |
| [Research](docs/RESEARCH.md) | Primary sources, relevant community projects, and evidence limitations |
| [UI guidelines](docs/UI_GUIDELINES.md) | E-ink design direction, references, and the design system still to be specified |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Discovery gates, prototype scope, and implementation sequence |
| [Testing feasibility](docs/TESTING_STRATEGY.md) | Desktop testing, Kindle firmware emulation, physical-device boundaries, and a proposed verification workflow |
| [Firmware emulator](docs/EMULATOR.md) | Pinned PW6 firmware setup, native ARM execution, input and launcher checks, and remaining evidence limits |
| [Scribe 2022 emulator](docs/SCRIBE_EMULATOR.md) | First-generation Scribe firmware, 1 GiB guest execution, native input flows, memory observations, and a later VM-only boot-recovery delta |
| [Physical testing](docs/PHYSICAL_TESTING.md) | Artifact-specific Scribe observations, incomplete interaction and lifecycle acceptance, and the deferred USB gate |
| [Open questions](docs/OPEN_QUESTIONS.md) | Decisions and capabilities that are not yet confirmed |
| [Contributor instructions](AGENTS.md) | Language, scope, and verification rules |
| [Implementation status](docs/IMPLEMENTATION_STATUS.md) | Design coverage, operating instructions, and remaining acceptance work |
| [Source adapter](docs/API_REFERENCE.md) | Pinned protocol reference, credential boundaries, and source-derived contracts |
| [Persistence](docs/PERSISTENCE.md) | SQLite schema, cache limits, and interruption recovery |
| [Reading and input](docs/READER.md) | Pagination, character anchors, local import, and pinyin input |
| [Kindle platform](docs/PLATFORM.md) | Driver boundary, refresh queue, lifecycle, and compatibility evidence |
| [Fonts](docs/FONTS.md) | Bundled Chinese font sources, licenses, and reproducible preparation |

## Evidence and status

The original research record predates implementation. The application has now been built and exercised on `test-env`, including desktop rendering, controlled protocol tests, SQLite recovery, Chinese input, and ARM cross-compilation. [Verification](docs/VERIFICATION.md) keeps remote unit, desktop, clipboard, launcher, and firmware-VM results bound to their source or binary. The earlier desktop checks predate the latest checkpoint regression fix and are not validation of every current lifecycle change.

The pinned PW6 5.20.1 firmware-emulator experiment now exercises the native ARM application, QEMU-injected touch, reading navigation, and stock-UI restoration. Its named experimental display configuration is separate from the design reference geometry. These results do not establish physical-device compatibility; see [the experiment record](docs/EMULATOR.md).

The same ARM executable also passed six native flows in a separate Scribe 2022 firmware 5.20.1 instance with its original 1 GiB RAM configuration and the nominal `scribe` display profile. The [Scribe record](docs/SCRIBE_EMULATOR.md) preserves that environment and separately records the later VM-only boot-recovery delta. Recorded `2b39140b` VM cases demonstrated continuous held selection, matching light/dark Reader text masks, and recovery of a saved Reader anchor. Quick Settings and Wi-Fi return results remain historical evidence for the removed experiments. These scoped guest-software results do not complete physical acceptance.

The physical record distinguishes the earlier `61fa9afd` results from the `2b39140b/0226f613` experience run. Native Quick returned to the application, but its visible stock-UI detour did not satisfy the requested default interaction; that work and in-app Wi-Fi were later removed. Reader night-mode persistence, same-position sleep/wake, and voluntary UI exit remain dedicated physical checks. The user explicitly skipped USB testing in this round. See [Physical testing](docs/PHYSICAL_TESTING.md) for the current gates and [Verification](docs/VERIFICATION.md) for retained historical Quick/Wi-Fi/helper records, including the `582b` and `3ec60` experiments.

Community applications such as Jobman are implementation references. Our application does not require Jobman or a KOReader plugin to run. Specific dependencies, device support, and release suitability must be established through our own implementation and target-device acceptance work.

No project license has been selected yet. Dependency licenses and the intended open-source or closed-source release model remain an explicit decision before implementation and distribution choices are finalized.
