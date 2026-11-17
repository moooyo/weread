# Native catalog entry integration proposal

Date: 2026-11-18, Asia/Shanghai.
Status: notification consumer and fixed-entry client prepared. Remote checks
pass: 233 launcher cases, 19 catalog-client cases, and eight package contracts.
The ARM client has been built. Shared protocol coordination and physical
verification remain pending. No new runtime has been
registered, deployed, or measured by WeRead for this proposal.

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

## Verified source differences

- The current shared hook and client select only `/mnt/us/documents/Aidoku.sh`.
  A WeRead marker alone cannot enable the existing runtime.
- The WeRead document is 43,067 bytes because its third line embeds the accepted
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

## Proposed coordination contract

These are proposed choices until reviewed with the shared-runtime owner.

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
4. Preserve WeRead's large embedded Icon and default FBInk behavior. A WeRead
   declaration belongs in the first six lines. Header limits must remain finite;
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
See [remote verification](review/catalog-entry-adapter-20261118/verification.json)
and [pinned client provenance](../vendor/kindle-catalog-client/UPSTREAM_REVISION).

The prepared consumer currently expects `WEREAD_CATALOG_NOTIFY_HELPER` and
`WEREAD_CATALOG_NOTIFY_SHA256`, an immutable `v1-<sha256>` directory below the
shared runtime root, and the fixed `weread-catalog-client` basename. The root,
version directory and client must be root-owned with mode 755; the content hash
must match the handoff. These choices remain subject to the shared-owner review.

The library waiter transfers these values only to its independent supervisor.
Each process consumes and clears the exported variables before spawning its
ordinary children. Successful final recovery admits one bounded
`--notify-protocol` invocation, with successful exit, confirmed helper cleanup,
and exactly `NOTIFIED` plus one newline required. An unconfirmed native cleanup
withholds notification. Pre-ownership failures use the validated fixed client's
own bounded transaction/alarm. Failure uses a standard timestamp/scan recovery
after native restoration; an early failure only restores the timestamp because
it owns no private launcher context for a new IPC writer.

The scoped host cases cover successful ordering, nonzero application status,
waiter retirement across sleep/wake, early configuration failure, explicit
helper failure, overflow, trailing protocol data, timeout and group retirement,
helper modification, and failed native restoration. They are Linux process and
service fixtures, not a physical catalog-service result. No opt-in marker has
been added to the document, so the current Aidoku-only runtime remains unchanged
and this prepared consumer has no enabled production catalog path.

The WeRead notification consumer and isolated host lifecycle fixtures were
prepared without holding a device lease. They reuse the supervised native-command
boundary and retain the accepted cover and all current persistence layouts.
No Rust rebuild or application-internal performance change was made.

Before device use, freeze the shared protocol/version, source and payload hashes,
remote host checks, public offline fixture, actual indexed-entry instrumentation,
and protected installation/registration baseline. Use a fresh owner/run/token;
Aidoku has the operator's highest device priority, and BiliComics currently owns
the announced validation window. No concurrent device operations are permitted.

Use a predetermined six-pair AB/BA comparison with the same actual indexed
WeRead document, application binary, cache snapshot, boot and stabilization
policy. Record first business-frame SEND and exit-to-native restoration; do not
infer network-cover improvements from the offline fixture. Check catalog identity,
recent access, thumbnail/MIME, pixels, notification completion, ordinary exit,
same-process sleep/wake and the preserved Aidoku/shared baseline. Keep raw tokens,
catalog IDs, device identifiers, private user data and research archives outside
the repository. Report failures and unverified gates separately.

Main integration, daily installation, any new shared registration, and numeric
WeRead benefits remain pending this implementation and independent qualification.
