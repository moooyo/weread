# Kindle firmware emulator

This is a development-only firmware-emulator experiment on the designated `test-env` Linux worker. It is not physical-device acceptance or a published device-support matrix. Firmware, guest disks, SSH keys, and third-party source checkouts stay outside this repository.

This document records PW6. The separate [Scribe 2022 experiment](SCRIBE_EMULATOR.md) reuses the same ARM executable in a 1 GiB Scribe guest and records its own native results.

## Recorded environment

- Host: Debian 13, x86-64, approximately 11 GiB RAM.
- Workspace: `/root/weread-emulator` on `test-env`.
- Emulator: [`katadelos/eink-emulator` at `c96062d51a1a54dd445d1668e740786d160a80bb`](https://github.com/katadelos/eink-emulator/tree/c96062d51a1a54dd445d1668e740786d160a80bb).
- QEMU: its pinned submodule at `b75b93cdc82d60d8c3f844dd114fd38dfb481df5`, reporting version 11.0.2.
- KindleTool: `NiLuJe/KindleTool` at `f31462807b2d50c6ecf2ab4357446b14b882068b`.
- Guest: Paperwhite 6, production profile, firmware 5.20.1, with the emulator's pinned `jb.sh` setup.
- Guest kernel: `5.15.41-lab126`.

The operator authorized remote disk cleanup and emulator installation on 2026-11-03. Rebuildable Go compilation cache, APT download cache, and the project's obsolete `store-target` and `reader-check-target` directories were removed. The root filesystem's reported available capacity increased from approximately 7.1 GiB to 16 GiB before installing the emulator dependencies. Existing services, source checkouts, current application binaries, and verification evidence were preserved.

## Host preparation

All commands in this section execute on `test-env`, not on the Windows operator machine. Install the upstream host requirements, including Ninja, Meson, Python venv support, GLib development headers, pixman, PNG/JPEG, e2fsprogs, OpenSSH, and KindleTool. KindleTool additionally needs libarchive, nettle, and zlib development headers.

The QEMU submodule URL was overridden in the checkout's local Git configuration to use HTTPS. No upstream source patch was needed for that change.

The resource-limited VNC build uses the upstream configuration with only the required AArch64 system target, GTK/SDL disabled, two LTO threads, and three Ninja jobs:

```sh
cd /root/weread-emulator/eink-emulator/qemu
./configure --target-list=aarch64-softmmu \
  --disable-docs --disable-werror --enable-slirp --disable-pvg \
  --disable-gtk --disable-sdl \
  -Dforce_fallback_for=slirp -Dqom_cast_debug=false \
  -Db_lto=true -Db_lto_threads=2
ninja -C build -j3 qemu-system-aarch64 qemu-img
```

The emulator's automatic builder otherwise builds additional targets without a Ninja job limit. The manually built binaries satisfy the PW6 CLI's tool checks.

## Create and start the instance

```sh
cd /root/weread-emulator/eink-emulator
./eink create weread-pw6-5201 \
  --model kindle-paperwhite-6 --profile production \
  --firmware 5.20.1 --jbsh
./eink run weread-pw6-5201 \
  --vnc 127.0.0.1:6 --serial-socket --qmp-socket \
  --ssh-port 4022 --wifi-ssh-port 4023 -- \
  -L /root/weread-emulator/eink-emulator/qemu/pc-bios \
  -daemonize -pidfile /root/weread-emulator/qemu.pid
```

The explicit `-L` is required by this uninstalled build to locate its VNC keymaps. The first attempted start failed before boot because the keymap directory was absent from QEMU's default data search path; adding the source-tree data path resolved that host setup issue.

Do not combine `--headless` with `--vnc`: this emulator revision gives `--headless` precedence. All forwarded ports bind to loopback. `--jbsh` prepares the executable userstore and jailbreak startup environment inside this disposable guest; it does not install KUAL.

Use the guest's firmware information and framebuffer geometry for application preflight. The initial QEMU boot surface alone is not evidence of final framebuffer geometry. In this experiment, the running guest reported a virtual buffer of 1272 x 3392, stride 1272, and 8 bits per pixel. The original firmware's Sangria device tree also specifies 1272 x 1696. Virtual height must not be used as visible height, and the difference from the design reference must not be described as padding without evidence.

## Guest access and observations

```sh
ssh -i build/ssh/id_ed25519 -p 4022 \
  -o HostKeyAlias=weread-pw6-5201 root@127.0.0.1
python3 scripts/eink-qmp.py --machine weread-pw6-5201 status
python3 scripts/eink-qmp.py --machine weread-pw6-5201 display
python3 scripts/eink-qmp.py --machine weread-pw6-5201 screendump /absolute/output.png
python3 scripts/eink-qmp.py --machine weread-pw6-5201 tap X Y --hold 0.08
```

Use `scp -O` for guest transfers because the Kindle guest has no SFTP server. Keep its private login key outside repository artifacts. The tap command uses actual scanout pixel coordinates at this revision; some upstream prose still describes an older fixed coordinate range.

For viewing from Windows, forward VNC through SSH and connect a VNC viewer to `127.0.0.1:5906`:

```powershell
ssh -N -L 5906:127.0.0.1:5906 test-env
```

## Application experiment

Deploy the existing development bundle under the guest's `/mnt/us`. Keep the original application as `weread/app/weread.native`; use a guest-only `weread/app/weread` wrapper to append `--demo --offline`, an explicit timeout, and an application capture path. The original launcher still controls profile/firmware checks, display ownership, power-event observation, and restoration. Its current product implementation does not forward arbitrary additional CLI arguments.

For repeatable final checks, keep the ELF under an immutable filename `weread-<first-12-SHA256-characters>` instead of replacing a mapped executable. Install [the test-only wrapper](../packaging/tests/emulator-wrapper.sh) as `weread/app/weread`, and write that ELF basename to `data/logs/test-binary`. Synchronize guest writes and compare the complete host/guest hashes before execution. The wrapper is not a product bundle input.

The guest-local `data/device-profile` contains `pw6`, and `data/device-allowlist` contains the exact experimental pair `pw6 5.20.1`. The additional `data/experimental-display` contains `pw6-5201-emulator`. This named specification accepts only that exact profile/firmware pair and requires visible framebuffer dimensions 1272 x 1696 with zero panning offsets. It does not accept arbitrary width/height overrides. These local entries enable this experiment and do not certify physical compatibility.

The design reference remains 632 x 840. The approved native viewport is 636 x 848 at scale 2, and application pagination, hit regions, and refresh areas use that runtime viewport. Desktop design captures retain their original reference dimensions. Native capture metadata records both reference and runtime logical sizes. Demonstration data stays in the application's separate demo directory; no real account is required.

Record application snapshots separately from QEMU screenshots. The former re-render the application state; the latter observe the emulated display update path. Test QEMU-injected touch independently from `--actions`, because scripted logical pointer events bypass the native evdev conversion.

Run the integration driver on the Linux worker:

```sh
python3 /tmp/weread-implementation/scripts/verify-firmware-emulator.py \
  --emulator-root /root/weread-emulator/eink-emulator \
  --guest-ssh /root/weread-emulator/guest-ssh \
  --binary /root/weread-build/target/armv7-unknown-linux-musleabihf/release/weread \
  --output-dir /root/weread-emulator/artifacts/delivery-native
```

The SSH helper uses the instance key, loopback port 4022, an instance-specific known-hosts file, and a connection timeout. The driver validates the guest firmware and ELF hash, starts the original launcher, injects hardware-path QMP taps, checks final native metadata, and checks lock cleanup, status-bar restoration, and resumed window-manager state. It retains separate QEMU screenshots and application snapshots. It does not assert pixel equivalence between unsynchronized images or measure panel timing.

## Verification results

Recorded on 2026-11-03 (Asia/Shanghai). The final ARM executable is `61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90`; host and guest hashes match. It is an ELF32 ARM EABI5 hard-float executable with no dynamic section. Six final integration cases passed:

| Case | Result |
| --- | --- |
| Native library, normal-coordinate tap to Settings, confirmation and exit | Passed; exit 0, native Settings metadata, firmware UI restored |
| Native library to Details to Reader using QMP-injected taps | Passed; final route Reader, exit 0, firmware UI restored |
| External TERM to the launcher while the application is running | Passed; launcher exit 143, app terminated, lock removed, firmware UI restored |
| Three further native startup/capture/shutdown cycles | All passed; exit 0, Library metadata, firmware UI restored |

All six executions retained the 632 x 840 design reference, 636 x 848 runtime viewport, 1272 x 1696 physical window/snapshot, and scale 2. They used offline demonstration data. No gzip integrity error recurred in these final six starts. The immediately preceding, semantically identical pre-format candidate `6a270d812b1e404a3750fbaf6f6825ec763f87f24c77b42ecb432f973cb0f718` also passed six cases; its results are retained separately and are not represented as executions of the final ELF.

Supporting worker checks passed: 206 application tests with the Kindle feature, 17 standalone backend tests, 13 isolated launcher tests, 1,000 additional fast-exit launcher stress cycles, formatting, and 133 desktop UI/persistence checks. The desktop binary for that run is `8002ab7d1dc64965be4ca6f8ab893633f300bdc528d02c7c95ffd2838c1701df`; desktop and ARM results are distinct evidence.

Development archive: `dist/weread-kindle-0.2.0-emulator-fix.zip`, 66,642,557 bytes, SHA-256 `906a019922d99ef4d4a63cb7449a69faa5822dab9a3c48f8652ec01b6a692424`. Its executable hash matches the final tested ARM file. Local generated evidence is under `artifacts/emulator-pw6-5201/delivery-native/`; the desktop report is `artifacts/emulator-pw6-5201/desktop-report.json`. These generated artifacts and the archive are ignored by Git. The provisioned host retained approximately 12 GiB free disk space after installation and packaging.

## Defects found and corrected

- The initial ARM candidate `3cccb2953c40c1c049fccf6a3e45e88ec150630d365beb53f30277a4fbb1b07e` exited 1 because a design-derived geometry guard rejected the actual guest framebuffer. The named experimental display specification now validates the exact measured visible geometry and zero panning offset. Runtime layout is separate from the desktop design reference.
- The geometry-only candidate `f603f71974bea950fd115d23ea4ae84ad76df181d817576f04980d1f9b2a8b92` reproduced a touch scaling defect. A normal physical tap left the navigation unchanged; a half-coordinate tap activated Settings. Press, move, and release now convert physical coordinates through rotation and the window scale. Final full-coordinate navigation passed independently through the guest touch controller.
- Fast application exit could leave an event observer blocked during cleanup because TERM could arrive before the asynchronous shell reset its traps. Observers hold no restoration state and are now deterministically killed and reaped; the app retains its TERM grace period and service-restoration order. A regression with an observer ignoring TERM failed against the pre-fix control and passed after the repair.

## Open observation

One first start of candidate `f603...` reported a bundled Serif Medium gzip trailer mismatch. The same file passed on immediate retry; all compressed source fonts passed host gzip integrity checks. Neither the next immutable candidate's six starts nor the final candidate's six starts reproduced it. Integrity checking remains enabled. FUSE/mapping/cache or emulator transience is only a hypothesis; the root cause remains unconfirmed and is recorded in [Open questions](OPEN_QUESTIONS.md).

## Review mode

The test wrapper also accepts a `review` case without capture or automatic exit. After tests, select it and start the original launcher to inspect the offline demo through VNC:

```sh
printf '%s\n' review | /root/weread-emulator/guest-ssh tee /mnt/us/weread/data/logs/test-case
/root/weread-emulator/guest-ssh sh /mnt/us/documents/weread.sh
```

Use the application's exit confirmation to return to the stock Home screen. Do not treat this modified guest's indefinite awake behavior as a physical power-management result.

## Evidence boundary

Results apply only to the pinned emulator, its modified guest image, and the tested binary. Synthetic display waveforms do not establish physical panel response, ghosting, flicker, contrast, refresh latency, or power use. Guest compatibility overrides modify suspend and USB behavior. Neither emulated networking nor offline demo data establishes real Wi-Fi, live WeRead authentication, account entitlements, or physical-device compatibility.
