# Resource lifetime review

## Finite closeout scope

This review covers same-process resource ownership, cancellation, error cleanup,
bounded queues, cache payloads, diagnostics, and launcher child processes. Each
round originally re-examined the integrated candidate, with new actionable
recommendations resetting the clean count. On 2026-11-08 the user explicitly
requested finite closeout instead of further full-domain discovery rounds.
The original goal of two consecutive complete four-domain rounds with no new
recommendations was not achieved; this report does not claim that it was.

Closeout retains the normal seven-font startup correction, catalog-commit and
Reader consistency, settlement of the application's own Article completion,
and safe maintenance of existing report-delivery evidence. Further extreme or
malformed-input hardening in isolated HTML, ZIP-footer, and variable-font
overlays remains deferred. Historical data, drafts, uncertain delivery proofs,
and the deployed private trial are preserved.

## Current combined verification

The complete Rust verification origin uses session 98803 / Python 2980606 on
`test-env`. Its actual post-format canonical leaf SHA-256 is
`f8c1b3944e17f5ce71257251cf1d8c09d1443a5868370b2bcede46515d1fa131`
over 532 matched/backcopied leaves, with zero final formatting changes. Its
67,399,168-byte transfer archive has SHA-256
`1fe308f7c2163affb53bdd317d82af04e3c8cd52e2f9a26c08de08e8c78904af`;
that earlier transfer hash is distinct from the compiled-source identity.

Current delivery/source-tool identity is
`58d8bf6ac774d8b103261d913c174ea1333c723f69a593b52d780353109acb15`
over the same 532 leaves. Only `packaging/tests/launcher_test.py` differs;
all 531 other leaf records and all four compiled Rust executables are unchanged.
Session 42855 / Python 3173164 runs the corrected launcher fixture, preserving
the original Rust log/run/source headers and six resource samples. Its separate
67,400,704-byte transfer archive is SHA-256
`4eb3c76ba8403beb5e8c6674cdad3ba01b6fdf8ec07820255737ea6dc1265151`.

| Check | Actual result | Duration |
| --- | --- | --- |
| One Cargo compilation | Exit 0 | 105.564 s |
| Corrected Article/report target | 7 passed, 0 failed | 1.07 s |
| Main font/readonly-CLI target | 49 passed, 0 failed | 5.13 s |
| Complete library | 1,481 passed, 0 failed, 3 manual benchmarks ignored, 0 filtered | 193.70 s |
| Complete Main | 52 passed, 0 failed, 0 filtered | 5.20 s |
| Complete Kindle backend | 65 passed, 0 failed, 0 filtered | 0.27 s |
| Complete Swash | 15 passed, 0 failed, 0 filtered | 0.48 s |
| Corrected launcher target | 8 passed, 0 failed | 52.555 s |
| Complete corrected launcher | 168 passed, 0 failed, no skipped/expected-failure result | 606.180 s |
| ARM preparation | Actual exit 0, single Cargo job, parallel with launcher | Cargo release log: 5m 21s |
| Guarded bundle/source archive | Exit 0; ELF, explicit members, CRC, permissions, and source bindings passed | Package fixtures: 3 passed / 0.023 s |

Formal execution receipts reconcile 388 library, 49 Main, 19 backend, and
15 Swash qualified names with their actual compiled lists and successful full
logs. The 28 orientation-related cases are accounted for across the library,
Main, backend, and launcher by actual full execution receipts. Ten formal
launcher names succeeded in the complete 168-case transcript. Only its Python
fixture changed after the original Rust run: exact source-leaf, executable,
list, log, resource, and qualified-name comparisons permit reuse of the four
unchanged Rust programs. Their original run/source headers remain intact.

Session 42855 / Python 3173164 naturally exited 0. Its ARM preparation also
exited 0; the preparation-to-join interval includes time waiting for the
concurrent launcher and is not presented as compilation time. The final
packaging guard accepted the complete current launcher and explicit Rust
origin, rechecked all current source leaves and prepared ELF/log bytes, then
verified the 35-member private archive and all three packaging fixtures.

| Final artifact | SHA-256 |
| --- | --- |
| Static stripped ARM ELF32 / EABI5 / hard-float, no dynamic section | `381a4b250ed5fb8b997a9d4bf34f29e0a964e57088ea8d4f6e169bbb1f4c97e3` |
| Private development ZIP | `522e37deb375cdc2778d4f9c97c81cf254105e62ea5a9b0cf68bee5552c4e4b9` |
| Deterministic current-source archive | `104026b777f08b630e67254f8add630e34d18c4c3a05d8cfe19abb2f8294aa60` |

The local delivery directory is `dist/resource-review-20261109/`. Its
`guard-resource-final.json`, `package-resource-final-contents.json`, and
`source-resource-final-provenance.json` bind the actual source, Rust origin,
launcher, ELF, ZIP, and source archive. The source archive contains the 532
explicit build/test/packaging/asset/vendor leaves; top-level README/docs,
external toolchains/registry caches, and runtime data are excluded. The ZIP
contains only the ELF, launcher, draft/configuration examples, and notices;
credentials, books, database/cache data, research, and private state are absent.

The complete source closes the ordinary Article/report regressions below.
Report cursor settlement now checks the measured account, book, and start
identity even at zero remaining demand. No identity assertion, admission
limit, historical record, uncertain delivery proof, or current draft was
discarded. The normal seven bundled fonts fit the unchanged 268,435,456
glyph-work budget with cumulative work 262,713,195.

## Same-run resource trend

The resource test performs 80 navigations, 320 painted frames, and 12
application drops. Samples are extracted from the same complete library
transcript, without a separate sampling repetition.

| Window | Descriptors | Threads | RSS (KiB) |
| --- | --- | --- | --- |
| Navigation warmup | 16 | 4 | 113,256 |
| Navigation first | 16 | 4 | 113,320 |
| Navigation second | 16 | 4 | 113,320 |
| Application drop warmup | 7 | 3 | 113,864 |
| Application drop first | 7 | 3 | 113,928 |
| Application drop second | 7 | 3 | 113,928 |

Descriptor/thread counts are stable and the two post-warmup RSS values match
within each workload. This bounded controlled Linux/fake-device trend does
not establish exact allocator peaks, an unrestricted CPU deadline, physical
Kindle behavior, or the absence of every possible leak.

These six samples retain the original complete Rust run's source/run identity.
They were not resampled or relabelled during the launcher-only correction.

[Active portrait orientation](ACTIVE_ORIENTATION.md) adds automatic 0/180-degree
changes while retaining independent manual application rotation, including
landscape. It uses protected same-launch calibration, finite event processing,
stale-contact cancellation, and a written-frame input gate. No artificial
debounce or synthetic native Home/sleep transition is added. Current
resource-review artifacts have not replaced the `2fd3/799d` Scribe 2022 /
firmware 5.19.5 trial, and this task has made no Kindle deployment.

## Historical verification checkpoints

The following chronology preserves the exact earlier sources, failures, and
passing subsets. References to a current candidate or a pending stopping rule
within that chronology describe that historical checkpoint; they are not
additional closeout gates or evidence for the final candidate.

## Latest failed complete run before the closing fixes (24840)

The corrected3 checkpoint on `test-env` used session 24840, Python 2965705,
and actual post-format canonical leaf SHA-256
`50fa94bc64677383439c38767bfaa62d8cc765597ab6faf4aa8002428173f92b`
over 529 leaves. Compilation passed in 30.545 seconds, and all 47 Main font
targets passed in 5.25 seconds, including the seven real bundled fonts and the
small CFF matrix fixture. Complete Main passed 50/50 in 5.26 seconds. Complete
library execution passed 1,468 cases, failed seven, and ignored three manual
benchmarks in 192.65 seconds. The driver naturally exited 1; backend, Swash,
launcher, and ARM/bundle work did not execute.
The small CFF correction changes only a same-length local FD identity matrix
in one test; production limits and output/fee/one-less assertions are retained.

Four Article failures shared a fixture mistake: its path getter excludes query
data, while the assertion expected both together. The correction retains exact
GET path and complete review-ID query checks. An idle report fixture now supplies
the actual required Source capability and progress-write preference while
retaining its no-network, no-cold-copy, and no-write assertions. Two existing
report-proof regressions exposed a production omission: a zero-demand return
trusted a durable cursor before checking the measured run's account, book, and
start time. Flush, archive, and the legacy test bridge now share borrowed
admission and identity checks before cursor settlement. No quota or identity
assertion was relaxed. These corrections await the final combined verification.

The same failed complete run sampled navigation at 16 descriptors, four
threads, and 113,020 KiB RSS in all three windows. Application-drop windows
reported seven descriptors, three threads, and 113,564/113,692/113,692 KiB RSS.
These are controlled Linux/fake-device samples, not device measurements or a
claim that every possible leak has been excluded.

Resource-review artifacts have not been deployed to a Kindle. The separate
`2fd3/799d` Scribe 2022 / firmware 5.19.5 private-trial evidence remains scoped
to that installation. The later user-authorized active-orientation change and
the necessary regression corrections are combined in one new final source
identity. A compile/target preflight runs independently while the isolated
touch-transition tail is finished; it cannot qualify as full verification.

The current completed RL01-RL85/extended-RL79 checkpoint passed 1,093 library
cases with three manual benchmarks ignored, and all 23 Main cases on
`test-env`. The same compiled executables passed 533 library targets and 20
Main font targets. Its actual post-format canonical identity is
`09122b359e97db1441fa8275ac0721b2e2b72fd673b708be93beb1ad7cb7a088`
over 268 staged source leaves. R27 subsequently accepted eight recommendations:
new RL86/RL87/RL88 and omitted callers in RL18/RL48/RL50/RL83/RL26. All changes
are formally refrozen with 44 new library regressions and seven new launcher
regressions (165 total). Original session 29197 failed compilation in 34.621
seconds because a private historical Snapshot lacked Default. Corrected session
34025 failed compilation in 91.368 seconds because a new Book fixture lacked
its Path import. Both mechanical corrections are refrozen; neither failed run
executed targets, Main, full suites, resource samples, or launcher tests.
Corrected2 session 46208 compiled in 114.61 seconds, passed 577 library targets
and 20 Main font targets, and passed all 1,137 library cases with three ignored
benchmarks in 170.18 seconds and all 23 Main cases in 1.01 seconds. Its separate
actual post-format source identity is
`eef2b4e0d112520a84d5be005239ff906e27012552a2c1e376087ffb9f7dcc3b`
over 277 leaves. The driver naturally exited 1 after those successful suites:
its single-line formal-case matcher rejected interleaved diagnostic output.
The same serial libtest transcript supplies a supplementary receipt confirming
all 44 formal cases; no Rust suite was repeated. Changed launcher targets and
the 165-case full launcher suite were not executed.

R28 completed all four fresh source domains and accepted ten recommendations:
new RL89-RL92 and omitted callers in RL18/RL26/RL51/RL65/RL79/RL88. The new
mechanisms concern reconstructed HTML styles, rejected native edits retained in
Undo, local note fragment output, and incremental report allocation. Corrected2
session 66686 compiled that candidate in 115.289 seconds. Its library target
passed 699 cases, failed 14, and ignored two in 135.67 seconds, with 561 filtered
out. The driver naturally exited 1 before Main, complete suites, resource
samples, backend tests, or the changed launcher. Its immutable post-format
identity is
`82fc1117960347db25e95134a0e125bb26283899c46aca8d736abaf033be6325`
over 390 leaves. Four test-only corrections preserve the setup, output-work,
pixel, dynamic native-flag, and zero-copy assertions; they have not been rerun.

R29 completed all four fresh source domains and accepted ten recommendations:
new RL93/RL94/RL95 and omitted callers in RL88/RL90/RL89/RL51/RL92/RL80/RL73.
The new mechanisms concern Article image input/output admission, reachable
font-outline expansion, and physical outline-mask allocation. Their isolated
corrections include the same-version Swash 0.2.10 source overlay and explicit
package tests. The remaining Store/report and reachable-loca-tail interfaces
are formally refrozen. The first R29 compilation stopped before producing any
test executable because its manually serialized Swash dev edge did not match
the lock; the corrected attempt then stopped because Swash package dev tests
require explicit workspace membership. Both natural exit-1 receipts are
preserved. Offline lock reconciliation retains every package version and
checksum. Swash is the only new workspace member, and default builds still
select the application. No current R29 candidate has passed validation.
The clean count remains zero; two later complete clean reviews and current
full verification are required. The earlier 158 passing launcher cases retain
their earlier source identity and do not cover the changed launcher.

