# Native catalog entry integration

Date: 2026-11-18, Asia/Shanghai.
Status: the three projects have coordinated ownership. Aidoku maintains the
shared runtime; WeRead owns its document, notification consumer, and comparison.
The host-tested candidate includes the agreed document declaration. The existing
Aidoku-only runtime does not recognize it. WeRead has neither changed the
registration nor enabled the feature on the shared Kindle. No device speedup
is measured here.

For the coordinated candidate, **285 tests passed** on `test-env`: 236 launcher lifecycle, 19 catalog client,
10 packaging and comparison, and 20 offline protection cases. The 13 notification
cases are included in the 236, not added again. The launcher suite exited
successfully before the verification host restarted; its temporary full log
was lost. Its tested source hashes match the final package and test inputs.
See [coordinated host verification](review/catalog-entry-adapter-20261118/coordinated-host.json)
for the results, receipt boundaries, and artifact identities.

## Accepted baseline and reference

WeRead main `e0fca43e76e6298a2e761092a59dbc171d5f3134` already retains the
Library waiting entry until its independent supervisor restores native state.
The accepted penguin thumbnail remains embedded, unchanged, in the document.

Aidoku reference `78c5b2bc09a469133c886eb0d73aaf9ed63195b2` implements a
quiet native catalog `lastAccess` update at entry and a normal notification
after native restoration. Its measurements are reference evidence, not WeRead
results. Aidoku's deployment record identifies its immutable
`v1-2db100d0bcb60a76571cd8de96bed7ea6c016e6ab73e388a885f692955cacd86`
runtime under `/var/local/kindle-catalog-entry` as the new shared registration.
WeRead has not independently read back the device registration in this round.
A future leased preflight must capture the then-current qualified version.
Old test harnesses must not select or restore the original shared executable
as the registered command.

Aidoku subsequently deployed application `9d92dc4cd01005a6ba2f48fe61786e5df5570c43feb5709fe3159bb164807d0f`
in transaction `promote-f2889bdb575f`. Its deployment receipt and ARM identity
agree: `/mnt/us/aidoku/app/aidoku` was the only changed program member. The
shared registration, Aidoku launcher, entry, cover, and helper remain protected.
The installed-smoke result and released lease are Aidoku's evidence; WeRead did
not inspect the Kindle. This update did not enable the WeRead marker or grant a
device window. The previous `62de2771...` executable must not be restored.
The updated offline guard passed 22 cases, including rejection of the old
snapshot. The application and candidate package were not rebuilt for this update.

## Verified source differences

- The current shared hook and client select only `/mnt/us/documents/Aidoku.sh`.
  A WeRead marker alone cannot enable the existing runtime.
- The unmarked WeRead document is 43,067 bytes because its third line embeds the accepted
  thumbnail. The reference hook reads at most 4,096 header bytes. A bounded
  extension must accommodate this exact metadata without replacing the image.
- The reference hook requires `# DontUseFBInk`. WeRead retains the original
  FBInk startup behavior. Extending entry selection must keep that behavior as
  an explicit per-entry policy, with its own tests and physical qualification.
- The reference launcher accepts only immutable `v1-<sha256>` directories and
  the `aidoku-catalog-client` basename for completion. An extended shared runtime
  must remain consumable by the already-installed Aidoku launcher.
- WeRead publishes prepared visible-cover changes in `flush_cover_ui` during its
  regular tick and maps `CoversLoaded` directly to a UI refresh. There is no
  equivalent 300 ms publication gate to shorten. Existing worker polling is a
  separate potential experiment and is not included in this catalog change.
- The [public image adapter](../src/source/client.rs) returns bounded encoded
  bytes. On an ordinary Library miss, [`prepare_cover_png_cancellable`](../src/cache.rs)
  decodes the original once to produce the thumbnail; the [worker](../src/app/sleep_screen.rs)
  publishes the resulting pixels without decoding its new PNG. A warm hit can
  decode the cached thumbnail, not the original. Aidoku's cold-cover gain does
  not transfer to this path. When the current Reader book also needs a larger
  sleep image, a separate `SleepOriginal` job can decode the source again for
  that different size. Any proposal to share that work needs its own latency,
  memory, cancellation and sleep/wake measurements; no change was made here.

## Agreed interface and ownership

Aidoku accepted the WeRead path, marker, default FBInk behavior, and completion
interface on 2026-11-18. BiliComics also requires default FBInk; its configuration
is a proposal for its own fixed document, not an enabled participant. The
compatible shared build, including installation and retirement policy, belongs
to Aidoku. It must be qualified before the first WeRead device experiment.

