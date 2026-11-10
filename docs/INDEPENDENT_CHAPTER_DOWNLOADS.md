# Independent chapter downloads

Recorded: 2026-11-10 (Asia/Shanghai).

## Accepted behavior

The user explicitly requested independent chapter completion and retry, without
a request-error policy that pauses the entire book. This supersedes the earlier
task-wide `-10102` cooldown described in
[Download settings and Library polish](DOWNLOAD_AND_LIBRARY_POLISH.md).

The configured chapter concurrency remains a maximum, defaulting to eight and
bounded from one through sixteen. A chapter that finishes successfully can be
committed before an earlier chapter. A rejected chapter persists its own retry
deadline or blocked state, releases its worker slot, and leaves other eligible
chapters running. A completed chapter is not fetched again during retry or
restart when its current cache still passes the original integrity and account
checks. Explicit retry of blocked chapters can occur while peers remain active.

User cancellation, account identity changes, and managed lifecycle transitions
retain their existing ownership fences. A persistence failure that prevents
safe state publication still fails closed. These boundaries are separate from
ordinary request errors. Login, purchase, and content-access rules are unchanged.

## Implementation contracts

- The current layout includes normalized task/chapter state keyed by task and stable
  source UID. It stores selection order, current catalog index, completion or
  retry state, bounded attempt count, deadline, and a redacted error.
- `completed_count` reports actual completed chapters. `next_index` retains
  the contiguous-prefix meaning for ordered checkpoints and recovery; it does not
  determine whether a later successful chapter can be published.
- Body/access transitions, image publication, chapter state, and task progress
  use guarded transactional paths. The original ordered Store APIs retain their
  strict contracts. New independent APIs do not bypass UID, owner, body,
  canonical-transition, or image-proof checks.
- Fresh batch image proofs preserve already-complete progress on restart.
  Incomplete images may retain a legitimate Full body without counting that
  image-enabled chapter as complete. Preview and Unknown content never certify
  full completion.
- Completion-order scheduling owns a bounded active set. Retry waits contain
  metadata only and do not retain network, packet, or decoder credits. Explicit
  retry uses a coalescing bounded ordinal bitmap and a notification, after the
  UI has persisted the exact state reset. Active peers and completed results
  are retained across this control action.
- Heavy image acquisition/preparation and UI packet publication remain bounded.
  Credits stay with real buffers and running blocking work through cancellation.
  Large-body admission drains permits without discarding successful peers or
  retaining a small permit while waiting to upgrade to the entire allowance.
- A book is Complete only when every selected chapter is complete. Retry-wait
  and blocked counts remain visible; an incomplete book is never presented as
  successfully finished.

## Reader-page admission

Reader requests continue to obtain fresh per-chapter contexts and entitlement
proofs. No unverified Reader-session or authorization reuse is introduced.

A shared 16 MiB response-reservation pool replaces the single large-page slot.
The task learns response tiers of 512 KiB, 2 MiB, 4 MiB, 8 MiB, 16 MiB, and the
existing 24 MiB transport allowance. An upgrade releases the smaller reservation
before acquiring the next one. A 24 MiB request reserves the whole pool and is
exclusive; it is an explicit exception to the 16 MiB aggregate allowance.

At most two blocking parser workers run concurrently. They retain the actual
HTML and all relevant credits until parsing and buffer release really finish.
Account and renewal generations are checked again after parsing. Existing
parser-work limits can still reject a response below 24 MiB; the transport cap
is not a guarantee of parsability. These are logical reservation bounds, not an
allocator-RSS or physical-device memory claim.

## Pre-release storage policy

The accepted 2026-11-11 cleanup removes the schema upgrade chain, historical task tuple parsing, and task-level retry fields. Current normalized chapter rows own retries and deadlines across restart. Stable UIDs retain state through catalog remapping; removed or mismatched identities cannot transfer completion credit. Terminal cleanup bounds retained chapter-state rows.