Corrected2 session 54503 then failed compilation in 102.254 seconds because one
Article fixture needed an explicit map-key type and the Slint Model trait.
Corrected3 session 72382 compiled in 121.25 seconds. Its actual test lists contain
1,342 library, 46 Main, 48 backend, and six Swash cases; all 246 carried/new
library declarations, 43 Main font declarations, two changed backend cases,
and six Swash cases were resolved against those lists. The library target
passed 775 cases, failed four, and ignored two in 144.40 seconds, with 561
filtered out. The driver naturally exited 1 before Main, full suites, samples,
backend, Swash, or launcher execution. Its actual post-format identity is
`3771dd4ad6af1baf20790c9041d0bed3270b4bb53b2f4176d192c765d0ee216a`
over 483 leaves.

Three target failures require strict fixture premises: a durable chapter packet
is Running until the fenced Finished transition; the encoding meter observes
one Store rather than an additional legacy setup connection; and the Source
image boundary rejects occurrence 2,049 before the later asset assertions. The
fourth failure confirmed a production tail in RL92: existing Pending delivery
maintenance had incorrectly reused the new-allocation Store envelope. A fixed
current-Outbox maintenance bridge preserves the same input/key/transaction and
cleanup checks independently of that allocation limit. These corrections are
formally refrozen but unexecuted. R30 completed all four fresh source domains on
that immutable failed-validation reference. It accepted thirteen cause groups:
new RL96-RL101 for inventory, layout collections, ZIP extra records, text-to-HTML
encoding, ReaderSession projection, and Zeno fill work; omitted consumers in
RL81/RL78/RL80/RL84/RL89/RL93/RL92 are grouped once each. The numeric resumed
download-prefix candidate was withdrawn as an unrequested bounded optimization.
All six implementation groups are formally source-frozen. Their new declarations
contain 101 library cases (Store 27, Cache/BookAssets 24, UI 18, Source 23, ZIP
seven, and two report-maintenance cases) and nine Swash fill-raster cases. These
declarations are separate from actual compiled lists and execution receipts.
The first R30 driver, session 18174/Python 2901404, naturally exited 1 after
compilation failed in 51.623 seconds with exit 101. Five errors concern App
error-type, inventory-API, and chapter-tuple bridges. Its actual post-format
identity is
`cc9d9559d134765d08223448220053846805408757f23ea864dbbf0dc80d4761`
over 524 leaves, with all leaves copied back. No target, Main, complete suite,
sampling, backend, Swash, or launcher run followed that compilation failure.
The isolated Source ReaderSession v3 and Zeno fill v2 limited source reviews
closed their earlier same-family tails without new limited-scope suggestions;
they do not establish application test results or complete clean rounds.
These are source mechanisms and arithmetic, not large-input execution or
Kindle observations.
Corrected session 33663/Python 2902887 compiled in 131.559 seconds and resolved
all 350 formal library, 43 Main, two backend, and 15 Swash declarations in actual
lists of 1,446, 46, 48, and 15 tests respectively. Its library target passed
876 cases, failed seven, and ignored two in 150.31 seconds, with 561 filtered.
The driver naturally exited 1 before Main, complete suites, samples, backend,
Swash, or launcher execution. Its independent actual post-format source is
`a2e299bfadebc9c3f8f6ef58f58a544e770ba6a500db749269f94f5510820243`
over 524 leaves. The failures concern exact test setup or test-entry contracts:
historical empty SQL bodies versus cache upsert, real schema columns, explicit
locked access evidence, explicit inventory retry, available-font preparation,
and the production fallible Article parser. Corrections preserve the original
case names, capacity limits, and state/error assertions; they have not executed.
No clean round is counted. Two later complete clean reviews and current full
verification still precede final ARM/build-package provenance.

The integrated RL01 through RL65 checkpoint passed its complete remote Rust
suite: 850 library cases and 23 binary-entry cases, with three manual benchmarks
ignored. Its library target passed 279 cases and its Main font target passed 20.
Round 23 accepted six further findings, RL66 through RL71, for NCX entity
expansion, image-anchor work, weighted progress, keyboard admission, account
snapshots, and Total chart output. Their first compilation passed, but a targeted
run failed an existing historical-note view regression before the complete
suite or sampling. That narrow RL69 correction is refrozen with two additional
callback cases. Round 24 accepted six further output/parser recommendations,
RL72 through RL77. Their corrected compilation passed, but its library target
failed two new calibration assertions before Main, complete suites, or sampling.
Those test-only corrections are refrozen. Round 25 accepted six further local
working-set/context recommendations, RL78 through RL83. Their combined candidate
compiled, but its library target passed 496 cases and failed three UI
regressions before Main, complete suites, or sampling. Two confirmed RL78
consumer tails and a strict test-only drawing-token adaptation are refrozen
in two files, retaining the original pixel and anchor assertions. Their three
regressions passed in the corrected3 target, which passed 498 cases and failed
one bulk snapshot identifier/stamp regression. Main, complete suites, and
sampling were not executed. That test now establishes an actual inode/size
change and retains both rejection assertions; its correction is unverified.
Round 26 completed four fresh domains with three recommendations: two new
mechanisms, RL84/RL85, and an omitted Source/BookAssets SVG guard in the existing
RL79 family. All four implementation groups are formally refrozen with 34 new
library regression functions: eight Publications, 16 statistics, eight
Source/Jobs SVG guard, and two BookAssets guard cases. They passed within the
later 1,093-case complete checkpoint; previous failed targets remain historical.
 The clean count remains zero. The earlier RL44/RL42
launcher passed its complete 158-case run. Two later consecutive clean reviews
and final ARM/bundle identity remain pending. These results add no
device-acceptance claim.

All execution and verification for this task run on `test-env`. Source editing
and review take place on the operator's Windows checkout. No local application
verification or physical Kindle session is part of this review. A passing Linux
fixture is not a new claim about Kindle hardware behavior.

Intentional process-lifetime font and renderer storage, bounded image caches,
downloaded books, local reading history, and unresolved delivery evidence are
not classified as leaks merely because they remain allocated.

Capacity findings are recorded separately from permanent leaks. In particular,
the input, cookie, and import cases below are demonstrated by constructed input;
they do not imply that the user's account or books produced that behavior.

## Operational capacity policies

| Resource | Policy |
| --- | --- |
| Metadata work | Eight credits cover active work, queued results, and handler processing; rejection is synchronous. |
| Public images | Two credits within the metadata budget, retained across document/account changes until old buffers are dropped. |
| Login work | One credit covers the in-flight operation and queued result; regeneration does not release a retained old buffer. |
| QR challenge | A 1 KiB opaque UID and 2 KiB complete confirmation URL, checked from the borrowed field and exact percent-encoded byte count before copying. |
| Download packets | One shared credit is acquired before chapter-data fetching or cloning and retained through processing or discard; reliable terminal results use the same credit. |
| Download planning | Only remaining task metadata enters a bodyless plan, with 8,192 records and 64 MiB retained metadata admission. Cached events carry identity/cursor data under the packet credit; the Store verifies the actual current chapter before an atomic cursor-only acknowledgement. |
| Download progress | One independent opportunistic credit; redundant visual updates are skipped when occupied. |
| Download preparation | One shared permit remains with the actual blocking worker. |
| Close-failure history | At most 32 records of 8 KiB, with duplicate/omitted counts. |
| Application diagnostics | Current file plus two backups, each at most 1 MiB; messages at most 8 KiB. |
| Managed supervisor diagnostics | A 64 KiB in-place threshold plus one finite diagnostic. Bootstrap/external output is identified separately. |
| Cookie state | At most 256 entries, 64 KiB including header separators, and 8 KiB per entry; atomic acceptance or rejection. |
| Account file | At most 1 MiB of encoded JSON, checked without constructing an unlimited serialized copy. |
| Imported bodies | Existing 32 MiB body and unique uncompressed archive limits remain. |
| Inventory working sets | Direct SQL metadata precedes selected key/manifest/task payloads in one snapshot. Store defaults admit 65,536 rows, 64 MiB encoded input and metadata/scratch, and 128 MiB logical work; FileCache uses its own target-sized row-slot forecast and 32 KiB key limit. |
| Derived Reader/Article layout | One staged operation shares 64 MiB live requested metadata and 128 MiB allocation work, including old/new buffer overlap. Release refunds live storage, never spent work. Failed preparation retains the current view. |
| TXT escaped output | Complete escaped HTML output is admitted against 8 MiB before building owned output; metadata and logical encoding work are admitted separately. |
| ReaderSession fields and requests | Selected opaque fields have 4 KiB individual and 16 KiB aggregate limits. Every relevant duplicate is prepaid before whole state decoding; an unusable strict clock prices the actual loose fallback first. Signed/request-derived bytes are checked before copying, hashing, or URI construction. |
| Derived import data | A separate 64 MiB retained-data estimate, including repeated metadata, with 16 KiB titles and the existing 4 KiB paths. |
| Repeated EPUB work | Separate cumulative raw-read and decoded-byte limits, including repeated entries; empty entries also consume the reading-order count. |
| Native command output | One protocol deadline covers output, EOF, and direct-child state; retained child tracking has a fixed slot budget. |
| Native handoff readiness | Workers must be joined and the existing Active/Retired command registry must be quiescent; normal checks do not start commands. |
| Remote image acquisition | Actual fetched bytes consume a separate 64 MiB work budget, including duplicate content from distinct URLs. |
| ZIP initialization | Bounded entry points admit at most 8,192 EPUB or 4,096 image records per candidate, with cumulative 128 MiB reads and 64 initialization attempts. |
| ZIP retained extra fields | One constructor operation shares 64 MiB retained metadata and 1,048,576 extra records across directories/candidates. Enum storage, capacity growth, and old-buffer overlap are admitted before push/reserve; rejection remains terminal. |
| Outline fill storage/work | The opt-in Zeno path admits 1,048,576 cells, 16 MiB cumulative requested storage/growth peaks, and 64 Mi logical raster/reserve/relocation accesses before the actual producer advances. Only successful staging replaces the caller image. |
| Image-anchor derivation | 2,048 visible occurrences, 16,384 retained events, 262,144 retained edges, and 64 × 1024 × 1024 Unicode scalar work units, charged before their corresponding allocation or operation. |
| Remote underline inputs | 4,096 records, 16 KiB quotes, 4 KiB opaque ranges, and 4 MiB aggregate selected-field bytes; borrowed preflight precedes copying. |
| Cached underline inputs | SQL admits at most 16 MiB encoded JSON and 4,096 records before copying or deserialization; borrowed selected-field limits then apply before replacing the Reader model. |
| Remote underline matching | One current document/list/account-version projection; distinct quotes pay a conservative 5 × body bytes + 3 × quote bytes fee before matching against a 64 MiB work allowance. Failed versions are cached too. |
| Remote page overlay | At most 1,024 actual line marks and 256 lines; each mark prepays B × (B + 1) + 8B logical caret-work units, with B the line's UTF-8 byte length, against 64 × 1024 × 1024 units. Excess rejects the entire remote overlay. |
| General caret geometry | The RL53 implementation admits 64 × 1024 × 1024 logical work units, 16 KiB input per line, and 8,193 positions per operation before allocating or measuring. App pages preflight complete geometry demand for at most 256 lines before publishing any derived marks. Real-font and App regressions passed within the 818-case checkpoint. |
| Power observer startup/retirement | Terminal signals are latched through finite Power/App fork registration. A seven-field ownership/GO handshake precedes subscriber exec; retirement uses captured original identities, including only proven same-birth PID1 orphans. |
| Idle HTTP connections | Reviewed fixed HTTPS/443 origins retain at most four idle connections per host. One shared arbitrary-resource transport retains no idle connections; every redirect destination is classified independently. |
| Cache verification | Operation-local unique-payload proofs use 32 KiB streaming buffers and prepaid reads. Reader verification admits 64 MiB plus 2,048 probes; recovery/publication admits twice the configured cache-byte budget. Publication destination stamps are checked before scoped debt, retirement, pin, and rollback mutations. |
| Thought display | Lightweight descriptors precede pagination. At most 16 selected cards materialize previews; cached JSON is limited to 1 MiB and 4,096 records before deserialization, previews to 8 KiB, quote layout to 16 KiB, and opened text to 64 KiB. Hidden thought views do not load previews. |
| Image-source membership | One operation admits 4,608 names, their bounded aggregate bytes, and separate 128 MiB member-copy and encoded-hash work before retaining aliases; first-seen order and ambiguity remain explicit. |
| Catalog metadata | At most 8,192 records, 4 KiB opaque book IDs/UIDs, 16 KiB stored account bindings, and 64 MiB retained metadata. New-row and inherited-account-binding copies are priced before metadata materialization; existing bodies remain in SQL during reindexing. |
| Font initialization | The frozen candidate retains 64 MiB/file, admits 16 raw faces/file and 128/initialization, and charges 128 MiB actual input work plus 256 MiB original-blob/full-metric-copy forecasts. Failed reads, probes, parsing, and overwritten registrations do not refund work. |
| Font metadata and inventory | Admit 64 raw variation axes, 8 MiB application/axis/name forecasts, 4 KiB final families, and selected raw name spans before temporary registration. Inventory has 256 entries and 1 MiB encoded path bytes. These are logical resource policies, not total Fontique/rasterizer RSS or wall-clock guarantees. |
| Readonly Book metadata | At most 8,192 selected records, 64 MiB retained work, and 131,072 SQL cells; existence/source/title/reading-state consumers use narrower scalar capabilities and do not load intro. Full Detail and required merge operations retain complete Book data. |
| HTML style projection | One immutable-DOM operation admits 64 MiB logical work and 8 MiB memo/scratch forecasts. Borrowed span identity permits repeated reconstructed styles to reuse the same result; exact filter/lowercase/contains semantics use constant matcher scratch. R28 revalidation is pending. |
| Local page fragments | Local, remote, and selection intersections share 1,024 marks and separate 64 MiB logical byte/work limits, admitted before mark or SharedString construction. Local solid rectangles are not charged as remote dashed rectangles. R28 revalidation is pending. |
| NCX entity expansion | DOM parsing is preceded by borrowed entity preflight: at most 16 MiB source, 8,192 definitions, 4 KiB names, 64 MiB expansion/copy forecast, and 64 MiB lookup/scanning work. Failed quota admission cannot become a heading fallback. Its initial implementation passed in the RL01-RL85 checkpoint. |
| Keyboard input | Search permits 4,096 Unicode scalars and thought drafts 16,384. App append, candidate, mode, suffix, and prepared-paste gates preserve previous input; stored oversized notes remain intact. R28 adds native insertion/composition admission to prevent rejected Undo residency, with revalidation pending. |
| Total statistics | At most 512 annual bars, prepaid at 512 logical bytes per row within 256 KiB. An unavailable Total projection preserves raw responses and history. Other periods retain fixed output; the initial implementation passed in the RL01-RL85 checkpoint. Remote selected-field output has a separate R28 correction. |