1. Maintain one shared dispatcher/hook implementation. Qualify explicit canonical
   entries and their markers; unrelated documents keep their original behavior.
   Preserve the original handler, scan request, privilege transition and MIME
   association. Do not install an application-specific replacement registration.
2. Keep the existing Aidoku declaration, notification environment names, helper
   basename and immutable directory layout usable without changing its installed
   application or launcher. Add WeRead through an explicit entry specification;
   give BiliComics the same extension path rather than competing installers.
3. Use a fixed WeRead catalog client or an equally strict fixed-entry selector.
   No arbitrary document path, URL, service, SQL statement or helper command may
   come from the document or application environment.
4. Preserve WeRead's exact third-line embedded Icon and default FBInk behavior.
   The fourth line is exactly `# WeReadCatalogAccess: deferred-v1`, with this
   capitalization, at `/mnt/us/documents/weread.sh`. Header limits must remain finite;
   oversized, malformed, missing or ambiguous declarations fall back entirely.
5. Commit recent access using the qualified native catalog service with quiet
   notification. Delegate exactly one completion to the independent supervisor,
   clear the handoff from application/helper child environments, and notify only
   after confirmed native display/input restoration. Keep the launch timestamp.
6. Bind the completion handoff to an immutable validated helper and an explicit
   entry identity. Preserve bounded execution, output validation, startup-failure
   recovery, cancellation, waiter retirement and sleep/wake ownership. A failed
   completion uses the existing safe reindex route after native restoration.
7. Preserve current shared registration and Aidoku installation hashes in every
   test preflight and restoration guard. Changes to the shared version require
   an agreed additive protocol and compare-and-swap registration. Retain active
   versions for existing workers and prevent older per-app installers from
   downgrading an extended runtime.

## WeRead implementation and verification plan

Candidate branch: `codex/catalog-entry-adapter`, based on `e0fca43`.
The prepared ARM client is 12,808 bytes with SHA-256
`55aeff9a286d8ff6da1ef46fff85e175a7286e4e23c9b7fd616722d899e21cab`.
It is a separate ARM hard-float glibc executable; the application and existing
helpers are unchanged. Static ELF inspection confirms its fixed WeRead document
path and no Aidoku document target. No device performance result is claimed.
See the [initial consumer verification](review/catalog-entry-adapter-20261118/verification.json)
and [pinned client provenance](../vendor/kindle-catalog-client/UPSTREAM_REVISION).

The prepared consumer currently expects `WEREAD_CATALOG_NOTIFY_HELPER` and
`WEREAD_CATALOG_NOTIFY_SHA256`, an immutable `v1-<sha256>` directory below the
shared runtime root, and the fixed `weread-catalog-client` basename. The root,
version directory and client must be root-owned with mode 755; the content hash
must match the handoff. The existing Aidoku client and its environment names
remain separate. WeRead does not ship a shared installer or dispatcher.

The library waiter transfers these values only to its independent supervisor.
Each process consumes and clears the exported variables before spawning its
ordinary children. Successful final recovery admits one bounded
`--notify-protocol` invocation, with successful exit, confirmed helper cleanup,
and exactly `NOTIFIED` plus one newline required. An unconfirmed native cleanup
withholds notification. Pre-ownership failures enforce the same exact reply
and exit-status contract within the fixed client's own alarm. Ordinary detached
diagnostics do not consume a Library handoff. Failure uses a standard timestamp/scan recovery
after native restoration; an early failure only restores the timestamp because
it owns no private launcher context for a new IPC writer.

The scoped host cases cover successful ordering, nonzero application status,
waiter retirement across sleep/wake, early configuration failure, explicit
helper failure, overflow, trailing protocol data, timeout and group retirement,
helper modification, and failed native restoration. They are Linux process and
service fixtures, not a physical catalog-service result. The fourth-line marker
exists in the branch and its host-built comparison inputs only. The installed
WeRead document and the device registration were not touched.

The WeRead consumer and isolated host fixtures were prepared without holding a
device lease. The ARM app and the two existing helpers in the candidate package
are byte-identical to the previous verified package; only the document,
supervisor and packaged README differ. See the [package and A/B input hashes](review/catalog-entry-adapter-20261118/host-package.json).

Before device use, freeze the shared protocol/version, source and payload hashes,
remote host checks, public offline fixture, actual indexed-entry instrumentation,
and protected installation/registration baseline. Use a fresh owner/run/token;
Aidoku has the operator's highest device priority. BiliComics reported releasing
its window; this does not grant WeRead a window. Coordinate an explicit window
with Aidoku before acquiring a lease. No concurrent device operations are permitted.

