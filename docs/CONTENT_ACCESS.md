# Content access and preview recovery

Status: content-access and resident-suspend corrections implemented; their
`2dd3` remote root suite passed 418 tests. Backend and launcher results have
their own recorded scope. The combined ARM build and private trial bundle
were installed on the identified Scribe. The `2dd3/971e` Reader sleep/wake
and ordinary-exit checks are accepted, and two preview caches were revalidated
to full text. An actual phone-side unlock transition remains pending. The newer
native night-mode handoff and Login-close candidate `2fd3/799d` is now installed.
Its controlled tests, ARM build, and scoped Scribe native/wake/normal-Login
exit checks passed. A full device reboot and broader support are not established.
The earlier unlock-only ARM build is a separate artifact.
Artifact-specific evidence belongs in [Verification](VERIFICATION.md)
and [Physical testing](PHYSICAL_TESTING.md).

## Confirmed defect and evidence boundary

The physical private trial saved short responses as if they were full chapters.
Twenty-five final chapter bodies were 151 to 208 Unicode scalars and ended in
`...`; their decoded source HTML was already short. The user confirmed that
the same account on the phone could also only preview those chapters or was
prompted to unlock them. This is evidence of a current reading-rights
restriction for the observed chapters. The repository contains no private book
title, identifier, or excerpt.

The legacy task's 111/111 count, nonempty bodies, valid encoded-response MD5
checks, and complete image manifests did not prove full text. Checksums prove
integrity of the returned data; image manifests prove resource binding and
availability. Neither establishes that the account may read an entire chapter.
The earlier whole-content conclusion is withdrawn.

Source review also identified a replacement defect: the worker preferred any
nonempty old body, the checkpoint rejected changed canonical text, and cached
chapters could be skipped when scheduling. That combination could retain an
old preview after the user later obtains full reading rights.

## Content states

| State | Meaning | Presentation and acquisition consequence |
| --- | --- | --- |
| `Unknown` | The body or its full-text entitlement has not been established in the current request scope. Missing, malformed, contradictory, or unsupported metadata remains unknown. | Keep an existing body readable, label it as awaiting verification, and offer revalidation. Do not label it locked or fully downloaded. |
| `Full` | An affirmative source entitlement for the requested account/book/chapter is consistent with the accepted decoded content. | Count a nonempty body as full content; image readiness remains an independent condition when images are requested. |
| `Preview` | The source supplies a usable excerpt while full access requires unlocking, or identifies an unpurchased trial edition. A complete sample chapter does not establish access to the whole trial book. | Keep the available text readable and label it as a preview. Do not advance the full-download cursor. |
| `Locked` | The source establishes an access restriction without a usable excerpt. | Show an unlock-required state and a later revalidation action. Do not fabricate chapter text or mark it cached. |

The application-owned state must remain distinct from upstream purchase-field
names. A missing `paid` value is not a free grant, and `paid=0` alone is not a
complete denial: valid membership, free chapter, limited-time, or other supported
rights may permit full reading. An inverse `needPay` result is not sufficient
positive evidence when its required inputs are absent.

The adapter binds fresh read-only Reader evidence to the requested account,
book, stable chapter UID, and renewal generation. Permission evidence and the
server `psvts` must come from the same strictly parsed `window.__INITIAL_STATE__`
object. The client `pclts` may be zero, missing, null, or blank and receive one
local fallback; invalid client-clock types or missing strict server-clock
evidence remain `Unknown`. A chapter list index is not interchangeable with
its source UID.
Unknown membership enum zero, contradictory membership fields, and an empty
membership summary remain `Unknown`. Permission fallback data, wrong-book
state, duplicate/missing target chapters, or identity changes cannot silently
authorize full content. Account changes downgrade existing permission proof
rather than transfer it to the new identity. A word count mismatch or trailing
ellipsis may flag an inconsistency, but neither by itself establishes that a
chapter is locked or complete.

## Unlocking interaction

The user obtains any required purchase, membership, or other reading right in
the official phone application. This application does not automatically
purchase, pay, subscribe, or change account entitlements.

For a preview or an unverified cached body, the application shows a concise
state and provides an explicit refresh-and-complete action. That action reads
fresh permission evidence and retrieves content through the existing adapter.
It does not reuse a completed-cache flag as a reason to keep the old preview
forever. `SourceError::UnlockRequired`, `ContentUnverified`, and `ContentChanged`
pause the task for explicit refresh; they neither trigger automatic network
retries nor prompt for login. Fully validated content retains the normal
uncluttered reading view.

## Persistence and safe replacement