These are project resource policies, not claimed upstream limits or a total
process RSS cap. The pinned Slint 1.17.1 core image cache also has a 5 MiB weighted
limit in `graphics/image/cache.rs`; it is separate from task PNG storage and
currently displayed images. That dependency source was read on `test-env`.

## Findings and changes

The final execution table and qualified-name receipts above are authoritative
for the combined candidate. Discovery-time evidence below retains the original
scope; its historical pending statements are not new closeout gates.

| ID | Finding | Resolution | Historical discovery evidence |
| --- | --- | --- | --- |
| RL-01 | Repeated shutdown attempts enqueue additional `Stop` messages while device I/O is blocked. | Send the stop command only once; retain the join handle for later bounded retries. | Remote core suite: repeated blocked-I/O stop/join polls retain one stop command. |
| RL-02 | Login cancellation fences results but leaves old network futures alive until transport timeout. | Notify and drop cancelled login futures while retaining account/request fences. | Remote core suite: real cancelled login futures and epoch fences release owned work. |
| RL-03 | Cancelling an outer download task does not stop a running blocking preparation worker. | Hold one shared preparation permit until the worker actually exits; reject cancelled queued work. | Remote core suite: queued and running blocking preparation retain the actual worker permit. |
| RL-04 | Rapid metadata operations can create unbounded concurrent requests. | Add a total request budget and a smaller cover budget without an unbounded waiting queue; report recoverable saturation. | Remote core suite: metadata/public-image admission and synchronous saturation recover without an unlimited waiting queue. |
| RL-05 | Cache publication, invalidation, or unlink failure can leave unindexed payloads outside the reported budget. | Persist cleanup identities, protect referenced payloads, include pending bytes, and retry cleanup before new publication. | Remote core suite: repeated same-instance publication/index/unlink faults preserve cleanup accounting. |
| RL-06 | Repeated failed close requests retain an unlimited collection of full error strings. | Keep 32 bounded records, coalesce adjacent duplicates, and count omitted occurrences. | Remote binary-entry and library suites: bounded close history, error strings, and omission counts. |
| RL-07 | Confirmed per-minute reading reports and their result settings remain in every future outbox snapshot. | Compact proven contiguous confirmed intervals into durable per-run high-water evidence; preserve unresolved delivery records. | Remote core suite: confirmed-range compaction and all side records commit atomically. |
| RL-08 | An initial supervisor identity write can fail before cleanup traps are installed, leaving its lock behind. | Install owned-resource cleanup before writing the identity. | Complete 135-case launcher regression: supervisor identity-write failures release owned locks. |
| RL-09 | Wrapper identity-file failure after fork can orphan a child and strand the event supervisor. | Add independent in-memory child ownership and bounded wrapper cleanup, including explicit failure events. | Complete 135-case launcher regression: wrapper-record failures retain and reap owned children. |
| RL-10 | Runtime storage failures and repeated launches can grow diagnostic files without a retention policy. | Add bounded application diagnostics and managed launcher log retention. | Remote core and complete launcher suites: bounded diagnostics; bootstrap/external output retains its separate scope. |
| RL-11 | Recent temporary payloads from an older cache version have no cleanup record and remain outside the budget. | Register and reclaim strictly named legacy temporary files immediately during single-owner startup recovery. | Remote core suite: strictly named legacy temporary payload recovery in the same cache instance. |
| RL-12 | One unresolved report gap prevents compaction of all later confirmed minutes in a long run. | Retain separate confirmed ranges without crediting or deleting the unresolved gap. | Remote core suite: an uncertain gap cannot retain an unlimited confirmed suffix or lose allocation evidence. |
| RL-13 | Deferred visible covers have no reliable rescan after unrelated requests release capacity; the first correction omitted Library/Detail and repeatedly scanned an idle Search page. | Resume only deferred current-page admission after capacity returns, using actual visible identities without a waiting queue or idle cache scans. | Remote core suite: current visible Library/Detail/Search refill, pagination, and idle behavior. |
| RL-14 | Dropping a cancelled wake guard detaches an in-flight worker before a new guard or sleep handoff. | Retain one stopping guard until joined and include it in lifecycle acknowledgements. | Remote core suite: one Active/Stopping wake worker remains owned until joined. |
| RL-15 | A transient child-birth observation failure can bypass wrapper cleanup or leave a final wait unbounded. | Retry ownership observation while the wrapper is alive and reap only observed exited children. | Complete 135-case launcher regression: transient birth observation and confirmed-exit waits. |
| RL-16 | A marker unlink failure can abort an EXIT trap before later process, display, and descriptor cleanup. | Attempt resource cleanup independently, preserve ownership gates, and return a visible failure after remaining cleanup completes. | Complete 135-case launcher regression: marker failures preserve independent cleanup and failure status. |
| RL-17 | API-key space/period actions bypass the same input bound enforced for ordinary keys. | Apply the existing 16,384 limit to both actions. This is an adjacent input-bound correction, not a claim of a background memory leak. | Source inspection confirms the 16,384 guards for space and period. The combined suite passed; no dedicated new boundary test is claimed. |
| RL-18 | Accepted cookie names can accumulate across otherwise bounded HTTP responses, inflating client/session snapshots and account files. | Apply atomic cookie count/byte limits and bounded account-file I/O without partial credential eviction or replaying elapsed POSTs. | Remote cookie/session/report suites and the 10-case login target: synthetic and loopback atomic-capacity cases, with no live-account growth claim. |
| RL-19 | Repeated EPUB spine titles and accumulated TXT headings can amplify small input into unbounded retained metadata relative to the body limit. | Account for repeated retained metadata before cloning and bound title construction while preserving normal text import. | Remote core suite: repeated retained spine metadata, TXT title bounds, and zero-byte reading-order entries. |
| RL-20 | The ZIP dependency allocates its central-directory index before the importer's 8,192-entry check. | Apply the limit inside the pinned parser before allocating each candidate index, including ZIP64 counts before narrowing. | Remote root synthetic ZIP regressions exercise the bounded constructor. The published crate lacks upstream tests/data; no upstream complete-suite claim. |
| RL-21 | A helper that exits while a descendant retains stdout can bypass the native command's process-only timeout. | Use nonblocking bounded reads and a shared output/exit deadline, with fixed direct-child tracking and bounded cleanup. | Remote core suite: constructed inherited-stdout deadline and owned-child retirement cases. |
| RL-22 | Distinct image URLs returning duplicate payloads consume network/hash work without consuming the unique retained-data budget. | Count actual fetched bytes independently and constrain each fetch by the remaining work allowance. | Remote core suite: duplicate remote digests still consume actual acquisition work. |
| RL-23 | Recursive article DOM walkers consume call stack proportional to input nesting depth. | Use explicit iterative traversal while preserving text, caption, suppression, block, and anchor behavior. | The 37-case article target and combined suite include isolated deep Unicode/body/caption traversal. |
| RL-24 | A failed temporary unpin can leave an unreferenced asset pinned until restart, exhausting reusable cache capacity. | Retain one dirty/reconciliation flag and require evidence-based pin/debt recovery before another publication. | The 29-case metadata/asset target and combined suite include dirty pin rollback and same-instance reconciliation. |
| RL-25 | Offline sleep/wake retries can retain cancelled, off-screen metadata identities across cycles and account changes. | Retain only current-page or explicit download intents and clear old-account retry work without changing resident rendering state. | Remote core suite includes the 1,024-cycle offline intent regression and account invalidation. |
| RL-26 | Wake-probe cancellation uses an unbounded wait that can strand cleanup when the wrapper or its sleep is stopped. | Track owned probe identities and apply bounded stop/continue/escalation/reap handling to the wrapper and direct child. | Complete 135-case launcher regression includes all owned probe deadline, stopped-child, and cleanup cases. |
| RL-27 | Joined hardware workers can leave a tracked native helper in an Active/Retired command slot. | Require command quiescence after worker joins before acknowledging native ownership or successful exit. | Remote core suite: workers and the Active/Retired command registry must both be quiescent. |
| RL-28 | Tolerated image/cache read errors can create cleanup debt without notifying the publication reconciliation gate. | Mark evidence/cache read errors dirty and reconcile before cached-only or image-free publication. | The 29-case metadata/asset target and combined suite include cached-only and image-free publication gates after tolerated reads. |
| RL-29 | Rejected ZIP footer candidates can repeatedly parse the same directory without a cumulative initialization-work limit. | Add read-byte and candidate budgets to the bounded constructor while preserving legacy entry points and ordinary fallback. | Remote root synthetic ZIP regressions cover cumulative reads/attempts, fallback, and restoration of the original reader. |
| RL-30 | A temporary input-discovery descriptor lacks atomic close-on-exec protection while native commands can spawn concurrently. | Set `O_CLOEXEC` on discovery opens and create the Linux wakeup pipe atomically with its inheritance/nonblocking flags. | Atomic source flags were inspected; the unchanged backend passed 47/47. No dedicated runtime descriptor-inheritance assertion is claimed. |
| RL-31 | Article image batches reparse the whole document for each image, and explicit load-all does not continue past a full two-request budget. | Reuse parsed image metadata, retain only current-document batch intent, and resume bounded batches without idle reparsing or automatic failed-image loops. | Remote core suite includes current-document load-all continuation, parsed metadata reuse, and idle/failure behavior. |
| RL-32 | Image transport permits are released before the UI consumes queued payloads, allowing continuation to exceed the current-document pending budget. | Count both in-flight and queued current-document tokens against two admission credits. | Remote core suite: current-document tokens and Jobs credits include queued and processing payloads. |
| RL-33 | Opaque upstream book IDs can collide with an internal article-image string prefix and bypass book-cover completion cleanup. | Distinguish book-cover and article-image results by explicit purpose rather than interpreting an ID prefix. | Remote core suite: typed cover/article-image purpose preserves opaque IDs, including constructed prefix collisions. |
| RL-34 | Empty-output compressed members can consume substantial compressed input while uncompressed/decoded counters remain zero. | Charge repeated compressed-member work before decompression, including overlapping image ZIP payload reads. | Remote core suite: empty deflate and repeated compressed-member work is charged before decoding. |
| RL-35 | Final or failed initialization cleanup can skip Std hardware worker joins when persistent state writes fail first. | Independently attempt terminal/exceptional hardware and command cleanup while preserving the original error and refusing successful handoff markers. | Remote core suite includes nine terminal/failure-cleanup cases, preserving the first SQL cause and independent producer cleanup. |
| RL-36 | Foreground supervisor system queries and restoration actions can block event consumption or cleanup without a tracked deadline. | Route production startup/wake queries and statusbar/refresh actions through owned bounded execution; preserve confirmed empty process snapshots and native-state evidence. | Twelve targeted cases and the complete 135-case RL36/RL37 launcher regression passed remotely. |
| RL-37 | A terminal signal after stopping a native window manager but before recording its identity can leave that non-child process stopped. | Register one pending stop identity before the signal and transfer it to the paused list only after success; independently restore pending and confirmed identities. | Five actual interruption-window fixtures and the complete 135-case launcher regression passed remotely; no new physical observation. |
| RL-38 | In-flight permits are released after sending a result to an unbounded channel, while queued payloads survive document/account resets or download cancellation. | Extend admission ownership through queued payload consumption or discard, including metadata, public images, and downloaded chapter packets. | All 26 Jobs cases and two same-turn App failure cases are included in the combined suite; eight cases use genuine queued producers. |
| RL-39 | Image-occurrence limits are checked after anchor collection, and each event retains up to 128 alignment edges without a cumulative graph/work bound. Repeated source identities bypass the separate URL-deduplicated acquisition count. | Admit occurrences, retained events/edges, and scalar work before their operations. Use a strict Result and typed quota error without partial metadata publication; preserve normal Unicode/caption alignment and conservative branch-overflow behavior. | The 37-case article target includes ten new low-quota/production-entry cases; metadata/asset target and combined suite passed. Constructed capacity evidence only. |
| RL-40 | A login UID admitted by the general JSON response limit is cloned, expanded by percent encoding, and cloned into flow state before the UI rejects an oversized QR payload. | Check borrowed UID bytes and the exact encoded confirmation-URL allowance before copying or constructing a challenge, without assuming a UUID format or revoking credentials. | The 10-case login target includes five new genuine loopback begin cases; exact byte boundaries and late-generation fences passed without a live login. |
| RL-41 | Per-line count/mark construction and thought/hit/selection paths repeatedly search the complete body for every remote underline. Caret construction can also rescan a growing zero-width cluster quadratically. | Preflight source fields and share one versioned, budgeted scalar projection across all consumers. Apply complete page admission, including worst-case caret work; skip no-op caret rescans and preserve nonzero kerning/spacing semantics, local drafts, and unknown ranges. | Reader target 24/24, underline target 21/21, and the 686-core combined suite passed remotely, including 24 new cases. Constructed work/counter evidence, no observed device freeze claim. |
| RL-42 | The supervisor records its power observer and App runner after fork. An early terminal signal, lost observer wrapper, or PID reuse can leave an original long-lived child unretired or target a different job. | Share finite first-signal registration guards, require subscriber pre-exec ownership/GO proof, retain original wrapper/subscriber identities, and perform bounded retirement of proven orphans without trusting mutable PID files. | Complete current launcher 158/158 passed remotely; earlier failing checkpoints and fixture evidence remain explicit below. |
| RL-43 | The locked HTTP pool retains idle connections by scheme/authority without a global origin or descriptor budget. Completed arbitrary-origin resource requests can outlive active Jobs credits in the idle pool. | Preserve finite first-party connection reuse while disabling idle retention for arbitrary-origin resource transports, including redirect destinations; verify actual TCP ownership and reuse. | Three genuine loopback keepalive cases, one destination-policy case, and the 735-core checkpoint passed remotely. Locked Reqwest/Hyper source confirms that the upstream timeout is not a global count limit. |
| RL-44 | The launcher acquires its lock before initializing cleanup state and installing terminal/EXIT traps. A catchable signal in this window leaves an empty lock behind. | Acquire only after complete cleanup registration, track owned lock state, and defer signals across successful mkdir/owner registration without deleting a foreign lock. | Seven new signal/ownership fixtures are included in the current 158/158 complete remote run; related to RL08's earlier record-write cleanup boundary. |
| RL-45 | Repeated occurrences of one cached asset read and hash the same payload again during a single integrity operation, publication, or Reader reflow. | Use operation-local identity/stamp verification reuse and prepaid unique read work; retain complete integrity evidence and refuse quota failure without partial publication or valid-content retirement. | Sixteen metered cache/asset cases and the 735-core checkpoint passed remotely, including unindexed physical destinations and shared aliases. No large read workload was executed. |
| RL-46 | Thought previews and quote layout are loaded for all page candidates before list pagination, even when the thought interface is hidden. Large persistent responses can be copied together for mostly invisible cards. | Page lightweight descriptors first, materialize only visible bounded previews/layout, and use limited SQL reads before string/JSON allocation while retaining local drafts and stored content. | Visible-page, hidden-view, bounded-cache, draft/selection cases passed in the 735-core checkpoint. Persistent content itself is not classified as a leak. |
| RL-47 | Same-digest image merging checks each long source name with a linear Vec search, amplifying a bounded archive into quadratic prefix-comparison work. | Maintain bounded operation-local asset/name membership and charge name work before copying or merging while preserving first-seen aliases and ambiguity rules. | Eight small ZIP/TAR/member-admission cases and the 735-core checkpoint passed remotely. No multi-gigabyte comparison was executed. |
| RL-48 | Catalog normalization reaches the UI before duplicate UID admission, where old cached bodies are loaded and repeatedly cloned; even valid large catalogs materialize all bodies for metadata refresh. | Validate identifiers before old-body access and preserve/rekey bodies within the catalog transaction using lightweight metadata rather than copying the corpus into Rust. | Borrowed input, new-row/owner-copy admission, SQL reindex/rollback, and genuine App cases passed in the 735-core checkpoint. Invalid input preserves old chapters, notes, tasks, and manifests. |
| RL-49 | Cached underlines are copied from SQL and deserialized before the existing record/field policy rejects them. | Gate encoded bytes and array count in SQL, then apply borrowed selected-field admission before replacing the Reader model. Retain the original cache and local state on rejection. | SQL/type/count/field and hidden-reload cases passed in the corrected 183-case target and complete 773-case library run. |
| RL-50 | A catalog admission failure returns before settling current-book Reader/download/image waiters, leaving producerless intents that can be retried after wake. | Share terminal catalog settlement for source, capacity, and Store errors; remove only that book's resident retry and preserve SQL/I/O causes. | Typed-cause, capacity/waiter, and wake cases passed in the corrected target and complete run; injection remains active through all original-state assertions. |
| RL-51 | Detail and download planning load whole-book bodies, and Jobs retain unrelated cached chapters before packet admission. | Plan from bounded bodyless metadata. Emit a credited cached identity checkpoint; validate actual task/UID/permission/nonblank SQL content and advance only its durable cursor before ACK. Read only the current visible chapter when needed. | Plan, actual SQL checkpoint/fence, background-no-body, guarded current-chapter reads, and genuine TCP calibration/no-additional-HTTP cases passed in the corrected target and complete run. |
| RL-52 | Pending cleanup debt can delete a late-appeared destination before validating the publication's original stamp, hiding the appearance and allowing publication. | Share an operation-local destination fence across scoped debt, retirement, pins, guards, recovery, and rollback. Advance expected identity only for the operation's own confirmed mutations. | All seven genuine late-debt/guard/stable-retry additions passed in the corrected target and complete run, including unwinding and stable-debt retry. |
| RL-53 | Supported custom fonts can give a zero-width combining sequence nonzero kerning. Local pointer selection and local-note marks construct the growing caret vector and reverse-scan equal positions without the remote overlay's work admission. | Admit generic caret input, count, and worst-case work before allocation or measurement; use fallible positions and shared operation-local geometry for local and remote consumers without guessing coordinates or discarding local drafts/selection. | Eleven pure/real-TTF/App cases passed within the 818-case checkpoint after correcting the known-origin fixture's phase accounting. No built-in-font or device failure is claimed. |
| RL-54 | Font collection faces each create an OwnedFace by copying the complete input blob. The 64 MiB single-file limit does not admit face count or the aggregate copies before registration and initialization. | Admit raw collection count, input/read work, variation axes, selected name spans, and metric-copy bytes before irreversible shared registration or copying. Prepare complete file choices before committing them, without refunding possibly retained registrations. | Fourteen genuine collection/read/metadata/stock-font cases passed among the 20 Main font targets after per-test-thread platform initialization. No large collection was executed. Intentional font storage itself is not called a leak. |
| RL-55 | Cold session resolution loads the entire book's bodies only to locate one unique chapter UID, then reads the target body again. | Resolve a unique UID/index using bounded scalar metadata and read only the target text for anchor/digest checks. | Scalar/Store and genuine cold-resume regressions passed within the 818-case checkpoint; non-target corpus materialization stayed zero. |
| RL-56 | Publication and Article summaries, hidden lists, and cache-presence checks repeatedly read stored article bodies before pagination. | Use SQL-only display presence and retain necessary visible-body loading; preserve Some(empty) versus NULL display behavior without treating it as download authorization. | Actual publication/article redraw and hidden-list producer regressions passed within the 818-case checkpoint; stored content itself is not a leak. |
| RL-57 | Entity decoding searches the entire remaining text for a semicolon before checking the accepted 12-byte entity length, causing quadratic tail scans for repeated ampersands. | Search only the first 13 borrowed bytes and preserve existing valid, numeric, unknown, and incomplete entity behavior. | API and storage found the same consumer and it was counted once. Three small scan/semantic regressions passed within the 818-case checkpoint; no large workload was executed. |
| RL-58 | Duplicate thin Shelf IDs repeatedly clone the same old Book introduction and metadata before replacing the previous result. | Borrow and merge duplicate IDs before one normalization per unique ID, preserving last-record and insertion/override behavior. | Four duplicate/override/copy regressions passed within the 818-case checkpoint. |
| RL-59 | MP output normalization copies the received account owner for every candidate without owner/count/aggregate admission; request count does not bound response count. | Preflight borrowed owner, selected fields, candidates, and retained metadata before mapping or copying, including the Shelf MP owner boundary. | Nine small admission/loopback regressions passed within the 818-case checkpoint. The source mechanism used a 16 KiB owner and 8,192 candidates; the earlier 1 MiB URL example was withdrawn. No live-service result is claimed. |
| RL-60 | Each cached image occurrence scans the complete prior manifest during both probe and materialization, repeating long-source comparisons and normalization. | Build one bounded operation-local prior-source index for both phases, preserving exact precedence and deferred ambiguity/error ordering. | Small source-index, alias, quota, and actual text-only commit regressions passed within the 818-case checkpoint. |
| RL-61 | Each removable ZIP extra field reconstructs nearly the complete original extra buffer, although only the final reconstructed value is returned. | Parse in the existing order, retain the last removal interval, and reconstruct once at completion while preserving immediate None and error behavior. | Root synthetic ZIP constructor/copy regressions passed within the 818-case checkpoint. This is not the upstream complete suite or a large-workload result. |
| RL-62 | Duplicate raw NAME directory entries repeatedly scan all name records before selected-span metadata fees are deducted. | Admit cumulative root/directory/table/record visits before scanning, independently of retained name-span costs, without refunding failed work. | Six small duplicate/alias/work regressions passed among the 20 Main font targets. No hundreds-of-millions-visit sample was executed. |
| RL-63 | NCX labels repeatedly traverse the same nested subtrees to find their first text element, without node-work admission when no title is retained. | Build one bounded first-text projection, prepay node visits and temporary metadata, and charge repeated label work before copying while preserving first-child/empty-first semantics and fallback error handling. | Nine small producer regressions passed within the 850-case checkpoint. Locked roxmltree permits the source-level nesting mechanism, but no large nesting workload was executed. |
| RL-64 | Each selected or pending catalog index searches old and new chapter lists again, amplifying long shared UID prefixes during reindexing. | Build borrowed UID/index mappings once and reuse integer lookups across selections, pending reads/downloads, resident Reader, and history. | Seven small mapping/fee/state regressions passed within the 850-case checkpoint. |
| RL-65 | Duplicate MP owners repeat complete cached article scans, unread queries, and latest-title copies before Publication pagination. | Preflight and deduplicate borrowed owner records, compute one summary per owner, and materialize only admitted visible rows; defend historical duplicate caches as well as future source snapshots. | Ten source and six genuine App regressions passed within the 850-case checkpoint; stored article history itself is not called a leak. |
| RL-66 | NCX parsing expands internal general entities before the project's node, label, and retained-output limits. Small literal definitions can derive large text or attributes inside roxmltree. | Preflight borrowed declarations and content/attribute summaries before DOM parsing, with aggregate expansion/copy, scanning, name, definition, and temporary metadata admission. | Nineteen small source/EPUB regressions were included in the first library target; complete revalidation is pending. No large entity expansion or physical-device sample was run. |
| RL-67 | Every image anchor can scan the same long zero-width suffix again during pagination. | Use one ordered scalar projection and memoized run ends, then preserve the original DOM order for equal positions. | Small anchor/Reader work regressions were included in the first target; complete revalidation is pending. No large repeated-run workload was executed. |
| RL-68 | Weighted book progress searches the full catalog for every chapter UID. | Build one admitted borrowed UID projection and accumulate weights without another retained vector, preserving first-match and zero-total behavior. | Small weighted-progress/actual Reader regressions were included in the first target; complete revalidation is pending. |
| RL-69 | Keyboard appends and candidate/suffix commits bypass the existing edited-field scalar limits for Search and thought drafts. | Carry an explicit input purpose and make every keyboard/candidate/mode/paste/suffix operation atomic under its scalar policy. Preserve composition and stored historical notes on rejection. | The first target included the initial primitive/App callbacks; the refrozen readonly-view correction adds two cases awaiting corrected execution. |
| RL-70 | Per-chapter permission labels repeatedly clone the complete session and may serialize/hash its cookie map again. | Resolve one operation-local account owner and share borrowed status/label functions across the directory and download predicates. | Six small semantic/genuine App regressions were included in the first target; complete revalidation is pending. No large account-copy workload was run. |
| RL-71 | A valid early Chrono timestamp makes Total statistics materialize hundreds of thousands of annual bars before the UI receives them. | Preflight an inclusive year-span and complete output forecast before bucket/vector/UI materialization. On rejection, retain raw data and summary with an unavailable chart. | Seven small source/SQL/software-UI regressions were included in the first target; complete revalidation is pending. The large annual range was not materialized. |
| RL-72 | The article QR action passes a response-provided original URL to URI parsing and QR segmentation before checking its byte allowance. The encoder collects segments before rejecting QR capacity. | Preflight a borrowed payload before article copying or URI parsing, and enforce the same 2 KiB policy at the shared QR helper. Preserve the original URL, local state, and Login behavior. | Locked qrcode 0.14.1 and actual article callers establish the source mechanism. Seven small shared/App/Login regressions are frozen but not executed; no large segmentation workload was run. |
| RL-73 | Reviews and a real single-range thought response map every received record into a RemoteReview before output count or metadata admission. Later SQL-cache and visible-card limits do not bound that producer. | Share borrowed candidate/selected-field and complete output forecasts before mapping, HTML conversion, collection, or extension. Preserve aliases, empty entries, and error/authentication behavior. | Actual response, queued-result, and cache/UI consumers were inspected. Small producer regressions are being prepared; no million-record workload or live-service result is claimed. |
| RL-74 | Ordinary Shelf and Search normalize book fields and retained raw trees before count or output admission. App Shelf deduplication occurs after the snapshot is already queued. | Admit ordinary book candidates, selected metadata, and the complete retained raw tree before normalization. Preserve Shelf last-field/first-order behavior, Search first-record behavior, numeric aliases, and unused MP raw-data handling. | Source-only inspection distinguishes retained Shelf/unique Search results from temporary duplicate Search copies. Corrections and small producer regressions are pending. |
| RL-75 | XML attribute/namespace duplicate detection, lookup, and propagation can repeatedly compare long prefixes before the existing node or NCX output-work limits apply. | Admit complete field, comparison, namespace, and parser-metadata forecasts before every container/package/encryption/NCX parse, including entity-generated markup. Preserve ordinary parsing and typed quota propagation. | Locked roxmltree source and all four actual importer entries were inspected. No large long-prefix XML was executed; corrections and small XML/EPUB regressions are pending. |
| RL-76 | Downloaded-source fallback lookup normalizes the whole prepared source map again in destination, probe, and materialization phases. The existing prior-manifest index does not cover downloaded sources. | Build one prepaid operation-local source/request projection, preserving exact precedence and deferred ambiguity/materialization errors. Use the actual 5,120 association and 20 MiB source-name upper bounds. | Actual source/acquire/prepare/publication paths establish this separate consumer. Small archive/lookup/rollback regressions are being prepared; no large failing lookup workload was run. |
| RL-77 | A zero-length extended ZIP timestamp subtracts its flags byte before validating length. In the actual overflow-checking development/test build this can panic before download cleanup and terminal delivery. | Reject zero length before reading flags or subtracting, retaining the existing UnsupportedArchive result and normal timestamp behavior. | The existing 36912 compiler-artifact record confirms overflow checks for ZIP. This is a conditional source finding for that build; the current release already returns an error, and no release or device failure is claimed. Small constructor/worker regressions are pending. |
| RL-78 | Local annotations load the whole book's quote/note text before current-page, range, ID, count, or statistics consumers decide what they need. | Query scalar/header working sets first, page before bounded field loading, and stream timestamp/count data for statistics. Preserve historical rows and local drafts without adding a total-history cap. | Actual Store and UI/statistics callers were inspected. Small SQL/page/field/context regressions are being prepared; no large history workload or device result is claimed. |
| RL-79 | Nested foreign SVG anchors remain valid nav candidates. Descendant selector matching and per-anchor label walks repeatedly visit the same suffix even when labels remain empty. R26 found the same ancestor-rescan mechanism in Source and BookAssets `svg image` guards on no-SVG MathML. | Traverse the parent forest once and prepay candidate node/text/path work including skipped child inspections. The omitted guards share a separately checked, call-local SVG ancestry summary and preserve original local-name/namespace and unresolved-count semantics. | Nav source and small regressions retain their prior target scope. R26 independently confirmed the two guard consumers and real Full/include-images producer; eight Source/Jobs and two BookAssets regressions are refrozen but unexecuted. The large MathML example is source arithmetic, not an executed workload. |
| RL-80 | SQL byte measurements using length(CAST(column AS BLOB)) load long overflow TEXT into the VM before the caller's byte comparison. Rust-side admission does not prevent that earlier read. | Use direct octet_length column metadata and checked encoding-aware bounds before selected field loads. Preserve NULL/type/Unicode semantics and validate borrowed UTF8 before ownership. | Locked SQLite 3.50.2 source confirms the VM branch; actual runtime/encoding and small overflow-row regressions remain pending. Metadata-only measurement is not a zero-I/O or zero-allocation guarantee. |
| RL-81 | A single enqueue or resumed-prefix operation invokes standalone image completeness for each chapter, recreating verification scopes and rehashing shared cached payloads. | Preflight one aligned bulk working set and unique payload work, reuse identity/stamp evidence within that operation, and preserve queued tasks when quota or I/O fails. | Actual Library and BookAssets callers were inspected. Bulk/file/queue regressions are being prepared; ordinary integrity proof and cached content themselves are not classified as leaks. |
| RL-82 | Late thought results use the mutable current view ID to select a cache key. A remote-to-local/offline view change can persist an unrelated batch under a key with no consumer. | Bind request, book/UID, view, and cache identity; fence view changes and reject late success/error without changing the new view. Preserve independent same-account Cookie/session CAS behavior. | Real producer, key readers, and handler paths were inspected. Small late-event/context regressions are pending; historical cache entries are not deleted by guesswork. Fixed Reviews order is a same-family compatibility case. |
| RL-83 | New reading positions retain the complete chapter title as summary, which each pending report batch copies even though the wire uses only the first 20 Unicode scalars. | Bound the unused summary tail when creating new report positions before repeated copies. Retain SQL titles and old requests/proofs; do not retroactively rewrite evidence or replay POSTs. | The current producer, batch allocator, and wire serializer were inspected. Small new-position/batch/Unicode regressions are pending; necessary unresolved delivery evidence is preserved. |
| RL-84 | Nonempty Publications filtering formats and lowercases entire cached titles/authors before visible-row admission, including excluded articles. Prior request normalization does not pay this render's work. | Prepay the actual render-local matcher input, Unicode expansion, cumulative transform work, and maximum temporary capacity before each transform. Preserve complete joined-string matching and overview/empty-query short circuits; retain raw caches and existing models on typed rejection. | Two actual Search/focus/inline consumers and persistent loading were independently reviewed. Eight small App/cache regressions are refrozen, not executed. Temporary peak belongs to the maximum examined record; cumulative work belongs to all examined text, not simultaneous batch residency or infinite RSS. |
| RL-85 | Local Total aggregation advances through every clipped session day before the annual-output gate; one valid SQL interval with one active second can still require a very long loop. | Preflight the complete main/calendar clipped-day work and temporary buckets before aggregation, retain raw earliest-date, first-ID, and integer apportionment semantics, and use an explicit unavailable result on refusal. Stored sessions and previous valid projections remain intact. | The current Store read/write boundary and report producer were reviewed; no extreme calendar workload or device case was executed. Checked core/UI bridges and 12 pure/Store plus four App regressions are refrozen, not executed. |
| RL-86 | API error handling clones a complete string or serializes an entire JSON error-code subtree before keeping only 64 safe ASCII characters. | Keep original raw success/alias checks and error classification, then borrow strings or use a bounded sanitizing sink for composite JSON before retaining diagnostic output. | Actual gateway, web, shard, and report-acknowledgement callers were freshly inspected. Small producer regressions are being prepared; eliminating full temporary residency does not imply constant 64-character scan work. |
| RL-87 | Resident selection preview can copy a large body range despite an unavailable stored quote, and local phrase initialization constructs a full-body scalar vector before applying its current-line limits. | Bound the actual derived span and process a proven local line/window before allocating, retaining global scalar anchors and fenced existing-note updates without copying the old quote. | Two actual pointer/note consumers were freshly inspected. Source changes and small App regressions are pending; stored bodies and old quotes remain intact. |
| RL-88 | Library, statistics, and local-search metadata consumers read all Book intros before filtering/paging, retaining unrelated corpus text in their transient worksets. | Use distinct non-persistable metadata projections without intro, admitted before selected ownership; preserve full Book reads for Detail and necessary merge/update operations. | Actual import/read/display consumers were inspected. Small EPUB/Store/App regressions are being prepared; persistent descriptions and book corpus are not deleted or reclassified as leaks. |
| RL-89 | HTML active-formatting reconstruction shares one long style span across multiple elements, but Anchor and Article collectors repeatedly allocate and lowercase it outside existing text/node budgets. | Admit actual style work and memo capacity, reuse borrowed immutable span results, and match the original normalized UTF8 stream without a full temporary String. Actual article loads use a typed checked document before retiring the old view. | Locked HTML primary and both collectors were independently inspected. Ten core/Source/Jobs and three exact Jobs-classification regressions are refrozen but unexecuted; no large reconstructed-style workload or device case was run. |
| RL-90 | Native paste, key, and IME paths retain full rejected edit snapshots in Undo before the App's text gate restores its valid value. Repeated non-adjacent rejected inserts retain additional payloads. | Add a narrow source admission hook to the already locked native TextInput implementation, checking the same borrowed insertion/composition snapshot before mutation while preserving accepted Undo/Redo, selection, and focus. | The exact published Slint 1.17.1 archives match Cargo.lock. Separate native-input and Window-lifetime primary reviews identified all early mutation sites; the narrow patch and genuine native regressions are still being prepared. No dependency upgrade or project-license choice is inferred. |
| RL-91 | Each local note can create a mark for every intersecting page line. SQL geometry and per-line caret limits do not admit that fragment product, and the existing remote-only mark gate omits it. | Preflight complete local/remote/selection intersections, computation, and logical output bytes against the shared 1,024-mark limit; render and hit testing use the same all-page admission. Preserve zero-width object and scalar-anchor behavior. | Six small genuine SQL/App regressions are refrozen but unexecuted. The 384-note/ten-line example is static reachable arithmetic, not a measured device workload; local solid rectangles do not receive the remote dash multiplier. |
| RL-92 | Recovery can expand a compact eligible counter into all remaining 60-second report batches at once. Each append scans prior batches, causing at least quadratic checking without aggregate admission. | Describe remaining demand numerically and durably allocate only an admitted consecutive prefix. Keep counters/deferred evidence and atomically persist the outbox with its allocated cursor; capacity refusal must allow old Pending delivery to continue. | The current fault/finish/recovery producers were freshly inspected. Small partial-commit/recovery regressions are being prepared. General sorting bounds do not imply that the ordered example necessarily reaches quadratic-logarithmic time. Old summaries, uncertain delivery, and proofs are preserved. |
| RL-93 | Article image URLs, occurrences, and retained references can be copied or transformed before the consumer's current operation admits them. | Check borrowed URLs and the complete selected reference demand before parsing, copying, or publication; preserve request identity and the existing image receiver. | Actual Source, Article, and App consumers established the shared family. Small URL/list/publication regressions retain their original output and error assertions. |
| RL-94 | Reachable glyf/CFF/variation outlines can expand repeated references and program work before font registration admits that demand. | Admit the complete reachable u16 glyph domain, point/scratch demand, and cumulative work before expansion or registration. Price borrowed INDEX spans and actual DICT prefixes without token-scanning unused data; share the locked BCD boundary semantics. | Tiny real Swash/Skrifa oracles and refusal-before-publication cases retain exact fees and limits. Normal seven-stock execution is separately recorded above; later IVS/HVAR/MVAR extensions remain deferred. |
| RL-95 | Physical outline masks can resize before dimensions, area, and output bytes are admitted. | Inspect the same mask configuration, check layout and bytes, prepay storage, then reserve/resize/render. Failed rendering preserves the caller image and metrics. | The actual software renderer uses Outline/Alpha. Same-version Swash package cases compare small Noto output and reject before allocation; unused bitmap/PNG paths are outside this claim. |
| RL-96 | Store, cache, and completeness inventories can materialize keys and records before admitting the selected working set. | Query scalar types, encoded lengths, rows, and current identity in one snapshot; price the complete selected operation before owning fields or mutating tasks. | Small real SQLite/cache/App regressions preserve historical rows, cancellation, cursor, and rollback behavior. No total-history deletion policy was added. |
| RL-97 | Derived Reader/Article layout collections can request new and overlapping old storage without a shared operation budget. | Prepay live requested metadata and allocation work, including growth and old/new overlap; stage the replacement and retain the old view on refusal. | Actual paginator/layout/model producers and small failure cases were inspected. Later Article loaded-map/anchor extensions remain follow-up work. |
| RL-98 | ZIP extra records retained across directories and candidate attempts can grow enum/vector storage before constructor admission. | Share one sticky constructor retention budget; charge records, requested capacity, and old-buffer overlap before push/reserve while retaining read/attempt limits and typed terminal refusal. | Real dual-directory Source/worker fixtures preserve format-error precedence, PNG publication, account/data state, and credits. Further footer storage hardening remains isolated and deferred. |
| RL-99 | TXT-to-HTML escaping can build its full expanded output before the downstream HTML limit is checked. | Forecast the complete escaped output and logical transformation work before the actual writer; preserve the existing text and empty-response semantics. | Small actual Source TXT/HTTP fixtures keep permission, fallback, encoding, and typed quota assertions. |
| RL-100 | Selected ReaderSession duplicates, fallback fields, and request-derived values can decode or copy earlier oversized data before checking the final selected value. | Admit every relevant occurrence before whole decoding, preserve last-selection semantics, preprice actual loose fallback when strict clock is unusable, and check derived request bytes before copying/hashing/URI construction. | Genuine duplicate, malformed, strict-unknown-field, and fallback HTTP cases retain numeric aliases and compatibility. The fixed v3 closes the earlier limited-source tails without claiming large-input or device execution. |
| RL-101 | Zeno raster loops and heap growth can perform cell/row/curve work or request overlapping buffers before mask-size admission applies. | Use the opt-in early-aborting fill path with cumulative cells, requested storage/growth peaks, and logical access/relocation work. Only successful staging replaces all caller image fields. | Same-version Zeno/Swash source and package tests retain small original mask oracles, exact/one-less limits, sticky fees, and old image/coordinate/metric assertions. These are logical request/work bounds, not allocator RSS or a kernel deadline. |

