# Local ZIP initialization bound

This source copy starts from zip 4.6.1 at the provenance recorded in
`UPSTREAM_REVISION`. The original MIT notice is retained in `LICENSE`.

## Accepted changes

RL-20 identified that `ZipArchive::new` reserves and builds the complete central
record index before the caller can check `archive.len()`. A small compressed
input with many records, or a malformed large record count, can therefore cause
an unnecessary allocation before an application rejects the archive.

`src/read.rs` adds `ZipArchive::new_with_file_limit(reader, max_files)`. It passes
an optional limit through the existing metadata search rather than implementing
another ZIP footer parser. Each candidate's raw ZIP64 `u64` count is checked
before conversion to `usize`, and the parsed count is checked again immediately
before file-vector reservation and central-record parsing. Duplicate filenames
consume records even though the final filename map deduplicates them.

Oversized candidates use the existing EOCD fallback search, retaining ZIP64,
archive-offset detection, self-extracting prefixes, and comments. If no acceptable
candidate remains after a count rejection, the bounded constructor returns an
`UnsupportedArchive` error identifying the file-count limit. Its quota is a bound
on indexed records, not a hard cap on all parser metadata or allocator overhead.

RL-29 identified that admitting each candidate separately still permits repeated
metadata initialization and rereading of the same small archive. The bounded
constructor now uses a `Read + Seek` adapter that borrows the original reader
only during metadata construction. Its default cumulative allowance is 128 MiB
of actual returned bytes; seeking does not reset that counter. It also permits
at most 64 outer directory-initialization attempts, including failed central
records and candidates rejected by the file-count limit. Internal EOCD search
and failed parsing share the same cumulative read allowance. The existing
footer parser and fallback rules are reused without a second EOCD parser.

`ZipArchive::new_with_limits(reader, max_files, max_metadata_read_bytes,
max_metadata_candidates)` exposes explicit quotas for controlled callers and
small regression fixtures. `new_with_file_limit` supplies the default work
quotas. The adapter is dropped after producing the immutable metadata; body and
extraction reads use the original reader without spending metadata allowance.
These are record/read/attempt limits, not a wall-clock, total metadata-size, or
process RSS cap. The application's separate 64 MiB import-file and 32 MiB image
archive limits remain in place.

The original `new`, `with_config`, and internal `get_metadata` APIs retain their
original file-count, actual-read, and candidate-search behavior. Their signatures
and configuration layout remain unchanged. The retained-metadata admission below
is shared by these paths, including `ZipWriter` append metadata initialization.

`src/result.rs` exposes the fixed `ZipError::FILE_COUNT_LIMIT_EXCEEDED` text using
the existing `PASSWORD_REQUIRED` associated-constant pattern. Callers can classify
the new constructor's quota error without adding a breaking error variant or
copying a message literal.
The same pattern exposes `ZipError::METADATA_LIMIT_EXCEEDED` for cumulative
bounded-initialization work. A read-quota rejection is preserved even if an
existing candidate fallback would otherwise replace its I/O error. Both quota
errors remain non-retryable image-resource size failures and do not request new
credentials.

RL-61 identified derived copying that the actual-read allowance does not price:
each removable ZIP64 extra field rebuilt nearly the entire original extra-field
buffer, overwriting the previous reconstruction. A small central record with
many accepted empty ZIP64 fields therefore performed quadratic temporary copies
before the application received its archive.

The parse_extra_field implementation now remembers the final removal range and
reconstructs the buffer at most once after the original parser loop finishes.
This deliberately preserves the existing return bytes, which remove only the
last removable range, rather than changing the parser to remove every ZIP64
field. The immediate None result when a removal consumes the entire original
buffer, parse/error order, ZIP64 size/offset/large-file side effects, and all
other extra fields stay unchanged. The correction applies to the shared
implementation used by bounded and legacy entry points. It adds no new parser
quota, feature, or dependency and does not change the 128 MiB actual-read
allowance.

RL-77 identified a zero-length extended-timestamp field that reads the following
byte as flags, then subtracts one from a zero unsigned length before validating
the field. The root task's reviewed ZIP test artifact enables overflow checks,
so this shape can panic during resource acquisition before the download worker
settles its terminal result. Only one following byte is required. Ordinary
release builds with overflow checks disabled instead reach the existing
unsupported-field error; this is not a claim of a release-device failure.

`extra_fields/extended_timestamp.rs` now rejects `len == 0` with that same
`UnsupportedArchive` text before reading flags or performing the subtraction.
Nonzero lengths, the length-five compatibility rule, undocumented flags,
timestamps, and all other extra fields keep their previous behavior. The RL-61
parse order and one-reconstruction correction remain unchanged.