Use a predetermined six-pair AB/BA comparison with the same actual indexed
WeRead document, application binary, cache snapshot, boot and stabilization
policy. Record first business-frame SEND and exit-to-native restoration; do not
infer network-cover improvements from the offline fixture. Check catalog identity,
recent access, thumbnail/MIME, pixels, notification completion, ordinary exit,
same-process sleep/wake and the preserved Aidoku/shared baseline. Keep raw tokens,
catalog IDs, device identifiers, private user data and research archives outside
the repository. Report failures and unverified gates separately.

## Rehearsal and measurement boundaries

`scripts/prepare-catalog-entry-comparison.py` reads a pinned candidate ZIP on
the verification host. It materializes the same `documents/weread.sh` path in
separate control/candidate directories, replacing only the opt-in line by an
equal-length ASCII comment in the control. It refuses a changed Icon or FBInk
policy and records the common runtime hashes. Neither output installs anything.
For a device comparison, the build owner must first confirm that the control
takes the normal path and that the candidate completes the new path. A marker
alone is not proof that the shared service consumed it.

Use `scripts/check-catalog-guard.py` on independently collected, sanitized
snapshots. Pin the owner-approved inventory digest outside the observation;
require the separately frozen version, complete registration projection, and
all listed Aidoku and BiliComics file hashes or explicit absence records.

For the reported `promote-f2889bdb575f` installation, check a separately
collected observation before the first device write and again after restoring
WeRead's own state. The same read-only comparison is used at both boundaries:

```sh
python3 scripts/check-catalog-guard.py --aidoku-handoff --observed "$observation"
```

`--aidoku-handoff` pins the [post-deployment inventory](review/catalog-entry-adapter-20261118/post-deployment-inventory.json)
and [expected file hashes](review/catalog-entry-adapter-20261118/post-deployment-protection.json).
The [verification record](review/catalog-entry-adapter-20261118/peer-handoff-verification.json)
identifies the exact owner evidence. Only the Aidoku application hash changed;
BiliComics' previous absences are carried forward as expectations, not recent
observations. The old snapshot is rejected under this profile. A mismatch stops
the batch; do not make a check pass by copying back an old peer binary, reverting
the registration, or adopting the observed hashes. If an owner changes their
installation or qualifies a shared extension, freeze that separate handoff first.

The [historical snapshot](review/catalog-entry-adapter-20261118/historical-protection.json)
and its [bounded inventory](review/catalog-entry-adapter-20261118/historical-inventory.json)
are references, not a new WeRead observation or a baseline for a future runtime.
BiliComics' [pinned restoration record](https://github.com/moooyo/bilicomics/blob/e8d06e356a7319eedf47857566afdeb627f2267c/docs/physical/native-library-entry-20261118/qualification-r2/restoration.json)
provides six Aidoku files and the decoded registration projection. Its
[entry report](https://github.com/moooyo/bilicomics/blob/e8d06e356a7319eedf47857566afdeb627f2267c/docs/NATIVE_LIBRARY_ENTRY_20261118.md)
and the maintainer's handoff identify the absent BiliComics install root and
both document spellings. Configuration files under that absent root were not
read or individually hashed. If BiliComics is installed later, freeze the
public install identities with its owner; never substitute candidate ZIP
hashes or read operator data to fill a gap. A matching snapshot is not proof
of who collected it or when. The November 17 runner assumes a legacy command
and rewrites the document header. Do not reuse it unchanged or restore an old
registration. The fresh, approved preflight and closeout must agree; a
mismatch stops the experiment without rewriting either peer's installation.

Measure request-to-first-Library-SEND from the file-open request and the actual
driver marker using the same uptime clock. Check Library route and view pixels
in separate qualification runs. The [warm offline Library fixture](review/catalog-entry-adapter-20261118/library-covers.md)
demonstrates all fifteen visible covers on the production desktop renderer. A
separate run with one cached image removed rejects just that book. These final
scale-one renders do not timestamp a physical Kindle frame. Requalify the
matcher at the real device geometry before reporting an all-covers endpoint.
Likewise, `--exit-after-ms`, disappearance of the launcher lock, and presence
of the native Home process do not by themselves measure return to the Library
view. Record a real exit request and a confirmed Library view on the same clock
in runs without shutdown screenshots, or explicitly leave that metric unmeasured.

The protected registration identity, peer compatibility, actual device slot,
six pairs, and sleep/wake qualification remain prerequisites for enabling the
WeRead path or claiming any speedup. Main integration and daily installation
are separate from this host candidate.