## Round ledger

| Round | New recommendations | Consecutive clean rounds | Evidence |
| --- | --- | --- | --- |
| 1 | 10 grouped findings | 0 | Independent UI, jobs, cache, native worker, and launcher source reviews established the first correction set. |
| 2 | 5 additional boundary findings | 0 | Reviewers exchanged domains and re-examined the integrated candidate for additional boundaries. |
| 3 | 2 integration findings (RL-13 extension and RL-16) | 0 | A second exchange found cleanup error propagation and deferred-cover integration gaps. |
| 4 | 1 adjacent input-bound recommendation (RL-17) | 0 | Resource lifecycle reviewers reported no new leak findings; the accepted input-bound correction still resets the strict stopping count. |
| 5 | 2 protocol/import capacity findings (RL-18 and RL-19), plus log-fixture correction | 0 | Retained operational state and derived import allocations received an additional input-boundary review. |
| 6 | 1 parser-initialization finding (RL-20) | 0 | The exact ZIP 4.6.1 source allocates `Vec::with_capacity` before returning an archive to the existing post-initialization check. |
| 7 | 3 interface/work/stack findings (RL-21 through RL-23) | 0 | A uniform command deadline, actual remote acquisition work, and DOM traversal stack usage received additional boundary review. |
| 8 | 3 asset/intent/probe findings (RL-24 through RL-26), plus a fixture module-name correction | 0 | Failed pin rollback, offline intent retention, and probe cancellation required additional lifecycle handling. |
| 9 | 3 quiescence/read-debt/parser-work findings (RL-27 through RL-29) | 0 | Cross-module review checked what remains owned after worker return, tolerated reads, and rejected ZIP candidates. |
| 10 | 1 atomic descriptor-inheritance finding (RL-30) | 0 | The discovery handle now matches the final touch handle; the wakeup pipe uses atomic creation flags. |
| 11 | 1 article batch-work/continuation finding (RL-31) | 0 | Article load-all must reuse its parsed list and continue admitted batches without repeating whole-document parsing. |
| 12 | 3 queued-payload/identity/compressed-work findings (RL-32 through RL-34) | 0 | Continuation credits, typed image destinations, and compressed input work required additional boundaries. |
| 13 | 2 terminal/supervisor cleanup findings (RL-35 and RL-36) | 0 | Persistent storage errors and foreground queries must not bypass the existing producer/child cleanup paths. |
| 14 | 2 signal/queued-result findings (RL-37 and RL-38) | 0 | Exchanged reviews found an unrecorded native stop window and permits that omit queued payloads; independent image and Jobs reviewers identified the same queued-result finding. |
| 15 | 2 anchor-graph/login-challenge capacity findings (RL-39 and RL-40) | 0 | UI/App and native/launcher/diagnostics reviews reported zero additional recommendations; strict anchor and borrowed-UID admission were integrated afterward. |
| 16 | 0 | 1 | Four fresh exchanged domain reviews covered the integrated RL39/RL40 source; the complete 662-core/3-binary candidate passed remotely. |
| 17 | 2 underline-work/observer-ownership findings (RL-41 and RL-42) | 0 | A further domain exchange found repeated body scanning in the UI and uncaptured power-observer identity/registration windows. |
| 18 | 6 transport/lock/integrity/preview/member/catalog findings (RL-43 through RL-48) | 0 | Fresh exchanged reviews inspected the actual locked dependency pool and current consumers; all provisional clean conclusions were discarded. |
| 19 | 4 cached-input/intent/body-producer/debt-ordering findings (RL-49 through RL-52) | 0 | Fresh full-domain review found cached SQL admission, unowned waiting state, whole-book body producers, and deletion before destination validation. The native domain reported zero additional findings. |
| 20 | 2 general-caret/font-copy findings (RL-53 and RL-54) | 0 | Fresh exchanged reviews reported UI/App 1 and API, storage/parser, native/diagnostics 0; the subsequent integration check of real custom-font registration confirmed the independent collection-copy finding with locked dependency source. The prior 773-case checkpoint includes neither correction. |
| 21 | 8 body-consumer/entity/Shelf/MP/asset/ZIP/name-work findings (RL-55 through RL-62) | 0 | Fresh full-domain results were UI 2, API 3, storage/parser 3, native 1. Entity decoding was independently found in API and storage and counted once; all provisional clean conclusions were discarded. |
| 22 | 3 NCX/catalog-map/MP-projection findings (RL-63 through RL-65) | 0 | Fresh independent full-domain results were UI 1, API 0, storage/parser 1, native 1. Actual parser source closed the NCX nesting question; all provisional clean conclusions were discarded. |
| 23 | 6 entity/anchor/progress/input/session/chart findings (RL-66 through RL-71) | 0 | Fresh UI 3, API 0, storage 1, and corrected native 0 findings plus two independently confirmed integration mechanisms. The historical settings observer has no current producer and was withdrawn. All provisional clean conclusions were discarded. |
| 24 | 6 QR/review/book/XML/source-index/checked-overflow findings (RL-72 through RL-77) | 0 | Complete fresh results were UI 1, API 2, storage 2 plus one actual build-condition finding, and native 0. Shared producer families were counted once; the ZIP condition is not a release/device failure. The existing RL69 readonly-view regression was corrected separately. |
| 25 | 6 notes/nav/VM-byte/bulk/context/summary findings (RL-78 through RL-83) | 0 | Complete fresh UI 4, API 0, storage 1, and native 0 findings plus one independently confirmed SQLite byte-measurement mechanism. Two bounded source slices completed the UI coverage; all source work used the immutable 25908 reference. |
| 26 | 3 recommendations: RL-84/RL-85 and an RL-79 Source/BookAssets guard extension | 0 | Complete fresh UI 2, API 0, storage 1, and native 0 results on immutable post-format e735/262 leaves. Storage used an independent SourceImages/ZIP/Article slice; the two guard consumers count once. The candidate target failed, so no complete pass or clean count is inferred. |
| 27 | 8 recommendations: RL-86/RL-87/RL-88 plus RL-18/RL-48/RL-50/RL-83/RL-26 caller extensions | 0 | Complete fresh reviews used immutable actual post-format 09122b/268 leaves. API's seven caller points were grouped by cause with UI's Book-metadata mechanism and the native probe-registration omission; selection preview and phrase consumers count once. Storage reported no additional recommendation. Current 1,093/Main23 pass precedes these source changes. |
| 28 | 10 recommendations: RL-89/RL-90/RL-91/RL-92 plus RL-18/RL-26/RL-51/RL-65/RL-79/RL-88 consumer extensions | 0 | All four fresh domains completed on immutable corrected2 eef2/277 leaves. Cross-domain Book and chapter consumers were grouped by cause; Article and remote Stats output omissions share the existing auxiliary projection family. The cancelled system-settings caller stays historical. Rust 1,137/Main23 passed before these tails, but its parent log matcher exited 1 and the changed 165-case launcher was not executed. |
| 29 | 10 recommendations: RL-93/RL-94/RL-95 plus seven existing-family extensions | 0 | Four formal domains completed on immutable82fc/390. Ordinary glyph expansion and physical mask admission joined the existing data/ownership families; later source corrections and package tests retain independent identities. |
| 30 | 13 grouped recommendations: RL-96 through RL-101 plus seven omitted consumers | 0 | Four formal domains completed on immutable3771/483. Inventory, layout, ZIP retention, TXT escaping, ReaderSession projection, and fill work were integrated without counting duplicate consumers separately. Failed validation remains historical. |
| 31 | 11 grouped source recommendations; user-directed finite closeout | 0 | Four formal domains completed on immutable22436943/524. The user stopped further full-domain rounds. Necessary normal-font and ordinary-flow fixes were completed; further isolated HTML/ZIP-footer/IVS hardening remains deferred. The later active-orientation feature has distinct source and execution identity. |
## RL72 through RL77 compilation and first target

