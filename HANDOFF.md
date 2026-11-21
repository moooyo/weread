# Handoff: Kindle Library startup

Updated November 21, 2026 (Asia/Shanghai). **Font optimization measured,
deployed and retested on Kindle Scribe 2022 / firmware 5.19.5.** The Kindle
package now deliberately selects `kindle,bundled-font-preflight`. The default
desktop build, custom-font validation, launcher wait and orientation queries
are unchanged. At the November 21 handoff WeRead held no device lease and
had no outstanding cleanup. This is not a statement of shared availability.

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
half a second each. A [November 21 orientation candidate](docs/review/startup-orientation-20261121/README.md)
is now available behind `startup-orientation-reuse`. It is **off** in the
normal Kindle package. Both ARM builds and the two-launch public Library
pilot are frozen on `test-env`; all 52 collector and regression tests passed
without the device. At 09:39 Asia/Shanghai on November 21, the original
Aidoku batch had no verified close and release receipt. WeRead had run no
physical orientation pilot or timed pair and had not installed the candidate.
Check for a newer verified handoff before starting; restore and release the
pilot before judging whether to time six pairs.
See the linked experiment for build results and remaining gates. The [power-observer
investigation](docs/review/startup-orientation-20261121/power-wait.md) keeps
the two-second wait unchanged. Neither changes the measured font result.
