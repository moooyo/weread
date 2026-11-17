# Formal Library entry lifetime

Status: implementation `1498dd7` merged to main with operator approval on
2026-11-17. Host, formal-entry
startup, and scoped physical sleep/wake qualification have passed. First Library
submission improves from **10.390 to 9.820 seconds** in the six-pair comparison;
five pairs improve and one is slower. The ordinary device installation remains
unchanged; source integration does not deploy the package.

## Scope and implementation

The operator authorized the cross-project startup investigation. Aidoku commit
`bed9167565106bb0e6382e7890b1a1170cc143e6` is the implementation reference;
[its report](https://github.com/moooyo/aidokuForKindle/blob/06b2149/docs/review/native-library-entry-20261117/README.md)
is not a WeRead measurement. WeRead starts from main
`abc0b4073059cf162fb1a408a9a5b70ce70bb378`.

The previous document returned as soon as it spawned the supervisor. That
allowed the shared Scriptlet to finish and Library to reactivate while WeRead
was starting. The candidate document invokes `launch.sh --library` and waits
for the actual supervisor status, including native restoration before exit.

WeRead previously packaged its entire supervisor as the Library document.
The candidate places the document wrapper at `documents/weread.sh` and the
supervisor at `weread/launch.sh`. Both are required executable archive inputs.
The document retains the same title and exact accepted penguin thumbnail in
its third-line Icon metadata. Both shell files must be installed together.

The waiting process owns neither the launcher lock nor display/input resources.
The worker retains its independent session and all existing native ownership,
sleep/wake, calibration, nonce, process-birth, generation, and restoration
checks. HUP, INT, and TERM latch their first cancellation status across worker
registration, signal only the directly spawned birth-verified worker, and wait
for cleanup. The mutable launcher lock never selects a cancellation target.
Retiring only the waiting process leaves the independent supervisor alive.

Direct `launch.sh` invocations retain detached admission. Explicit arguments to
the document are forwarded for existing supervised diagnostics. There remains
one Library item. No shared handler, MIME association, scanner, recent-access
behavior, daemon, interposition library, or extra application registration is
introduced. The app, helpers, UI, database layout, network adapter, and cover
pipeline are unchanged.

## Verification method

All execution is on `ssh test-env`; no local tests or application probes run.
The entry-only candidate reuses the exact previously qualified ARM application
and two helpers. The source audit compares 562 source, UI, asset, vendor, and
build-configuration files with main and the previous frozen build. The only
previous-build source difference is the already-recorded CRLF/LF spelling in
`packaging/kindle/native_query.c`; its normalized source is identical. No Cargo
or native compiler is invoked and no other task's cache is modified.

The unmodified production staging tail packages those audited binaries with
the candidate shell files. The archive has 38 explicit members; missing
supervisors, executable permissions, CRC, payload bytes, metadata, accepted
thumbnail, and private-data exclusion are checked separately.

The complete remote launcher suite passes **223 tests**, including the seven
new Library ownership/status/signal cases. The archive suite passes **eight
tests**, and all three changed shell files pass POSIX syntax checks. See
[host evidence](verification.json). The original detached-entry managed-log
case and all existing native/sleep recovery tests remain enabled.

Physical measurements use the identified **Kindle Scribe 2022 / firmware
5.19.5**. Each batch acquires a fresh shared lease only after its artifacts,
public offline fixture, and collection scripts are prepared. Native ownership,
lighting, the original document, and ordinary installation/data are restored
before removing owned staging and releasing the exact lease.

The formal document is opened through its real system `file://` route. A
common instrumentation envelope preserves installed metadata and routes the
control to the previous detached supervisor and the candidate to its waiting
mode. Both variants use the same application/helper files, the same restored
offline demo snapshot, unchanged shared handler and registration, and eight
seconds of native stabilization before each request outside its measured
interval. Warmups and capture/lifecycle qualifications are separate from
balanced AB/BA timing samples.

External request-to-ready/SEND intervals use collector `/proc/uptime` receipt
timestamps. Internal markers retain their monotonic clock and are not
subtracted from uptime. The endpoint is the first Library driver submission,
not optical panel completion or physical tap latency. This fixture does not
exercise network cover loading; no complete-network-cover speedup is claimed.
Earlier temporary-document timings and Aidoku results are not directly
comparable absolute baselines.

## Preparation observations

- The first scoped test run confirmed exit-status propagation, same-process
  sleep/wake, and independent recovery after entry retirement. Three new signal
  tests initially expected one repaint; the existing supervisor intentionally
  repaints on both sleep handoff and final recovery. Their assertions were
  corrected without changing runtime behavior.
- The initial device preparation batch `library-entry-50c5828c` failed before
  replacing the formal entry or launching WeRead because the user partition
  cannot create symbolic links. Both variants now address one shared staged
  application directory directly. Native state and lighting were confirmed,
  owned staging was removed, and the lease was released. It supplies no timing
  sample.
- Batch `library-entry-4edae423` stopped during candidate warmup when the
  collector required an asynchronously published supervisor ownership record
  that was not yet present. File timestamps show the application produced its
  final image three seconds after the failed probe; the earlier attribution to
  exit cleanup was incorrect. The collector
  never published completion and its enclosing command timed out. No primary
  timing pairs had started. The corrected collector records the committed
  driver marker and EOF without optional live-process probes; lifecycle
  inspection belongs to longer qualification cases. Two remote synthetic
  [collector cases](collector-verification.json) confirm successful completion
  without an ownership record and rejection of EOF without a driver marker.
  The original entry and native state
  were restored, owned staging removed, and the exact lease released. This
  sampling failure is preserved separately from application correctness.

The successful six-launch pilot `library-entry-b95f5725` preserved one catalog
item, MIME and thumbnail fields, and updated document/recent-access timestamps.
Candidate qualification retained the shared Scriptlet as the active framework
application; the control had already returned to native Home. Both actual
framebuffer captures exactly matched their corresponding software screenshots.
Control/candidate pixels below the dynamic 56-row status bar were identical;
whole-frame differences are retained in [pilot evidence](pilot.json). Its one
timing pair improved first SEND by 650 ms, which is not yet a stable performance
conclusion. Qualification launches are not timing samples.

## Formal entry measurements

Authoritative startup batch: `library-entry-8cbce1e6`. Six predetermined AB/BA
pairs follow one warmup per variant; two separate capture qualifications follow
the measured pairs. All sixteen launches completed and restored native state.
See [every observation](comparison.json).

| Endpoint from the native file-open request | Previous detached entry | Library lifetime entry | Difference of medians |
| --- | ---: | ---: | ---: |
| Software ready | 9.505 s | 8.980 s | 525 ms / 5.5% |
| First Library driver submission | 10.390 s | 9.820 s | 570 ms / 5.5% |
| Rust main entry | 5.570 s | 5.030 s | 540 ms |

Five of six pairs improve both primary endpoints. The third pair is 180 ms
slower at software readiness and 220 ms slower at first SEND; it is retained.
The paired first-SEND savings are 1,100, 380, -220, 330, 660, and 620 ms, with
a 500 ms median paired saving. Independent variant medians give the 570 ms
difference in the table. Candidate first-SEND samples span 9.610-10.230 s;
controls span 10.010-10.710 s. This is a measured median improvement, not a
guarantee that every launch is faster.

Most of the observed gain precedes Rust main. Font registration is still about
3.1 seconds and is unchanged by this entry work. No new font, cover-processing,
network, input-readiness, or optical-completion improvement is attributed to it.
Earlier temporary-entry absolute times are not interchangeable with this batch.

All six software image pairs match below the live 56-row status bar. Four
also match over the complete image; two differ only in that status bar. Both
actual-framebuffer qualifications match their software snapshots over all
1860 x 2480 pixels, and the final control/candidate native images also match
over the complete frame. No rotation, polarity, contrast, or pixel normalization
is used to conceal differences.

Every case retains one catalog row and its MIME/thumbnail fields, and advances
the real document modification time. Recent-access updates are retained in the
evidence. The candidate's active framework application remains the shared
Scriptlet during qualification; the control has already returned to native
Home. The installed document, application, helpers, profile, allowlist, and
ordinary library retain their original bytes after restoration. Lighting and
native ownership are restored, owned staging removed, and the lease released
before analysis.

The physical envelope intentionally preserves the installed document metadata.
That installation does not yet use the accepted penguin cover. Its exact bytes
are checked in the candidate package; this batch does not claim to have deployed
or visually accepted that cover on the native Kindle Library.

## Physical sleep and wake

Separate batch `library-lifecycle-24cb23f3` contains two warmups and one physical
screen-saver/scheduled-wake cycle per variant. Both runs stop and resume the
same application PID and birth, retain matching per-cycle sleep/wake records,
and complete their managed neutral-cover handshake. Each variant's actual
pre-sleep and resumed framebuffer is identical over all 1860 x 2480 pixels.
Control/candidate comparisons and later software snapshots match below the
live status bar; the whole-image differences are retained in
[lifecycle evidence](lifecycle.json).

The candidate retains the shared Scriptlet as the active framework application
before and after wake. Both applications then complete their normal scheduled
exit. The original document and ordinary installation/data retain their bytes,
native control and lighting are restored, owned staging is removed, and the
fresh lease is released. The local six-second scheduled wake bounds the cycle;
extended deep suspend is not qualified. The separate host SIGKILL case qualifies
waiting-entry retirement while the supervisor retains recovery ownership.

## Delivery and remaining acceptance

The development ZIP contains the matching document and supervisor and the exact
qualified application/helpers. [Delivery identity](delivery.json) records the
complete hashes, archive membership, source-audit digest, and package checks.

| Runtime input | SHA-256 prefix |
| --- | --- |
| Unchanged static application | `d3a6b663d9fb` |
| Unchanged frame guard | `689ef367eabe` |
| Unchanged native query helper | `149fa95f3c40` |
| Candidate supervisor | `a619c909c9c7` |
| Candidate document with accepted penguin metadata | `085efcac8d33` |
| Development ZIP | `35a4717daf86` |

The operator approved main integration on 2026-11-17. Implementation `1498dd7`
and its evidence are merged to main; the ordinary device installation remains
unchanged. Authenticated use, extended deep
sleep, physical touch latency, other model/firmware combinations, and installation
of the accepted penguin cover remain outside this entry-only comparison.
