# Kindle Scribe 2022 firmware-emulator experiment

Recorded on 2026-11-03 (Asia/Shanghai). The operator selected the first-generation Scribe released in 2022. All provisioning and application execution ran on `ssh test-env`; no Windows application build or test was run.

This is a separate firmware-emulator result. It does not certify a physical Scribe, another Scribe generation, pen support, or live WeRead account behavior. The existing PW6 instance and its evidence were preserved.

## Exact environment

- Emulator: `katadelos/eink-emulator` at `c96062d51a1a54dd445d1668e740786d160a80bb`.
- QEMU submodule: `b75b93cdc82d60d8c3f844dd114fd38dfb481df5`, version 11.0.2, AArch64 system emulator.
- Model: `kindle-scribe-1`, board `barolo`, `dvt` hardware profile, machine `weread-scribe1-5201`.
- Firmware: 5.20.1, explicitly selected rather than `latest`; the version index reports build 4893930050.
- Guest kernel: `4.9.77-lab126`; userspace reports `armv7l` and contains `/lib/ld-linux-armhf.so.3`. The QEMU architecture does not dictate the application ABI.
- Guest configuration: two Cortex-A53 CPUs, 1 GiB RAM, 8 GiB sparse disk. Memory was not increased for these checks.
- Guest display: visible 1860 x 2480, 8 bits per pixel, virtual buffer 1872 x 4960, stride 1872, reported rotation 3. Virtual dimensions and stride were not substituted for visible resolution.
- The original launcher checks profile `scribe` and firmware `5.20.1`. No PW6 experimental-display file is present.

The original `61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90` ARM executable was reused without modification or recompilation. Host and guest hashes match. Native preflight confirmed the expected 1860 x 2480 framebuffer with zero panning offsets. The runtime and design viewport are both 930 x 1240 at scale 2.

## Provisioning and access

The same Linux host requirements and out-of-tree QEMU data path as the [PW6 experiment](EMULATOR.md) apply. The model builder imports the selected vendor firmware and prepares its own guest compatibility changes; this is not an unmodified physical-device environment.

```sh
cd /root/weread-emulator/eink-emulator
./eink create weread-scribe1-5201 --model kindle-scribe-1 \
  --profile dvt --firmware 5.20.1 --jbsh
./eink run weread-scribe1-5201 \
  --vnc 127.0.0.1:7 --serial-socket --qmp-socket \
  --ssh-port 4032 --wifi-ssh-port 4033 -- \
  -L /root/weread-emulator/eink-emulator/qemu/pc-bios \
  -daemonize -pidfile /root/weread-emulator/scribe1-qemu.pid
```

SSH uses the generated instance key and loopback port 4032. The helper `/root/weread-emulator/guest-ssh-scribe1` selects that port and an instance-specific known-hosts file. Keys, firmware archives, disks, and raw kernel logs remain outside the repository. Guest file transfers use `scp -O`.

For viewing from Windows:

```powershell
ssh -N -L 5907:127.0.0.1:5907 test-env
```

Connect a VNC viewer to `127.0.0.1:5907`. The PW6 VNC port remains 5906.

After the checks, the Scribe instance was left in offline demo review mode. If it has returned to the stock Home screen, start it again through its own helper:

```sh
printf '%s\n' review | /root/weread-emulator/guest-ssh-scribe1 tee /mnt/us/weread/data/logs/test-case
/root/weread-emulator/guest-ssh-scribe1 sh /mnt/us/documents/weread.sh
```

## Native application checks

Deploy the existing development bundle, keep the ELF under the immutable basename `weread-61fa9afd1b5b`, and install the [test-only wrapper](../packaging/tests/emulator-wrapper.sh). The guest-local configuration contains:

```text
data/device-profile: scribe
data/device-allowlist: scribe 5.20.1
data/logs/test-binary: weread-61fa9afd1b5b
```

The wrapper always selects isolated demo data and disables application networking. The existing product launcher retains display ownership, event observation, and restoration responsibilities. The generalized driver keeps PW6 as its default and selects Scribe-specific viewport assertions and physical tap coordinates only when requested:

```sh
python3 /root/weread-emulator/verify-firmware-emulator.py \
  --emulator-root /root/weread-emulator/eink-emulator \
  --guest-ssh /root/weread-emulator/guest-ssh-scribe1 \
  --binary /root/weread-build/target/armv7-unknown-linux-musleabihf/release/weread \
  --output-dir /root/weread-emulator/artifacts/scribe1-5201/native \
  --machine weread-scribe1-5201 --device-profile scribe --sample-memory
```

All six cases passed:

| Case | Observed result |
| --- | --- |
| Library to Settings, exit confirmation, normal exit | QMP-injected taps selected the correct controls; Settings metadata; exit 0; firmware UI restored |
| Library to Details to Reader | Correct native Reader route; exit 0; firmware UI restored |
| TERM delivered to the launcher | Exit 143; app ended; lock removed; status bar and window manager restored |
| Three further startup/capture/shutdown cycles | All exit 0; Library metadata; firmware UI restored |

Application snapshots and running QEMU display captures are separate artifacts. Every captured application metadata record reports `profile=scribe`, `backend=kindle`, logical 930 x 1240, physical window/snapshot 1860 x 2480, and scale 2. QMP captures demonstrate the guest framebuffer update path; logical scripted pointer events were not substituted for native touch.

