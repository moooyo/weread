# Kindle packaging draft

This directory is a packaging scaffold, not an accepted device-support matrix or a published KPM package. `manifest.draft.json` follows the inspected KPM manifest shape; `kindlehf` is a package platform label, not a compatibility result. The application version and package identity remain provisional. No project license has been selected.

Implementation and device acceptance are still in progress. The user removed the application drop-down Quick panel and application Wi-Fi, including the project-owned C Wi-Fi helper, from product scope. Historical checks apply only to their identified executable and environment. The recorded ARM candidate has SHA-256 `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` and ran with launcher SHA-256 `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` in the Scribe 2022 firmware 5.20.1 guest. Current physical evidence belongs in [Physical testing](../../docs/PHYSICAL_TESTING.md), and overall acceptance remains open.

## Layout

The build script stages this layout under `dist/kindle/`:

```text
documents/weread.sh
weread/app/weread
weread/app/weread-frame-guard
weread/app/licenses/
weread/app/THIRD_PARTY_NOTICES.md
weread/app/README.md
weread/app/config/device-profile.example
weread/app/config/device-allowlist.example
weread/app/manifest.draft.json
```

Fonts, icons, the dictionary and interface strings are embedded in the executable; their notices are bundled separately. The launcher creates persistent state under `/mnt/us/weread/data/`, including `fonts/` and `logs/`. Replacing `app/` must preserve `data/`. The build script never copies credentials, books, or operator state into the staged output.

The resource-review candidate retains application diagnostics in `data/logs/application.log` and two fixed backups, each limited to 1 MiB with an 8 KiB message limit. A diagnostic I/O failure closes that sink and suppresses repeated fallback messages. Normal document launches maintain `launcher.log` with a 64 KiB threshold and in-place truncation; `last-launch.log` is replaced on a normal launch and `application-bootstrap.log` on each application generation. The latter two capture finite bootstrap or unexpected external output and are not general byte-capped collectors. Direct `--detached` fixtures retain their captured output.

Keep the matching executable and launcher together. New report snapshots retain compact confirmed ranges and cause older readers to fail closed rather than ignore those ranges and repeat delivery. Preserve the existing database; do not remove report evidence to force an older binary to accept it. [Resource lifetime review](../../docs/RESOURCE_LEAK_REVIEW.md) records the current candidate's review rounds and verification scope.

The main application remains a static musl Rust/Slint executable. The visual
handoff candidate additionally stages the small `weread-frame-guard` native
helper, which runs in its own glibc process and dynamically opens the Kindle's
existing X11 runtime. It does not load glibc libraries into the musl application.
No firmware libraries or loader are redistributed. The removed Wi-Fi helper
remains outside the product. The independent application and conditional-server
decisions remain unchanged; the project license is still unselected.

This helper preserves the displayed reading frame in an owned native window
while the supervisor temporarily permits native system-theme or screensaver
work. The window stays below the native screensaver and must not hide a system
lock or an unrelated dialog. Its process, window, private records, snapshot,
and cleanup are bounded by the owning launcher identity. Unsupported native
window or pixel formats fail closed. Controlled X11 tests and the read-only
Scribe 2022 / firmware 5.19.5 layout probe do not establish the final physical
handoff result; see [Native reading corrections](../../docs/NATIVE_FIX_ACCEPTANCE.md).

Before a supervised device acceptance run, copy the example profile into `data/device-profile` and create `data/device-allowlist` with the exact configured profile and firmware version. The allowlist is intentionally empty in source control. Adding a local entry enables that experiment; it does not certify the combination. Profiles are `pw5`, `pw6`, `scribe`, and `scribe2025`. The native platform still checks actual framebuffer geometry.

`WEREAD_HOME` may override `/mnt/us/weread` with an absolute path. The launcher defaults an absent `DISPLAY` to `:0.0` and preserves an explicit display value, including for native restoration helpers. Configuration files are parsed as data and are never sourced as shell code. The executable receives `--kindle --device PROFILE --data-dir PATH`.

After the allowlist check, the launcher exports `WEREAD_APPROVED_PROFILE` and `WEREAD_APPROVED_FIRMWARE`. The native platform requires these values and checks that the profile matches, so passing `--kindle` alone is not an implicit compatibility override.

## Normal-mode private trial

