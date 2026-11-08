# Download performance investigation

Status: priority 1 and priority 2 were explicitly authorized and implemented.
Remote correctness checks, controlled component benchmarks, and the ARM build
have completed for that candidate. Its physical continuation completed the
transfer workflow but included 25 previews, so full-book completeness and a
physical full-book speed comparison remain unverified. The measurements below
are not a Kindle speedup claim. Device findings refer only to Scribe 2022 / firmware 5.19.5. Verification
runs on `test-env` or the exclusively leased device, never on the operator's
Windows machine.

## Confirmed baseline behavior

- An uncached EPUB chapter uses four serial content requests: Reader HTML,
  `e_0`, `e_1`, and `e_3`. TXT uses Reader HTML, an `e_0` format probe, `t_0`, and
  `t_1`. Thus 111 chapters need 444 normal content requests before catalog,
  images, redirects, or retries. Request count is not measured elapsed time.
- The HTTP client is shared and already has a connection pool. Adding a second
  connection-pool abstraction would not remove a known missing feature.
- The Kindle feature graph has gzip/deflate but no HTTP/2 feature. The client
  explicitly requests `Accept-Encoding: identity` for all operations.
- Each chapter waits for its content and images, CPU image preparation, and the
  UI's durable checkpoint acknowledgment before starting the next chapter.
  The application polls work every 100 ms. There is no normal one-second
  inter-chapter sleep; one second throttles progress notices only.
- Every `Downloaded` event still marks the route dirty and can rebuild the
  Library model and its cover-cache lookups. The existing progress throttle
  does not prevent this work.
- Image preparation precedes the persistent cache check. Repeated raw image
  digests can therefore be decoded, resized, quantized, and encoded again even
  when the resulting file is already cached.
- Manifest persistence, chapter/body upsert, and task persistence are separate.
  A chapter upsert recounts cached chapters. Task validation/cloning and JSON
  persistence repeatedly traverse the full selected chapter list. SQL operation
  count is not equivalent to physical fsync count; metadata uses WAL/NORMAL.
- Cache lookups verify object contents and update LRU state; pinning an already
  pinned object still executes an UPDATE. Commit also obtains the whole pin
  snapshot, and a new put loads eviction candidates before checking the loop's
  already-satisfied budget condition.

## Limited live observations

Only aggregate counts were retained for the affected request. Its cached
catalog had 111 entries, 19 nonempty resource-archive URLs, and 19 distinct such
URLs. This sample does not support prioritizing an exact-URL archive cache as a
major network saving. Across the already committed portion, 27 image
occurrences formed 25 distinct chapter/asset pairs and 18 raw image digests;
seven preparation uses could potentially reuse a previous chapter's result.
Repeated occurrences within one chapter are already deduplicated.

The attempted 25-chapter continuation could not supply a throughput baseline:
it stopped before the first new checkpoint with Web business code `-2012`.
Official public Web-client code treats this and `-2010` as states requiring
session renewal. Authentication recovery must work before comparing download
rates; time spent waiting for manual recovery is not normal transfer latency.
The earlier successful continuation and retry interruptions are retained in
[Physical testing](PHYSICAL_TESTING.md).

A separate unauthenticated public-page check on `test-env` returned HTTP 200
over HTTP/2 for `https://weread.qq.com/r/weread-skills`. Requesting identity
transferred 62,805 body bytes; requesting gzip transferred 14,464 body bytes and
returned `Content-Encoding: gzip`. This is evidence that the official origin
supports compression and HTTP/2, not a measured Kindle Reader or full-book
speedup. The authenticated Reader path and its decompression CPU/size-limit
behavior still require their own comparison.

## Accepted scope and implementation

- Reader HTML and appropriate JSON requests negotiate gzip. Encrypted shard
  bodies and image resources retain identity encoding. Decompressed body limits
  remain enforced, including a compressed-response over-limit regression.
- Each task keeps a scope-bound TXT format hint only after verified content.
  Later TXT chapters skip `e_0`; mismatches clear the hint and re-probe with a
  bound. Client, account, and book scope changes invalidate the hint.
- EPUB reads overlap only `e_1/e_3`, with two requests and deterministic assembly.
  Both results share the chapter's authentication budget. Cookie/header changes
  conservatively trigger a fresh Reader and serial reread, and disable parallel
  reads for the remaining task. Failed renewal retains its real error and cannot
  be hidden by that fallback. Cancelling the parent cancels the joined reads.
- An 8 MiB task-owned LRU reuses verified prepared PNGs through `Arc`, keyed by
  original digest and transformation identity. Every raw packet still passes
  digest validation and every chapter binds its own sources. Cancellation closes
  the old cache before late CPU work can refill it. The limit covers retained
  PNG payloads, not total process RSS or an in-flight packet.
- Worker preparation shares one DOM for image anchors and unresolved-image
  analysis. UI commit checks HTML/text hashes without another DOM parse. Source
  content extraction and image planning retain their separate parsing; this is
  not a claim that the entire pipeline parses HTML only once.
- Background checkpoints update relevant UI fields at one Hz without repeated
  cover lookups. Active Reader results, requested chapters, and terminal states
  remain immediate. Filter membership and device-specific row-height changes
  still trigger the required layout update.
