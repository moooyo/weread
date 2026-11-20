# Font A/B after the verification worker rebuild

On November 20 the operator confirmed that `test-env` had been rebuilt.
SSH works again; the November 19 experiment directory is absent. The
[original binaries](arm-build.json), [desktop measurements](desktop-ab.json)
and [ARM admission results](arm-admission/README.md) remain historical.
No replacement executable or generated SQLite snapshot inherits their identity.

## One build environment

WeRead's existing Rust 1.99.1 (including `armv7-unknown-linux-musleabihf`),
Zig 0.15.2 and cargo-zigbuild 0.24.0 were moved, not reinstalled, into
`/opt/kindle-toolchain/tools`. Aidoku supplied the common Ubuntu 24.04
userspace, native compiler, desktop dependencies and QEMU; BiliComics
stopped its separate bootstrap. WeRead removed its unused CentOS installroot.
The host OS is CentOS Stream 10; the build userspace is Ubuntu.

Use `/opt/kindle-toolchain/ubuntu-run --build PROJECT_DIR COMMAND ...` on
`test-env` for compilation. This takes `/opt/kindle-toolchain/build.lock`
once; do **not** wrap it with a second `flock`. Without `--build` the same
entry can run lightweight checks. The project is mounted as `/work`; tools,
Cargo's registry and Zig cache are shared, not the project's Cargo target or
results. The build lock is not a Kindle lease. Do not remove shared tools or
caches during project cleanup.

## Replacement inputs

The WeRead workspace on the worker is
`/root/weread-font-ab-rebuild-20261120.uHKbMu4A`.
Its `src` was exported from commit `8f49fcb3514db0498b73caf5af3754df410ef079`
(documentation-only changes after experiment commit `7b41404`).
Both project and offline-fixture lockfiles were fetched into the same shared
Cargo registry. Neither ARM build uses a desktop timing patch.

The [ARM rebuild driver](rebuild-arm.py) runs each lane separately under the
shared build lock. Use the same `source` and `output` arguments for both.
It cleans the application and Kindle backend between lanes but keeps dependency
artifacts, requires fresh Cargo records for both packages, checks the exact
feature set and seven vs. zero generated receipts, and rejects an unexpected
ELF architecture, ABI, interpreter or dynamic segment. Only the accepted
binary is copied to `arm-pair/{baseline,candidate}/weread`; each lane keeps
its Cargo JSON stream, build log and result. A failed lane directory is
not overwritten: preserve its diagnostics before retrying.

| November 20 lane | Cargo features | New executable bytes | Generated receipts |
| --- | --- | ---: | ---: |
| A (baseline) | `kindle` | 95,136,624 | 0 |
| B (candidate) | `kindle,bundled-font-preflight` | 95,137,176 | 7 |

Both executables passed the static ARM EABI5 hard-float checks; the app
and Kindle backend were compiled in each lane. Both reported `weread 0.2.0`
when run with `qemu-arm -cpu cortex-a8 <executable> --version`.
These executable sizes differ from the November 19 artifacts. The +552-byte
difference is within the **new** pair, not evidence of runtime speed.

The [ARM admission probe](arm-admission/README.md) was then compiled against
the new candidate's generated declarations and ARM release dependencies.
On the shared Ubuntu QEMU 8.2, `--verify` produced the same detailed output
as the [recorded verification](arm-admission/verification.txt): seven faces,
seven restricted-budget cases with identical partial charges, 32-bit target
state of 4,544 bytes, and per-glyph peak of 79,248 bytes. No QEMU benchmark
or firmware run was used. The disposable probe and ARM Cargo target were
removed after retaining the two application binaries, logs and receipts.

For a **fresh pair**, after copying `rebuild-arm.py` to the project root on
the worker, the operator can use PowerShell:

```powershell
$worker = '/root/weread-font-ab-rebuild-20261120.uHKbMu4A'
ssh test-env "/opt/kindle-toolchain/ubuntu-run --build $worker env CARGO_BUILD_JOBS=8 python3 /work/rebuild-arm.py baseline /work/src /work/arm-pair"
if ($LASTEXITCODE -ne 0) { throw 'Baseline did not pass.' }
ssh test-env "/opt/kindle-toolchain/ubuntu-run --build $worker env CARGO_BUILD_JOBS=8 python3 /work/rebuild-arm.py candidate /work/src /work/arm-pair"
if ($LASTEXITCODE -ne 0) { throw 'Candidate did not pass.' }
```

