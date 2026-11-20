# Handoff: Kindle Library startup

Updated 2026-11-20 (Asia/Shanghai). **Next milestone: measure the font change
on the physical Kindle.** The experiment is integrated as a default-off Cargo
feature, `bundled-font-preflight`. No release, installation, or device change
is part of this handoff. Keep the two-second launcher wait and both orientation
queries unchanged; the new orientation markers are diagnostic only.

Resume from `origin/main` containing experiment commit `7b41404` (based on
`c810ea5`). The `source_base` in the build records names that parent; the
recorded source manifest and executable hashes identify the measured builds.

## What is known

| Environment and endpoint | Result | Boundary |
| --- | --- | --- |
| Desktop release, `test-env`, six warm A/B pairs | Font stage: 325.337 -> 0.721 ms. Rust main to software ready: 456.981 -> 115.500 ms. | All six pairs improved. Library, Reader, and one-missing-cover qualification images matched pixel for pixel. Software ready is not display submission. [Samples](docs/review/library-startup-latency-20261119/desktop-ab.json) |
| ARMv7, QEMU user mode | All seven fixed fonts matched the original resource accounting, including seven restricted-budget failure cases. | Target-ABI correctness, not a firmware run or a hardware speed measurement. Custom fonts still take the full runtime path. [ARM checks](docs/review/library-startup-latency-20261119/arm-admission/README.md) |
| Earlier physical Scribe 2022, firmware 5.19.5, November 17 | Median 9.820 s (9.610-10.230 s), from the indexed file-open request to the first Library driver SEND. | Older offline candidate, not the current daily installation, finger-to-ink latency, or the result of the font change. [Physical baseline](docs/review/native-library-entry-20261117/README.md#formal-entry-measurements) |

The completed November 19 runs passed 56/56 default Main tests, 58/58 opt-in
Main tests, and 150/150 Kindle backend tests on the Linux host. The feature
prechecks only the seven immutable font payloads at build time. The corrected
build used all seven receipts in the candidate and none in the control. Do not
reuse the earlier anonymous-storage candidate, which could miss a receipt.
See the [experiment report](docs/review/library-startup-latency-20261119/font-preflight-experiment.md).

## Resume here

1. Follow [AGENTS.md](AGENTS.md). Fetch `origin/main`, inspect the checkout,
   and re-establish `ssh test-env` **before** planning device work. It became
   unreachable before alternative staging was verified. Do not fall back to
   local verification or direct, uncoordinated Kindle access.
2. Verify the retained ARM executables and public fixture against the [ARM
   build hashes](docs/review/library-startup-latency-20261119/arm-build.json)
   and [fixture identity](docs/review/library-startup-latency-20261119/desktop-ab.json).
   The last known host directory was `/root/weread-font-ab-20261119.HXItGgya`;
   its availability after recovery has not been checked. The following is a
   read-only PowerShell command for the next operator, not a device launch:

   ```powershell
   ssh test-env 'cd /root/weread-font-ab-20261119.HXItGgya && sha256sum bin/arm-baseline bin/arm-candidate fixtures/seed.tar.gz'
   ```

   Expected prefixes: `e73f99bdd62c` (A, `kindle`), `af54acc18a6f` (B,
   `kindle,bundled-font-preflight`), `446e1afb4b17` (offline 15-cover seed).
   Check **full** hashes before reuse. Stop on mismatch; missing artifacts
   require a newly audited, identically built pair, not a substitute binary.
3. Coordinate a fresh exclusive window with the priority owner, freeze the
   current peer installation and shared registration expectations, then use
   a new owner-checked lease for the actual device batch. The previous R3
   handoff superseded the November 18 protection profile; neither an old
   receipt nor an absent lock grants a window. Do not run the old November
   17 collector unchanged. See [shared-device rules](docs/CATALOG_ENTRY_ADAPTER.md#rehearsal-and-measurement-boundaries).
4. Verify isolated staging and cleanup on `test-env` first. The standalone
   CPU probe is 61.5 MiB and the full apps are larger. **Do not put them in
   the Scribe's 64 MiB `/tmp`/`/var/tmp` filesystem** or evict user caches.
   The alternative user-partition staging draft was not verified. Do not
   overwrite the indexed document, daily installation, private data, or any
   peer registration to get a comparison to run.
5. On the confirmed Scribe 2022 / 5.19.5, use the same entry, launcher, public
   fixture, cache policy and boot for both arms. Exclude one warmup per arm,
   then run `AB, BA, AB, BA, AB, BA`. Record first Library SEND (or an
   independently qualified shelf-pixel bracket), plus `splash_orientation_*`
   and `event_loop_orientation_*` with `WEREAD_STARTUP_TIMING=1`. Keep clock
   domains consistent. An actual tap, all covers present, and stable ink are
   separate endpoints; neither software-ready nor a CPU probe measures them.
6. Separately compare Library/Reader pixels and confirm native display,
   input and lighting recovery, protected-state integrity, cleanup and lease
   release. Keep all samples, including failures. Only then decide whether
   to enable fonts by default; consider reusing orientation only if the new
   markers demonstrate redundant work. A shorter launcher wait needs its own
   startup-interruption, sleep/wake and exit experiment.