The first source-formatted candidate stopped at a single missing optional-key
type annotation (`compile-round24-source-1465-failed.log`); no target, Main,
complete, or sampling run occurred. A one-line borrowed tuple annotation fixed
compilation without changing limits, loops, or assertions.

The corrected candidate compiled in 110.799 seconds. Its library target passed
413 cases, failed two, ignored two manual benchmarks, and filtered out 561 in
116.80 seconds (`target-round24-source-25908-failed.log`). Main font targets,
complete suites, and sampling were not executed.

The XML failure compared an outer anyhow context instead of the typed inner
limit cause. Its test-only correction keeps the precise cause and strengthens
the cumulative through-stage, one-unit-over, and before/after DOM assertions.
The ZIP worker failure expected a direct timestamp-field error from the public
archive constructor, which uses its existing EOCD fallback. Its test-only
correction adds a valid ZIP/timestamp control differing from the malformed
case by one length byte, checks both exact interface errors, and retains all
real archive GET, terminal delivery, credit reuse, and original-state checks.
Both test-only corrections were included in the later RL78-RL83 candidate.
That combined target failed three UI regressions; it is not a complete pass
for either group.

## RL78 through RL83 compilation and first target

The combined source contains 84 prepared R25 library regressions: 13 nav,
40 Store, 21 App, and 10 bulk-verification cases. The first two compilations
failed before any target, Main, complete suite, or sampling: session 87707
reported a bounded-preview borrow-order error in 24.450 seconds, and session
78806 reported a missing test-module import in 74.202 seconds. Their narrow
corrections preserve admission limits and assertions.