Priority-1/2 download optimizations include gzip for Reader/JSON responses,
scope-bound TXT format hints, two EPUB shard reads with serial fallback on
session changes, task-owned prepared-PNG reuse, and reduced checkpoint/UI/cache
work. Integrity, account isolation, and ordered durable checkpoints remain
required. Controlled component benchmarks are not physical Kindle speed claims;
see [Download performance](../../docs/DOWNLOAD_PERFORMANCE.md).

For daily use on the operator's identified device, install the real ARM executable as `/mnt/us/weread/app/weread` and this launch script as `/mnt/us/documents/weread.sh`. Keep the dependency notices and create the device profile and exact firmware allowlist in `/mnt/us/weread/data/`. The document entry is named **WeRead for Kindle**. This direct installation does not create a KPM package-manager record or publish a release.

Start the document entry with the default home directory. For an SSH launch, use `WEREAD_HOME=/mnt/us/weread DISPLAY=:0.0 sh /mnt/us/documents/weread.sh`. The script detaches itself and starts the application with only the native backend, profile, and data-directory arguments. No smoke wrapper, watchdog, `--demo`, `--offline`, scripted actions, capture, or exit deadline is used.

A fresh private-trial data directory initially opens Login. Authenticate on the device to synchronize the real Library, or choose the local Library and import books. Later ordinary starts use Library when a session or local books exist. Managed sleep recovery retains the previous Reader position. Preserve existing regular `data/` on future updates. Keep former demo progress and notes in a separate operator-controlled backup rather than promoting demo records into a real account's library.

The content-access correction migrates SQLite metadata to schema 2. Exit the application and back up the existing database and account material before replacing the executable. First launch retains all text and personal records but marks legacy WeRead bodies as unverified. Use the book's access-refresh action to revalidate them; previews do not count as complete downloads. An older schema-1 executable cannot open the migrated database, so a rollback must restore its matching pre-upgrade database backup as well as the old executable. Runtime backups remain private and are never package inputs.

The resident-sleep candidate requires its matching launcher and executable to enable same-process recovery. A per-cycle private handshake confirms that state and hardware workers have stopped, touch has been released, and drawing is paused before the supervisor stops the application process. Wake confirms native dismissal before continuing that process; complete framebuffer restoration and touch reacquisition precede resumed work. Capability absence or a failed handshake retains the bounded saved-session fallback. An interrupted or failed recovery must restore native ownership instead of leaving a stopped application behind. See the current verification documents for the candidate-specific acceptance status.

For the pinned PW6 firmware-emulator experiment only, create `data/experimental-display` containing the single line `pw6-5201-emulator`. The launcher accepts it only with profile `pw6` and firmware `5.20.1`, and exports the recognized specification to native initialization. Unknown identifiers and mismatched pairs fail before services change. An inherited environment value is cleared unless the local file enables it.

This specification expects an exact visible framebuffer of 1272 x 1696 with zero panning offsets. It preserves the design reference of 632 x 840 while using a native viewport of 636 x 848 at scale 2. The nominal design dimensions are not an attestation of hardware dimensions. Without the experimental file, the original nominal geometry check remains in effect. See [the emulator record](../../docs/EMULATOR.md); this experiment does not establish physical-device support.

## Lifecycle

The launcher checks the allowlist before modifying services. It leaves `powerd` and `lab126_gui` running. It stops `statusbar` only if Upstart reported it running and pauses only `awesome` processes that were running. Each display takeover records its own baseline, and restoration returns the services changed by that takeover. Exit and signal cleanup retain those ownership boundaries.

Startup feedback displays the project name, startup stage, and activity dots from embedded grayscale artwork before runtime font decompression. Dot animation is limited to 0.8 Hz. Its temporary worker is joined before backend environment changes and releases its framebuffer before the normal event loop opens display and input resources. The current Reader defaults to the body view with `notes_panel=false`. These source behaviors do not imply that the latest executable has completed physical acceptance.

The current supervisor first requests a durable resident pause, releases application input and display ownership, then stops the verified application process. A confirmed wake continues that same process after fresh native-mode and portrait observations, a complete redraw, and input acknowledgment. Failed or unavailable resident handshakes retain the earlier saved-session cold recovery as a bounded fallback. That fallback must not be reported as same-process acceptance. The backend's optional direct RTC suspend schedule remains disabled. Artifact-specific physical results are recorded separately.

