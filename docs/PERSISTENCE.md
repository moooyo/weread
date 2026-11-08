# Local persistence implementation

Status: implementation contract. Runtime and device acceptance evidence is recorded separately.

## Ownership and locations

`src/domain.rs` defines application-owned models without upstream response schemas. `src/store.rs` owns the metadata SQLite connection. `src/cache.rs` owns the bounded cover/thumbnail file cache and its independent SQLite index. The presentation layer must not execute SQL or construct cache paths.

The operator supplies the runtime data directory. Application state, account credentials and downloaded content must stay outside the source checkout. Credentials are not stored in the metadata database or cache index; a separate private session store owns them. Cache keys should use stable book/resource identifiers, never signed URLs or credentials.

The metadata connection belongs to the application thread. Network work should send normalized results to that thread rather than share the connection. The file cache assumes one application process owns its directory; running multiple application processes against the same cache directory is unsupported.

## Resource admission and cached checkpoints

Catalog and detail planning use bodyless metadata. Identifier, record-count, and
retained-metadata admission precede old-row materialization; catalog reindexing
retains existing bodies and permission bindings in one SQL transaction while
remapping positions, annotations, bookmarks, and tasks by stable UID. Temporary
positive indices above the normal chapter range remain transaction-local.

A text-only cached download checkpoint validates the exact persisted task,
selection, cursor, current chapter UID, full-content account binding, and
nonblank TEXT in SQL. It advances only the cursor and does not rewrite the body,
increment cached counts, or reconstruct anchors. Lightweight body-presence
metadata is a planning hint, not authorization evidence. The actual checkpoint
applies Rust's Unicode whitespace semantics; background success copies no body
into Rust. A later display/session failure cannot reverse an already committed
acknowledgement.

Cached underlines admit 16 MiB encoded JSON and 4,096 records in SQL before
copying or deserializing, then apply the existing selected-field limits before
replacing the Reader. Thought JSON similarly uses a 1 MiB/4,096-record SQL gate;
only selected visible cards load bounded previews. Rejection preserves cached
values, local drafts, body text, and account state.

Local annotations have separate scalar geometry, count, header, and selected
payload interfaces. Stable row keys, field types, encoding-aware byte bounds,
and aggregate work are admitted before reading selected text in the same
transaction. Direct `octet_length(column)` preserves SQLite's long-value
metadata optimization; converting the column inside its argument would defeat
that gate. UTF-16 history uses conservative UTF-8 and conversion-buffer prices.
Existing character-count operations keep their TEXT semantics; this does not
claim that every Store scalar avoids payload work. Rejection limits the current
working set and retains persistent history. Statistics group timestamps/counts
without collecting annotation text.

Readonly Book projections omit intro and cannot be serialized into a replacement
Book. Existence, source, title, and reading-state consumers select only their
needed fields; current chapter metadata preserves nullable-text presence and
separate readability/permission evidence. It does not replace the canonical
body reads required for a displayed chapter or a saved-body digest. New report
allocation retains eligible counters and deferred run evidence while
committing only an admitted consecutive prefix with its allocated cursor in one
transaction. Capacity refusal must not delete old requests, summaries, receipts,
or unallocated seconds, and does not authorize replay of uncertain writes.

