# Download settings and Library polish

Recorded: 2026-11-10 (Asia/Shanghai).

## Accepted scope

The user requested configurable chapter-download concurrency with a default of
eight, stable Library loading with many books, and removal of redundant
explanatory interface copy. This authorizes bounded cross-chapter fetching and
supersedes the earlier serial-only acquisition decision. It does not change
content entitlement, the local-cache architecture, or device compatibility.

## Implementation

- Settings > Downloads and storage exposes a chapter-download concurrency
  stepper. The default is eight, the supported range is one through sixteen,
  and the preference survives restart. Existing preference records receive
  the same default. A task captures the setting when it starts or resumes.
- Chapter text can be fetched concurrently. Checkpoints remain ordered and
  advance only after durable acknowledgment; image preparation remains serial.
  Admission follows owned work through cancellation and queued results.
  A preview or failed chapter cannot be skipped by a later completed fetch.
  The first candidate capped each concurrent decoded response at 512 KiB and
  each completed HTML/text/style result at 4 MiB. Its task-wide serial fallback
  was found to affect ordinary long catalogs during the physical follow-up
  below. The corrective source separates Reader-page acquisition from shard
  limits: small Reader pages remain concurrent; a larger page is acquired and
  parsed under one shared full-budget permit, and the task remembers that mode.
  Shards retain their 512 KiB limit and completed results retain the 4 MiB
  admission check. An oversized chapter body is retried alone, then a successful
  durable acknowledgment restores the configured window. These are logical
  payload limits, not peak decoder or process-RSS guarantees.
- Cover arrivals no longer mark the entire route for reconstruction or request
  a full-screen flash. Each application tick patches the current visible rows
  as one batch, retaining row order, selection, and pagination. Offscreen
  completions only populate the local cache. Cover refreshes use dirty-region
  UI updates and do not spend the periodic content-refresh budget. Reader and
  article image refresh policies remain independent.
- The interface omits repeated login introductions, local-storage explanations,
  download guidance, font-import and refresh explanations, and redundant image
  or typography notes. Compact labels, actionable errors, content-access
  status, and destructive-action confirmations remain available. Panels shrink
  with their removed text instead of retaining empty explanatory space.

## Verification and acceptance

Verification runs on the designated `test-env` worker. The integrated
`cargo test --locked --all-targets` run passed 1,507 library tests and 52 Main
tests, with three existing manual benchmarks ignored. The new cases cover
preference migration and limits, concurrent requests against synthetic HTTP
fixtures at the default and boundary values, out-of-order results, rejected
checkpoints, cancellation, account changes, closed receivers, oversized-chapter
fallback, shared session
renewal, stable visible cover models, and offscreen cover completion.

The desktop suite passed all 133 interface and persistence checks, including
the concurrency stepper and restart persistence. Six additional captures passed
for the font, page-turn, and article-image panels at PW6 and Scribe geometries.
Those captures and the Login/download settings screens were visually inspected.
Formatting and the 464-key translation check also passed.

The static ARM application and the development ZIP were built on the same
worker. The ZIP contains 36 approved build/notice entries, passed CRC checks,
and contains the matching staged ARM executable. The final workspace archive
matches all 541 recorded build and packaging inputs. These identities bind the
passing software checks and the delivery artifact:

| Artifact | SHA-256 |
| --- | --- |
| Source input manifest | `607cde1a9e7baf2b4ff34673a8fe830f96796d3d58b40bc61c848c877cf5ed66` |
| Desktop executable | `a124f37917e2332da0751da962ec5494f95b3742c50330101126018bc74b00c9` |
| ARM executable | `74e01933c32c92096346f6e3a3dc0b3b7444e36e61538ef2499fc8ccf4f56a9d` |
| Development ZIP | `0343bc8338936aca4cd1d32525304d91c6106137f5bebc9c6d81a6a140ef5eb8` |

Local delivery: [development bundle](../dist/weread-kindle-0.2.0-ui-polish-20261110-dev.zip).
Local reports and captures are under `artifacts/interface-polish-20261110/`.
Complete logs, the source manifest, the source-match receipt, and the package
receipt are retained under
`test-env:/root/weread-ui-polish-20261110-r1/evidence/`. The initial compilation
failure and the corrected passing run remain separate; the supplementary
capture script's import correction changed no application source.

No application test, build, or runtime probe is authorized on the operator's
Windows machine. Remote desktop captures do not establish e-ink panel
behavior. Physical Library loading and authenticated concurrent downloading
remain separate acceptance checks on the existing Scribe 2022 / firmware
5.19.5 target.

## Physical deployment

On 2026-11-10, the user authorized deployment and testing after the other device
coordinator returned its lease. A fresh exclusive lease and native precheck
confirmed the identified Scribe / firmware, idle application locks, running
native services, and the original native dark appearance.

The previous application and complete existing data directory were backed up
privately on `test-env`. Only the main executable was replaced, through a
hash-checked same-directory staging file and atomic rename. The launcher
`a5fec72f` and frame guard `a235b41c` remain identical. Before starting the
application, all 54 existing data entries, including 48 files, matched the
backup by type, permissions, size, and file content. No account reset, book
removal, or configuration replacement was performed.