This candidate requires a fresh data directory when replacing a versioned development database. The application rejects unsupported layouts without changing records. Keep any earlier executable/data pair privately if needed. The user subsequently authorized a clean Kindle installation without backups after remote verification. Its new receipt is recorded separately; earlier deployment and migration observations below belong only to their named artifacts.

## Verification status

All execution ran on the designated `test-env` worker. The final candidate
passed 1,603 library tests and 52 Main tests. Four manual benchmarks were ignored
by the normal run; the bounded Reader comparison below was executed separately.
The first integration build and two earlier suites retain their failures in
separate logs. Their corrections cover legacy serialization/migration,
cache/admission expectations, and a synthetic error that the source layer had
legitimately retried before the manual-control test boundary.

Controlled HTTP and controller cases verify out-of-order publication while an
earlier chapter is blocked, independent deadlines and exhaustion, restart and
UID remapping, cached work reuse, transaction rollback, cancellation fences,
simultaneous oversized responses, and the active retry ACK/Join/terminal races.
Legacy nine-field task sequences retain their nonzero timestamp; extended
sequences preserve their original retry positions. Missing fields default,
while explicit invalid null values remain errors.

The desktop suite passed all 133 standard checks. Another 44 synthetic offline
checks cover PW6/Scribe progress with completed count two while the first-gap
cursor is zero, the reserved status-row geometry, real pointer actions, retry
of only blocked chapters, retained complete/future-waiting state, and static
waiting pixels. Representative screenshots were visually inspected. These are
desktop evidence, not physical e-ink or Kindle touch measurements.

A bounded synthetic Reader comparison used the same compiled source, sixteen
chapters, roughly 1.35 MiB Reader pages, and a fixed 90 ms fixture response delay.
The single-page configuration simulates the former admission policy; it is not
an execution of the old installed binary. The observed one-run times were
3,383 ms for one page, 1,791 ms for two eligible Reader requests, 997 ms for four,
and 1,059 ms for eight. Actual Reader HTTP concurrency and reservation/parser
bounds were asserted by the fixture. The four/eight results do not establish
a statistically optimal value or a Kindle speedup.

The configured chapter limit remains eight by default. The Reader byte budget
decides how many pages of the currently learned size can be held concurrently;
the parser gate remains two. The close four/eight results in this single sample
do not justify raising the worker limit or removing either resource bound.

## Delivery and remaining acceptance

The static ARM build completed and the development ZIP passed CRC, safe-member,
staged-executable, and launcher identity checks. At that software delivery,
all 558 recorded workspace build and packaging inputs matched the candidate.

| Artifact | SHA-256 |
| --- | --- |
| Source input manifest | `b99a7d725861dbc8aa5ee669710a4e17d5bd4b2c8c03ed91300012f990817905` |
| Desktop executable | `1859934a28cc66f3b4b7e84edf87df65c85e84059376c29b4406f551c0e5b979` |
| ARM executable | `dd8ac6c3e11aec6b9b850125865653f6a877f3bdcf062f9c5ea093a8a4530057` |
| Development ZIP | `c130df685de66141d415f7a504190b497d3b9e6a4a5d63ca80191656dc6a37be` |

Local delivery: [independent-chapter development bundle](../dist/weread-kindle-0.2.0-independent-chapters-20261110-dev.zip).
Local reports and captures are under `artifacts/independent-chapters/`.

The follow-up documentation changes only `packaging/kindle/README.md` among
those 558 frozen input paths; the other 557 still match. Production Rust/Slint,
assets, launcher, build configuration, ARM, and ZIP identities are unchanged.
`artifacts/independent-chapters/followup-input-review.json` records that remote
comparison. The existing software suite was reused for this unchanged candidate
rather than rerun for documentation and private experiment-tool changes.

Evidence is under
`test-env:/root/weread-ui-polish-20261110-r1/evidence/independent-chapters/`.
Software, migration, desktop UI, and ARM gates are complete for this candidate.
At the original software delivery, no Kindle connection or installation had
been performed. The follow-up below separately records the subsequent physical
deployment and bounded observations; this candidate does not inherit physical
acceptance from its predecessor.

## Native Reader comparison and physical follow-up

