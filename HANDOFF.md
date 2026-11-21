# Handoff: Kindle Library startup

Updated November 21, 2026 (Asia/Shanghai). **Font optimization measured,
deployed and retested on Kindle Scribe 2022 / firmware 5.19.5.** The Kindle
package now deliberately selects `kindle,bundled-font-preflight`. The default
desktop build, custom-font validation, launcher wait and orientation queries
are unchanged. At the November 21 handoff WeRead held no device lease and
had no outstanding cleanup. This is not a statement of shared availability.

**Primary startup metric: not measured by the runs below.** Use [ordinary
stock Library tile release to the first complete WeRead Library frame](docs/PHYSICAL_TESTING.md#library-first-frame-startup).
Record the release and the qualifying render/submission in the same launch
on one device clock; label synthetic input and the actual end event. Do not
combine an indexed file-open timing with a separate installed-entry check.
The November 21 ordinary-entry retest confirmed responsive native return and
released its lease at 11:11:25 (Asia/Shanghai). Its status remains `not_timed`;
it supplies no release-to-render, release-to-submit or interaction latency.
In the later [automatic stock-card run](docs/review/library-startup-latency-20261119/daily-render-20261121.md),
the unchanged installation showed a complete grid in framebuffer memory by
**8.258 s** after synthetic release (one launch; no frame-specific completion
or `SEND` timestamp). All close checks passed; the lease was released at 12:28:06.

## What actually ran

| Endpoint | Result | Scope |
| --- | --- | --- |
| Desktop release: window created to fonts registered | 325.337 → 0.721 ms | November 19, six warm pairs on `test-env`, not a device measurement. [Samples](docs/review/library-startup-latency-20261119/desktop-ab.json) |
| ARMv7 QEMU: admission correctness | Seven faces and seven restricted-budget cases matched | Includes identical partial charges and target-dependent state. [ARM verification](docs/review/library-startup-latency-20261119/arm-admission/README.md) |
| Physical CPU-only: seven-face admission | 2,709.496 → 3.557 ms median | One guarded non-UI process on Scribe, not a launch. [Samples](docs/review/library-startup-latency-20261119/physical-20261120.md) |
| Physical launch: temporary indexed file-open request to first Library SEND | **9.964 → 6.847 s median; all six pairs improved** | Same temporary entry, public 15-cover seed, one boot, balanced order; median paired saving 3.119 s. Library and Reader bodies matched software and physical scanout. Not tap-to-ink or an all-covers endpoint. [Report and samples](docs/review/library-startup-latency-20261119/indexed-font-ab-20261120.md) |
| Installed ordinary WeRead document: functional retest | One catalog entry, expected production executable and arguments; state open, display and input live, clean exit | The measured candidate was installed with the uninstrumented launcher. No demo, offline switch, test deadline, account inspection, data reset or private backup. Not an additional timed sample. [Deployment](docs/review/library-startup-latency-20261119/indexed-font-ab-20261120.md#normal-installation) |

The November 17 9.820 s [formal-entry baseline](docs/review/native-library-entry-20261117/README.md#formal-entry-measurements)
belongs to an older build and a different indexed item. Do not mix it with the
November 20 samples. The monolithic November 15 daily launcher observed in
the early November 20 read-only audit is likewise **not** the current install.

## Current state

- The ordinary document is `/mnt/us/documents/weread.sh`, the production
  wrapper. `/mnt/us/weread/launch.sh` now exists and is its supervisor; the
  deployed ARM app is the 95,137,176-byte qualified candidate. Both projects
  sharing the device were notified of this change. Do not restore an old
  WeRead absence or checksum over the installation.
- The temporary test item and its catalog row were removed. Both comparison
  and deployment used separate owner/token-checked leases; the last release
  was **17:32:29**. The native foreground, statusbar, window manager, input
  roster, lights, approved peer files, and scoped shared registration were
  checked. No shared installer or database write was used.
- Verification, artifacts and private recovery records stay on `ssh test-env`.
  [Worker recovery and common toolchain](docs/review/library-startup-latency-20261119/rebuilt-worker.md)
  documents the clean rebuild; [the experiment tools](tools/kindle-font-trial/README.md)
  document clock, fixture, safety and deployment gates. Keep source/target
  isolated by project and take the **host** device lease only for actual work.
  Follow [AGENTS.md](AGENTS.md); do not fall back to local verification.

## Orientation Experiment

Both orientation queries timed out during all twelve timed launches, about
half a second each. A [November 21 orientation candidate](docs/review/startup-orientation-20261121/README.md)
is now available behind `startup-orientation-reuse`. It is **off** in the
normal Kindle package. All 52 collector and regression tests passed on
`test-env`. The [two-launch physical pilot](docs/review/startup-orientation-20261121/physical-pilot.md)
then passed: both candidate sites actually reused calibration, the control
timed out twice, and both the software and scanout Library bodies matched.
The device returned to native and the pilot lease was released at 11:39:10
(Asia/Shanghai). The [separate six-pair batch](docs/review/startup-orientation-20261121/physical-balanced.md)
passed its timing gate and released at 11:53:25. Warm indexed request to
first Library SEND was 6.604 s for A and 5.356 s for B (medians); all six
pairs improved, with a median paired saving of 1.323 s. The two query sites
explain 0.965 s of median paired saving within their measured intervals.
Do not treat the rest as orientation-query time, or any of it as a measured
saving from ordinary tile release. The [pose qualification](docs/review/startup-orientation-20261121/pose-qualification.md)
checks screen/touch mappings and wake on the Linux worker. The opposite
physical portrait, a turn during startup, and a turn across actual sleep/wake
remain untested on the Scribe; leave the package flag off. In a separate
[real candidate Reader visit](docs/review/startup-orientation-20261121/physical-pose-visit.md),
the splash and Reader matched the expected digital orientation in the same
`D:1` pose. The wake-command guard refused before sleep. The device was
restored and released at 13:28:38; no daily installation was changed.
The [operator-assisted shelf visit](tools/kindle-font-trial/ORIENTATION.md#operator-assisted-opposite-portrait-and-shelf-tap)
was attempted on the Scribe: the physical prelaunch reference remained `D:1`
and the collector did not open the candidate. The device was released at
15:36:49. A later read-only window returned `D:1` on all 18 native reads and
released at 15:47:11 without uploading anything. The earlier ten sensor reads
were from a separate lease, not the live turn window. Neither opposite-portrait
startup nor a physical tap was observed; see the
[follow-up](docs/review/startup-orientation-20261121/physical-pose-visit.md#native-only-follow-up).
See the linked experiment for build results and remaining gates. The [power-observer
investigation](docs/review/startup-orientation-20261121/power-wait.md) keeps
the two-second wait unchanged. Neither changes the measured font result.

## Independent launcher change

The working supervisor no longer reads the X window tree at startup for
the retired Wi-Fi and Quick settings controls. Explicit native requests
still require current, live checks. In an independent Linux fixture, six
of six pairs reached the mock app faster (46.071 ms median paired saving).
The [source comparison and samples](docs/review/startup-launcher-window-20261121/README.md)
are **not** a new Scribe timing or installation. Keep the existing orientation
pilot and its worker sources frozen, including their original supervisor.