The normal document entry started the new ARM application `74e01933` with the
expected production arguments and no demo, offline, scripted-action, or product
exit deadline. The running process hash and its launcher/runner/helper ownership
were checked. The pre-update database contained 45 shelf books and no unfinished
download tasks; its absent concurrency field receives the new default of eight.

The user confirmed stable Library display, default concurrency eight, and
working touch on the device. Settings persistence and successful completion of
an authorized download remain separate gates. The user then reported that
full-book progress advances one chapter at a time and requested verification of
actual concurrency. Two image-enabled tasks were cancelled at early cursors;
their large catalogs and the saved value of eight were observed without
recording book identifiers or content.

The visible count follows ordered durable checkpoints and cannot identify the
network parallelism. The initial candidate also silently switches a task to
serial fetching after a speculative size-limit failure. Existing logs cannot
establish whether that happened in these physical tasks. Bounded numeric
download diagnostics are being prepared to distinguish concurrency, capacity
fallback, image acquisition, and persistence time before claiming a conclusion.
An identity-bound managed stop preserved current progress and restored the
original native dark appearance and services. This planned stop is not a
voluntary UI-exit result. Private backups, process identities, and raw receipts
remain outside the repository. The diagnostic follow-up and return are recorded
below.

## Download diagnostic follow-up

The diagnostic candidate preserves the existing concurrency, limits, retries,
and checkpoint behavior. It records only process-local run numbers, configured
and effective concurrency, actual chapter-read and Reader/shard HTTP activity,
response-budget phases and byte counts, successful Reader response size, and
cumulative stage times at five-second intervals. Finished queued results do not
count as active reads. Image-stage time also includes its resource catalog and
archive work. A cancellation snapshot can precede child-future destruction and
is not, by itself, a leak diagnosis.

On `test-env`, this candidate passed 1,513 library and 52 Main tests, with the
same three manual benchmarks ignored, and its ARM build completed. The ARM
SHA-256 is `6d9a5aac34b2f2fd4e4b1c5d962b0d899d7ff5428fcaab4ca6cc181d4ab26d12`;
the diagnostic ZIP SHA-256 is
`296a3f5914beb8af58d848cc5bdd6be0ba43d1f2ffd4ec17db2c6cdd289e2527`.
Only the device executable was updated. All 69 data entries, including 63
regular files after the user's download attempts, matched the new private
backup before startup. Its running binary and normal arguments were checked.

The user repeated the selected image-enabled download. The diagnostic build
recorded a chapter-read and chapter-HTTP peak of eight, followed by Reader
response admission failures at the 524,288-byte limit and an explicit effective
concurrency change from eight to one. Successful full-budget Reader responses
reached 1,420,274 bytes. This establishes the long-catalog fallback in this
physical task, rather than inferring it from one-at-a-time UI progress.

After 54 new durable checkpoints, the source returned business error `-10102`.
Two short read retries failed with the same code, and the task was saved as
Failed at its existing cursor. Its 67,984 ms diagnostic interval included
63,116 ms of text waiting, 272 ms of image-stage work, 173 ms of preparation
waiting, and 4,307 ms of acknowledgment waiting. These are one task's cumulative
application measurements, not panel latency or a before/after speed comparison.
The book retained 71 cached chapters out of 1,863. The user had not cancelled
this attempt.