Recorded: 2026-11-11 (Asia/Shanghai). All host-side execution and analysis used
`test-env`. The explicitly identified physical target remained first-generation
Kindle Scribe 2022 / firmware 5.19.5. No local verification was performed.

### Native controlled fixture

The separately built native ARM fixture used the reviewed source input above,
with a test-only loopback fixture. Each setting read sixteen synthetic chapters
from a 1,863-entry catalog, using gzip, a fixed 90 ms response delay, and roughly
1.35 MiB decoded Reader pages. There was one sample per setting, in fixed order,
with one warm-up read. This excludes images, application coordination, durable
checkpoints, UI, and real upstream conditions.

| Chapter workers | Elapsed ms | Actual Reader HTTP peak | Peak reservations | Parser peak |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 3,620 | 1 | 2 MiB | 1 |
| 2 | 2,004 | 2 | 4 MiB | 2 |
| 4 | 1,330 | 4 | 8 MiB | 2 |
| 8 | 1,340 | 8 | 16 MiB | 2 |
| 16 | 1,402 | 8 | 16 MiB | 2 |

The fastest single sample used four workers, only 10 ms (0.75%) ahead of eight.
The fixed byte pool limited sixteen configured workers to eight actual Reader
HTTP requests for these pages. This supports a measured saturation range around
four/eight, not a statistically established optimum. Neither the default eight,
the 16 MiB pool, nor the two-parser limit changes. Whole-process 100 ms sampled
RSS peaked at 24,700 KiB across all five cases; per-case RSS was not captured.
That fixture memory figure is not the production application's font/UI memory.

All five cases passed and the process exited zero. The scratch executable and
device transports were removed, native dark metadata matched, and the exact
lease was released at 2026-11-10 23:42:48.905739 +08:00. An independent absence
check followed. The original result-analysis failure was a parser expecting the
marker at the beginning of a Rust test-runner line; offline analysis now accepts
the observed test-name prefix without rerunning the benchmark.

The anonymous receipt is
`artifacts/independent-chapters/physical-reader-benchmark-summary.json`.
Private evidence remains under
`test-env:/root/weread-independent-physical-20261110-r3/evidence/`.

### Production deployment and eight-worker window

The reviewed ARM `dd8ac6c3` was installed after a fresh coherent private backup
of the schema-2 `985efefc` executable and data. Launcher `a5fec72f` and frame
guard `a235b41c` were retained. Ordinary startup used the installed document
entry with normal production arguments. No demo, offline, scripted UI, test
screen, product timer, report-preference change, or replacement task was used.
Only the existing eligible current-account task could run.

The first startup guard ended after ten seconds with schema 2 still observed
and no new download-start record.
The attempted stop exposed the device's integer-only `sleep` command. Immediate
identity-bound recovery used integer seconds, verified cold/native state,
removed the helper, restored settings, and released that lease at 00:04:30.682944
+08:00. The failed receipt is retained. The next fresh batch allowed at most
45 seconds for cold initialization and required a new process-local download
record before treating a normalized task snapshot as a valid window.

The valid eight-worker window had 25 samples over 24,360 ms. Every sample kept
the same privately verified task/account/selection, monotone completion and CPU
counters, and normalized chapter-state totals. The exact t0/t1 log byte segment
was proven against the cold capture, including line boundaries and the absence
of rotation. Durable completion increased from 288 to 309 of 1,846 selected
chapters: 21 chapters, or 0.8621 chapters/second.

- CPU: 82.88% of one core, using the target's measured 100 Hz clock.
- Sampled RSS peak: 241,632 KiB; process-lifetime HWM: 246,812 KiB.
- Download-lifetime chapter-read/HTTP peaks: eight/eight. These may include
  startup before t0; periodic in-window HTTP-active samples peaked at four.
- Reader admission: eight pages and 16 MiB peak reservations; parser peak two.
  These are admitted fetch/wait/parse lifetimes, not Reader-only HTTP concurrency.
  Production Reader-only HTTP peak is unavailable.