The correction adds `Chapter.content_status` and opaque
`Chapter.content_account_key` metadata. The latter is an identity hash, not a
credential or a perpetual authorization token. The current development layout creates these fields directly and has no schema migration path. Historical migration observations below apply only to the recorded candidates. Imported/demo content remains local full content, and full-text counts depend on explicit content evidence.

A preview checkpoint can persist the excerpt and its state but cannot advance
the full-download cursor or increment full-content totals. The task pauses for
explicit revalidation. Cancelled tasks remain cancelled during
recovery; phone-side unlocking is not permission to resurrect them.

An identical response can establish a stronger state without moving anchors.
A changed body can replace an unverified or preview body only after full access
is established and a checked exact prefix extension preserves the existing
text anchors in the same chapter/account scope. A changed `Unknown` body requires the same verified account binding and UID/prefix checks. An unbound body cannot be extended into an account-owned full download.

When a terminal `...` or `…` preview marker is removed for the exact-prefix
comparison, reading positions and bookmarks in that marker are clamped to the
retained prefix. An annotation touching the removed marker rejects replacement
and preserves the old body and all user records. Incompatible text, conflicting
ownership, and other unsafe anchors likewise surface `ContentChanged` without
silently rewriting the old body. A longer response alone does not authorize
replacement.

Body, content state, account binding, full-content count, and durable task
checkpoint remain atomic. Store preflight and the transaction each validate
the complete serialized task plan in their compare-and-save check. This safety
check adds plan serialization; the earlier zero-serialization component
observation belongs only to the `4bef` implementation. Stale task/account/catalog
responses are rejected. Text-bound images are rebuilt or rebound against the
accepted canonical body; a complete image manifest is not a full-text
entitlement.

## Verification gates

The earlier 334 normal library tests and three component benchmarks belong to
the `4bef` performance source. They did not cover the preview/unlock defect and
must not be reassigned to the correction.