The reviewed [official Reader bundle](https://cdn.weread.qq.com/web/wrwebnjlogic/js/9.625560b7.js)
does not identify `-10102` as a specific rate, permission, or login condition;
its general business-error path offers a retry. The code's meaning remains
unverified. A download-only bounded cooldown policy and persistent failure
feedback are being implemented as client policy, without changing global
reading-report retries or treating this code as authorization evidence.

The application was stopped through its verified launcher identity. Native
dark state and services were restored; all ten recorded process identities,
the actual guard window, both application locks, and device transports were
absent. The exact lease was released at 2026-11-10 17:47:28.654747 +08:00 and
independently absent at 17:47:28.667038 +08:00. Subsequent repair is host-only
until a fresh device lease is acquired. The diagnostic binary remains installed
and user data are retained.

Diagnostic build receipts use `diagnostics-*` in the existing remote evidence
directory. A sanitized local record is
`artifacts/interface-polish-20261110/physical-download-diagnosis.json`.
At that diagnostic checkpoint, the corrective large-catalog, shared-permit,
and pipeline checks had passed on `test-env`, including two eight-chapter request
windows. Integrated recovery verification, a corrected ARM build, and a fresh
physical repeat were still pending; the next section records their later status.

## Corrected candidate and cache reuse

The integrated correction now passes 1,534 library tests and 52 Main tests on
`test-env`, with three manual benchmarks ignored. Standard desktop validation
passed 133 checks. Another 34 synthetic offline interface checks cover the
waiting/interrupted notice, its actual pointer-to-dialog action, the reserved
48px list offset, and stable waiting pixels on PW6 and Scribe geometries. Those
captures were visually inspected. They do not measure physical e-ink flashing.

Download recovery persists three scalar fields in the existing task JSON:
`retry_cursor`, `retry_count`, and `retry_not_before`. No SQL schema change is
required. The bounded 60/120/240-second policy applies only to the observed
download-read rejection; global error retry classification and reading reports
are unchanged. Restart, queue reload, explicit continuation, cancellation, and
the waiting-dialog transition are covered. The same code is excluded from fast
chapter retry and format re-probing. Its upstream meaning and eventual service
recovery are not guaranteed by this client policy.

The additional user request requires complete cached chapters to be skipped
when downloading the full book again. New-task filtering already handled many
of these chapters, but image-enabled queued/resumed tasks still fetched their
bodies. Startup now verifies relevant complete image caches in one bounded
operation and adds only index/UID-bound hints to the bodyless plan. Both
prefetch scheduling and execution honor those hints. On receipt, the current
body and resources are checked again before consuming a non-cloneable
`VerifiedChapterImages` proof at the matching SQL checkpoint. The text-only
checkpoint still rejects image tasks without that proof. Tests exercise a
second full-book pass with zero additional HTTP requests, actual application
scheduling, empty-image manifests, and rejection of damaged images, wrong
ownership, changed UID/body, Preview, and Unknown content. Partially missing
images continue through the existing repair download; this change does not
claim an image-only recovery cache for those chapters.

| Corrected artifact | SHA-256 |
| --- | --- |
| Source input manifest, 549 files | `edd25562f008c065beab0a3327de4a75dfc355f42a38527e790834d7fb843722` |
| Desktop executable | `0aa94aefe481de7acfc95677da34371c1612f95cb0559aadff879096449e0b8b` |
| ARM executable | `985efefca33ed8c1a1bd923050421a3a267de6840eb3f42056148995a2b9a409` |
| Development ZIP | `7ea3fe5ae35878e5c14fa8435c459629bd8468aae91b91496c0f554d1f038223` |

[Corrected development bundle](../dist/weread-kindle-0.2.0-download-fixes-20261110-dev.zip)
passed CRC and staged-executable matching checks. The current workspace matches
all 549 verified build/packaging inputs. Receipts use `download-complete-*`,
`download-final-*`, and `download-recovery-ui/` in the remote evidence directory.

A new exclusive physical window, `download-fixes-20261110-r2`, was acquired
after a fresh absence check. The identified native Scribe/firmware and original
dark state were rechecked. Only the executable was replaced; all 69 data
entries, including 63 regular files and the 71 cached chapters, matched a fresh
private backup before startup. The running `985efefc` process and production
arguments were verified. An initial log-read connection timed out; the user
later woke the device, restored networking, and started the same download.
New-candidate logs were separated from the prior candidate by a verified
pre-update log prefix.

The physical repeat recorded configured and effective chapter windows of eight,
a chapter-read peak of eight, and a chapter-HTTP peak of eight. HTTP activity
varied between samples, including values from zero through three; this is not
a claim of eight continuously active HTTP requests. Large Reader pages used
their shared full-budget permit without switching the entire task to serial
fetching. UI progress still counts ordered durable checkpoints and can advance
by one or several chapters between display updates.

The first resumed attempt began with 1,792 remaining chapters, preserving the
71 already cached out of 1,863. After 40 new checkpoints, the source again
returned `-10102`; the persistent task became Paused with a retry deadline,
rather than an unrecoverable silent stop. Subsequent captured attempts began
with 1,752 and 1,727 remaining chapters in the same process, and the task
returned to Running without restarting its saved selection. Cache counters
advanced through 111, 165, and 169 during the captured recovery observations.
The actual source error was not eliminated and its meaning remains unverified.

At the user's request, observation was ended. An identity-bound managed stop
retained 305 cached chapters. The durable task remains recoverable on a later
normal launch; this is not a full-book completion or voluntary UI-exit result.
The original native dark metadata and services were restored. All five owned
process identities, the actual guard window, both application locks, and device
transports were absent. R2 was exactly released at 2026-11-10
20:27:12.514175 +08:00 and independently absent at 20:27:12.520670 +08:00.
No additional observation is scheduled for this closed scope.

The sanitized result is
`artifacts/interface-polish-20261110/physical-closeout-summary.json`. A complete
cached-book second pass with zero HTTP calls was exercised in controlled remote
regressions, not separately to completion on this physical device. No other
model/firmware, USB, reboot, sustained throughput, or full-book completeness
claim follows. Private connection material, process identities, raw logs, and
backups remain outside the repository.

## Subsequent independent-chapter follow-up

The later user instruction superseded the task-wide recovery policy above with
independent chapter completion and retry. Its schema-3 ARM `dd8ac6c3` replaced
the schema-2 executable during the bounded 2026-11-11 Scribe 2022 / firmware
5.19.5 follow-up. The original executable/data pair remains privately backed
up. Native restoration, original concurrency eight, and exact lease release
were confirmed after the device batches. Native Reader comparisons, the valid
eight-worker production window, retained sampling failures, source rejection,
and outstanding physical gates are recorded in
[Independent chapter downloads](INDEPENDENT_CHAPTER_DOWNLOADS.md). The earlier
Library/default-eight/touch acceptance remains scoped to its original artifact.