- t1: 31 retry-wait, 187 blocked, and 1,319 ready chapters, with the task Running.
  The later cold snapshot contained 193 blocked, 31 retry-wait, 309 complete,
  and 1,313 pending chapters. It is not the t1 snapshot.
- The exact window contained 194 Reader-session HTTP-426 diagnostic events and
  31 `-10102` occurrences. Cold blocked errors were `source.http.426`; waiting
  errors were `source.api.-10102`. Counts of log events do not equal unique
  failed requests, and this experiment does not establish their upstream cause.

Completion and contiguous-prefix counts were equal in every physical sample.
The window therefore does not prove out-of-order completion on the device.
Likewise, its duration deliberately excludes waiting 60/120/240 seconds for a
retry deadline. Controlled remote regressions remain the evidence for those
recovery cases. Existing successful progress and all new checkpoints were kept;
no database was replayed between settings.

The anonymous receipt is
`artifacts/independent-chapters/physical-production-summary.json`.
Private progress, CPU, exact log segments, and cold database captures remain
under `test-env:/root/weread-independent-production-20261111-r5/evidence/`.

### Sampling failures, restoration, and remaining gates

The initial four-worker qualification checked the new download marker while
the new process was still starting and observed only the previous process's
normalized task state. That attempt stopped before a valid window. A corrected
four-worker follow-up failed its full process/display identity gate within the
45-second startup boundary. Its application download logs are partial evidence,
not the planned normalized 25-second window or a fair production ranking.
The original failure receipts are retained; they are not passing UI/lifecycle
results and do not by themselves establish an application startup fault.

That last attempt nevertheless has separate partial download evidence: the
exact new log segment contains one four-worker start, with 30 checkpoints and
59 Reader responses over a 29,643 ms task lifetime. Cold snapshots retain the
same private task/account/selection and advance 309 to 339 complete chapters.
Admitted Reader pages peaked at four, reservations at 8 MiB, and parser workers
at two. The combined Reader/shard HTTP peak was six; multiple shard reads mean
that counter need not equal the chapter-worker limit. Actual Reader-only HTTP
peak, qualified fixed-window CPU, and RSS remain unavailable. The segment also
contains 207 HTTP-426 log events and 29 `-10102` occurrences. These observations
are not a replacement for the failed identity gate or a fair eight/four ranking.
The anonymous receipt is
`artifacts/independent-chapters/physical-four-worker-followup.json`; raw evidence
remains under `test-env:/root/weread-independent-production-20261111-r6/evidence/`.

All device batches restored the original concurrency field to eight, retained
new cache/retry/session changes, verified native dark metadata and closed
transports, and performed owner/token-checked release. The later releases were
2026-11-11 00:10:03.618297 and 00:14:03.544744 +08:00, each followed by an independent
absence check. No WeRead device window remains reserved. The temporary SQLite
helper was removed, and the original executable/data backup remains private.
The helper's sixteen synthetic checks and cross-build ran only on `test-env`;
it was an experiment utility, not a new product dependency or package input.

Software and ARM gates remain complete for the unchanged candidate. Physical
deployment, schema migration, one bounded production window, and restoration
are now recorded. The upstream errors and incomplete four-worker qualification
leave a production optimum unestablished. New-candidate visual/touch acceptance,
physical out-of-order publication, retry-deadline and manual-retry behavior,
full-book completion, sustained throughput, USB, reboot, and other model/firmware
claims remain separate gates. The earlier user-confirmed stable Library,
default eight, and touch results retain their original executable scope.

## Relaxed Reader budget experiment

The user subsequently authorized larger Reader budgets and parser concurrency.
The separate [experiment record](READER_BUDGET_EXPERIMENT.md) compares 16/32 MiB
and two/four parser permits at fixed chapter concurrency, with a default-eight
check. All eighteen native Scribe 2022 / firmware 5.19.5 cases passed. Larger
limits did not produce reliable total-time gains for the measured synthetic
workload, so this implementation retains its 16 MiB/two-parser policy and the
installed candidate. The device was restored and exactly released at
2026-11-11 01:20:25.081063 +08:00 before results were analyzed.
