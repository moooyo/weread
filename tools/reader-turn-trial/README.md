# Isolated Reader turn pilot

This opt-in probe is not part of daily builds. It measures synthetic Slint pointer release on a fixed, upright Scribe 2022 / 5.19.5, not physical touch or optical settling. See the [physical trial record](../../docs/review/performance-opportunities-20261122/physical-reader.md) for the actual status.

**Prepare on `test-env` (Bash).** Use a private `$WORK`, fresh disposable source exports (A: baseline, B: candidate), and the same worker ARM toolchain. Set `$T` to the absolute path of this tools directory. Never patch the working checkout. Generate one public 1,863-chapter seed; reuse its unopened database for every arm. For each export, with `$SRC` and `$ARM` set:

```bash
python3 "$T/seed.py" "$WORK/data"  # once, before either arm
python3 "$T/apply_app.py" "$SRC" && python3 "$T/apply_backend.py" "$SRC"
(cd "$SRC" && WEREAD_TURN_BUILD_ARM="$ARM" CARGO_NET_OFFLINE=true sh scripts/build-kindle.sh)
BIN="$SRC/dist/kindle/weread/app"
test "$(qemu-arm -cpu cortex-a8 "$BIN/weread" --reader-turn-trial-arm)" = "reader-turn-v1 $ARM"
python3 "$T/make_payload.py" --app "$BIN/weread" --helpers "$BIN" --seed "$WORK/data/library.sqlite" --output "$WORK/$ARM.tgz"
```

**Rehearse before taking a lease.** In the worker userspace, run `python3 -B -m unittest discover -s "$T"` and preflight both archives as below. QEMU's arm query checks identity, not firmware or display behavior. Complete the applicable emulator gate and arrange a current peer handoff before any native visit. `$RUNS` must already exist; `$LEASE_TOOL`, `$OWNER` and `$HANDOFF` are the worker's coordinated controls.

```bash
visit=(python3 "$T/visit.py" --runs "$RUNS" --lease-tool "$LEASE_TOOL" --owner "$OWNER" --lane "$ARM" --chapters 1863 --payload "$WORK/$ARM.tgz")
"${visit[@]}" --preflight
"${visit[@]}" --handoff "$HANDOFF"   # only for the authorized visit
python3 "$T/report.py" --visit A "$ID_A" "$RUNS/visit-$ID_A" --visit B "$ID_B" "$RUNS/visit-$ID_B"
```

Run each visit in its planned order; reverse the order for a second pair. The runner acquires the exclusive lease, uses a fresh public stage, restores native state and releases; never install over the daily app or reset private data. If closeout fails, use `visit.py --runs "$RUNS" --lease-tool "$LEASE_TOOL" --owner "$OWNER" --recover "$RUNS/visit-$ID"` before investigation.

The report excludes three warmups from 24 forward turns and separates partial from sixth-turn full refreshes. SEND means driver acceptance; only a successful WAIT for that exact marker supports driver completion. Neither is ink stability. The separate offscreen software runner does not exercise SEND or WAIT and cannot supply missing native results.