The unlock-only correction baseline passed `cargo test --locked --all-targets` on
`test-env`: 386 library tests passed, zero failed, and three benchmarks remained
ignored. The main target contained zero tests. Formatting was applied and the
462-key interface-resource check passed. The log and separate earlier desktop
render identity are recorded in [Verification](VERIFICATION.md#content-access-correction-remote-checks).
The regression scope includes affirmative purchases, valid membership
with `paid=0`, expired or contradictory membership, free chapter boundaries
where list index differs from UID, missing/unknown permission fields, wrong
identity, short legitimate content, inconsistent short previews, legacy cache
migration, preview checkpoints, same-account full-text extension, unsafe text
replacement, note/bookmark/position retention, account changes, cancellation,
and transaction rollback. Five final HTTP combination regressions cover the
single extra Reader permission refresh after renewal, the shared chapter
authentication budget, further-generation rejection, stable-unknown
non-retry behavior, and chapter-local refresh budgets. No local verification
fallback was used.

Four synthetic light-state captures from an earlier desktop artifact were
visually reviewed for `Preview`, `Unknown`, `Locked`, and `Full`; the last had
no access notice, and no body overlap was found. That artifact predates the
final source correction and is not final-binary or dark-mode evidence. The
unlock-only ARM cross-build succeeded with SHA-256
`546efae824dbf9e74658f19ddd46520ff5531de2ed4ab6b61040472b0ea7ee3f`.
It is a stripped static ELF32 ARM EABI5 hard-float executable. That unlock-only
artifact was not installed; the later physical observations below belong to
the combined candidate.

The 386-test result and this ARM artifact both precede the resident managed
suspend implementation. The combined source first passed 414 root-library
tests, with three ignored benchmarks. A subsequent anonymous public Reader
metadata check found the real SSR `pclts=0` placeholder and empty token; it
used no account authentication or chapter-content request. That shape check
revealed an overly strict client-clock gate and did not establish private
account entitlements.

The narrow clock-placeholder correction added four regressions. The final
combined source passed 418 root-library tests, zero failed, and three ignored;
the main target had zero tests. Backend 47/47 and launcher 65/65 results remain
applicable because their source did not change in that final source-only
correction. The 414-test snapshot and a focused rerun of an existing launcher
case are not added to these final counts. [Verification](VERIFICATION.md#combined-source-remote-checks)
records the scopes and `complete-root-tests.log` evidence.

The final combined ARM build completed in 3 minutes 32 seconds. Its ELF has
SHA-256 `2dd3bfc996d9c2161df155b26649e1fa6714dd9eb2c58068ef21e655c8e5d656`;
its launcher has SHA-256
`971efb417b4af5d6ec5111dcfa6b98bf4a0b032fd933303102433f3551c6c54c`.
The private trial ZIP is
`dist/weread-kindle-0.2.0-content-access-resident-trial.zip`, with SHA-256
`71f0d8b0a7488ad26974fd31e967b1d7892f07794e0b1b291e9da796d33d9201`.
[Verification](VERIFICATION.md#combined-arm-build-and-private-trial-bundle)
records immutable remote copies and the build log.

An early transfer failed after idle sleep. After the user woke the device
again, the final `2dd3` ELF and `971e` launcher were installed with matching
device hashes. Schema 1-to-2 migration retained 118 nonempty bodies and two
position rows; the annotation and bookmark tables were empty and unchanged.
One cancelled task remained cancelled and two old complete tasks became
paused for revalidation. All 1,974 catalog rows were `Unknown`, including the
118 bodies. Those initial observations establish migration and unknown-state
initialization; the subsequent revalidation adds separate live-content evidence.

Stable-UID comparisons with the preupgrade backup found index 105 grow from
192 preview scalars to 11,613 full scalars, and index 109 from 181 to 9,058.
Two tasks reached complete. Catalog word counts 11,869 and 9,286 are compatible
in scale but are not an independent full-text checksum. No purchase or unlock
action on the phone was observed, so this is not a tested phone-unlock
transition. After confirmed logout and fresh QR login, the two full proofs
were invalidated and all 118 bodies remained present as unverified, with four
paused tasks. This is account-operation-related invalidation, not an ordinary
Exit defect.

Three cycles retained PID 28138, generation 1, and process start time
703837374. Reader cycle 2 was light only; after explicitly enabling night mode,
cycle 3 retained chapter 105 / scalar 496 / dark mode 1, and the user confirmed
the same page and working page turns. Voluntary exit restored native Library
touch, and process/lock/service checks and exclusive-device handoff completed.
[Physical testing](PHYSICAL_TESTING.md#same-process-sleep-and-reader-recovery)
records the scoped results without private content.

The accepted `2dd3` dark Reader was application-only: after exit,
`prefs.dark_mode` remained 1 while native inversion getters returned `Y8`.
The newer native exit synchronization, launch/wake XOR compensation, and
48-by-48-pixel Login close X are implemented in a separate candidate. Its
remote core suite passed 440 tests with three ignored benchmarks, its targeted
closing subset passed three cases, and its launcher passed 86 fixtures in
359.502 seconds; 463 translation keys passed their check. The old vendor
library was unchanged and its prior 47-test result was not rerun as new
coverage. [Native theme continuity](NATIVE_THEME.md) records the accepted delta,
while [Verification](VERIFICATION.md#native-night-mode-handoff-and-login-close-candidate)
binds its controlled results and diagnostic corrections to their scope.

The new native-theme ARM build completed in 7 minutes 7 seconds, including a
full target-dependency rebuild, with ELF SHA-256
`2fd3d6ab2d648aec2671d0d2305a5e5ebe30fa6fd5e51f11705324d561317011`
and launcher SHA-256
`799d5bba5eb9e7c2b534437a1112e6810e5c85d73582742d770c96c6f72af234`.
[Verification](VERIFICATION.md#native-theme-arm-build-and-private-trial-bundle)
records the immutable copies and retrieved private trial archive. These are
build results, not device performance or installation.

After an initial SSH timeout, the new candidate was installed with both device
hashes matching the build. Its ordinary Exit changed native property and
persistent configuration from `Y8` to `Y8INV`, with a user-confirmed dark
native Library. Next startup captured native `Y8INV` without double inversion.
The new resident cycle kept PID 11804 and this Reader's saved chapter 1500 /
scalar anchor 0 / dark mode 1; the user confirmed the same dark page and normal
page turns. The managed native-mode snapshot was `Y8INV`.

The normal Login X exited directly to a dark, touch-responsive native Library.
Final application/lock/service checks passed, dark mode 1, schema 2, all 118
nonempty bodies, and one cancelled task remained, and native property/config
still read `Y8INV`. SSH and the owned lease were closed, with BiliComics
directly informed of the final global mode. [Physical testing](PHYSICAL_TESTING.md#native-theme-and-login-x-physical-acceptance)
records the artifact-specific checks. Busy/code/key/error/failure Login states
retain controlled coverage; only one normal Login state was tested on the
device. Successful persistent readback does not establish a full reboot.
No result from the earlier `2dd3/971e` baseline is reassigned to this candidate.

Physical acceptance is scoped to the first-generation Scribe released in 2022
on firmware 5.19.5. The recorded Reader/wake/exit passes do not establish a
wider support matrix or nonempty note/bookmark retention. An actual
phone-side unlock followed by full-text replacement is still unverified until
the user obtains the right and the resulting device behavior is observed.
Synthetic fixtures do not establish that live transition or a wider device
support matrix.