- Body, cursor, and cached-count changes commit in one transaction. Checkpoints
  validate persistent task scope and update mutable JSON fields without Rust
  cloning/serializing the whole plan or recounting every cached chapter. SQLite
  still stores the resulting task payload; no in-place physical JSON rewrite or
  reduced fsync count is claimed. Terminal compare-and-save cannot overwrite a
  newer plan, account, cursor, or cancellation. Image-manifest recovery ordering
  is retained.
- The later [content-access correction](CONTENT_ACCESS.md) strengthens each
  checkpoint with preflight and transactional comparisons of the complete
  persistent chapter plan before permitting a body upgrade. This serializes
  the plan for the comparison; the earlier zero-plan-serialization observation
  above is historical, not a property of the correction. The plan is still not
  rewritten by Rust for every checkpoint, and full-content counts remain
  incremental.
- Commit cache lookup preserves size/symlink/SHA checks without LRU promotion.
  Pin snapshots query involved keys, repeated pin is a no-op with the same
  existence semantics, and sufficient capacity avoids loading eviction victims.

## Verification and controlled measurements

The final source passed 334 normal library tests on `test-env`. Three manual
benchmarks are ignored by the normal correctness run and were then explicitly
executed: all three passed equivalence/integrity assertions. They used the
remote debug test harness, synthetic data, and isolated temporary databases;
they do not measure production-release or Kindle throughput.

| Controlled scenario | Reference | Implemented path |
| --- | --- | --- |
| 8 EPUB chapters, 50 ms mock latency per request | 32 requests, 1681 ms | 32 requests, 1272 ms |
| 8 TXT chapters, same mock latency | 32 requests, 1671 ms | 25 requests, 1306 ms |
| Same synthetic 800 x 1000 PNG prepared eight times | 8 cold conversions, 1573.862 ms | 1 cold + 7 hits, 222.238 ms |
| 111 chapter metadata checkpoints, 7040-byte bodies | 222 metadata transactions, 12.677 ms | 111 transactions, 17.278 ms |
| 1000 equivalent metadata checkpoints | 2000 metadata transactions, 580.004 ms | 1000 transactions, 167.969 ms |

The short metadata fixture was slower, despite fewer transactions. The long
fixture avoided repeated plan handling and count scans. Neither observation
establishes how much full-book network/CPU/storage time is saved on a Scribe.
The component results are not additive, and metadata mutation/transaction
counts are not physical fsync counts.

Evidence: `test-env:/root/weread-emulator/artifacts/download-performance/`:
`final-core.log`, `final-benchmarks.log`, `final-arm.log`, and
`final-arm-sha256.txt`. The initial integrated run passed 333 tests; an independent
review found a failed-renewal/serial-fallback error-classification edge case,
which was fixed and covered by the final additional regression.

## Roadmap and remaining work

| Phase | Change | Expected mechanism and acceptance gate |
| --- | --- | --- |
| 0 | Add redacted stage measurements | Business-code renewal is implemented and tested; complete stage timing still needs Reader/shard/resource counts and duration, preparation, checkpoint wait/commit, retries, bytes, wall time, and RSS without private values. |
| 1 | Compress Reader HTML and suitable JSON | Implemented and covered remotely; measure authenticated-page bytes and Scribe CPU cost on device. |
| 1 | Reduce UI and storage work | Implemented and covered remotely; device interaction and storage comparison remain pending. |
| 1 | Reuse prepared images | Implemented with a bounded task cache and verified source binding. |
| 2 | Cache TXT format hints within a task | Implemented with verified-content-only hints and bounded fallback. |
| 2 | Read `e_1` and `e_3` with concurrency two | Implemented with deterministic assembly, shared recovery, and serial fallback. |
| 3 | Overlap one next chapter's network with current preparation/commit | Use an ordered producer/consumer pipeline with explicit window permits and a byte budget. Acquire the permit before fetching; release it only after that chapter's ACK. Fall back to serial operation when the extra buffer budget is insufficient. |
| Later | Consider HTTP/2 and resource-URL caching | Verify server negotiation and actual resource repetition first. Any cache must be scoped to account, book, catalog snapshot, full URL/query, and relevant request headers. |

## Preserved boundaries

The current adapter retains per-chapter Reader refresh based on the pinned Lua
implementation. New official-page evidence for scoped reuse is recorded in
[further research](DOWNLOAD_PERFORMANCE_NEXT.md); it is not implemented and does
not establish an arbitrary `psvts` TTL. All chunks, images, and anchors
must still be verified, and checkpoints advance only in order after persistence.
Cancel or account changes discard all prefetched work. A one-item channel alone
is not a memory bound: a producer can construct another packet outside it.
The existing archive, expansion, and prepared-packet caps cannot be summed into
an asserted process-RSS limit.

A future "text ready, images pending" option would improve time to reading and
needs separate durable phases and truthful progress. It must not report a
complete illustrated download before images are ready. It is an optional UX
proposal, not a substitute for reducing full-download time.

Compare identical chapter sets and image options, distinguish cold and warm
caches, and report retries and completeness alongside time. Use isolated data
for repeated benchmarks rather than deleting the user's completed chapters.
No multiplier or percentage speedup is promised before these measurements.