The current native adapter has no pen, eraser, pressure-sensitive handwriting, or multi-touch acceptance. These tap checks do not imply those capabilities.

## Memory observations

The guest reports 987,532 KiB of usable RAM. Before starting the app, `MemAvailable` was 659,808 KiB. Periodic `/proc` sampling covered initialization, navigation, shutdown, and final capture until the native process disappeared; sample counts per case ranged from 13 to 29.

- Highest observed application RSS/high-water reading: 239,580 KiB, approximately 234 MiB, in the Reader case.
- Lowest sampled guest `MemAvailable`: 393,692 KiB, approximately 384 MiB.
- All six runs completed in the original 1 GiB guest configuration. These samples are not an exhaustive peak measurement or a worst-case memory guarantee.
- The captured kernel log contained no Linux OOM-killer entry during these cases. A CMDQ SRAM-allocation warning occurred during early kernel boot, before the application; it is not evidence that the app exhausted guest RAM.

Elapsed case times include SSH, sampling, QMP operations, screenshots, and restoration. They are not measurements of physical Scribe performance.

## Artifacts and remaining gates

Remote evidence is under `/root/weread-emulator/artifacts/scribe1-5201/`. Operator-local generated copies are under `artifacts/emulator-scribe1-5201/`, including the native report, per-case metadata, screenshots, and memory samples. These artifacts are ignored by Git.

The existing development ZIP `dist/weread-kindle-0.2.0-emulator-fix.zip` already contains the tested ARM file; no new application bundle is required for this additional experiment. Its current SHA-256 remains `906a019922d99ef4d4a63cb7449a69faa5822dab9a3c48f8652ec01b6a692424`.

Physical panel response, ghosting, latency, battery use, real Wi-Fi, suspend/USB behavior, pen input, installation on a real device, and live-account authentication/content acceptance remain separate gates. The synthetic waveform, modified guest startup, and virtual peripherals do not establish those properties.

## Current environment recovery after the six-case run

This follow-up describes a later VM environment state. The six cases, binary `61fa9afd`, and memory observations above did not include the boot-recovery change below. Those historical results have not been rerun or relabeled under the changed environment.

The guest's stock Home is recovered. ARM executable `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` and launcher `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` were deployed to the Scribe 2022 firmware 5.20.1 guest and have now run the scoped software cases below. Deployment and stock Home recovery alone are not application acceptance results. The latest launcher separately passed 50 of 50 isolated fixtures in 205.691 seconds; [Verification](VERIFICATION.md) records that substituted-process evidence.

The only persistent guest environment delta used for recovery is in `/etc/upstart/system_setup.conf`: the final `platformmetricsd-send-notification reboot reboot` command now ends with `|| :`. The disabled Minerva notification queue contained 65,584 bytes against a 65,536-byte limit, causing the original notification command to return 255 and block boot completion. The queue was not cleared, no application or boot events were fabricated, and the root filesystem was returned to read-only after the change. This is a VM-only adjustment, not a physical-device prerequisite or a product installation step.

Recovery evidence is under `test-env:/root/weread-emulator/artifacts/scribe1-system-setup-recovery-20261103T195754Z/`, including `ENVIRONMENT_DELTA.md`, `recovery-report.json`, the original setup text, logs, and metadata. The timestamp is part of the existing evidence directory name; no new run date is assigned by this documentation update.

Physical SSH access remains unavailable, the new ARM candidate has not been deployed to the physical Scribe, and integrated physical acceptance is pending. See [Physical testing](PHYSICAL_TESTING.md) for the separate firmware 5.19.5 observations and outstanding gates.

## Current extended software cases

These observations belong to ARM `2b39140b` and launcher `0226f613` in the recovered guest. XInput, input-grab, and driver code were not changed for these cases. The VM-only notification-command delta above remained in place.

The scoped guest-software cases covered persisted dark startup, held selection, Quick Settings return, an actual powerd sleep/wake transition with a saved Unicode anchor, Wi-Fi return without changing network settings, and explicit exit restoring native Home. The detailed generation transitions, image comparisons, and exit-code boundary are centralized in [Verification](VERIFICATION.md#current-extended-scribe-vm-cases) and [Physical testing](PHYSICAL_TESTING.md#related-remote-and-vm-evidence).

An initial close tap at about five seconds had no effect. The same coordinates closed the panel after it finished drawing. This observation does not establish a general lack of native QMP input support or an input-restoration bug. The preceding `daae495c` attempt's incomplete return remains historical evidence; the results above are new executions of `2b39140b`.

Evidence is under `test-env:/root/weread-emulator/artifacts/ux-extended-final/`: `quick-return-report.json`, `wake-anchor-report.json`, `wifi-return-report.json`, `drag-report.json`, `explicit-exit-report.json`, and the startup/held-selection/wake/Wi-Fi/exit captures. Downloaded copies are under `artifacts/ux-extended-final/`, with the aggregate record at `artifacts/ux-extended-acceptance/extended-acceptance-summary.json`. These results cover the stated guest software paths, not physical panel timing, radio behavior, physical suspend, or integrated acceptance on physical firmware 5.19.5. Physical USB testing remains skipped for this round.
