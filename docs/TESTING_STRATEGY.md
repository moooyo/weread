# Testing Feasibility and Proposed Strategy

Research date: 2026-11-03 (Asia/Shanghai).

Status: primary-source research and an implementation proposal. This does not change the accepted Rust + Slint architecture, freeze dependencies, or authorize application implementation. No desktop application, emulator, build, test, or device probe was executed for this research.

## Conclusion

Implementation update: this document preserves the earlier feasibility investigation. Current desktop, controlled-adapter, launcher, and cross-compilation evidence is recorded in [Verification](VERIFICATION.md). Firmware-emulator and physical-device acceptance remain separate gates.

Computer-based development and testing are feasible at two different levels: a desktop Slint application and a full-system Kindle firmware emulator. Physical Kindle acceptance remains necessary for device behavior. A working desktop mode and a compatible firmware-emulator setup still need to be implemented and demonstrated for this project.

Do not wait until application development is complete to test a physical device. Display, input, event delivery, and lifecycle are early feasibility gates in the existing implementation plan.

## Source findings

### Slint desktop execution and automated testing

The official [Slint 1.17.1 backend documentation](https://slint.dev/releases/1.17.1/docs/rust/slint/docs/cargo_features/index.html) lists Windows, macOS, and Linux desktop support through Winit, including the `winit-software` renderer. This supports a proposal to share the project's UI and application logic between desktop and Kindle entry points. It does not make the Kindle ARM executable a Windows executable.

The official [testing backend documentation at v1.17.1](https://github.com/slint-ui/slint/blob/v1.17.1/internal/backends/testing/README.md) establishes two distinct capabilities:

- The default testing backend supports event-loop and interaction testing without a display. It renders no pixels and measures text using a fixed font size, so it cannot establish visual correctness.
- The MCP-oriented headless backend can rasterize pixels and capture screenshots without a display server. Its entry point is explicitly unstable. It is an optional development tool, not a production dependency decision.

The testing crate is an internal API, does not follow semver, and must exactly match the selected Slint version. Version 1.17.1 is the researched reference, not a newly frozen project dependency.

The official [platform interface](https://docs.slint.dev/latest/docs/rust/slint/platform/index.html) and [software window API](https://docs.slint.dev/latest/docs/rust/slint/platform/software_renderer/struct.MinimalSoftwareWindow.html) provide the framework mechanisms for separate platform implementations. These moving documentation links describe available interfaces; the eventual implementation must use its selected version's API.

### Full-system Kindle firmware emulation

The community [eink-emulator README at c96062d51a1a54dd445d1668e740786d160a80bb](https://github.com/katadelos/eink-emulator/blob/c96062d51a1a54dd445d1668e740786d160a80bb/README.md) describes running Kindle firmware in a customized QEMU. It lists PW5, PW6, and Scribe 1, 2, and 3, with persistent storage, pointer input, SSH, screenshots, headless/VNC access, and separate firmware instances. Firmware is supplied separately; documented downloads are available for specified models.

The [PW6 guide](https://github.com/katadelos/eink-emulator/blob/c96062d51a1a54dd445d1668e740786d160a80bb/docs/paperwhite-6.md) reports firmware 5.18.6 and 5.20.1 testing by the emulator author. It describes touch passing through an emulated FT5536G controller and the stock guest kernel driver. These are upstream reports, not this project's results or a guarantee for every firmware version.

The candidate Kindle backend opens `/dev/fb0`, maps framebuffer memory, and sends display ioctls; its input path reads Linux input events. See the pinned [framebuffer implementation](https://github.com/KindleTweaks/slint-kindle-backend-patch/blob/03aa48d3531e9eae7b360bdc24e90a72e9f307a4/src/framebuffer/mod.rs) and [touch implementation](https://github.com/KindleTweaks/slint-kindle-backend-patch/blob/03aa48d3531e9eae7b360bdc24e90a72e9f307a4/src/touch.rs).

In full-system emulation, the guest Kindle kernel handles those interfaces above the emulated hardware. The emulator's QEMU revision includes [MT8113 display](https://github.com/katadelos/qemu/blob/b75b93cdc82d60d8c3f844dd114fd38dfb481df5/hw/display/mt8113_hwtcon.c) and [FT5536G input](https://github.com/katadelos/qemu/blob/b75b93cdc82d60d8c3f844dd114fd38dfb481df5/hw/i2c/ft5536g.c) models. This makes native-backend integration a reasonable experiment. It does not establish that the selected Rust ABI, Slint version, backend, and launcher work together in this emulator.

Ordinary [QEMU user-mode emulation](https://www.qemu.org/docs/master/user/main.html) forwards system calls to the host. Running an ARM executable that way does not supply the Kindle kernel, display hardware, or firmware services required by the backend.

### Emulator limitations that require physical acceptance

The emulator's [display-data documentation](https://github.com/katadelos/eink-emulator/blob/c96062d51a1a54dd445d1668e740786d160a80bb/docs/firmware.md#display-data) explicitly states that synthetic waveforms do not reproduce physical panel response. A correct screenshot cannot establish ghosting, flicker, contrast, refresh latency, or appropriate refresh policy on a real panel.

The [guest compatibility changes](https://github.com/katadelos/eink-emulator/blob/c96062d51a1a54dd445d1668e740786d160a80bb/docs/guest-overrides.md) include disabling automatic suspend on the PW5/PW6 family, skipping a missing hibernate payload, and changing USB jobs to keep Ethernet access. Scribe families have their own modifications. Firmware emulation therefore cannot establish unmodified-device sleep, power consumption, USB transitions, or restoration behavior.

The [networking documentation](https://github.com/katadelos/eink-emulator/blob/c96062d51a1a54dd445d1668e740786d160a80bb/docs/networking.md#limits) also excludes physical radio behavior and WPA. Emulated connectivity does not establish real Wi-Fi reconnection behavior.

## Proposed verification layers

| Layer | Proposed coverage | Evidence boundary |
| --- | --- | --- |
| Rust core with controlled adapters | Application state, navigation, capability handling, retries, cancellation, SQLite, cache eviction, and interruption recovery | Mock responses do not prove upstream APIs or acquisition capabilities exist |
| Desktop Slint | Shared screens, Chinese glyph coverage, pointer/keyboard flows, pagination, fixed cover geometry, empty/offline/error states, and screenshot regression | Desktop fonts, input methods, timing, and scaling can differ from Kindle |
| Kindle firmware emulator | Candidate ARM binary startup, framebuffer/input integration, files, launcher and packaging experiments, and firmware-specific service calls | Results apply to the pinned emulator and modified guest image only |
| Physical Kindle | Panel quality and refresh, physical touch/scaling/rotation, Chinese composition and candidates, memory and latency, battery, Wi-Fi recovery, sleep/wake, USB transitions, installation, exit, and any verified reader handoff | Record each exact model and firmware; one device does not establish the entire target matrix |

The default Slint testing backend covers logical interactions. Use a rendering backend for pixel comparisons. Even rendering tests should use bundled fonts, explicit geometry and scale, and deterministic fixture images; they remain layout evidence rather than electronic-paper acceptance.

## Proposed implementation sequence

1. Add desktop and Kindle entry points that share UI, application use cases, source adapters, and persistence. Select their platform implementations separately so the desktop build does not depend on Kindle-only device interfaces.
2. Provide controlled source fixtures and simulated platform events for unavailable, offline, timeout, and recovery states. Keep simulated capabilities distinct from verified upstream contracts.
3. Before expanding feature scope, use a small native prototype to establish startup, one rendered screen, touch, Chinese text/input, and background-to-UI delivery in the candidate firmware emulator and on one explicitly identified target Kindle.
4. Develop the bookstore flows with desktop and automated feedback while repeating physical acceptance when platform behavior changes. Evaluate Kindle-specific installation and service integration in the emulator where its modified environment is suitable.
5. Before claiming support, pass physical acceptance on each advertised model/firmware combination. Preserve emulator and device reports as separate evidence.

The existing upstream feasibility workstream remains independent. Neither a mock bookstore nor a firmware emulator proves WeChat Read login, catalog, full-text acquisition, or reader compatibility.

## Windows and execution environment

Slint documents Windows desktop support. The inspected firmware-emulator instructions require a compiler, Ninja, QEMU dependencies, e2fsprogs, OpenSSH, and KindleTool, with POSIX-oriented build tooling and explicit macOS notes. Native Windows, WSL2, and Docker operation are not established by this research.

A candidate operational path is to prepare a suitable Linux verification host through `ssh test-env`, then use SSH and VNC from Windows. Confirm the remote host's operating system, dependencies, storage, and display access before relying on it; this research did not connect to or qualify that host. WSL2 is a separate candidate requiring explicit local verification authorization.

Follow AGENTS.md for all execution. Local tests, builds, validation, smoke tests, and runtime probes require explicit authorization in the current task. Otherwise use the designated remote environment; if unavailable, report verification as blocked. No emulator or physical-device setup is claimed to exist today.