The [public fixture recipe](../catalog-entry-adapter-20261118/library-covers.md)
uses the production store/cache, not private account data. A regenerated
seed contains fresh timestamps. Validate its ordered public cover bindings,
close the database, and restore the **same frozen archive** for each lane.
The [staging rehearsal](rehearse-staging.py) accepts only a disposable
worker directory, the new pair and that seed. It copies one real arm at a
time, restores the fixture after mutation, checks that a simulated interrupted
copy never publishes an executable and is cleaned up, and preserves three
neighboring sentinels. It does not contact a device, mount anything, run an
ARM app or qualify Kindle filesystem semantics or capacity.

On the rebuilt worker, `fixtures/seed.tar.gz` is a new closed 26,856-byte
archive (247,710 expanded file bytes), not the November 18 archive. The
published 15 ordered book/URL/thumbnail bindings matched. Replaying the
archived desktop frames with the new references matched 15/15 covers on
the warm shelf and rejected only `demo-7` on the real missing-cover frame;
the four published fallback/swap/gray controls also passed. This replay
does not generate new frames. For the worker-side staging rehearsal, both
full ARM files were copied from the retained pair, each restore used the
same seed, the simulated 1 MiB interruption left no published executable,
and all three protected sentinels and the scratch cleanup passed.

Separately, both desktop feature sets were rebuilt in the shared Ubuntu
userspace with the development profile. Each of six fresh Winit/software
captures restored the new frozen seed; the missing-cover runs removed only
the bound `demo-7` thumbnail. The route and 930 × 1240 scale-one geometry
were checked, along with opaque, nonuniform output. Each A/B comparison
(Library, Reader and one missing cover) differed by **zero pixels**.
Both Library captures matched all 15 cover interiors; both missing-cover
captures matched 14 and rejected only `demo-7`. These are newly rendered
desktop images, not pictures from the ARM builds or a timing experiment.

The worker retains `arm-pair/{baseline,candidate}/`, `fixtures/seed.tar.gz`,
`fixtures/reference-{replay,missing}.json`, `logs/arm-admission.txt`,
`logs/staging.json`, and `desktop-check/` (six images, metadata and
`pixels.json`). Restore a fresh seed directory from the archive if needed.
After the checks both private Cargo targets, the mutable seed tree and
disposable trials were removed. The shared toolchain and dependency cache
were left in place. No account, private book, or device registration was accessed.

## Outstanding device gates

Do not run the old collector or assume `WEREAD_HOME` set in an SSH shell
reaches a native indexed-file request. A new approved entry-routing method
must preserve the daily executable, indexed document, private data and peer
registration. Directly launching a staged binary would be a different
endpoint. A separate Aidoku-approved window, current protected inventory,
owner-checked lease, device preflight, warm A/B runs, pixel checks, native
recovery and closeout are still required. No physical timing or default
feature change follows from restoring the build environment.

On November 20 the Aidoku owner explicitly reported **no current frozen
installation/registration projection**. Its November 19 R3 installation
receipt is historical; a CLI-only test that does not read the registration
cannot renew that receipt. Do not select the November 18 guard profile.

The three projects use `/var/tmp/kindle-build-coordination` for status and
the existing **host** `/tmp/codex-kindle-physical-test.lock` for actual device
work. The build userspace has its own `/tmp`; its locks cannot coordinate a
device. Acquire only when ready to operate, release the matching lease as
soon as recovery and transport closeout finish, and never reserve it during
host preparation. Aidoku subsequently reported restoring a pinned worker SSH
configuration from the operator's prior trust record. WeRead did not connect
to the Kindle, acquire a physical lease or independently verify its live
identity. An available transport, absent lock or historical closeout is not
a current device observation or a WeRead window. The [subsequent November 20
physical run](physical-20261120.md) used two fresh windows for a guarded
CPU-only comparison and an independent read-only audit. It did not qualify
the formal Library route.

The [subsequent temporary indexed-entry comparison and deployment](indexed-font-ab-20261120.md)
closed the font promotion gates in later, separately acquired windows. Its
normal-entry retest supersedes the earlier installed-launcher description.
