# WeRead Source Adapter

Recorded: 2026-11-03 (Asia/Shanghai).

## Evidence and scope

The adapter follows the wire behavior found in
[`moooyo/weread.koplugin` revision `2943080c2493a1ae262cc97c74908ed62928c935`](https://github.com/moooyo/weread.koplugin/tree/2943080c2493a1ae262cc97c74908ed62928c935).
The revision was inspected from a public checkout outside this repository. No
account credentials, downloaded books, or research checkout are included here.

This is a static source contract, not a Tencent API availability guarantee. Pure
protocol fixtures are verified separately from the limited live service
observations recorded below.

The private daily trial subsequently produced a user-confirmed login and five
committed chapter checkpoints before an HTTP 401 stopped acquisition. This is
partial live evidence, not acceptance of complete downloads or all upstream
operations. The diagnosis and replacement-artifact results are tracked in
[Physical testing](PHYSICAL_TESTING.md).

The implementation remains a direct client: it does not require a project server,
server database, or hosted cover storage. Covers are public image references and
never establish content-acquisition capability.

## Authentication boundaries

`AuthSession` stores the API key, web-cookie jar, account display name, account
identifier, login time, and optional web authorization headers. Its `Debug`
implementation prints only credential-presence flags. Source errors contain
bounded codes rather than response bodies, request URLs, or credential values.

The Skill gateway requires a Bearer API key. Web-reader, renewal, progress-write,
and public-account requests require the web-cookie session. Configuring one does
not fabricate the other. The application owns credential persistence and private
file permissions; the adapter performs no filesystem writes.

Gateway requests never include web cookies. Public cover requests never include
account authorization headers. Trusted web API requests do not automatically
follow redirects. Book-resource requests authenticate only the primary origin
and the specifically reviewed resource origin described below. Other image
redirects remain HTTPS and are bounded to five hops. Responses are size-limited,
and all network requests have explicit connection and total timeouts.

Ordinary trusted web responses now merge rotating cookies and optional reader
headers into the shared session. Late ordinary responses cannot overwrite a later
credential revision or another account. The application persists changed
snapshots outside the metadata database and flushes the latest snapshot during
shutdown. Queued renewal events use the current shared snapshot rather than
writing an older event payload back to disk.

An HTTP 401 or a confirmed Web business-session code from a read-only trusted
operation can request one serialized renewal and one replay of that operation.
Concurrent failures can reuse an acknowledged replacement; failed renewal never
authorizes an unacknowledged replay. Renewal commits response credentials only
after explicit success.
Reading-position and elapsed-time POSTs are not automatically replayed. Public
or cross-origin resource responses cannot update account cookies, and resource
401/403 responses do not by themselves require account login. Diagnostics retain
only fixed operation labels, status numbers, and bounded error codes.

Chapter shard responses reject JSON business errors before encrypted-content
decoding. Only the known `e_0` book-format marker can enter the TXT branch;
messages, unknown JSON, and error objects containing a book ID cannot be treated
as chapter bytes. A pure chapter read may retry twice after 800 and 1600 ms for
the bounded transient-read error set. Each attempt obtains a fresh Reader
session and all shards, and every attempt must pass the unchanged checksum and
decode checks. Typed missing-login errors, HTTP 401/403, unsupported operations,
and oversized responses do not enter the ordinary transient-read retry loop.
Known identity failures have the separate one-recovery budget described below.
Other `Api` errors still receive only the bounded read retries; their codes are
not all classified as transient or permanent. This policy does not make
integrity failures globally retryable and never replays reading-time writes.

## Reviewed Web session business errors

The official public [Web app bundle](https://cdn.weread.qq.com/web/wrwebnjlogic/js/app.d51b45eb.js)
checks `-2010` and `-2012` in its GET/POST wrappers, requests
`/web/login/renewal`, and resends after `succ == 1`. Its
[Reader bundle](https://cdn.weread.qq.com/web/wrwebnjlogic/js/9.625560b7.js)
also classifies `-2013` as identity failure, without establishing it as an
automatic-renewal case. The bundles were read statically; their obfuscated code
was not executed. This is newly reviewed official Web evidence, distinct from
the pinned Lua adapter, which has no automatic business-code case.

This client applies automatic business-code recovery only to trusted read
operations and only to `-2010/-2012`. A chapter shares one authentication budget
between its Reader request and every shard. If renewal occurs while obtaining a
shard, the whole Reader state and shard set are fetched again. `-2013`, a second
identity rejection, or any failed renewal is a terminal result for that logical
read. Renewal itself never recurses. Account identity and the shared renewal
generation protect concurrent recovery; a generation advances only after an
acknowledged session has been committed. Position/time POSTs are not replayed.

## QR login

Reference: [`qr_login.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/qr_login.lua),
[`cookie.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/cookie.lua).

1. Warm `GET https://weread.qq.com/r/weread-skills` with a fresh cookie jar.
2. Obtain `uid` from `GET /api/auth/getLoginUid`.
3. Render a real QR code for `https://weread.qq.com/web/confirm?uid=...`.
4. Poll `GET /api/auth/getLoginInfo?uid=...&otp`, with an optional four-digit OTP.
5. Handle `NEED_OTP`, `OTP_NOT_MATCH`, `LOGIN_TIMEOUT`, and `OTP_EXPIRED` explicitly.
6. On `succeed=true`, build the new session from `webLoginVid`, `accessToken`,
   optional `refreshToken`, and the current login jar. Previous-account cookies
   are not reused.
7. Fetch `GET /api/userInfo?userVid=...` and
   `GET /api/skills/apikeyGet?only_show=1` with Cookie, `X-Vid`, and `X-Skey`.
8. Complete only when the Skill API key is present. Enabling WeRead Skill is a
   user action in WeRead; the adapter does not generate or enable keys.

`LoginFlow` owns a five-minute challenge lifetime and a cancellation generation.
An old response cannot authenticate a newly started login flow. It returns a
session to the application, which decides when to persist it and synchronize.

## Skill gateway contracts

Reference: [`client.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/client.lua),
[`library.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/ui/library.lua).

Every row below uses `POST https://i.weread.qq.com/api/agent/gateway`,
`Authorization: Bearer <api_key>`, and a JSON object containing `api_name`,
`skill_version="1.0.5"`, and the listed fields. The Skill version is an observed
protocol value, not this application's dependency or release version.

| `api_name` | Request fields | Adapter use |
| --- | --- | --- |
| `/shelf/sync` | No additional fields | Books and public-account subscriptions |
| `/store/search` | `keyword`, `count` | Store search |
| `/book/info` | `bookId` | Book metadata |
| `/review/list` | `bookId`, `reviewListType`, `count` | Recommended (`1`) or latest (`3`) book reviews |
| `/book/getprogress` | `bookId` | Cloud reading position |
| `/book/underlines` | `bookId`, `chapterUid` | Popular underline ranges |
| `/book/readreviews` | `bookId`, `chapterUid`, `reviews` | Thoughts in batches of at most 30 ranges |
| `/readdata/detail` | `mode`, optional positive `baseTime` | Reading statistics |

Search results are grouped under `results[].books[]`; a book can be wrapped in
`bookInfo`. Public-account subscriptions are books whose identifiers start with
`MP_WXS_`, as in the reference UI. The adapter does not assume that `mp` is the
only source of subscriptions.

Thought requests contain `{range,maxIdx:0,count:30,synckey:0}` entries. Returned
`pageReviews` and nested `review` objects are normalized for presentation.
Statistics modes are `weekly`, `monthly`, `annually`, and `overall`; the application
must present periods using Asia/Shanghai time rather than the execution host's
timezone.

## Catalog and reading content

Reference: [`content.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/content.lua),
[`protocol.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/protocol.lua),
[`reader_state.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/reader_state.lua).

- Catalog: `POST https://weread.qq.com/web/book/chapterInfos` with
  `{bookIds:[bookId]}` and web authentication. Normalize `data` or top-level records
  and their `updated`, `chapterInfos`, or `chapters` arrays. Non-readable entries
  have zero word count or are the cover chapter.
- Reader state: fetch `/web/reader/<encoded-book-id>k<encoded-chapter-id>` and
  parse `window.__INITIAL_STATE__` as JSON without executing JavaScript. Refresh
  `psvts`, `pclts`, and the reader token for each chapter request.
- XHTML: POST `/web/book/chapter/e_0`, `e_1`, and `e_3` with the signed content
  parameters from the inspected protocol. Validate every shard's uppercase MD5
  prefix before concatenating and decoding its obfuscated Base64 body.
- Text-format fallback: an `e_0` JSON response containing `bookId` selects
  `/web/book/chapter/t_0` and `t_1`. A missing `t_1` with HTTP 404 is treated as an
  absent optional shard; transport failures are not silently converted to success.
- Returned `ChapterContent` contains authenticated XHTML or plain text and an
  extracted text representation. Empty, corrupt, or invalid UTF-8 content fails.
  Remote scripts are never executed.

These are authenticated reading-transport contracts found in source. They are
not an EPUB download endpoint, evidence of access to every title, or proof that
a real account can fetch a particular chapter. The current text reader does not
claim full EPUB/CSS, PDF, mathematical-layout, or book-image rendering fidelity.

### Request-bound access evidence

The content-access correction also inspects a single strict
`window.__INITIAL_STATE__` script assignment from that fresh Reader response.
It matches the requested book and unique chapter UID, cross-checks the current
chapter, and binds account-sensitive evidence to the authenticated identity
and acknowledged session generation. Permission evidence and the server's
`psvts` clock must come from the same state. The client's `pclts` clock may
legitimately be zero, blank, null, or absent in SSR; the reviewed local fallback
is materialized once in that Reader session and never borrowed from another
script. Anonymous public Reader metadata confirmed the zero default during
the correction; this does not verify a private account's entitlement.
A generation change while obtaining the Reader
permits at most one additional Reader fetch before any shards are sent, using
the existing chapter authentication budget.

The reviewed state includes `reader.bookInfo`, `reader.chapterInfos`,
`reader.currentChapter`, `reader.memberCardSummary`, and the root user identity.
Positive purchase, free-chapter, and supported coherent membership evidence can
establish full chapter access. Missing, malformed, contradictory, or unsupported
fields remain unknown; a missing `paid` value is never interpreted as free.
The official [Reader bundle](https://cdn.weread.qq.com/web/wrwebnjlogic/js/9.625560b7.js)
and [permission utilities](https://cdn.weread.qq.com/web/wrwebnjlogic/js/utils.2f050835.js)
are static primary-source evidence. The client adds conservative consistency
checks rather than claiming exact equivalence with every official helper.

Restricted nonempty responses remain previews; an unpaid trial edition also
retains its edition limit. Checksums, resource manifests, word counts, and
successful item counts cannot independently prove full content. Gross text
inconsistency can withhold `Full`, but cannot by itself prove a lock.
`UnlockRequired`, `ContentUnverified`, and `ContentChanged` pause acquisition
without global login failure or automatic permission retries. See
[Content access](CONTENT_ACCESS.md) for storage, safe upgrades, and separate
synthetic/live acceptance boundaries. No purchase endpoint is introduced.

## Progress and reading time

Reference: [`read_report.lua`](https://github.com/moooyo/weread.koplugin/blob/2943080c2493a1ae262cc97c74908ed62928c935/weread/lib/read_report.lua)
and `protocol.lua` at the recorded revision.

`report_progress` refreshes the reader session and posts an enter payload followed
by a reading payload to `https://weread.qq.com/web/book/read`. Fields include
encoded book/chapter IDs, chapter index, offset, progress, a summary limited to 20
Unicode code points, session timing, elapsed reading seconds, and protocol
signatures. The response must contain `succ=true/1` or `synckey` before the
application may display a successful upload.

Reading time must come from actual reading activity. Opening a dialog, displaying
fixture statistics, or running a timer outside the reader is not an upload.

### Durable reading-time batches

`source::report::ReadReportRequest` carries a local batch identity, a stable hashed
account VID, the real batch start and active seconds, and a verified source
position. The source checks the current web-account identity, positive elapsed
seconds, and elapsed seconds no greater than the batch's real wall-clock window
before making a request. Missing credentials or an unverified chapter mapping do
not fall back to another book.

The protocol sends `rt` in seconds, `ct` as Unix seconds at transmission, and `ts`
in milliseconds. The local `session_id`, account key, and original start time are
not invented server parameters. The inspected scheduler defaults to 30-second
intervals with a ten-second minimum cadence; the payload builder accepts any
nonnegative integer elapsed time. The server's maximum credit, historical-period
crediting, and throttling limits have not been verified with a live account.

The observed route always carries `pr`, `co`, `ci`, `b`, and `c` together with
`rt`. No position-neutral time-report endpoint was found. Consequently a caller
must explicitly allow the accompanying cloud-position write. Turning off that
permission blocks network time reporting while keeping genuine local activity.

Before either enter or reading-time POST, the adapter fetches current cloud
progress and the book's current catalog. Both positions require a catalog UID and
positive chapter word counts; raw default indices and standalone percentages are
not proof. Payload chapter indices come from the matched catalog, and offsets and
percentage are normalized with its word counts. Missing API-key access or position
proof returns `read_report_progress_check_required` before any position write.

A current Reader whose local choice has been explicitly confirmed can pass the
local-only `active_reader_intent` flag and preserve intentional rereading or a
chosen local position. An old outbox batch uses its local position only when both
update timestamps are trustworthy and the local one is newer; otherwise it keeps
the freshly fetched cloud position, including a lower percentage that may reflect
intentional rereading on another device. No synthetic wire parameter is added.
The upstream contract has no conditional update or compare-and-swap operation,
so another device can still change position between preflight and POST. This
fresh-snapshot policy does not claim atomic cross-device synchronization.

The reference provides no server idempotency key. A transmitted report followed
by timeout, connection loss, an HTTP server error, or an unrecognized successful
response returns `DeliveryUncertain`. Such a batch stays unconfirmed and must not
be automatically retransmitted. A connection failure before transmission or an
explicit authentication rejection can be handled separately. Only an observed
`succ=true/1` or `synckey` acknowledgement produces a durable receipt.
Delivery uncertainty, retry eligibility, authentication requirements, and a
redacted failure code are written together as one `ReadReportFailure` record,
so a process interruption cannot split those meanings across partial settings.

Request construction is checked against independent values produced by the real
Lua protocol at the recorded revision, including the enter signature, SHA256
reader signature, and final report signature. These checks do not constitute a
live-account reporting result.

## Book chapter image resources

Reference: `content.lua` at the recorded revision, especially
`download_remote_images`, `download_chapter_assets`, and their file-based variants.
Book-image resources are derived from authenticated chapter HTML and catalog
metadata, independently of cover URLs.

- The catalog's `tar` field supplies a chapter resource archive. A leading `//`
  becomes HTTPS, and a leading `/` is relative to `https://weread.qq.com`.
  The GET Referer is the encoded book/chapter reader URL.
- The field may point to a TAR archive or, for converted documents, a ZIP archive
  identified by its `PK` signature. PNG, JPEG, GIF, and WebP files are selected by
  actual file signatures and validated with a bounded image decoder.
- Absolute or protocol-relative chapter `img src` values are fetched with
  `https://weread.qq.com/` as Referer. The implementation requires HTTPS, bounds
  redirects, and sends cookies only to exact `weread.qq.com` and
  `res.weread.qq.com` origins on HTTPS port 443. Other subdomains, suffix lookalikes,
  nonstandard ports, and external CDNs do not receive account cookies. Leaving
  this reviewed origin set permanently disables cookies for the redirect chain.
  Resource requests never carry the Skill Bearer key or reader authorization
  headers. Resource-origin responses cannot replace account credentials.
- No separate resource-ID endpoint, image encryption scheme, or source-provided
  cryptographic image checksum was established. TAR header checksums, ZIP CRC,
  image decoding, and local SHA256 cache identities are distinct checks; they do
  not invent upstream integrity guarantees.

This resource-domain exception follows `client.lua`'s `is_weread_url` and resource
request behavior at the pinned revision. The reference permits WeRead subdomains;
this implementation deliberately reviews individual origins instead of accepting
a wildcard. A physical comparison confirmed that the same catalog archive on
`res.weread.qq.com` returned 401 without its Cookie, then 302 with it and 200 after
the external download redirect without a Cookie. The earlier exact-primary-only
rule rejected this legitimate authenticated resource and misclassified it as a
global login failure. [Physical testing](PHYSICAL_TESTING.md) retains the scoped
diagnosis and replacement-artifact results.

The resource adapter retains exact HTML source aliases. Qualified paths must
match qualified archive paths; equal basenames in different directories are not
silently treated as the same image. A bare source filename is accepted only when
it has one unambiguous archive path. Query strings remain part of the alias;
fragments and redundant `./` components can be normalized. A leading `../` is
not generally resolved without a demonstrated chapter base path.

A separate, book-scoped converter alias handles the physically observed root
member `CB_{book_id}_note.png` referenced as `../Images/note.png`. The current
plan supplies the exact `CB_{book_id}_` namespace. Only root members with that
prefix can establish an alias, and the remaining filename, query, and case must
match completely. Ordinary exact paths take precedence. Other archive paths
sharing that alias count as ambiguity even when their bytes are identical;
different qualified references cannot collapse onto one scoped alias. Plans
without an explicit namespace retain strict matching. This observed converter
contract is project evidence, not a claim that the pinned reference already
implements CB-prefix stripping. Missing, ambiguous, SVG, embedded data,
lazy-only, or otherwise unsupported resources still fail explicitly.

Application limits are stricter than the reference's general archive limits:
32 MiB for an archive, 8 MiB for one image, 64 MiB total expanded image bytes,
512 images, and 4096 archive entries. Archive paths are never used as output
paths; cache objects are identified by their verified content digest.

Chapter image downloads produce one bounded packet containing the canonical
body, original HTML, verified resources, and exact source-to-asset mapping. The
CPU work to verify, decode, composite, resize, quantize, and encode acquired
images runs in a blocking worker, producing an opaque packet of prepared PNGs.
The UI thread commits those prepared objects and metadata without decoding the
original large JPEG archive again. The
worker waits for a one-shot acknowledgement after the application has persisted
the image manifest, chapter body, and task cursor. A failed cache commit, dropped
acknowledgement, cancellation, or account change cannot advance the checkpoint
or declare the next chapter complete. A text-only packet preserves real image
placeholders instead of claiming that missing images were downloaded.

## Public accounts and renewal

| Operation | Contract |
| --- | --- |
| Article list | `GET https://weread.qq.com/web/mp/articles?bookId=...&maxIdx=...&count=...` |
| Article HTML | `GET https://weread.qq.com/web/mp/content?reviewId=...` |
| Session renewal | `POST https://weread.qq.com/web/login/renewal` with `{rq:"%2Fweb%2Fbook%2Fread",ql:false}` |

Public-account requests include web cookies and configured `x-wr-ticket` and
`x-wrpa-0` headers. Renewal cookies and authorization headers replace the active
session only after an explicit success acknowledgement. Article IDs and original
URLs are normalized from the nested `reviews[].subReviews[].review.mpInfo` shape.
No public-account media is fetched merely because an article was listed.

## Unsupported remote writes

The reviewed source does not establish a remote shelf-add, shelf-remove,
bookmark-write, or personal-annotation-write contract. The adapter advertises
`remote_shelf_mutation=false` and `remote_annotation_write=false`. Local shelf,
bookmark, highlight, and note operations must remain distinguishable from cloud
changes until a separate reviewed contract is available.

## Verification and release evidence

Protocol golden vectors were generated on `ssh test-env` using the reference's
real LuaJIT modules, outside this repository. Rust tests check numeric and string
ID encoding, sorted query signatures, web app ID, checksum rejection, and single
and multiple-shard decoding against those independent values. Additional tests
inspect request method, URL, gateway payload, and credential isolation without
sending network requests.

The reference repository is licensed under AGPL-3.0 and identifies its copyright
as `Copyright (C) 2026 finlater and contributors`. This records a dependency and
implementation-reference obligation for release review; it does not select this
project's license. Slint, the Kindle backend, fonts, and any code derived from the
reference still require a compatible distribution decision before publishing a
release.
