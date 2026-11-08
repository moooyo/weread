# Kindle Application Research

Research record date: **2026-11-03** (Asia/Shanghai).

This document records the primary-source findings used to choose the initial direction for the independent weread Kindle application. It separates source-code capability, author-reported device testing, packaging support, and our own verification. Repository activity dates below describe the inspected snapshots; they are not promises about later revisions.

WeChat Read (WeRead) identifies the upstream cover source. The lowercase name weread identifies this project. The upstream role beyond covers remains an open question.

No application code was implemented as part of this research. No local or remote tests, builds, smoke tests, or runtime probes were run. Statements about device testing are attributed to the respective project authors.

## Decision supported by the research

The selected direction is an independent Rust + Slint client, with an application-owned Kindle platform boundary and an initially pinned candidate backend derived from the fork used by Jobman. This is a project decision, not a claim that Rust + Slint has the largest share of the Kindle community.

Jobman is an application and a useful source of device-adaptation evidence. It is not a mandatory application SDK or a service that our client must install or run. The reusable third-party layer is the Kindle backend; we can review, pin, patch, or eventually replace it behind our own interface.

The application should have its own package identity, installation directory, launcher, icon, data directory, and lifecycle. KPM is the selected distribution channel. Its package platform labels do not change a binary's ABI or establish hardware compatibility.

## Evidence standards

- A repository's stars measure GitHub attention, not downloads, installed devices, market share, or correctness.
- A recent commit demonstrates activity, not successful builds or successful device testing.
- A configured CI workflow does not establish that the workflow passed.
- A device-name check or a display-driver structure in source code does not establish a successful test on that device.
- Emulator coverage and physical-device coverage are distinct.
- An upstream release tag, a downstream package version, a package platform name, a compiler target, a firmware version, and a device generation identify different things.
- An Atom feed's updated timestamp is not necessarily a release's publication date. An empty release feed is not a release.
- Links pinned to commits or release tags are preferred for reproducible claims. Moving documentation links are identified where a pinned source was not retained.

## KOReader: the mature LuaJIT and native-code route

KOReader is a mature, actively maintained example of a complete Kindle application stack. Its frontend uses Lua and its own widget toolkit, with LuaJIT, LuaJIT FFI, and C/C++ components beneath it. It is not a GTK application.

Its Kindle port includes framebuffer rendering, device-specific refresh ioctls, Linux input handling, and integration with LIPC, sysfs, rotation, sleep/wake, and the Amazon framework. Its release bundle includes the Lua application, LuaJIT, native libraries and tools, resources, and launch scripts. It is not a single self-contained static executable.

Relevant primary sources:

- [KOReader development guide](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/doc/Development_guide.md).
- [KOReader porting guide](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/doc/Porting.md).
- [Kindle device implementation](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/frontend/device/kindle/device.lua).
- [Kindle power implementation](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/frontend/device/kindle/powerd.lua).
- [Inspected development commit](https://github.com/koreader/koreader/commit/7649e93f5ee2a5dfc3c69bacf0385aac37352949), dated 2026-11-02.

### Four Kindle build targets

The inspected stable tag was v2026.07.1, with main-repository commit 9192014d8bd82a91dc1012473be0f238dedfdb54 and koreader-base commit 6e4bc81af4e04f78d81677a323a780a71b29702a.

| KOReader target | Intended family or userspace distinction | Cross-compiler triplet | CPU and floating-point ABI |
| --- | --- | --- | --- |
| kindle-legacy | K2, DX/DXG, K3 | arm-kindle-linux-gnueabi | 32-bit ARMv6J, ARM1136JF-S, VFP, ARM mode, softfp |
| kindle | K4, Touch, PW1 | arm-kindle5-linux-gnueabi | 32-bit ARMv7-A, Cortex-A8, NEON, Thumb, softfp |
| kindlepw2 | PW2-era and later devices using the older userspace ABI | arm-kindlepw2-linux-gnueabi | 32-bit ARMv7-A, Cortex-A9, NEON, Thumb, softfp |
| kindlehf | The newer hard-float userspace, documented for firmware 5.16.3 and later | arm-kindlehf-linux-gnueabihf | 32-bit ARMv7-A, Cortex-A9, NEON, Thumb, hard-float |

The target name is kindle-legacy, not kindlelegacy. The koxtoolchain names also differ: its kindle toolchain corresponds to KOReader's kindle-legacy target, while kindle5 corresponds to KOReader's kindle target.

The inspected GLIBC_VERSION_MAX policies are 2.5 for legacy, 2.12 for kindle/kindlepw2, and 2.20 for kindlehf. These are build compatibility policies; they should not be presented as a complete statement about every device's installed libc.

Sources:

- [Pinned koreader-base target and compiler definitions](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/Makefile.defs#L117).
- [Pinned koxtoolchain target mapping](https://github.com/koreader/koxtoolchain/blob/6a4407a3cbb5fb2e3513e63c0fd12fcdbe1380af/README.md#L50).

### Build, package, and launcher boundaries

The inspected stable build path cross-compiles on Linux/Docker, using device-specific toolchains and a make-driven release process. The historical CI invokes kodev release for each target; the release path ultimately assembles the Kindle package.

For v2026.07.1, all four targets share make/kindle.mk. The stable packaging rules generate ZIP and OTA archives. The ZIP contains koreader/ and extensions/; legacy also includes launchpad/. The KUAL extension is included, but the KUAL application itself is not.

The launcher invokes reader.lua, whose shebang selects the bundled LuaJIT. It also handles Kindle framework coordination, logging, updates, and application restarts.

Sources:

- [Historical Kindle CI configuration](https://gitlab.com/koreader/nightly-builds/-/blob/443f8336c995586c9b74f07d73d5de3ba6ff9d8f/.gitlab-ci.yml#L39).
- [Historical release build script](https://gitlab.com/koreader/nightly-builds/-/blob/443f8336c995586c9b74f07d73d5de3ba6ff9d8f/build_release.sh#L16).
- [Stable Kindle packaging rules](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/make/kindle.mk).
- [Stable Kindle launcher](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/platform/kindle/koreader.sh#L329).
- [Stable Lua entry point](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/reader.lua#L1).

The inspected development branch has a later GitHub Actions release workflow and ZIP-only Kindle packaging. Those later changes must not be retroactively attributed to v2026.07.1.

KOReader remains a valuable implementation reference, but this project has chosen an independent client rather than a KOReader plugin or fork.

## Rust + Slint: a viable current route with a separate hardware adapter

Slint is a general GUI framework. slint-kindle-backend is a third-party implementation of Slint's platform interface. It is not the only permitted way to develop Kindle software.

A minimal independent application uses Slint plus the Kindle backend, installs that backend, and runs its own window. It does not depend on Jobman.

Sources:

- [Minimal independent application dependencies](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/examples/minimal/Cargo.toml).
- [Minimal independent application](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/examples/minimal/src/main.rs).
- [Backend installation through Slint's platform interface](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/src/lib.rs#L185).

### Upstream backend: explicitly experimental

The inspected upstream commit is 864bb5e3adec8b7728cbc9d5c39e883334a1b3ff, dated 2026-09-27 23:06 UTC. Its Cargo version is 0.7.1, with Slint 1.18 and Rust 1.92 requirements.

The README explicitly calls the backend experimental. Its reported tested devices are PW1, PW3, PW4, Kindle Touch, and Oasis 3. Source code includes an MTK refresh ABI associated with newer devices, including PW5 and Scribe, but the upstream README does not thereby establish PW5, PW6, or Scribe test coverage.

The backend uses Slint's software renderer, direct framebuffer access, display-driver ioctls, Linux input events, and poll-based waiting. Fonts must be supplied by the application.

Sources:

- [Pinned upstream README and device list](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/README.md#L175).
- [Pinned upstream dependencies](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/Cargo.toml).
- [Framebuffer implementation](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/src/framebuffer/mod.rs).
- [Input implementation](https://github.com/sverrejb/slint-kindle-backend/blob/864bb5e3adec8b7728cbc9d5c39e883334a1b3ff/src/touch.rs).

### Jobman: an actually distributed application with relevant device reports

The inspected Jobman commit is 9727c0e0a060c5d7e6dfeae2d572b69b6f556753, dated 2026-11-01 14:29 UTC. Its application version is 1.2.2. Its README documents KPM installation and still labels the project WIP.

The author reports testing in default and recovery modes on these directly relevant combinations:

| Device | Reported firmware |
| --- | --- |
| PW5 | 5.17.1.0.3 |
| PW6 | 5.20.1 and 5.19.2 |
| KS1 | 5.20.1 |

These are author-reported combinations, not our own test results, and do not establish coverage for all Scribe generations.

Jobman uses Rust 2024 edition and Slint pinned to 1.17.1. Its Cargo patch selects KindleTweaks/slint-kindle-backend-patch rather than the unmodified upstream crate. Cargo.lock pins the backend to 03aa48d3531e9eae7b360bdc24e90a72e9f307a4, dated 2026-11-01. This is the initial candidate revision to review behind our own platform interface.

The fork supports 8bpp and 32bpp framebuffer formats, probes Legacy/Rex/Zelda/MTK update ABIs, and uses GC16 full refreshes plus AUTO dirty-region partial refreshes. Its 32bpp path copies grayscale into RGB channels; that is not evidence of a color UI implementation. Input is converted to a single-pointer Slint model. The reviewed implementation does not establish Scribe-specific pressure, tilt, or handwriting support.

Sources:

- [Jobman pinned README and tested devices](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/README.md#L36).
- [Jobman application dependencies](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/Cargo.toml).
- [Jobman lockfile](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/Cargo.lock).
- [Pinned backend commit](https://github.com/KindleTweaks/slint-kindle-backend-patch/commit/03aa48d3531e9eae7b360bdc24e90a72e9f307a4).
- [Fork framebuffer implementation](https://github.com/KindleTweaks/slint-kindle-backend-patch/blob/03aa48d3531e9eae7b360bdc24e90a72e9f307a4/src/framebuffer/mod.rs#L67).
- [Fork input implementation](https://github.com/KindleTweaks/slint-kindle-backend-patch/blob/03aa48d3531e9eae7b360bdc24e90a72e9f307a4/src/touch.rs#L57).
- [KPM manifest](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/companion/jobman_1.2.2_kindlehf/manifest.json).

Jobman's GUI build uses armv7-unknown-linux-musleabihf with cargo-zigbuild. Its full bundle also includes SSH components with different build choices. Therefore the complete Jobman distribution must not be described as a single fully static executable.

The launcher still coordinates firmware services and UI. Jobman's rootless restriction is a property of that application's installer; it must not automatically be applied to every independently developed Slint application.

Sources:

- [Jobman build script](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/build.sh#L29).
- [Jobman launcher](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/companion/dist/documents/jobman.sh#L17).
- [Jobman installer](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/companion/jobman_1.2.2_kindlehf/install.sh#L3).

### What static musl builds do and do not provide

A static musl application can reduce dependence on the device's installed glibc. It still depends on compatible kernel system calls, framebuffer geometry and pixel layout, vendor display ioctls and their exact structures, Linux input protocols, device permissions, and a suitable lifecycle integration.

Static linking does not turn the display into a generic desktop display or eliminate firmware-specific work. It also does not imply that Kindle's system userspace itself uses musl.

The app may reuse a backend, implement its own Slint platform adapter, or implement a custom renderer. Ignite provides a separate Rust custom-rendering example, rather than evidence for Slint adoption:

- [Ignite compatibility matrix at the inspected revision](https://github.com/markrai/ignite/blob/3279dca16b2af4f42fa31ed1e7d16227e9c7aad1/docs/compatibility.md).
- [Ignite framebuffer and display-driver implementation](https://github.com/markrai/ignite/blob/3279dca16b2af4f42fa31ed1e7d16227e9c7aad1/ignite/kindle/kindle-platform/src/linux/display.rs).
- [Ignite runtime LIPC loading](https://github.com/markrai/ignite/blob/3279dca16b2af4f42fa31ed1e7d16227e9c7aad1/ignite/kindle/scribe-lifecycle/src/lipc.rs).

## React Native: real system integration with limited hardware evidence

react-kindle is a current community wrapper around Kindle's native React Native runtime. The inspected commit is 158ba72cad47cb3007a5b17da75220404a1f78de, dated 2026-10-13.

The inspected stack uses React 17.0.2, React Native 0.66.2, and Hermes bytecode version 84. Metro output is compiled with hermesc and launched through KPPSimpleLauncher. This is not a browser DOM application.

The author's compatibility record distinguishes simulated and physical coverage. Simulated coverage includes PW3, PW4, Basic 2024, and Scribe 3 combinations. The recorded physical-device coverage is KT3 on firmware 5.16.2.1.1, for a subset of controls, editing, lifecycle, and chrome behavior.

This is evidence that third-party use of the native RN runtime is possible. It is not sufficient evidence to promise PW5/PW6/Scribe production compatibility or to call it the established majority stack.

Sources:

- [Inspected react-kindle commit](https://github.com/katadelos/react-kindle/commit/158ba72cad47cb3007a5b17da75220404a1f78de).
- [Pinned compatibility and hardware coverage](https://github.com/katadelos/react-kindle/blob/158ba72cad47cb3007a5b17da75220404a1f78de/docs/guides/compatibility.md).
- [Pinned build and launch implementation](https://github.com/katadelos/react-kindle/blob/158ba72cad47cb3007a5b17da75220404a1f78de/packages/react-kindle/bin/react-kindle.cjs).
- [Pinned component documentation](https://github.com/katadelos/react-kindle/blob/158ba72cad47cb3007a5b17da75220404a1f78de/docs/components/index.md).

## Web applications: modern browser and Mesquite WAF are different runtimes

ReKindle demonstrates an active HTML/CSS/JavaScript PWA route in the Kindle system browser. The inspected commit is 6cf122efa5e1f126fe17ebe017be4e99d78140a9, dated 2026-10-27.

Its main edition targets the Chromium browser available on firmware 5.16.4 and later and uses modern JavaScript. Its Lite edition targets the older Mesquite/WebKit environment and ES5. The inspected README does not provide a PW5/PW6/Scribe per-model testing matrix. Its optional full-screen script is explicitly experimental with limited physical-device testing.

Sources:

- [Pinned ReKindle README](https://github.com/ReKindleOS/ReKindle/blob/6cf122efa5e1f126fe17ebe017be4e99d78140a9/README.md).
- [Inspected ReKindle commit](https://github.com/ReKindleOS/ReKindle/commit/6cf122efa5e1f126fe17ebe017be4e99d78140a9).

Mesquite WAFs must not be treated as if they automatically acquire Chromium's capabilities when the system browser is updated. They use an older WebKit-based environment and Kindle-specific WAF objects. Chromium does not expose the same WAF object model.

A concrete recent bookstore-like reference is opds-kindle: ES5 HTML/CSS/JavaScript in a Mesquite WAF, XMLHttpRequest to a local Go HTTP service, OPDS browsing, and file downloading. Its author reports physical testing only on Basic 2022 with firmware 5.19.2. That does not establish PW5/PW6/Scribe coverage.

Sources:

- [Author's Mesquite WAF technical notes, moving documentation](https://github.com/penguins184/Penguins-Kindle-Wiki/blob/main/Mesquite-WAFs.md).
- [Pinned opds-kindle README](https://github.com/dochoithuvi/opds-kindle/blob/7b55eb22203c49e648df308acd4f6b9260119e46/README.md).
- [Pinned ES5 WAF implementation](https://github.com/dochoithuvi/opds-kindle/blob/7b55eb22203c49e648df308acd4f6b9260119e46/waf/app.js).
- [Pinned Mesquite registration and launcher](https://github.com/dochoithuvi/opds-kindle/blob/7b55eb22203c49e648df308acd4f6b9260119e46/launcher/OPDSLibrary.sh).
- [Pinned local Go backend documentation](https://github.com/dochoithuvi/opds-kindle/blob/7b55eb22203c49e648df308acd4f6b9260119e46/backend/README.md).

The modern browser, Mesquite WAF, and native React Native routes coexist. They should not be collapsed into a single JavaScript runtime or a single compatibility claim.

## KUALA and native GTK

KUALA is a C++ rewrite using native Kindle GTK, aimed at firmware 5.19.x and later with an armhf userspace. It is not a JavaScript application.

The inspected repository contains device identification for PW5, PW6, KS, and KS2. Identification code is not a per-device test report. The repository does not provide a build manifest establishing the GTK major version, so it should not be labeled definitively GTK2.

Sources:

- [KUALA README at the inspected revision](https://github.com/kasparcode/kuala/blob/f00b4dbfd88d12d6d638010214fb8c847d59267e/README.md).
- [KUALA implementation at the inspected revision](https://github.com/kasparcode/kuala/blob/f00b4dbfd88d12d6d638010214fb8c847d59267e/main.cpp).
- [Inspected KUALA commit](https://github.com/kasparcode/kuala/commit/f00b4dbfd88d12d6d638010214fb8c847d59267e), dated 2026-08-23.

Other recent author examples establish both GTK2 and GTK3 development, but not a single universal modern Kindle GUI stack:

- [C++17 and GTK2 template](https://github.com/clintharrison/template-kindle-gtk/blob/0c33ab9fd6222a87fa558973b2803736a9fc7171/meson.build), dated 2025-06-11.
- [C99 and GTK3 example](https://github.com/clintharrison/example-gtk3-kindle/blob/b9bf35df3b8d0ffc16a7d9d86d1f466e891b58d1/meson.build), dated 2026-05-18.
- [Author's GTK3 Kindle branch and SDK setup, moving branch](https://github.com/clintharrison/gtk3/blob/kindlehf/.devcontainer/setup-toolchain.sh).

The GTK3 work includes an author-maintained GTK fork and bundled shared libraries. A sysroot or SDK snapshot can therefore be one part of a community-maintained build environment, rather than a single mandatory platform release covering every application.

## Community SDK versions and build targets

KindleModding/kindle-sdk is an unofficial complement to koxtoolchain. It supplies headers, firmware-derived libraries, pkg-config metadata, and Meson cross-compilation configuration. It is not a mandatory application framework or a hosted platform service.

At the inspected repository state, there was no public GitHub release or tag sequence. The installer banner identified itself as v2.0.0. The main and fw-merge branches both carried that banner; branch names are not separate published SDK versions.

The documented current tutorial targets are kindlepw2 and kindlehf. The installer also has legacy kindle/kindle5 entries and an undocumented kindlepw4 branch with incomplete supporting tooling. These entries should not be treated as equally mature supported SDK editions.

The inspected kindlehf installer overlays libraries from Paperwhite and Scribe firmware 5.16.3. That is a sysroot source baseline, not a promise that an application works on every later model or firmware. UI framework versions, compiler versions, userspace ABI, firmware baseline, and device acceptance must remain separate decisions.

Sources:

- [KindleModding SDK documentation, moving documentation](https://kindlemodding.org/kindle-dev/kindle-sdk.html).
- [Inspected SDK installer version](https://github.com/KindleModding/kindle-sdk/blob/b4a6c99d718a7cf74935f36105c62491b4336a61/gen-sdk.sh#L225).
- [Inspected SDK target and firmware selection](https://github.com/KindleModding/kindle-sdk/blob/b4a6c99d718a7cf74935f36105c62491b4336a61/gen-sdk.sh#L263).

## Independent packaging

Our application does not need Jobman's installer, manager UI, SSH server, recovery features, or running process.

An application can expose its own library scriptlet and KUAL entry. The inspected rereader project demonstrates independent directories and launch scripts; this is a packaging example, not proof that its WeRead functionality has completed Kindle hardware acceptance:

- [Independent rereader launcher](https://github.com/RealZhouXin/wereader/blob/7dacfb2b8d670c04633d19e684e39aa6ba844551/packaging/rereader/start.sh).
- [Independent scriptlet](https://github.com/RealZhouXin/wereader/blob/7dacfb2b8d670c04633d19e684e39aa6ba844551/packaging/documents/rereader.sh).
- [Independent KUAL menu](https://github.com/RealZhouXin/wereader/blob/7dacfb2b8d670c04633d19e684e39aa6ba844551/packaging/extensions/rereader/menu.json).

KPM packages have their own ID, version, supported platform labels, and lifecycle hooks. These are package-level identities; they do not imply automatic registration as an Amazon-native app URL.

The inspected manifest-v2 packaging helper normally creates a gzip-compressed tar archive with a .kpkg extension. Package-root hooks include install.sh, launch.sh, and uninstall.sh, executed through sh. Applications manage their own launcher scriptlets. The package wraps an already built application; declaring a supported platform does not cross-compile or convert the executable.

Sources:

- [Official KPM example manifest](https://github.com/KindleModding/example_kpm_package/blob/fa4d169716d3f3f07cd2ac215002014a9b600d4b/manifest.json).
- [KPM platform filter in installation](https://github.com/KindleModding/KPM/blob/4800e09412ceee6cc85ca924dc13fb27ba867d9b/src/install.c#L516).
- [Official package-creation documentation, moving documentation](https://kindlemodding.org/kindle-dev/kpm/creating-a-package.html).
- [Inspected KPM archive creation implementation](https://github.com/KindleModding/KPM/blob/4800e09412ceee6cc85ca924dc13fb27ba867d9b/kpm-helper.py#L230).

A downstream package version must not be substituted for an upstream source revision. For example, the inspected official KOReader KPM package used package version 2026.7.2 while its bundled git-rev identified v2026.07.1; the package update addressed scriptlet packaging.

Sources:

- [Official repository manifest, moving index](https://repo.kindlemodding.org/manifest.v2.json).
- [Official KOReader Kindle HF package inspected as an archive](https://repo.kindlemodding.org/packages/koreader/artifacts/koreader_2026.7.2_kindlehf.kpkg).
- [Scriptlet packaging update](https://github.com/KindleModding/repo/commit/937d7834b5109b1eba579ee3d3965e8793699d6a).

## WeRead data-source boundary

The established product requirement is that book covers come from WeRead. No independent object-storage service is required merely to serve those covers again.

That requirement does not establish the availability of any particular catalog, authentication, content-download, progress, notes, or synchronization API. It also does not establish that a book's full text can be downloaded, that download is available for every title, or that its format can be opened by the stock Kindle reader.

The client should isolate upstream integration behind a provider boundary. Account/session handling, catalog data, cover retrieval, and any verified download capability should remain independently replaceable. SQLite stores local metadata and state; Kindle-local files store cached images and any content that is confirmed to be available.

The initial plan prioritizes direct client integration. A thin backend is conditional on a concrete need for a proxy, format conversion, or synchronization. Axum, PostgreSQL, and S3 are not preselected V1 infrastructure.

The existing rereader repository is a related implementation reference, not this project's code or a guarantee of upstream API support. Its inspected README states that it provides bookshelf functionality, has not connected the reading UI, and has not completed Kindle hardware acceptance:

- [Related project README at the inspected revision](https://github.com/RealZhouXin/wereader/blob/7dacfb2b8d670c04633d19e684e39aa6ba844551/README.md).

## Desktop testing and Kindle firmware emulation

The follow-up [testing feasibility research](TESTING_STRATEGY.md) identifies official Slint desktop and automated-testing facilities, plus a community full-system Kindle firmware emulator that lists the target device families. It proposes separate desktop, emulated-firmware, and physical-device verification layers. The emulator modifies guest behavior and does not reproduce physical panel response. No build, emulator run, or device test was performed by this project, and the candidate Slint backend's emulator compatibility remains unverified.

## Items that remain open

1. Confirm the exact Scribe generation or generations and firmware versions in scope. Jobman's KS1 report must not be generalized to all Scribe models.
2. Review and pin a compatible Slint/backend pair. The Jobman fork is a candidate based on relevant author reports, not an accepted compatibility baseline for our application.
3. Establish physical-device acceptance for Chinese font rendering and input, image-heavy lists, refresh behavior, touch mapping, HTTPS, download persistence, sleep/wake recovery, USB transitions, and exit-to-system recovery on PW5, PW6, and the specified Scribe models.
4. Confirm the upstream APIs and account flows that the provider may actually use. Full-text download and remote write capabilities remain unproven.
5. Confirm downloadable file formats and the exact reading handoff. Fetching a cover does not prove that EPUB or another format can be opened directly by the stock Kindle reader.
6. Confirm data-directory placement, storage capacity, cache eviction, and credentials handling for the supported device/firmware combinations.
7. Confirm applicable Slint and other dependency licensing before choosing a commercial closed-source distribution model. This research records a licensing review item, not a legal conclusion.
8. Keep dependency versions, compiler targets, sysroot assumptions, firmware compatibility, and KPM platform declarations documented separately.

The outcome of this research is a justified starting direction and a bounded prototype plan. It is not completed application implementation, runtime verification, or a claim that every investigated route works on the target hardware.