The corrected2 session 47210 compiled in 127.996 seconds and naturally exited
with status 1 after its library target: 496 passed, three failed, two ignored,
561 filtered, in 125.85 seconds. Main font, complete library/Main suites, and
resource sampling were not executed. Its 262 actual post-format source leaves
have canonical SHA-256
`e43316fe8e232cc7ae59086a6927de8a9268ffab60bcde92483d68b3dbd443aa`.
The earlier transfer archive SHA-256
`423371b6429873a2e929199bfa2e05c2c07f41172d1696c98d9ebcce2f326dc6`
identifies the pre-format stage, not the compiled-source identity.

The whole-page caret rejection failed because hit admission queried only the
clicked line's local notes while render admission used the complete page.
The trial-access scenario failed because closing the TOC also skipped the
lightweight current/access chapter model. These are RL78 consumer tails, not
new review rounds. The persistent-mark scenario compares canonical stored IDs
with row-based internal drawing tokens. Source review confirmed that actual
mark rectangles do not consume their drawing IDs; pointer/open/save consumers
retain canonical persistent IDs. Its test helper must map the original durable
ID to one admitted header and exact current row token, without restoring
full-page owned IDs or accepting missing marks. Pixel, source-anchor, reflow,
reopen, trial-access, draft, and
selection assertions remain required. A later frozen candidate needs fresh
verification and two complete clean reviews.

