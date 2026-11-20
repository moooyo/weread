# ARM32 outline-admission check

This research probe embeds the seven product OTFs and the receipts emitted by
the actual `bundled-font-preflight` Cargo build. It calls the product's slow
and receipt paths from `main_glyph_budget.rs`, `main_glyph_cff.rs`, and
`main_font_budget.rs`. The build script extracts the unchanged static-slice
lookup from `src/main.rs`; the harness does not implement its own trust test.
The probe prints public counts and durations. It does not open a framebuffer,
input device, database, network connection or application data directory.

`--verify` regenerates each host-built receipt on ARM and checks that all four
loop fields agree. Full and fast `FontUsage` must agree for each face on its
own and at each step of the cumulative seven-face run. Truncated and copied
font slices must not select a receipt. Seven lower budgets cover total work,
visits, points, target-sized parser state, and per-glyph memory/visits/points;
both paths must return the same error and preserve identical partial charges.
These are outline-admission checks. Slint registration, text metrics, frame
pixels and elapsed Library startup time require their own comparisons.

`--bench` is optional and separate. The same optimized ELF times the seven
faces in normal application order, with fresh budgets for each path. Round 0
is a warmup, followed by six alternating full-first/receipt-first pairs. Each
pair checks equal accounting; monotonic `full_ns` and `receipt_ns` exclude
process launch and printing. Cache residency is not controlled outside this
process. A QEMU run is **not** a Kindle performance measurement.

`--bench-guarded PARENT_PID` additionally requires the caller's live shell
PID on entry and after arming parent-death `SIGKILL`, and disables core dumps.
It runs the same benchmark only after also arming a 120-second one-shot
`CLOCK_BOOTTIME` timer with `SIGKILL` in the probe process. It
does not create another process, wake a sleeping device, or depend on an SSH
transport surviving. An unsupported clock/timer fails closed before sampling.

## Reproduce on the verification host

Use the **same frozen source** for this harness and the product build. In a
Linux shell on `test-env`, set `FEATURE_OUT` to the product Cargo `OUT_DIR`
containing the generated `bundled_fonts.rs` and `fonts/*.otf` and set
`ARM_RELEASE_DEPS` to the matching compiler's `armv7-unknown-linux-musleabihf`
release-dependencies directory. `rustc` must be on the `PATH` and have this
Rust target installed. Select a fresh private output directory; the script
will not overwrite one that already exists.

```sh
sh docs/review/library-startup-latency-20261119/arm-admission/build.sh \
  "$FEATURE_OUT" "$ARM_RELEASE_DEPS" "$NEW_PRIVATE_OUTPUT"
"$QEMU_ARM" -cpu cortex-a8 "$NEW_PRIVATE_OUTPUT/arm-font-admission" --verify
```

The executed QEMU is Debian `qemu-user` 10.0.13, from a package privately
extracted on the verification host without installation or changes to
`binfmt`. The ELF is statically linked ARM EABI5 hard-float. No firmware VM,
shared Kindle session, or account is required for this check.

## Result on November 19

Using the corrected named backing arrays, all seven faces matched their
host-generated receipts, fresh/cumulative budgets matched, and all seven
negative budget controls preserved their errors and partial accounting.
The final ARM32 peak for target-sized parser state was **4,544 bytes**.
The seven-face total was 262,710,115 units of glyph work, 2,959,385 visits,
and 29,888,387 points. The per-glyph peak was 79,248 bytes. Other `FontUsage`
fields are zero here because this probe measures only outline admission.
See the [complete stdout](verification.txt) and [input identities](evidence.json).

Separate ARM QEMU fixtures compiled the same deadline module. An invalid
clock and zero interval were rejected. A one-second timer delivered `SIGKILL`
to its own process, and a guarded child received `SIGKILL` when its verified
test parent exited. The final ELF rejected a mismatched parent PID with status
2 before emitting any sample. These do not establish device-kernel support or
confirm recovery of an interrupted physical run. No QEMU speedup is reported.