Durable cursor settlement checks the same measured account, book, and start
identity before a zero-demand return. Flush, archive, and the legacy unit bridge
share borrowed input admission and the retained-proof identity check. The
current complete library record passed these ordinary-flow and rollback
regressions; exact source/executable reuse is recorded in
[Verification](VERIFICATION.md#finite-resource-closeout-and-active-orientation).

Bulk completeness uses one aligned operation over selected chapter metadata,
manifests, bodies, and unique PNG proofs. Each aggregate is admitted before its
corresponding read/allocation phase, and the batch evidence completes before
retirement. It checks original physical identities throughout and returns no
partial success on quota failure. Restored-prefix and download
planning consumers retain the task and its cursor when integrity is unavailable.
These logical prices are not total SQLite/allocator RSS limits.

Thought/review request completion uses its captured book, chapter/view, and
cache identity rather than the mutable current selection. View drift rejects
late display or cache publication while preserving independent same-account
credential CAS. New reading-report summaries retain the existing wire's first
20 Unicode scalars before batching; old SQL titles, requests, and delivery
proofs are neither rewritten nor replayed by this correction.

Image verification uses one operation-local proof per unchanged physical
payload, with prepaid work and fixed-size streaming buffers. Publication also
captures indexed or unindexed destination identity before mutation. Every
scoped debt drain, retirement, pin change, guard cleanup, and rollback checks
that fence; a late-appeared or changed file is not adopted or deleted. Only the
operation's own confirmed creation, rename, or unlink advances its expectation.
Inventory snapshots price direct column metadata and full selected row/field
aggregates before retaining keys or parsing requested manifests and task
payloads. Default Store inventory limits are 65,536 rows, 64 MiB encoded input,
64 MiB metadata/scratch, and 128 MiB logical work. Original-key and snapshot
fences remain active until all required body and image proofs complete before
the first cache mutation. Spent work is not refunded after a rejected attempt.
These limits preserve persistent rows; they do not impose a database-size or
SQLite RSS guarantee.

Local bookmark counts and geometry do not load labels or IDs. A selected
canonical header and label are admitted in the same snapshot. Annotation
range checks borrow an admitted body and retain the original Unicode, NUL,
UTF16, and SQLite conversion semantics. Demanded Book search transforms are
prepaid before the existing contextual Unicode lowercase operation; empty
queries and earlier matching fields retain their original short circuits.

Existing report delivery uses a separate current-Outbox maintenance bridge.
It retains the literal seven-side-key transaction checks and the default
encoded-setting envelope while leaving the new-allocation limit independent.
Capacity refusal preserves old requests, original summaries, receipts, and
unallocated seconds; it does not authorize another elapsed-time POST.

The [resource review](RESOURCE_LEAK_REVIEW.md) records verification separately
from these implementation contracts and from physical durability evidence.

## Schema version 1 baseline

| Table | Responsibility |
| --- | --- |
| `books` | Normalized metadata, shelf membership, local progress and derived cached chapter count |
| `chapters` | Stable zero-based chapter index, upstream identifier, title and optional extracted text; version 1 did not distinguish preview and full text |
| `positions` | Unicode scalar offset, chapter index, timestamp and local device label |
| `annotations` | Half-open Unicode scalar range, quoted text and optional note |
| `bookmarks` | Chapter and scalar position, label and timestamp |
| `download_tasks` | Persisted explicit chapter selection, next item cursor, status and options |
| `settings` | Versionable JSON values under application-owned keys |
| `reading_sessions` | Active reading seconds and the corresponding elapsed time interval |
| `query_history` | At most 20 unique recent queries |

Schema creation and `user_version` update occur in one transaction. A database with a newer version is rejected rather than overwritten. WAL mode, a five-second busy timeout and foreign keys are enabled. Unix database permissions are restricted to `0600`; Windows permissions follow the runtime directory's ACL.

## Schema version 2 content-state migration

Schema version 2 implements the [content-access correction](CONTENT_ACCESS.md).
It adds `Chapter.content_status` and an opaque `content_account_key` binding.
The states are `unknown`, `full`, `preview`, and `locked`. The account key is an
identity hash, not a credential or an authorization grant. Missing or malformed
entitlement metadata must not become an affirmative grant or an assertion that
the user has no rights.

Existing WeRead bodies are retained and become `unknown` pending revalidation;
existing imported/demo bodies remain local full content. The migration
recomputes full-content counts without deleting positions, annotations,
bookmarks, or body text. Legacy completed tasks containing unverified bodies
no longer claim complete full-text acquisition. Explicitly cancelled tasks
remain cancelled. The storage regressions passed within the 386-test
unlock-baseline suite and the later final 418-test combined root suite;
[Verification](VERIFICATION.md#combined-source-remote-checks) records the
separate source scopes. The final combined candidate is the `2dd3bfc996d9`
ARM artifact in [the build record](VERIFICATION.md#combined-arm-build-and-private-trial-bundle).
Its subsequent physical installation and narrowly observed migration are
recorded below; the remote tests alone do not establish device durability.

On the Scribe 2022 / firmware 5.19.5, the installed combined candidate migrated
SQLite `user_version` from 1 to 2 and retained 118 nonempty bodies. Stable
ordered row digests for positions, annotations, and bookmarks compared equal
before and after. Their row counts were two, zero, and zero respectively; this
does not physically test preservation of nonempty notes or bookmarks. One
cancelled task remained cancelled and two legacy complete tasks became paused.
All 1,974 catalog rows were initialized as `Unknown`, including the 118 rows
with bodies. [Physical testing](PHYSICAL_TESTING.md#combined-installation-and-migration-observation)
records the installation scope. Subsequent stable-UID revalidation replaced
two preview bodies with full text: index 105 grew from 192 to 11,613 Unicode
scalars and index 109 from 181 to 9,058, with two completed tasks. No phone-side
unlock action was observed.

Three managed sleep/wake cycles retained the same application identity.
The explicitly enabled dark Reader case retained chapter 105 / scalar 496 /
dark mode 1 in saved state, with user-confirmed same-page rendering and working
page turns. After the user confirmed logout and fresh QR login, permission
proof for the two full bodies was invalidated and all 118 nonempty bodies
remained present as unverified; four tasks were paused. This is associated
with account operations, not ordinary Exit. The subsequent voluntary-exit
check retained `prefs.dark_mode=1`; system getters still returned `Y8`,
consistent with application-only dark rendering in `2dd3`.

The newer native night-mode exit synchronization, launch/wake XOR compensation,
and Login close control are a separate implemented candidate. Its controlled
core suite passed 440 tests with three ignored benchmarks, and the launcher
passed 86 fixtures; the three targeted closing cases are an overlapping
subset. [Native theme continuity](NATIVE_THEME.md) distinguishes first-use
native inheritance from an authoritative stored application preference and
keeps Login close separate from logout. The `2fd3d6ab2d64` ARM candidate and
`799d5bba5eb9` launcher were installed with matching device hashes, as recorded
in [Verification](VERIFICATION.md#native-theme-physical-acceptance-on-scribe-2022).
Ordinary Exit changed native property and persistent configuration from `Y8`
to `Y8INV`, with launcher `persistence=confirmed`; both read back `Y8INV` after
the final normal Login X exit as well. Application dark mode remained 1.

The next application start captured native `Y8INV` correctly. Its resident
sleep/wake retained PID 11804, current Reader chapter 1500 / scalar anchor 0,
and dark mode 1, with a matching managed-native-mode snapshot. The final
physical check retained schema 2, 118 nonempty bodies, and one cancelled task.
These are real readback and application-cycle observations, not a whole-device
reboot test. Busy/code/key/error/failure Login variants remain controlled
cases; the real Login X check covered a normal Login state. The earlier
418-test/`2dd3/971e` results keep their separate scope. Nonempty note/bookmark
preservation and an observed phone-side unlock transition remain unverified.

An incoming preview or unverified body may be saved for reading but cannot
advance a full-download cursor or increment the full-content count. A source
refresh after phone-side unlocking rechecks fresh Reader evidence for the
account, book, chapter UID, and renewal generation before accepting full
content. Body/status/account, count, and task checkpoint changes are
transactional with stale-task rejection. Account changes downgrade old
permission proof rather than transfer it to the new account.

Safe preview replacement requires identical text or a checked exact prefix
extension from an unverified/preview body to confirmed full text. A legacy
`Unknown` body with an empty owner binding can pass the same UID/full-access
and prefix checks, but is not bound to an account by default. Removing a
terminal `...` or `…` marker clamps positions and bookmarks to the retained
prefix; an annotation touching the removed marker rejects replacement and
preserves the original records. Merely receiving a longer body is insufficient.
Incompatible text, different ownership, or unsafe note-anchor changes preserve
the old body and user records and surface `ContentChanged` instead of silently
rewriting them. Original text-bound image manifests are not proof of full-text
authorization.

The preflight and transaction both compare the complete serialized task plan.
This safety check adds plan serialization; the earlier `4bef` benchmark's
zero-serialization observation describes that older source only. The permission
errors `UnlockRequired`, `ContentUnverified`, and `ContentChanged` pause a task
without an automatic retry or login prompt. Only an explicit refresh retries
those states.

`Book.progress` is a finite fraction in `0..=1`. Chapter indices are zero-based. Text anchors use Unicode scalar offsets, not UTF-8 byte offsets or font-dependent page numbers. They describe the extracted local text and are not assumed to be upstream offsets.

## Consistency and recovery

- Bulk metadata and shelf replacement operations are transactional. Replacing shelf membership preserves cached chapters, positions, notes and bookmarks.
- The version-1 cache flag and count were derived from any nonempty body. Physical use showed that valid preview responses could satisfy that gate, so those legacy fields do not prove complete text. The content-state correction requires a nonempty body classified as `full` before counting it as fully downloaded. Remote catalog metadata and image manifests cannot supply that classification by themselves.
- Full-text download completion requires the cursor to reach the selected chapter count with every selected body verified as full in the appropriate account scope. A preview must pause acquisition without advancing that cursor. An interrupted `running` task becomes `paused` on startup; cancellation must not be undone during recovery.
- Content clearing removes replaceable chapter bodies and resets cache counts. Reading positions, annotations and bookmarks survive.
- Session duration cannot exceed elapsed time. Statistics can be calculated from local sessions without claiming a successful upstream reading-time report.

The current body cache stores extracted text in SQLite. It is an offline reading representation for content the adapter is actually authorized and able to retrieve, not evidence that an upstream EPUB/PDF download exists. Original purchased books, response archives and account data are not bundled with the repository.

## Cover and thumbnail cache

`FileCache::open(root, maximum_bytes)` creates a digest-only payload namespace and an index containing size, access sequence and pin state. Resource keys never become path components. Objects are addressed by SHA-256 of both the key and content; replacing an entry therefore uses a new filename and avoids a non-atomic overwrite on Windows.

Writes use an exclusive temporary file, flush the payload, rename it into the object namespace and commit the index transaction. A durable cleanup ledger tracks both temporary and final names before publication. Retired objects and least recently used unpinned victims enter that ledger in the same transaction that changes the active index. Cleanup checks the committed index before unlinking, so an ambiguous commit error cannot delete a referenced object. Failed deletion retains its identity for a later attempt.

This cache has one application-process owner. Startup registers and reclaims unindexed digest objects and strictly named legacy `.tmp-PID-counter` files immediately, including files without a cleanup ledger entry. Unknown names are left alone. Directories are never recursively removed. Missing entries are retired, while other filesystem errors remain visible.

The configured budget covers active indexed payload bytes and unreferenced payload bytes still awaiting cleanup. SQLite metadata and a temporary replacement's bounded crash-safe overlap are additional disk usage. A pending cleanup failure blocks further payload publication, preventing repeated retries from escaping the budget. Pinned objects are never evicted; an operation that cannot fit while preserving pins fails and keeps its previous entry. Startup rejects a budget smaller than the pinned working set.

Thumbnail generation enforces encoded-byte, decoded-dimension and memory limits, reserves caller-selected output geometry, and quantizes grayscale values to the 16 e-ink levels. Payload reads verify the content digest, including corruption that preserves the file size. The cache does not implement display-driver dithering or prove panel quality. Object symlinks are rejected rather than followed.

## Image publication recovery

`BookAssetStore` retains one reconciliation flag. Pin changes, ambiguous
publication failures, incomplete maintenance, and cache/manifest read errors
leave that flag set. This includes read errors the Reader tolerates by showing a
placeholder. Before another publication, the store must validate the complete
manifest reference evidence, retry pending file cleanup, restore referenced
pins, and remove unreferenced assets. Missing or unreadable evidence prevents
new publication instead of guessing which shared asset is safe to unpin.

A manifest already known to be committed still returns success if later cache
maintenance fails; reconciliation remains required for the next operation.
This avoids reporting a successful publication as rolled back or replaying it.
Normal successful reads do not trigger a global scan. The flag is initialized
on open, so interrupted-process recovery uses the same evidence-based path.

## Reading-time report proof

Reading-time delivery uses a durable outbox because the upstream operation has
no idempotency key. Uncertain delivery must remain unresolved until an explicit
matching acknowledgement arrives; a local timeout cannot authorize a replay.

Confirmed canonical batches are compacted into ranges scoped to an account,
book, start time, and Reader run. Adjacent ranges merge. Unconfirmed intervals,
including permanent uncertain gaps, remain separate records: they do not prevent
later confirmed minutes from sharing one bounded range, and the contiguous
reported cursor never crosses their gaps. Allocation uses the maximum endpoint
of all retained batches and confirmed ranges, including after recovery from an
older active-run cursor. Numeric overlaps and reuse of a compacted canonical
session identity are rejected. Arbitrary legacy session identities retain their
full records when they cannot be represented losslessly.

The compacted snapshot and exact per-batch receipt, error, failure, retry,
authentication-wait, and uncertainty setting deletions commit in one SQLite
transaction. Failed commits preserve both the previous queue and its side
records. A completion for a retired or unknown session cannot recreate those
records. Normal per-run reading history remains retained user data.

The outbox setting key remains `report.outbox.v1`, but new snapshots serialize
the required record field as `batches_v2`. The current reader accepts the legacy
`batches` field as an alias and accepts original prefix-only proof without the
new range field. Supplying both batch field names is rejected. An older binary
must reject a new snapshot because its required `batches` field is absent;
silently ignoring suffix proof while accepting a zero contiguous cursor could
otherwise reallocate already acknowledged minutes after a crash. This is a
fail-closed downgrade barrier, not permission to erase or reconstruct evidence.
New snapshots also require an explicit `compacted_runs` map and a
`confirmed_ranges` field for every run. Missing or null proof cannot be
defaulted to empty evidence merely because the reported cursor is zero. Only
the legacy record format permits an absent proof map or prefix-only proof.

These are implementation invariants. The resource review records controlled
execution separately; no new authenticated reporting or physical-device claim
follows from the synthetic outbox regressions.

## Operational account capacity

Cookie updates are validated atomically: at most 256 entries, 64 KiB including
header separators, and 8 KiB per entry. Over-budget updates retain the complete
previous jar and associated session fields. They do not evict login cookies or
claim that the credentials have expired. Stale-response and renewal gates apply
before committing an accepted candidate. An explicit elapsed-time receipt stays
confirmed even when response cookies are rejected; an unconfirmed POST remains
uncertain and is never automatically replayed.

The account file is limited to 1 MiB. Loading checks metadata and reads at most
the limit plus one byte. Saving validates fields and performs bounded JSON
serialization before opening the temporary output, then uses a bounded writer
and atomic replacement. The single application owner may reclaim an old regular
`account.pending` directory entry before creating its new output. This avoids
blocking renewal after an interrupted save and does not truncate a hard-linked
account file. Directories and symbolic links in that slot are rejected. Failed
publication does not explicitly remove the previous account; temporary cleanup
only targets the output owned by that save. Multi-process writers to one data
directory are outside this single-owner contract.

These are project resource policies rather than statements about upstream
cookie limits. Synthetic cases and source review are recorded in the resource
review; no live account-growth event is claimed.

## Verification scope

Additional resource regressions exercise same-instance cleanup debt, dirty image-reference recovery, shared pins, atomic outbox compaction, account-file limits, and repeated import/ZIP work budgets. Committed or unresolved evidence is preserved across failure; a typed capacity rejection does not revoke credentials or authorize elapsed-time replay. The [resource candidate record](VERIFICATION.md#resource-lifetime-candidate) supplies integration results and artifact identities independently of earlier persistence suites.

The baseline unit tests cover migration/reopen behavior, transaction rollback, shelf replacement, body-presence cache status, interrupted download recovery, user-data preservation, search-history limits, digest path safety, LRU/pin behavior, failed replacement, missing/orphan cleanup, corruption detection, thumbnail geometry/grayscale and symlink rejection. They did not establish full-text entitlement for a valid short preview. Additional content-state regressions for legacy migration, preview checkpoints, account binding, safe full-text extension, unsafe replacement rejection, preserved anchors, and transaction rollback passed within the final 418-test combined remote root suite; the earlier 386-test unlock-only and 414-test combined snapshots remain separate results. Verification runs through `ssh test-env` under the repository's policy. These tests do not establish physical-device durability, firmware behavior, live unlocking, upstream availability or authenticated API success.