The user now requests immediate system night-mode changes. A supported Kindle, including an explicitly supervised demo or offline session, uses the owning launcher's protected same-process theme transaction while rendering and input are paused. The system setter, readback, and available persisted state must agree before the retained Reader is redrawn and input resumes. Known system mode overrides stale appearance preferences; preference-mirror failure cannot roll back an already committed system change. Sleep, USB, faults, or unknown observations do not independently authorize theme writes. New ordinary exits no longer publish a stale target for a second transaction. The former local-only and explicit-exit policies remain historical results in [Native theme continuity](../../docs/NATIVE_THEME.md). Continuous long-press selection, including cross-line and reverse movement, retains its 80 ms update interval and editing protections.

Automatic brightness is outside the product, design-fidelity, and acceptance
scope. Manual brightness and device-supported warmth remain available controls.

USB takeover ends application ownership and restores the firmware. The user explicitly skipped USB testing in the current physical round, so this behavior remains unaccepted. Launcher recovery cannot survive `SIGKILL`, kernel failure, or power loss. A stale `/tmp/weread-launcher.lock` must be investigated before a new launch; the script does not delete another process's lock or guess at service restoration. Device acceptance must assess real panel refresh, touch, firmware service behavior, sleep/wake, USB transitions, and exit restoration.

## Source references

