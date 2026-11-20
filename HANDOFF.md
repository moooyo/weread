# Handoff: Kindle Library startup

Updated November 20, 2026 (Asia/Shanghai). **Font optimization measured,
deployed and retested on Kindle Scribe 2022 / firmware 5.19.5.** The Kindle
package now deliberately selects `kindle,bundled-font-preflight`. The default
desktop build, custom-font validation, launcher wait and orientation queries
are unchanged. There is no outstanding WeRead device lease or cleanup.

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

## Separate future work

Both orientation queries timed out during all twelve timed launches, about
half a second each. A reuse experiment would need independently validated
normal/inverted portrait calibration, changed/missing calibration fallback
and wake recovery. No such change was deployed. Similarly, changing the
two-second power-observer wait needs its own interruption and sleep/wake
tests. Neither remaining opportunity changes the measured font result.
