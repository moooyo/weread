# Kindle platform implementation

Recorded: 2026-11-03 (Asia/Shanghai).

Implementation and acceptance remain in progress. The historical records below apply to their identified executable and environment; they do not establish acceptance of the latest source.

## Boundary and evidence

`src/platform/` owns device profiles, application refresh policy, fixed physical scaling, hardware capabilities, and task wake guards. The UI and source adapter do not issue device commands. `DeviceControl` is replaceable; desktop execution reports unavailable hardware capabilities rather than changing a UI value and claiming that a device operation succeeded.

The initial native adapter vendors [`slint-backend-kindle` at `03aa48d3531e9eae7b360bdc24e90a72e9f307a4`](https://github.com/KindleTweaks/slint-kindle-backend-patch/tree/03aa48d3531e9eae7b360bdc24e90a72e9f307a4) with project-owned queued-refresh, clipboard, touch-scale, startup-feedback, and software-rendering patches. Its manifest requires Rust 1.92 and exactly Slint 1.17.1. The original dependency licenses and exact revision are preserved under `vendor/slint-backend-kindle`; [PATCHES.md](../vendor/slint-backend-kindle/PATCHES.md) describes the modifications. This is a reproducible candidate for implementation, not a frozen release support matrix. No physical Kindle or firmware emulator result is implied by desktop or cross-compilation results. The native emulator experiments are recorded separately in [Firmware emulator](EMULATOR.md) and [Scribe 2022 emulator](SCRIBE_EMULATOR.md).

The user supplied KOReader as the hardware implementation reference. The inspected KOReader revision is [`7649e93f5ee2a5dfc3c69bacf0385aac37352949`](https://github.com/koreader/koreader/tree/7649e93f5ee2a5dfc3c69bacf0385aac37352949). Reference behavior informs this project's independent adapter; KOReader is not a runtime dependency.

## Geometry and initialization

| Profile | Logical dimensions | Physical dimensions | Layout |
| --- | --- | --- | --- |
| `pw5` | 618 x 824 | 1236 x 1648 | Paperwhite |
| `pw6` | 632 x 840 | 1264 x 1680 | Paperwhite |
| `scribe` | 930 x 1240 | 1860 x 2480 | Scribe |
| `scribe2025` | 990 x 1320 | 1980 x 2640 | Scribe |

These are design profiles from the supplied v0.2 tokens. They are not verified device identities. A native launch checks the framebuffer's actual `FBIOGET_VSCREENINFO` resolution against the selected profile before taking over the renderer. The read-only request uses Linux's 160-byte `fb_var_screeninfo` ABI; no framebuffer mode is written.

All target design profiles use 1 dp = 2 physical pixels. `configure_window()` dispatches Slint's `ScaleFactorChanged` event with 2.0 for native execution and 1.0 for a desktop design preview. Desktop dimensions are set in logical units. The larger layout shows more content and does not increase control sizes proportionally.

Native startup first validates the selected geometry, then draws the project name and startup status from embedded grayscale artwork before decompressing the runtime fonts. A temporary framebuffer owner displays the stage label and activity dots; dot animation is limited to 0.8 Hz. It does not grab touch input. The worker is stopped and joined before `install_kindle_backend(font_bytes)` stages the default font and changes `SLINT_DEFAULT_FONT`; feedback can then resume during application initialization. The startup owner is joined and releases its framebuffer before the normal backend opens display and input resources in the event loop. Keep the installed backend handle alive for additional font registration. Configure the Slint window before starting its event loop.

The vendored compatibility patch aligns its direct font dependencies with Slint 1.17.1's actual `fontique 0.10` and `fontdb 0.23` instances. The fontique instance uses `fontconfig-dlopen`, avoiding a link-time native `libfontconfig` requirement for musl builds. Fontconfig may be absent at runtime because the application registers its bundled fonts directly; no Kindle system font coverage is assumed.

The backend renders through Slint's software renderer to `/dev/fb0` and reads Linux input events. It has a thread-safe event-loop proxy. Its input adapter exposes a single pointer; this does not establish multitouch pinch, EMR pressure, eraser, or Scribe handwriting support. Those capabilities remain unavailable in `PlatformCapabilities` until an adapter provides and verifies them.

The current Reader implementation defaults to `notes_panel=false` so the body opens without the notes panel. Long-press selection updates while contact remains down, with an 80 ms update interval, including movement across lines and reverse movement. Return-context handling and selection protection preserve the intended editing context. Physical acceptance of these interactions is recorded per executable in [Physical testing](PHYSICAL_TESTING.md).

## Hardware controls

The inspected [KOReader power adapter](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/frontend/device/kindle/powerd.lua) uses the `com.lab126.powerd` LIPC properties below. This adapter invokes the corresponding installed LIPC command directly with fixed arguments, never through a shell.

| Operation | Property | Bounds |
| --- | --- | --- |
| Frontlight | `flIntensity` | 0 to 24 |
| Warmth | `currentAmberLevel` | 0 to 24 |
| Battery | `battLevel` | 0 to 100 |
| Reset idle timeout | `touchScreenSaverTimeout` | Write 1 |

The native power-property commands share a two-second userspace deadline across bounded nonblocking output, EOF, and direct-child completion. Output is limited to 4096 bytes; one additional probe byte distinguishes an exact reply from overflow. Cleanup has a separate bounded reap allowance. Up to eight Active/Retired slots retain ownership of a child whose termination remains unconfirmed, without retaining parent-side pipes or adding a reader thread. Spawn and kernel scheduling cannot be preempted by a userspace timer. The adapter returns a typed failure when a utility, property, permission, output, or deadline is unavailable. It does not guess model-specific sysfs backlight curves.

`hold_awake()` first resets the idle timer and only returns a guard if that reset succeeds. Its worker repeats the request once per minute while the guard exists. A later reset failure is retained for the application to report. This prevents normal idle expiry while a task is active; a user-requested power transition still takes precedence. A successful explicit bounded `shutdown()` join is the ownership proof used for handoff.

The application uses `hold_awake_deferred()` so the initial device command runs on its worker. It retains one Active/Stopping task guard across cancellation and replacement, and creates no replacement until the old worker has joined. Lighting and battery status work likewise retain timed-out handles for bounded retry. After those workers join, native ownership readiness separately requires the command registry to be quiescent; a retained child is not treated as stopped merely because its worker returned an error. Normal quiescence uses a zero-duration observation. Unknown brightness/warmth values do not enable a numeric control. The [resource lifetime review](RESOURCE_LEAK_REVIEW.md) records synthetic fault evidence and separates it from physical-device observations.

Terminal and complete-application initialization-error cleanup request every producer to stop before spending one shared 2,250 ms cleanup budget. Persistence or synchronous job-handler failure cannot skip hardware cleanup; unjoined workers remain owned, and successful native handoff requires an empty Active/Retired command registry. After explicit terminal cleanup, application and worker-field destruction use zero-duration retries. Controlled results belong to the [resource candidate](VERIFICATION.md#resource-lifetime-candidate); physical evidence remains scoped to the recorded `2fd3/799d` Scribe trial.

The native adapter implements Slint's public clipboard interface with independent process-memory Default and Selection slots. This supports in-application copy/paste, not a claim of stock Kindle clipboard integration.

Application dark mode is implemented at the software RGB-to-grayscale output boundary, including startup feedback. It preserves the same text layout while changing rendered pixel luminance. This is separate from hardware inversion: `hardware_inversion` remains false and `set_inverted()` returns `Unsupported`. Brightness and warmth controls are also unavailable during desktop execution.

The later [native theme correction](NATIVE_THEME.md) retains that per-process
rendering boundary while adding supervisor-owned synchronization after a
successful explicit exit. Startup and wake use a private, request-bound native
mode snapshot; software inversion is the selected application mode XOR actual
EPDC inversion. No in-process setter is sent to the paused window manager.
Read-only Scribe 2022 / 5.19.5 observations and official 5.20.1 source evidence
preceded the `2fd3/799d` physical setter, next-start, and Reader-wake checks.
Those scoped checks passed with native and persisted `Y8INV`; they do not
establish behavior on other models or after a full device restart.

## User-removed Wi-Fi and Quick experiments

The user removed the application drop-down Quick panel and application Wi-Fi from product scope. Their UI, native handoffs, and project-owned C Wi-Fi helper are historical experiments, not current features or pending deliverables. The product does not require the helper. Reader brightness, night mode, startup feedback, continuous selection, and sleep supervision remain in scope. The independent Rust/Slint application, conditional-server decision, and unselected project license are unchanged.

The earlier ABI experiment used a short-lived glibc helper and bounded private binary pipes because the static musl application could not `dlopen` the firmware's glibc `liblipc`. It used a fixed KOReader/OpenLIPC review without introducing a Jobman, KOReader, or Lua runtime or packaging OS libraries. Its 17 synthetic checks, `GLIBC_2.4` requirement, read-only Scribe 2022 firmware 5.19.5 status/probe observations, and empty-enterprise-flag handling remain historical evidence. They did not test a new real network connection.

The removed `095f` overlay candidate and the later application `582b72345d863e918e403457cd32ef25c1efc8b8dce314027930df6f2fc0880d` / helper `3ec60abc4b335cfbbd23bfe4265aadae67fd52f55446a4da5eabe6c72f75f4da` experiments retain their probe, private-mock, and VM records in [Verification](VERIFICATION.md) and [Physical testing](PHYSICAL_TESTING.md). Earlier native Quick/Wi-Fi traces below are also retained as history; none commits the product to restoring the removed features.

## Refresh policy and remaining backend work

`RefreshScheduler` models the supplied design policy independently of the display driver:

| Trigger | Plan |
| --- | --- |
| Navigation | `FlashUi`, or `Ui` if navigation flashing is disabled |
| Content page or tab | `Ui`; `Full` after six such partial updates |
| Press feedback | `Fast` |
| Overlay opened | `Ui` for its logical rectangle |
| Overlay closed | `FlashPartial` for the previous rectangle |
| Progress tick | `Fast`, at most once per second |
| Reader turn | `Partial`; `Full` at the selected interval of 1, 6, or 12 pages; 0 disables periodic full updates |
| First image display | `Full` for the image rectangle |

The [reviewed backend framebuffer module](https://github.com/KindleTweaks/slint-kindle-backend-patch/blob/03aa48d3531e9eae7b360bdc24e90a72e9f307a4/src/framebuffer/mod.rs) selects Legacy, Rex, Zelda, or MTK update ABIs and originally renders GC16 full updates plus AUTO partial updates. The vendored patch adds a thread-safe `RefreshHandle` without opening a second framebuffer for updates. Producers queue plans and wake the existing event loop; the renderer writes current dirty pixels first, then applies the resolved request through its existing framebuffer and detected ABI.

`Platform::set_refresh_sink(Arc::new(KindleRefreshSink::new(backend.refresh_handle())))` connects the installed backend. `request_refresh()` plans and submits in one call; `plan_refresh()` and `submit_refresh()` also remain available separately. `explicit_refresh` is true only after a sink is installed. Desktop execution without a sink performs no hardware operation.

UI/reading partial updates retain AUTO; fast feedback uses reviewed DU; flashing/full requests use GC16. Empty rectangles are rejected and other rectangles are clamped and rotated before submission. A2 is available in the dependency API with driver-specific values but is not selected by the initial application policy. The bounded queue conservatively requests a full refresh on overload. The dependency's wakeup write retries `EINTR`, and both event-loop exit and platform destruction mark producer handles terminated.

Queue acceptance is not a panel completion receipt. Actual ghosting, latency, driver behavior, and whether the selected policy matches each physical model/firmware still require device acceptance.

The backend's optional direct suspend schedule remains disabled. Resident managed sleep persists local work, fences account jobs, joins hardware helpers, releases pointer/touch/display ownership, and waits for a matching private generation/nonce/cycle acknowledgement before the supervisor stops application threads and restores the firmware UI. Native dismissal resumes the same process, reacquires touch, and repaints its retained frame. Saved-session cold relaunch remains a bounded fallback; checkpoint and ordinary-exit intent remain distinct. The `2fd3/799d` Scribe 2022 / firmware 5.19.5 trial established scoped same-process Reader return and touch/theme behavior, as recorded in [Verification](VERIFICATION.md#native-theme-physical-acceptance-on-scribe-2022). Later resource candidates retain separate remote evidence. USB takeover is covered by controlled launcher fixtures; physical USB acceptance remains deferred.

## Launcher and packaging

See [the packaging draft](../packaging/kindle/README.md) for directory layout, lifecycle restoration, and the development bundle command. Application files live under `/mnt/us/weread/app`; persistent state lives under `/mnt/us/weread/data`.

The launcher parses a profile and an exact profile/firmware allowlist as data. The committed allowlist example has no accepted combinations. A supervised acceptance run adds its operator-local combination, which enables that experiment without declaring general compatibility. After matching, the launcher exports `WEREAD_APPROVED_PROFILE` and `WEREAD_APPROVED_FIRMWARE`; native initialization requires them and checks framebuffer geometry.

The independent KPM identity and manifest remain drafts. No project license, automatic update service, or published compatibility matrix is selected by this scaffold.

The launcher installs its complete cleanup/signal handlers before taking the
lock. Shared finite registration guards retain the first terminal signal while
recording lock, power-observer, and application-runner ownership. Cleanup acts
only on its owned lock and captured original process identities. The power
subscriber's bounded pre-exec ownership/GO handshake permits orphan retirement
only after live ancestry was proved; mutable diagnostic PID files do not grant
signal authority. The earlier 158-case remote fixture result and historical
startup-timeout limitation retain their original source identity. The final
corrected launcher passed all 168 remote fixtures, including the seven
wake-probe regressions and three new portrait-calibration cases. The complete
backend passed all 65 cases. Their current identities and the eight earlier
fixture failures are recorded in the
[resource review](RESOURCE_LEAK_REVIEW.md#current-combined-verification).

Automatic portrait sensing is limited to upright/upside-down holding (0/180).
Manual application rotation, including landscape, remains independent; no
additional artificial debounce is used. Protected startup calibration and an
event-driven dedicated sensor precede mapped touch. Loss-aware finite drains
retain a written frame/input gate until synchronization is Ready; active flips
retain Reader and native display ownership. See
[Active portrait orientation](ACTIVE_ORIENTATION.md) for the source and remote
verification boundary. The matching final ARM bundle has not been deployed or
physically accepted; it does not replace the `2fd3/799d` private trial.

Outline rendering also admits actual downstream fill work, independently of
physical mask dimensions and bytes. The same locked Zeno 0.3.3 overlay admits
1,048,576 logical cells, 16 MiB cumulative requested storage/growth peaks, and
64 Mi logical accesses, including reserve attempts and potential relocation.
Failure stops the real producer and retains the previous caller image. The
opt-in path covers the software renderer's Outline/Alpha producer; it does not
extend the promise to unused bitmap/PNG paths or physical panel timing.

## Verification

Pure geometry and refresh tests can run on `ssh test-env` without framebuffer access. Launcher lifecycle tests replace firmware paths and service commands with isolated fixtures. Cross-compilation can establish an ARM executable's buildability. None of these checks establish actual panel response, input sizing, power behavior, or firmware compatibility. Record the device model, firmware, backend revision, procedure, result, and limitations separately for each physical acceptance run.

On 2026-11-03, six geometry/refresh unit tests passed on `test-env` in an isolated `rustc --test` harness. Both packaging shell files passed remote `sh -n`, and six isolated launcher lifecycle tests passed there. The latter cover allowlist rejection before service changes, normal cleanup, failure exit-code preservation, an already-stopped status bar, Windows configuration line endings, and power takeover cleanup. This historical fixture run did not include native framebuffer execution or a physical-device run.

An earlier full vendored dependency `cargo test --lib` run passed on `test-env`: 16 tests, including refresh/driver cases, upstream touch cases, and five clipboard cases. It used the actual nonblocking wakeup pipe for cross-thread producer tests and did not open a framebuffer. These results superseded the earlier isolated dependency harnesses for those paths; they are retained as historical evidence.

Previously reported remote regression results are 235 passing core tests and 31 passing backend tests. Desktop UI and clipboard checks passed 133 and five cases respectively with executable `e11d74c23e309f2aa4e1daa6c4e33fe3d611f747d7e5c904a40bd2537eb24b4c`. That executable predates the additional checkpoint regression repair; the Reader source was unchanged, but these results are not a run of the repaired executable or the later user-removed Wi-Fi/Quick experiments.

The recorded ARM candidate has SHA-256 `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609`. Remote `file` and `readelf` inspection identified a stripped, statically linked ELF32 ARM EABI5 executable with no dynamic section. It ran in the Scribe 2022 firmware 5.20.1 guest with launcher SHA-256 `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`. XInput, input grabbing, and the input driver were unchanged for those runs. Its emulator observations below retain that executable identity; subsequent physical and overlay-candidate results are tracked in [Physical testing](PHYSICAL_TESTING.md).

The recorded full isolated launcher fixture run passed 50 of 50 cases in 205.691 seconds. Its log is `test-env:/tmp/weread-native-queries.iYZFwu/launcher-fixtures.log`; the tested launcher source SHA-256 is `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`. These historical fixtures included native-settings paths that are no longer product features.

The preceding isolated launcher run on `test-env` under `/tmp/weread-native-failure.pwWnI0`, after removal of unsupported native cancel assumptions, passed 48 of 48 cases in 199.782 seconds. It included normal exit, sleep, USB, and application-generation regressions. Opening failure or an interface that never becomes mapped before the opening timeout retained firmware display/input ownership and exited without restarting the application. Full evidence belongs in [Verification](VERIFICATION.md).

Physical evidence is limited to the identified first-generation Scribe released in 2022, firmware 5.19.5, with demo data and application networking disabled:

- Executable `61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90` exercised the Library, Reader page turns and font-size changes, and active exit. One real note was saved and persisted to local storage. The earlier sleep interruption returned exit 143 and restored the stock UI and touch input; it did not establish the latest automatic same-position resume behavior.
- Intermediate executable prefix `9a13abe6` introduced startup feedback, the default body view, and sleep-recovery implementation. This does not establish same-position resume acceptance. The user explicitly found its drag-to-select behavior insufficient, so this is not acceptance of continuous selection.
- These earlier physical records do not accept later native-settings, dark-mode persistence, selection, or resume changes. Subsequent per-executable results are maintained in [Physical testing](PHYSICAL_TESTING.md). USB transitions were explicitly skipped by the user in this round.

An earlier Scribe 2022 firmware 5.20.1 emulator run used executable `daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3`. With QMP contact held down and moved, the changed-region bounding-box width (px) progressed from 49 to 341 to 827, reached 1,604, and contracted to 438 on reverse movement. Cross-line growth was evidenced separately by an increase in bounding-box height; the horizontal extent is not an underlined-pixel count or total underline length. The light/dark body text-mask match was 1.0. In that earlier native-settings trace, quick settings mapped and application input was released, but the observed native XInput path had no QMP touch events. Native navigation left `activeView` empty, so the supervisor withheld application restart. That trace did not pass its native round trip and is retained as history.

The recorded `2b39140b` / `0226f613` guest run passed these specific software-layer cases. Its Quick/Wi-Fi cases are history of the user-removed experiments:

- Startup and selection: `startup-06` captured the saved dark startup page. New held-contact QMP captures measured changed-region bounding-box width (px) of 49, 341, 827, 1,604, and 438 on reverse movement. Cross-line selection was separately evidenced by vertical bounding-box growth. Switching the UI to light mode produced a Reader body text-mask match of 1.0 against dark mode. These are fresh observations of this executable.
- Quick settings: Reader to native Quick, a real close tap, and application generation 1 to 2 completed. Reader body ROI pixel difference was 0, and dark-background samples remained 0 before and after the round trip. An initial close tap at about five seconds had no effect before the panel finished drawing; clicking the same point after complete drawing worked. This timing observation does not establish an input-restoration defect or general native-QMP incompatibility.
- Sleep and anchor: at font size 38 with wide margins, on page 2 of 2 of the Lantingji Xu passage, an actual `powerButton=1` request entered `screenSaver`. The resume acknowledgement matched generation 2 and persisted `chapter_index=1` with Unicode anchor 150. A `wakeUp=1` request produced native wake events and generation 3; the restored original-text ROI pixel difference was 0.
- Wi-Fi: the Settings Wi-Fi row opened the native Wi-Fi list, QMP tapped Done, and generation 3 returned as generation 4. Settings content ROI pixel difference was 0. No network configuration was changed.
- Active exit: Settings exit followed by confirmation ended the application, removed the display lock, and left `statusbar` running. The application log contained only four geometry lines and no display-feedback error. The process exit code was not recorded, so no exact exit code is asserted.

Reports are `quick-return-report.json`, `wake-anchor-report.json`, `wifi-return-report.json`, and `drag-report.json` under `test-env:/root/weread-emulator/artifacts/ux-extended-final/`, alongside startup, held-selection, wake, Wi-Fi, and exit captures. These results retain their per-candidate evidence boundary; Quick/Wi-Fi cases are no longer product scope. See [Verification](VERIFICATION.md) and [Physical testing](PHYSICAL_TESTING.md) for current Reader and hardware gates.

The emulator's stock Home was recovered with a guest-only boot delta: append `|| :` to the final `platformmetricsd-send-notification reboot reboot` command in `/etc/upstart/system_setup.conf`. With Minerva disabled, a 65,584-byte notification queue exceeded the 65,536-byte limit, causing the original command to return 255 and block boot. The queue was not cleared, no application or boot events were fabricated, and the guest root filesystem was returned to read-only mode. The earlier six-case Scribe experiment did not use this patch and retains its original environment record. Details are in [Scribe 2022 emulator](SCRIBE_EMULATOR.md), with `ENVIRONMENT_DELTA.md` and `recovery-report.json` under `test-env:/root/weread-emulator/artifacts/scribe1-system-setup-recovery-20261103T195754Z`.

The guest cases above remain software-layer evidence within the recorded VM-only environment delta. Current Reader and hardware progress is tracked separately in [Physical testing](PHYSICAL_TESTING.md). The explicitly skipped physical USB gate remains open. Overall acceptance is incomplete; the removed Wi-Fi/Quick experiments are not outstanding product work.
