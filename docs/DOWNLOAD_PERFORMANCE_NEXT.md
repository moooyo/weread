# Further full-book download research

Status: research only, following the implemented priority-1/2 work. No product
code, credentials, account APIs, chapter APIs, or physical-device tests were
used in this investigation. Public source was read on `test-env`; research
archives remain outside this repository. Proposed behavior below is not an
implemented capability or a measured speedup.

## Most promising new protocol evidence

The current public Reader page references the official
[Reader bundle](https://cdn.weread.qq.com/web/wrwebnjlogic/js/9.625560b7.js).
Static inspection finds page-level `psvts/pclts`, chapter changes using
`history.pushState` followed by content fetch, and next-chapter prefetch using
that state. The action requests shards concurrently; CSS is separate from the
three HTML shards. This supports investigating page-session reuse, not an
assumed validity period or a ready-made batch-download contract.

Reviewed script SHA-256:
`ba993fec345c6f2e498379b729ebcbb306fb54bfb398a8d396bc62c7ce954203`.
Original zero-based character offsets include chapter change 96975, content
parameters 88211, and prefetch 784945. The public-page script manifest and static
notes are at `test-env:/root/weread-next-research/official-reader-20261105/`.
The script was not executed.

Our content signer currently derives `pc` per request and uses `sc=1`; the
official content action uses page `pclts` and defaults to `sc=0`. These are
material differences. A reuse experiment must not simultaneously change those
fields and then attribute the result to caching.

### Experiment A: task-scoped Reader context

Keep the complete Reader context in the existing task-owned hints, bound to the
shared client, account, book, protocol mode, and authentication/session revision.
Populate it only after verified content. Do not persist it across tasks or use
an arbitrary time-to-live. Invalidate on relevant credential changes, renewal,
scope change, or a context/content failure; discard the entire attempted shard
set and obtain a fresh Reader before bounded recovery. Reading-time reporting
continues to acquire its own fresh state.

The request-count opportunity, if validated for the whole task, is:

| 111 chapters, excluding catalog/images/retries | Current priority-1/2 | Reader-context reuse |
| --- | ---: | ---: |
| EPUB | 444 (`4N`) | 334 (`3N+1`) |
| TXT with verified format hint | 334 (`3N+1`) | 224 (`2N+2`) |

This could remove 110 Reader GETs in either case. It does not imply the same
percentage reduction in elapsed time. First compare baseline and reuse with
unchanged signing fields, strict checksums, content equality, expiry/rejection,
renewal, Cookie rotation, and mixed formats. Leave the conservative default until
the account-authorized device comparison passes.

## First-download throughput: bounded pipeline

The next architectural candidate overlaps the next chapter's network with the
current chapter's CPU and persistence. Use one sequential network producer and
one ordered consumer, with at most two chapter envelopes in flight. Keep a
chapter's Reader and shard exchange together; do not interleave signed requests
from two chapters or prefetch only a context that can age while queued.

The ownership rules matter more than the channel size:

- Acquire a two-slot window permit before fetching. Carry it with every owned
  raw/prepared/UI packet until acknowledgment and buffer release. A channel of
  capacity one does not prevent construction of a third packet outside it.
- Start with a proposed 8 MiB *extra prefetch* byte budget, separately measured
  from the existing 8 MiB PNG LRU. Reserve before allocating or growing buffers,
  account for simultaneous copies, and fall back to serial acquisition for a
  packet that cannot fit. Never claim these budgets are total process RSS.
- Queue a next-chapter failure in ordinal order so the preceding successful
  chapter can commit first. A failed preceding checkpoint cancels the producer
  and discards all later work. Completion follows the last successful ACK.
- Cancellation and account changes close the whole structure. A CPU closure
  returns values only; it cannot publish or persist late data. Its memory/CPU
  permit remains owned until it actually exits. An aborted async parent cannot
  stop an already running [`spawn_blocking` operation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).
  A shared CPU gate of one prevents old cancelled and new task image decoders
  from consuming large working sets simultaneously.

The image acquisition path currently performs TAR/ZIP expansion, hashing, and
header checks synchronously in its async function. Separate bounded byte
fetching from blocking validation/binding/preparation before using that path as
a producer. Archive scanning is not full pixel decoding; measure its actual
cost rather than assuming it dominates.

For network cost `F` and CPU/commit/ACK cost `R`, the serial chapter cost is
approximately `F+R`; a well-filled pipeline can approach a cadence of
`max(F,R)`. Gains can be small when `R` is small, only one chapter remains, or
byte-budget fallbacks are frequent. The prior PNG reuse optimization can also
reduce the amount of CPU time available to hide.

## Recovery and image completion: avoid repeated successful work

The main queue already filters complete chapters. Remaining waste occurs when
a prefix resource failure rewinds a task, or when images are requested for an
already cached body. The worker's `include_images` path currently fetches
content again for each item after that cursor, including later items that may
still be complete.

The smallest useful addition is task-local staged recovery: retain a whole
strictly decoded chapter result while retrying its failed resources, and retain
each complete verified resource while later resources are retried. Do not join
individual old shards to a new Reader/authentication generation: their separate
checksums provide no common content-version identity.

A later persistent reuse API can expose `Complete`, `ImagesPending`, and
`FetchContent` snapshots. Bind account, book, source chapter UID, catalog
snapshot, HTML/body hashes, complete source set, and transform version. Legacy
or ambiguous snapshots fall back to source fetching. A catalog fingerprint is
local consistency evidence, not proof that the server text never changed.

Per complete EPUB chapter safely reused, this can avoid four content requests
plus its resources. A trustworthy cached HTML snapshot can similarly avoid
those requests during image completion. For direct images, fetch only missing
ones; for a missing member of a TAR, the observed contract still requires the
whole TAR. Four cached members do not imply an 80 percent TAR-transfer saving.

## Secondary candidates and unsupported shortcuts

- **HTTP/2:** the official origin accepted HTTP/2 in the public-page check, but
  this application's reqwest feature graph currently omits `http2`. Its
  [pinned feature definition](https://github.com/seanmonstar/reqwest/blob/v0.12.28/Cargo.toml)
  and [client implementation](https://github.com/seanmonstar/reqwest/blob/v0.12.28/src/async_impl/client.rs)
  support normal rustls ALPN negotiation with HTTP/1.1 fallback. Keep concurrency
  bounded; do not force prior knowledge. This does not reduce chapter request
  count, and warm HTTP/1.1 connections can already overlap reads.
- **Three HTML shards concurrently:** the official action provides a design
  precedent. Current client assembly, Cookie recovery, memory, and fallback
  still need a separate comparison. Do not add the unused CSS shard or general
  multi-chapter concurrency merely to resemble the website.
- **HTTP resource caching:** retain validators and freshness metadata, honor
  `Vary` and applicable cache controls, and scope keys to account/book/snapshot,
  complete URL/query, and relevant headers. See [RFC 9111](https://www.rfc-editor.org/rfc/rfc9111.html#section-4).
  A 304 saves the body, not the request, and ETag does not replace image SHA
  validation. The observed 19 archive URLs were all different, so this sample
  gives no first-download exact-URL TAR hits.
- **No verified EPUB/TXT bulk route:** the inspected public repository HEAD
  remains the [pinned revision](https://github.com/moooyo/weread.koplugin/commit/2943080c2493a1ae262cc97c74908ed62928c935).
  Catalog, AI outline, purchase, and image TAR requests are not bulk-content
  contracts. The observed PDF-specific download URL is type-gated and does not
  establish EPUB/TXT capability. `/web/getConfig` did not establish a replacement
  for `psvts/pclts`. `/web/book/read` writes time/progress and is not a read-only
  download accelerator.

## Recommended research order

1. Measure successful full-book stages and validate task Reader-context reuse
   with one protocol variable changed at a time.
2. Preserve verified whole-chapter/resource results across bounded retries and
   skip still-complete items after cursor repair.
3. Prototype the byte-bounded two-envelope pipeline with slow-network,
   slow-CPU, slow-commit, cancellation, and large-packet fallback fixtures.
4. Evaluate HTTP/2 and three-shard scheduling independently; compare request,
   connection, retry, byte, wall-time, and memory measurements.

All future authenticated comparisons must use content the current account may
read, preserve normal access controls, and keep private content and research
archives out of the repository. No device action or product implementation is
claimed by this research document.