- [KOReader launcher at `7649e93f5ee2a5dfc3c69bacf0385aac37352949`](https://github.com/koreader/koreader/blob/7649e93f5ee2a5dfc3c69bacf0385aac37352949/platform/kindle/koreader.sh): firmware version source, `statusbar`, `awesome` stop/continue, and restoration patterns.
- [Jobman launcher at `9727c0e0a060c5d7e6dfeae2d572b69b6f556753`](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/companion/dist/documents/jobman.sh): detached document launch and `powerd` screen-saver/USB event observation.
- [Jobman KPM manifest at the same revision](https://github.com/KindleTweaks/jobman/blob/9727c0e0a060c5d7e6dfeae2d572b69b6f556753/companion/jobman_1.2.2_kindlehf/manifest.json): candidate manifest shape only.

These are static source references. This project's launcher has not been accepted on a physical Kindle merely because a reference project reports device tests.

## Development archive

After staging, `python3 scripts/package-development.py` creates
`dist/weread-kindle-0.2.0-dev.zip` and a SHA-256 sidecar. This private development
handoff contains the ARM application, native frame guard, launcher, configuration
examples, and notices. It is not a published KPM release package. The frame
guard is an explicit executable input and digest target; the removed Wi-Fi
helper is not. The archiver never collects `data/`, runtime state, credentials,
downloaded books, or private research directories.

## Verification

On 2026-11-03 (Asia/Shanghai), both shell files passed `sh -n` on the designated `test-env` Linux host. Six isolated launcher tests passed there in 11.202 seconds. The tests cover refusal before service changes, application arguments and approval metadata, successful and failing exit cleanup, preservation of a previously stopped status bar, Windows configuration line endings, and power-event interruption.

The harness substitutes temporary firmware paths, a harmless character device, and stub service commands. These historical results establish fixture behavior only; they do not establish framebuffer operation, Kindle firmware integration, or physical-device compatibility.

The recorded full isolated launcher fixture run passed 50 of 50 cases in 205.691 seconds. Its log is `test-env:/tmp/weread-native-queries.iYZFwu/launcher-fixtures.log`; the tested launcher source SHA-256 is `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`. These historical fixtures included native-settings paths that are no longer product features.

The preceding isolated launcher run on `test-env` under `/tmp/weread-native-failure.pwWnI0`, after removal of unsupported native cancel assumptions, passed 48 of 48 cases in 199.782 seconds. It included normal exit, sleep, USB, and application-generation regressions. Opening failure or an interface that never becomes mapped before the opening timeout left firmware display/input ownership intact and exited without restarting the application. USB fixture coverage does not change the explicitly skipped physical USB gate.

The recorded 235 core tests passed. Remote `file` and `readelf` inspection identified the `2b39140b` ARM executable above as stripped, statically linked ELF32 ARM EABI5 with no dynamic section. It executed in the guest with the launcher identified above. Later physical and overlay-candidate evidence is tracked separately in [Physical testing](../../docs/PHYSICAL_TESTING.md).

The user-removed Wi-Fi experiment retains its 17 synthetic checks, read-only Scribe probe, private-mock checks, and VM records as history. This includes the `095f` overlay candidate and the later application `582b72345d863e918e403457cd32ef25c1efc8b8dce314027930df6f2fc0880d` / helper `3ec60abc4b335cfbbd23bfe4265aadae67fd52f55446a4da5eabe6c72f75f4da` pair. The former helper's glibc/OpenLIPC ABI work and empty-enterprise-flag repair are not current product requirements or pending connection work. See [Verification](../../docs/VERIFICATION.md) and [Platform implementation](../../docs/PLATFORM.md) for the archived boundaries.

The physical experiment identifies a first-generation Scribe released in 2022 on firmware 5.19.5, running demo data offline. Executable `61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90` exercised Library navigation, Reader page turns and font-size changes, active exit, and persistence of one real saved note. Its earlier sleep interruption exited 143 and restored the stock UI and touch input. That result does not accept the latest checkpoint-and-restart resume path.

Intermediate executable prefix `9a13abe6` introduced startup feedback, the default body view, and sleep-recovery implementation. It does not establish same-position resume acceptance. The user explicitly found its drag-selection behavior insufficient; it is not acceptance of continuous drag selection. The user's separate Wi-Fi/Quick LIPC observations are retained as history of the removed experiments.

The earlier Scribe 2022 firmware 5.20.1 emulator executable `daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3` showed continuous selection during held QMP contact and matching light/dark body text masks. Its native-settings trace mapped quick settings and released application input, but the observed native XInput path had no QMP touch events. Native navigation left `activeView` empty, so the supervisor withheld application restart. This failed trace is retained as historical evidence.

The recorded `2b39140b` / `0226f613` run used unchanged XInput, input grabbing, and input-driver behavior. It separately captured the saved dark startup page and continuous held-contact selection. New changed-region bounding-box width (px) measurements were 49, 341, 827, 1,604, and 438 on reverse movement; cross-line growth was evidenced by the bounding box's vertical extent. Switching to light mode produced a body text-mask match of 1.0 against dark mode.

Reader to native Quick and a real close tap returned application generation 1 as generation 2, with Reader body ROI pixel difference 0 and dark-background samples 0 before and after. An early close tap at about five seconds had no effect before the panel completed drawing; the same point worked after complete drawing. This is not a general native-QMP limitation or evidence of an input-restoration defect. Settings to the native Wi-Fi list and QMP Done returned generation 3 as generation 4, with Settings content ROI pixel difference 0 and no network change.

The same run exercised actual power-daemon sleep and wake from font size 38, wide margins, and page 2 of 2 of the Lantingji Xu passage. The resume acknowledgement matched generation 2, saving `chapter_index=1` and Unicode anchor 150. Native wake events returned generation 3 with original-text ROI pixel difference 0. Settings exit and confirmation then ended the application, removed its lock, and left `statusbar` running. The application log had four geometry lines and no display-feedback error; no process exit code was recorded.

The recorded reports are `quick-return-report.json`, `wake-anchor-report.json`, `wifi-return-report.json`, and `drag-report.json` under `test-env:/root/weread-emulator/artifacts/ux-extended-final/`, with the corresponding captures. These retain their original executable and environment identity; the Quick/Wi-Fi cases now describe user-removed experiments. See [Platform implementation](../../docs/PLATFORM.md) and [Verification](../../docs/VERIFICATION.md) for details.

Guest boot recovery appended `|| :` only to the final `platformmetricsd-send-notification reboot reboot` command in `/etc/upstart/system_setup.conf`. With Minerva disabled, the 65,584-byte notification queue exceeded its 65,536-byte limit, making the original command return 255 and block boot. The queue was not cleared, no application or boot events were fabricated, and the guest root filesystem is read-only again. The earlier six-case Scribe run did not contain this guest patch and retains its original environment record. See [Scribe 2022 emulator](../../docs/SCRIBE_EMULATOR.md) for the environment delta and recovery report under `test-env:/root/weread-emulator/artifacts/scribe1-system-setup-recovery-20261103T195754Z`.

Current Reader and hardware acceptance is recorded per executable in [Physical testing](../../docs/PHYSICAL_TESTING.md) and [Verification](../../docs/VERIFICATION.md). Reader brightness, night mode, startup feedback, continuous selection, and sleep behavior remain in scope. USB remains explicitly skipped. Quick and Wi-Fi-helper records are history, not outstanding product deliverables, and do not complete overall acceptance.
