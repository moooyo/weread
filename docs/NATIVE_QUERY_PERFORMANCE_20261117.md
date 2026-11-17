# Native portrait query transport

Date: 2026-11-17 (Asia/Shanghai).
Status: implementation candidate; host verification and packaging passed.
The physical preflight could not connect to the Kindle over SSH. No candidate
was uploaded or launched, and the fresh lease was released. Device performance
and actual firmware-helper qualification remain blocked pending connectivity.

## Scope and build audit

This follows the operator-authorized cross-project startup work. The baseline
is WeRead main `4bc333e9321376a3fda23940368b77a187255b71`, retaining the approved
penguin cover. Aidoku implementation `4e069260fa612e465b8fde8730596a0c60e0261f`
is a design and ABI reference, not a WeRead performance result.

Aidoku reported a stale Cargo dependency after source copies preserved older
mtimes. Before proceeding, both of WeRead's previous measured variants were
rebuilt after explicitly cleaning `weread` and `slint-backend-kindle`. Each
audit used an independent writable overlay above a read-only previous cache;
no other task's cache was modified. Logs show both project packages recompiling.

Both rebuilt executables match their previously measured bytes exactly:

- Previous baseline: `00a763f9372bf0b03650d456959b903cb6ea1241bb5966097a2a3742f7f4be67`.
- Current-main baseline: `5271918d104eeb3d8dad630b8bb0cfa33525970201d2b891da6d1fc709fc453f`.

The earlier WeRead comparison remains valid. Its optional startup timing code
is intentionally present in both published source variants and binaries.
That is distinct from diagnostic-only source missing from a manifest. New
authoritative ARM builds also clean the main and backend packages first.

## WeRead-specific adaptation

The old portrait transaction launches four `lipc-get-prop` readers and two
read-only application probes, with per-value shell framing processes. A single
early Rust probe now owns one helper session and performs the framebuffer and
sensor observations directly, preserving the original order:

1. Require active power.
2. Read current sensor capability and framebuffer rotation.
3. Read native direction when automatic portrait observation is available.
4. Repeat the framebuffer/sensor read and require an identical pair.
5. Repeat and compare native direction when applicable.
6. Require active power again, then require helper EOF and successful exit.

The helper accepts only two fixed read-only opcodes: power state and native
accelerometer direction. Each session admits at most four requests and each
value at most 128 bytes. It exposes no setter or caller-supplied property name.
The main program remains static musl; the small independent C process uses the
device's glibc/LIPC ABI. No firmware library is redistributed. The source and
protocol are project-owned; the project license remains unselected.

The Rust transport shares the existing eight-slot command registry, operation
deadline, bounded direct-child cleanup, retained-child ownership, and quiescence
checks. A missing helper, unsupported execution ABI, or incomplete initial
handshake can return unavailable only after child cleanup. A connected query
failure, changed observation, malformed response, trailing bytes, nonzero exit,
or unconfirmed cleanup cannot fall through to a mixed legacy sample.

The launcher keeps its existing process-group watchdog, nonce, process-birth,
generation/cycle, power-event, and publication checks. The legacy transaction
remains the complete fallback when the helper is unavailable. Initial and
resident-wake calibration use the same path. Existing runtime battery/lighting
workers and setters are unchanged; WeRead does not have Aidoku's synchronous
first-view status snapshot to replace.

## Verification boundary

All execution runs through `ssh test-env`, with six Cargo jobs. Builds use the
same thin-LTO, one-codegen-unit release profile as the previous WeRead baseline.
Firmware access requires a fresh shared lease after the artifacts, fixture,
and collection tools are ready. The daily installation and private data must
remain unchanged. Device findings will be recorded independently here.

| Check | Result |
| --- | --- |
| Application library | 1,707 passed; four existing benchmarks ignored |
| Main/hidden CLI dispatch | 56 passed |
| Native C helper with a private mock library | Six passed |
| Development archive contracts | Five passed |
| Launcher suite | 215 passed |
| Additional helper resident-wake case | One passed |
| Formatting and clean ARM build | Passed |
| Real development archive | 37 explicit members; CRC, modes, image, and runtime bytes passed |

The wake fixture initially counted unrelated required power confirmations as
legacy calibration reads. It was corrected to reject legacy metadata/direction
reads while retaining those independent power fences. The final targeted wake
case passed without changing runtime source. An initial package check expected
an optional ARM floating-point attribute; the actual ELF flags `0x5000400` and
`/lib/ld-linux-armhf.so.3` interpreter establish the hard-float ABI instead.

The new main executable is 95,183,208 bytes, 14,904 bytes larger than current
main. The query helper is 4,820 bytes. Actual device loading, six balanced
warm A/B rounds, native-frame comparisons, and a missing-helper launch are
prepared but have not run. No WeRead timing benefit is claimed from Aidoku's
measurements or the host mock tests.

## Evidence and candidate identity

- [Clean rebuild audit](review/native-query-20261117-4bc333e/build-audit.json).
- [Source/build identity](review/native-query-20261117-4bc333e/candidate-source.json): 533 runtime/build/public-asset files agree with the reviewed checkout after Git text-line-ending normalization; raw tested hashes are retained.
- [Host verification](review/native-query-20261117-4bc333e/verification.json).
- [Real package verification](review/native-query-20261117-4bc333e/package-verification.json).

| Artifact | SHA-256 |
| --- | --- |
| Main ARM | `d3a6b663d9fbcacc08e88926c46fe8146c0fc979f8767a735020947600a8be62` |
| Native query helper | `149fa95f3c40ff575c23065fa6127597abbd85ab3e90a50cdbba30fb072f0b5f` |
| Launcher | `05a3b39d61899162658ddf55c7e0251fea8bc2ee6c6b0c4bdeb36f8376520bf3` |
| Frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Candidate development ZIP | `03b438838d25b968331d58cddc4673a7e09ce6e933fc22b01bef006b34860326` |

The named ZIP is `weread-kindle-0.2.0-native-query-20261117.zip`. Its only
member change after the compiled-package checks is the public packaging README;
runtime bytes and permissions remain identical. It is an unqualified test
candidate, not a deployed or published release.

Raw logs, mock inputs, frozen variants, and the prepared six-pair/fallback
collector remain under `test-env:/root/weread-native-query-20261117-r1/`.
Private transport configuration and lease tokens are excluded from this record.
