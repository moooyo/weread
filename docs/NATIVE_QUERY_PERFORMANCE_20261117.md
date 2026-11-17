# Native portrait query transport

Date: 2026-11-17 (Asia/Shanghai).
Status: implementation approved for main integration, with completed host, package, and scoped
physical verification. Six balanced warm A/B pairs on Scribe 2022 / firmware
5.19.5 reduced first Library SEND from **9.430 s to 9.090 s**, a **340 ms / 3.6%**
improvement. Every pair favored the candidate. A separate missing-helper launch
passed. The operator approved main integration on 2026-11-17. The normal
installation remains unchanged; source integration does not deploy the package.

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
remain unchanged. Device findings are recorded independently below.

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
| Actual firmware helper | Direct calibration succeeded; all candidate cases confirmed the helper path |
| Native warmups and paired launches | Fourteen successful launches: two warmups and six balanced pairs |
| Helper deliberately absent | One additional launch succeeded using the complete legacy path |
| Final native/software captures | Identical across all 1860 x 2480 pixels, including the fallback capture |

The wake fixture initially counted unrelated required power confirmations as
legacy calibration reads. It was corrected to reject legacy metadata/direction
reads while retaining those independent power fences. The final targeted wake
case passed without changing runtime source. An initial package check expected
an optional ARM floating-point attribute; the actual ELF flags `0x5000400` and
`/lib/ld-linux-armhf.so.3` interpreter establish the hard-float ABI instead.

The new main executable is 95,183,208 bytes, 14,904 bytes larger than current
main. The query helper is 4,820 bytes. WeRead's benefit is measured independently
from Aidoku and is smaller than the reference project's result.

## Physical measurements

Implementation commit: `06a4f70142ffec74f3defa1d983d1c5d4c3e498c`.
The baseline is the cleanly reproduced current-main executable above. Both
variants ran on the same Scribe boot, with the same compiler/release settings,
the same offline demonstration snapshot restored before each case, and the
normal Scriptlet FBInk behavior. The launcher timing markers are matched on
both variants. One warmup per variant is excluded from the six measured pairs.

| Endpoint | Baseline median | Candidate median | Reduction |
| --- | ---: | ---: | ---: |
| Software ready | 8.605 s | 8.230 s | 375 ms / 4.4% |
| First Library SEND receipt | 9.430 s | 9.090 s | 340 ms / 3.6% |
| Native portrait calibration interval | 665 ms | 360 ms | 305 ms |
| Native request to application entry | 4.675 s | 4.350 s | 325 ms |

| Round | Order | Baseline SEND | Candidate SEND | Reduction |
| --- | --- | ---: | ---: | ---: |
| 1 | AB | 9.660 s | 9.080 s | 580 ms |
| 2 | BA | 9.400 s | 9.070 s | 330 ms |
| 3 | AB | 9.460 s | 9.250 s | 210 ms |
| 4 | BA | 9.240 s | 9.100 s | 140 ms |
| 5 | AB | 9.340 s | 8.930 s | 410 ms |
| 6 | BA | 9.560 s | 9.230 s | 330 ms |

All six readiness and first-SEND comparisons improved. Every measured candidate
confirmed the helper transport and a valid calibration; baseline and the
separate absent-helper case confirmed the legacy transport. The direct helper
probe also completed on the actual firmware. Final paired software captures,
native framebuffer captures, and the absent-helper captures matched pixel for
pixel. Input selections agreed across all fifteen launches.

The candidate's font validation/registration interval remains about 3.077 s.
Its 4.350 s pre-exec interval still includes the existing two-second settling
delay and native Scriptlet/ownership work. These are larger remaining costs;
this change does not remove their checks or claim their entire duration as
available savings. Independent stage medians must not be added as one timeline.
The earlier 9.30 s baseline was a different measurement batch; only the
matched 9.430-to-9.090 s comparison above supports this round's gain.

Request and collector receipt timestamps use device `/proc/uptime`, at 10 ms
resolution with ordinary scheduling/pipe delay. Internal intervals use only
`CLOCK_MONOTONIC`. First SEND is not optical panel completion, physical-tap
latency, or completion of the full-frame input barrier. This is warm startup
evidence; physical gestures, sleep/wake, cold boots, other models, and other
firmware do not gain new qualification from this batch. The helper wake path
has separate host fixture coverage.

An initial connection attempt timed out before any device command ran; its
lease was released without upload or launch. After the operator restored
connectivity, a fresh lease covered the completed continuous batch. Native
control and lighting were restored, ordinary application/configuration/database
hashes were unchanged, owned staging and transports were removed, and the exact
lease was released before host analysis. No daily-use installation was replaced.

## Evidence and candidate identity

- [Clean rebuild audit](review/native-query-20261117-4bc333e/build-audit.json).
- [Source/build identity](review/native-query-20261117-4bc333e/candidate-source.json): 533 runtime/build/public-asset files agree with the reviewed checkout after Git text-line-ending normalization; raw tested hashes are retained.
- [Host verification](review/native-query-20261117-4bc333e/verification.json).
- [Real package verification](review/native-query-20261117-4bc333e/package-verification.json).
- [Complete measurements and fallback evidence](review/native-query-20261117-4bc333e/native-query-ab.json).
- [Candidate demonstration capture](review/native-query-20261117-4bc333e/candidate-library.png).

| Artifact | SHA-256 |
| --- | --- |
| Main ARM | `d3a6b663d9fbcacc08e88926c46fe8146c0fc979f8767a735020947600a8be62` |
| Native query helper | `149fa95f3c40ff575c23065fa6127597abbd85ab3e90a50cdbba30fb072f0b5f` |
| Launcher | `05a3b39d61899162658ddf55c7e0251fea8bc2ee6c6b0c4bdeb36f8376520bf3` |
| Frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Candidate development ZIP | `03b438838d25b968331d58cddc4673a7e09ce6e933fc22b01bef006b34860326` |

The named ZIP is `weread-kindle-0.2.0-native-query-20261117.zip`. Its only
member change after the compiled-package checks is the public packaging README;
runtime bytes and permissions remain identical. It is a scoped, verified test
candidate, not a daily-use installation or published release.

Raw logs, mock inputs, frozen variants, and the six-pair/fallback
collector remain under `test-env:/root/weread-native-query-20261117-r1/`.
Private transport configuration and lease tokens are excluded from this record.