## Integration and verification

### Retained extra-field metadata

A legal extended-timestamp field with length one and flags zero occupies only
five archive bytes but appends an `ExtraField::ExtendedTimestamp` with three
absent timestamps. All central entries retain their enum vectors in the shared
archive before the application hashes or deduplicates image bodies. The previous
raw-read, candidate, and file-count limits did not price this retained expansion.
This is distinct from the earlier zero-length timestamp and repeated ZIP64
buffer-reconstruction changes.

`src/read/metadata_retention.rs` adds one checked retention operation shared by
all parsed entries and failed directory candidates. Its defaults are 64 MiB of
selected metadata fees and 1,048,576 timestamp/NTFS record admissions. The raw
128 MiB actual-read and 64-candidate defaults are unchanged. Explicit retention
limits can only reduce their defaults.

Before central-index allocation, the raw ZIP64 count is checked and index/file
storage is priced. Before owning entry fields, the declared lengths price the
retained raw filename, decoded name/comment, filename-map key, original extra
buffer, and Arc/vector control data. Unicode replacements and a final ZIP64
reconstruction also receive admission before their buffers are allocated.

Each retained record receives one enum-size fee before construction or push.
Extra vectors grow by checked doubling with `try_reserve_exact`; requested
capacity increments are charged once and the old buffer's temporary overlap is
checked before reservation. This avoids creating a per-record quadratic reserve
loop. Neither failed candidates nor later parsing failures refund admitted fees.
A retention denial returns the existing typed metadata `UnsupportedArchive`
error immediately, so ordinary EOCD fallback cannot hide it or reset its budget.

`new_with_retention_limits` and `with_config_and_retention_limits` expose lower
retention limits and small numeric operation usage to controlled callers. The
original reader is returned directly after successful construction. The limits
price selected metadata and requested storage; they are not an allocator-padding,
total process-RSS, all dependency-memory, or wall-clock guarantee.

Root regressions add complete small two-entry CDF/EOCD archives with real PNGs,
flags-zero duplicate timestamps, NTFS values, exact/one-less record and byte
quotas, old-buffer growth overlap, and a genuine later invalid candidate with an
earlier valid directory. They preserve the existing zero-length timestamp cause,
ordinary parse order, duplicate fields, length-five compatibility, CRC reads,
and reader ownership. A root `cfg(test)` constructor dispatch permits the same
small quota to reach actual image acquisition and a real Jobs download worker;
it does not change production transport or dependency features. Push/reservation
usage and a root-only rejected-payload allocator observer witness refusal before
extra-vector growth, image payload copies, or chapter publication.

These additional regressions have been prepared by source editing only. Their
presence is not compilation, passing evidence, or an upstream complete-suite
claim. The root task owns combined integration and remote verification.

The application selects this path dependency at the exact reviewed version and
uses the bounded constructor before retaining imported EPUB metadata (8,192
records) or parsing a downloaded image ZIP (4,096 records). Both existing
post-construction entry-count checks remain as additional guards. Image ZIP count
failures remain non-retryable resource errors and do not request new credentials.

Root importer tests construct synthetic archives for count admission, oversized
declarations, duplicate paths, ZIP64, prefixes, comments, and EOCD fallback. Large
declarations are passed only to the bounded constructor; comparisons against the
legacy API use small archives and low test quotas.

The published crate excludes its upstream `tests/data` assets. Do not treat the
root synthetic regressions as execution of the upstream crate's complete unit
suite. Verification results and execution environment are recorded by the root
project, not asserted by this source patch note.

The additional work-quota regressions use small ZIPs, low explicit allowances,
and a metered reader to check rereads, failure candidates, and the byte ceiling.
Synthetic multi-candidate comments retain a valid earlier directory. Legacy
comparisons and append checks use those small fixtures; no gigabyte-scale work
or unlimited constructor with a malicious large declared count is required.

Root image-archive regressions also construct small central records with repeated
empty ZIP64 fields. A test-only, thread-local allocator observer counts the
distinct reconstruction capacity during the real public constructor, alongside
assertions for returned extra bytes, unknown fields, malformed-field behavior,
single-field None, ZIP64 side effects, and self-extracting prefixes. The observer
does not change the production allocator or expose a vendor test API. These
cases require root-project remote verification; their presence is not passing
evidence or an upstream complete-suite claim.

Future replacement of this revision must preserve the early admission boundary
and re-run the compatibility and import regressions before removing this patch.