The two-file correction is formally refrozen: hit admission uses complete
page-local geometry, and closed TOCs publish a bounded metadata page containing
the current chapter. Body presence affects access labels without changing
owner/status authorization. The test-only helper checks the original durable
annotation against one bounded header before selecting its exact row token.
No case or quota was removed; complete verification remains pending.

Corrected3 session 70952 compiled in 103.484 seconds, then naturally exited 1
after its library target: 498 passed, one failed, two ignored, 561 filtered,
in 122.06 seconds. All three preceding UI regressions explicitly passed. The
remaining `prepaid_cache_snapshot_rechecks_current_identifiers_and_stamps_on_every_lookup`
case failed its `current_path(...).is_err()` assertion at line 649; the actual
index/stamp fence and the mutation precondition are being distinguished before
any correction. Main font, complete library/Main, and sampling were not run.
Its 262 actual post-format inputs have canonical SHA-256
`e735035139499f2d1757fc748de05721eda44aa4730aad1c93f724985953da54`.
The separate pre-format transfer archive SHA-256 is
`6df536c95f4df954a54553e963b846f3dd7052ea53aa101ae06e11862454bc7a`.
The complete target log and status are preserved under
`artifacts/resource-review/target-round25-source-70952-failed.log` and
`verification-round25-source-70952-failed-status.json`. R26's four fresh
exchanged domains read this immutable post-format candidate provisionally;
there is no complete pass or clean-round count to infer from these results.

## RL66 through RL71 compilation and first target

The frozen 53-case correction set compiled on `test-env` in 115.386 seconds.
Its library target passed 342 cases, failed one, ignored two manual benchmarks,
and filtered out 561 in 115.35 seconds
(`target-round23-source-36912-failed.log`). Main font targets, complete suites,
and sampling were not executed after this failure.

The existing `opened_thought_reviews_page_before_copying_and_keep_oversized_local_notes`
case still showed an earlier remote-review row instead of the original
unavailable local-note view. Source inspection confirmed that RL69's editor
admission returned before changing the readonly view. The narrow correction
must retain historical viewing, the original unavailable-content assertion,
draft/composition/selection, and stored data while preventing an unadmitted
note from being overwritten by another draft. This is a production integration
regression, not a relaxed fixture expectation or a passing full result. The
three-file correction is now refrozen with two additional genuine callback
cases. It restores the bounded readonly view and keeps an independent editor
gate through stale callbacks and view closure; new selections, admitted notes,
documents, and accounts reset that context. Its corrected validation will run
with RL72-RL77 rather than repeating an intermediate complete suite.

## RL63 through RL65 verified checkpoint

The frozen, source-formatted RL63-RL65 candidate compiled on `test-env` in
1 minute 44 seconds. Its library target passed 279 cases with zero failures,
two ignored manual benchmarks, and 572 filtered out in 116.12 seconds. Its
Main font target passed 20 cases in 0.94 seconds. The same compiled programs
then passed 850 complete library cases with three ignored manual benchmarks
in 161.93 seconds and all 23 Main cases in 1.01 seconds
(`core-round22-final.log`). These counts are subsets of the complete suites.

The same complete run supplied three navigation windows with 16 descriptors,
four threads, and RSS 101,896 KiB, and three App-drop windows with seven
descriptors, three threads, and RSS 102,632 KiB (`runtime-round22-final.log`).
It covered 80 navigation cycles, 320 software frames, and 12 App/window drops.
These controlled Linux/fake-device windows are unchanged for this workload;
they are not a total RSS limit or a new physical-device result. The checkpoint
precedes every RL66-RL71 correction.

## RL55 through RL62 verified checkpoint

The corrected frozen Kindle-feature candidate passed 818 library cases with
zero failures and three ignored manual benchmarks in 157.90 seconds. All 23
binary-entry cases passed in 1.04 seconds (`core-round21-corrected-final.log`),
including the 20 font cases that passed their target in 0.95 seconds.

The library target previously passed 247 cases with zero failures, two manual
benchmarks ignored, and 572 filtered out in 112.64 seconds. After a Main-test-only
platform fixture correction, all 232 unchanged post-format source leaves and
the exact compiled library executable SHA-256 matched. That target was reused
without another targeted execution; the complete library and binary runs above
were freshly executed. Target counts are subsets, not additions to the totals.

The same complete run painted 320 software frames across 80 navigation cycles
and repeated App/window creation and destruction 12 times. Navigation windows
had 16 descriptors and four threads, with RSS 102,044, 102,108, and 102,108 KiB.
App-drop windows had seven descriptors and three threads, with RSS 102,652,
102,716, and 102,716 KiB (`runtime-round21-corrected-final.log`). Later warm
windows were equal for this controlled Linux/fake-device workload; this does
not establish universal leak freedom or a total RSS cap.

The first Main font target passed four cases and failed 16 because its libtest
threads lacked a Slint platform context (`target-main-font-round21-first-failed.log`).
The fixture now initializes a real MinimalSoftwareWindow platform once per
test thread; shared registration and all quota/copy/default-choice assertions
remain unchanged. Production Main already initializes its backend/window.
The failing run stopped before the complete run and sampling; its failure is
not relabeled as a production fix or a passing result.

## RL53/RL54 compilation and first target

The frozen 25-case correction set passed compilation on `test-env`. Its library
target passed 113 cases and failed one, with 673 filtered out, in 10.98 seconds
(`target-round20-caret-first-failed.log`). The driver stopped at this failure;
Main font targets, a complete library/binary run, and same-run sampling were not
executed. The earlier 773-case result remains the RL49 through RL52 checkpoint.

The failing known-origin drag fixture expected one admitted geometry after
hold. The actual production path computes the origin anchor and then its
preview focus separately; a legal selection render can also build the small
origin geometry. The next candidate will calibrate these real stages while
retaining exact rejection, no-large-target allocation/measurement/history,
known selection, draft, body, and account assertions. Production behavior is
not claimed fixed by a changed counter expectation. This fixture correction is
integrated with RL55 through RL62 before fresh verification.

## RL49 through RL52 verified checkpoint

The frozen formatted candidate passed 773 library cases with zero failures and
three explicitly ignored manual benchmarks in 157.29 seconds. Binary-entry
cases passed 3/3 in 0.07 seconds (`core-round19-final.log`). Its expanded target
passed 183 cases with zero failures and the same three manual benchmarks
ignored in 5.39 seconds (`target-round19-final.log`). One cargo compilation
produced the exact library and binary test executables used for these runs;
targeted counts are subsets of the complete library result.

The same complete run painted 320 software frames over 80 navigation cycles
and repeated App/window creation and destruction 12 times. Navigation windows
had 16 descriptors and four threads, with RSS 102,432, 102,496, and 102,496 KiB.
App-drop windows had seven descriptors and three threads, with RSS 103,040,
103,168, and 103,172 KiB (`runtime-round19-final.log`). Descriptor/thread counts
remained stable; the later drop windows differed by 4 KiB. This is controlled
Linux software/fake-device sampling, not a total RSS cap, a long-running live
service experiment, or proof of the absence of every possible leak.

The combined candidate passed compilation after two mechanical fixture fixes
(an atomic import and one old untyped download call). Its first targeted run
then passed 180 cases, failed three, and ignored three manual benchmarks in
5.46 seconds (`target-round19-admission-failed.log`). The driver stopped there;
no complete suite or resource sampling was executed for that failing candidate.

The failures concerned a visible cached-checkpoint read counter, a Store-capacity
rejection fixture, and a TCP calibration request constructed outside a Tokio
context. Fixture-only corrections retain actual cached-content proof,
permission fences, durable acknowledgements, and original-data assertions.
The corrected target passed 183 cases with zero failures and three manual
benchmarks ignored in 5.39 seconds (`target-round19-final.log`). Its same compiled
library/binary candidate then passed the complete run above. No clean-round
result is inferred from tests alone.

The visible fixture now measures loader/reflow/session paths separately and
requires exactly three current-chapter reads. A noncurrent invalid-UTF-8 BLOB
causes any attempted body materialization to fail while remaining valid
bodyless planning metadata; the whole-corpus counter must not increase. The
catalog fixture keeps the oversized owner and all old-state/waiter assertions
active through rejection, then restores only its original injected owner before
the real terminal saver. The TCP fixture enters Tokio before constructing the
request and retains its real CONNECT calibration, three credited checkpoints,
and zero subsequent HTTP requests. Production code was unchanged by these
corrections.

## RL43 through RL48 Rust checkpoint

The corrected frozen candidate passed 735 library cases with zero failures and
three explicitly ignored manual benchmarks in 156.36 seconds. Binary-entry
cases passed 3/3 in 0.07 seconds (`core-round18-final.log`). Its expanded target
passed 122 cases with one manual benchmark ignored in 1.95 seconds; targets are
subsets of the complete library result.

That same complete run painted 320 software frames over 80 navigation cycles
and repeated App/window creation and destruction 12 times. Navigation samples
were 16 descriptors, four threads, and 101,440 KiB RSS in all three windows.
App-drop samples were seven descriptors, three threads, and 101,856, 101,920,
and 101,920 KiB RSS (`runtime-round18-final.log`). These controlled Linux samples
precede RL49 through RL52 and do not prove a universal absence of leaks.

The first combined compilation failed a missing `SystemTime` import. A later
complete run passed 734 cases and failed the shared-asset repair/rollback case:
new read-only candidate collection returned too early for a missing prepared
source, preventing the original earlier shared-PNG repair. The correction
preflights only actual put candidates and leaves logical preparation errors in
the original materialization order. The old complete/pin/new-asset cleanup
assertions were retained; the expanded target and corrected complete run above
passed.

## RL41/RL42 verification checkpoint

The frozen, formatted Kindle-feature Rust source passed 686 library tests with
zero failures and three explicitly ignored manual benchmarks in 152.94 seconds.
Binary-entry tests passed 3/3 in 0.07 seconds (`core-underlines-final.log`). Reader
tests passed 24/24; the underline target passed 21/21, including eight projection
cases, six isolated App scenarios, and seven source-input cases
(`reader-underline-final.log`, `underline-target-final.log`). The three added
Reader cases make 24 new RL41 cases in total; targeted counts are subsets of the
combined suite. App scenarios require a private completion marker so an
incorrect exact-test name cannot turn zero executed cases into a passing result.

The caret correction skips zero-adjustment reverse scans. Nonzero kerning and
spacing retain their original numeric and epsilon behavior, which can still be
quadratic; the page budget therefore prepays the worst-case work before either
endpoint is constructed. Genuine small combining-line and nonzero-kerning cases
measure actual visits and exact/one-less admission without executing a
multi-gigabyte or billion-step workload. An already-open remote thought uses the
same page admission as marks, lists, hits, and selection; rejection preserves
local drafts and active local writing.

The same complete run painted 320 software frames across 80 offline navigation
cycles and created/destroyed the App and window 12 times. Navigation samples
were identical: 16 descriptors, four threads, and 100,488 KiB RSS. App-drop
samples had seven descriptors and three threads, with RSS 100,840, 100,840, and
100,844 KiB (`runtime-underlines-final.log`). Descriptor/thread counts stayed
stable and the warm RSS windows varied by 4 KiB. This is controlled Linux
software/fake-device evidence, not a total-memory or universal leak-free claim.

The observer correction added 16 fixture methods to the prior 135. Its first
new target passed 16/16 in 19.870 seconds. The first complete 151-case run failed
one old window-manager identity fixture because its `process_birth` shim was
defined after the new startup consumers; the required mutation marker exposed
the missing precondition (`launcher-observer-final.log`, 494.381 seconds).
Moving the unchanged shim to its definition made the five STOP-window targets
pass in 11.584 seconds. The next full run exposed two submode failures in one
observer-record fixture: a reader-entry hook could miss publication, and its
execution marker was shared across submodes (`launcher-observer-corrected.log`,
505.923 seconds).

The fixture now validates and saves the actual wrapper/child identity, mutates
the pending ownership record, and only then publishes it atomically. Each mode
has separate markers. Production code and identity, no-signal, process/lease,
FIFO/descriptor, and native-state assertions remain unchanged. The combined
16 new observer plus five STOP-window targets passed 21/21 in 30.927 seconds
(`observer-published-target.log`). Neither earlier failing full run is final
passing evidence. A third
complete 151-case run also failed one owned-record rename submode because the
strict group-liveness assertion found a remaining event child
(`launcher-observer-published-final.log`, 492.190 seconds). The test's infinite
shell/usleep replacement has since been replaced by a same-PID, nonforking
unresponsive helper, with all original identity/liveness assertions retained and
bounded diagnostics added. Its outcome must be established by the new full run,
which also includes the seven RL44 lock cases.

That first 158-case run failed an existing managed-wake scenario before its
initial application-ownership precondition was reached
(`launcher-lock-final.log`, 505.167 seconds). Static inspection did not establish
the historical failure's cause. A fixture-only bounded diagnostic records safe
process/marker state if it recurs; its eight-second precondition and all cleanup
assertions remain unchanged. The related five-case target passed in 21.627
seconds and the fresh complete run then passed 158/158 in 502.126 seconds
(`managed-lock-diagnostic-target.log`, `launcher-lock-diagnostic-final.log`).
This diagnostic target/full run did not reproduce the initial timeout; it does
not prove a cause or a production fix for that historical observation.

The current tested launcher SHA-256 is
`fa79a316964d0f984bd149457665f758b080e5a36d0998122e4be2a504a17526`;
its fixture SHA-256 is
`69bcef5f6eba21196878ce0c9c33c4c712939b7176a1434d3975f89f2e4e992b`.
The preceding lock/observer target passed 32/32 in 44.055 seconds. These shell
results add no Kindle or firmware-VM observation.

## RL39/RL40 Rust checkpoint

The frozen, formatted Kindle-feature candidate passed 662 library tests with
zero failures and three explicitly ignored manual benchmarks in 151.93 seconds.
Binary-entry tests passed 3/3 in 0.07 seconds (`core-derived-final.log`). The
article target passed 37/37, including ten new anchor-budget cases and the three
deep traversal scenarios (`derived-article-final.log`). The metadata/asset target
passed 29 tests with one existing manual benchmark ignored
(`derived-metadata-final.log`); the login target passed 10/10, including five new
loopback challenge cases (`derived-login-final.log`). These targets are subsets
of the combined library suite and are not added to its total.

The anchor cases use small explicit quotas and observable admitted-work
counters. A 2,049-occurrence production case retains one deduplicated acquisition
reference but is rejected by the strict metadata preparation entry, preserving
the previous body, manifest, and cache usage. Existing Unicode offsets, optional
caption disambiguation, hidden-source checks, and the small-stack deep traversal
contracts remain covered. Login cases verify borrowed UID and encoded URL
admission, exact limits, Unicode/numeric support, and a cancelled late reply
without making a live WeRead request.

The same complete run painted 320 frames across 80 offline navigation cycles and
repeated application/window creation and destruction 12 times. Navigation
samples were identical: 16 descriptors, four threads, and 100,080 KiB RSS.
Application-drop windows had seven descriptors and three threads throughout,
with RSS 100,432, 100,564, and 100,564 KiB (`runtime-derived-final.log`). This is
same-process software/fake-device evidence; it is not a total RSS cap or proof
that every possible workload is leak-free.

The RL36/RL37 production launcher checkpoint passed 135/135 cases in 663.649 seconds
(`launcher-resource-final.log`) and its separate actual interruption-window
target passed 5/5 in 18.429 seconds (`window-stop-corrected.log`). The unchanged
backend passed 47/47 (`backend-fd-final.log`). Later RL41/RL42 source and launcher
changes require new combined evidence; ARM/package identities are recorded after
the complete stopping condition is met.

## Frozen RL38 Rust checkpoint

The complete Kindle-feature library run passed 647 tests with no failures and
three explicitly ignored benchmarks in 155.09 seconds. Binary-entry tests passed
3/3 in 0.07 seconds (`core-queued-final.log`). The preceding Jobs target passed
26/26, including eight genuine queued-credit regressions and all eighteen
existing admission, cancellation, cached-checkpoint, and typed-image cases
(`queued-jobs-final.log`). The two new App scenarios exercise an actual
synchronous Busy result whose side-setting write fails, then attempt ordinary
or terminal exit in the same turn. They preserve the first SQL cause and type,
allow a failed ordinary exit to retry, and independently clean terminal resources
despite a later preferences-write failure. Their fixture has no trusted launcher
ancestry; actual marker content remains covered by existing protocol fixtures.

The same complete run painted 320 navigation frames over 80 navigation cycles
and repeated application/window creation and destruction 12 times. Navigation
samples were identical: 16 descriptors, four threads, and 101,812 KiB RSS.
Application-drop samples retained seven descriptors and three threads, with RSS
102,164, 102,168, and 102,168 KiB (`runtime-queued-final.log`). These controlled
Linux software/fake-device samples do not prove the absence of every possible
leak. The later RL39/RL40 changes were outside this checkpoint and are covered by
the subsequent combined result above.

## Historical integrated remote results

The following checkpoints retain their original source boundaries. Later-change
references identify what each checkpoint did not include; the current combined
result above covers the integrated corrections.

The first candidate passed 484 core tests with three explicitly ignored
benchmarks and three binary-entry tests. At that intermediate checkpoint, the
second-round corrections had not been verified on their integrated source.

The same-process resource regression painted 320 navigation frames, including a
cached image, and then repeated application/window creation and destruction 12
times. Its warm navigation samples were identical: 16 descriptors, four threads,
and 105,464 KiB RSS. After application destruction the first warm sample was
seven descriptors, three threads, and 105,816 KiB RSS; both later destruction
windows had seven descriptors, three threads, and 105,820 KiB RSS. Retained
process-global font/allocator storage did not grow across the measured windows.

Intermediate logs: `test-env:/root/weread-emulator/artifacts/resource-review/`
(`core-round1.log`, `runtime-round1.log`, and `format-round1.log`).

The second integrated Rust candidate passed 499 core tests with three ignored
benchmarks and three binary-entry tests. Its log is `core-round2.log` in the same
remote directory. This includes the confirmed-range proof, legacy temporary
cleanup, and tracked wake-maintenance changes. The later launcher and visible-cover
integration corrections were outside this checkpoint.

The third candidate passed 504 Kindle-feature core tests and three binary-entry
tests (`core-final.log`); three optional benchmarks remained ignored. The native
backend passed 47 tests (`backend-final.log`). A corrected set-based comparison
confirmed that its isolated lock introduced no new shared dependency versions;
the initial comparison incorrectly collapsed legitimate multiple versions of
the same package name and is not evidence of dependency drift.

The first full launcher run passed 107 of 109 cases. Two new fixture assertions
incorrectly required the supervisor log to end at an injected marker or remain
empty. Actual retained tails were `fixture-log-end\nKilled\n` and `Killed\n`:
the shell's finite child-cleanup status follows the injected marker. Corrected
fixtures keep the byte limits, old-content removal, process cleanup, and original
open-file identity/content checks. Both targeted cases then passed. Production
logging behavior was not weakened to satisfy the fixtures. Logs are
`launcher-final.log`, `launcher-logs-corrected.log`, and
`launcher-log-tail-evidence.json` in the same private artifact directory.

The cookie/account/import candidate then passed 556 Kindle-feature core tests
with three ignored benchmarks and three binary-entry tests
(`core-capacity-final.log`). The import-only run passed 21 tests, including the
11 new derived-capacity cases (`import-budget.log`). This source snapshot is
`source-round5.tar`, SHA-256
`60d2626b5df160bf52110c2312c1188da90617bf89690783b2bbb15ef3ed4266`.
The later ZIP initialization correction was outside this snapshot.

The ZIP-bound candidate passed 564 Kindle-feature core tests with three ignored
benchmarks and three binary-entry tests (`core-zip-final.log`). This includes
the bounded constructor and eight new import/image archive cases. Later native
command, remote-image work, and DOM traversal corrections were outside this run.

After correcting the new EPUB fixture's `crate::import` reference to the existing
`crate::importer` module, the next candidate passed 581 Kindle-feature core tests
with three ignored benchmarks and three binary-entry tests
(`core-traversal-corrected.log`). The prior compile failure was confined to that
fixture reference. Native command cases passed 9/9 (`native-command.log`), image
cases 24/24 (`remote-image-work.log`), and archive-contract cases 3/3
(`package-tests.log`). The asset, offline-intent, and wake-probe corrections were
outside that candidate.

The quiescence/read-debt/ZIP-work candidate passed 602 Kindle-feature core
tests with three ignored benchmarks and three binary-entry tests
(`core-closure-final.log`). That result predates the small atomic descriptor
flag correction and its later source/build checks.

After the descriptor-creation correction, the isolated backend passed 47/47
tests (`backend-fd-final.log`) and the Root candidate again passed 602 core
tests with three ignored benchmarks and three binary-entry tests
(`core-fd-final.log`). The complete production launcher passed 118/118 cases in
546.651 seconds (`launcher-closure-final.log`), including all nine probe cases
and corrected managed-log assertions. Later article batch changes retain their
own verification scope.

The article batch checkpoint compiled and passed eleven pure state cases
(`article-batch-state.log`) and seven isolated App scenarios
(`article-image-app.log`). Those runs precede the queued-payload credit and
typed-result corrections and are intermediate evidence only.

The typed-image/credit/compressed-work candidate passed 630 Kindle-feature core
tests with three ignored benchmarks and three binary-entry tests
(`core-typed-final.log`). Jobs-specific typed results and cancellation passed
18 cases (`typed-image-jobs.log`). Later terminal cleanup and supervisor-query
corrections retain their own verification scope.

The asset-reconciliation target passed 26 tests with one existing manual
benchmark ignored (`asset-reconciliation.log`). The lifecycle target passed
16 tests, including the 1,024-cycle offline case (`offline-retry-intents.log`).
The nine new probe cases passed in 41.045 seconds (`wake-probe-targeted.log`);
the complete changed-launcher regression was performed at a later checkpoint.

A later same-process software sample kept navigation at 16 descriptors, four
threads, and 100,364 KiB RSS in all three windows. Application-drop windows had
seven descriptors and three threads throughout, with RSS 100,588, 100,652, and
100,656 KiB (`runtime-final.log`). This includes the asset and offline-intent
candidate, with the same offline/fake-device limits as the first sample.

An intermediate ARM candidate built in 3m41s and was identified as a static,
stripped ELF32 ARM EABI5 executable, SHA-256
`7e5f822cfabb1f4fac1c09fa462b5210dacb3b4c488572805ad4565c602032b5`.
It predates the quiescence/read-debt/parser-work corrections and is not the final
handoff artifact. Its build log is `arm-resource-candidate.log`.

## Covered regression workloads

- Blocked fake device I/O and repeated shutdown 1,024 times, then released and joined.
- Repeated login/download cancellation and counted destruction of owned work.
- Saturated metadata and cover budgets, rejected excess work, and reused released slots.
- Injected cache transaction/unlink failures repeatedly in one process; counted real
  payloads and bytes rather than relying only on the cache index or restart.
- Compacted confirmed report batches without crossing uncertain gaps, losing
  allocation evidence, or allowing duplicate delivery after restart.
- Repeated navigation and actual software rendering in one process, followed by
  application/window destruction; measured weak references, file descriptors,
  threads, and warm-process RSS windows.
- Injected launcher identity-write failures and checked lock/child cleanup; retained
  existing sleep, native theme, explicit exit, and observer regressions.
- Exercised bounded diagnostics with repeated, concurrent, and failed writes.

RSS measurements describe this controlled workload and its allocator/cache
plateau. They do not prove the absence of every possible leak, and process-exit
cleanup alone is not accepted as evidence for same-process stability.
