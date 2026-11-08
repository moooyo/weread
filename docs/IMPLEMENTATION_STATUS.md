# Implementation status

Recorded: 2026-11-03 (Asia/Shanghai).

The date above belongs to the original implementation record. Later changes and observations below are separated by artifact; no new run date is assigned here.

The user authorized implementation from the WeRead for Kindle design handoff v0.2. This repository now contains an independent Rust and Slint application, a desktop verification path, a Kindle platform adapter, and a development-bundle builder. Iteration continues and acceptance is incomplete. Limited physical observations on Scribe 2022 / firmware 5.19.5 do not constitute an accepted model/firmware support matrix or acceptance of the latest source.

The user subsequently approved moving from supervised tests to a normal-mode private daily trial. The real release ELF and standard **WeRead for Kindle** document launcher are deployed under `/mnt/us/weread` without demo/offline flags or an exit deadline. The old device test installation was removed after verified off-device backups, and the user confirmed the login screen. [Physical testing](PHYSICAL_TESTING.md#normal-mode-private-trial-deployment) records the deployment; authenticated-service observations and the remaining physical lifecycle gates retain their own scope.

The live trial identified an authenticated resource origin incorrectly treated as public, a resource 401 misreported as account expiry, and a book-scoped converter filename alias. These repairs were deployed without losing committed checkpoints. The affected request advanced from five to 86 entries, with two subsequent source integrity interruptions retained as failures. A later performance candidate resumed the request and reported 111/111, but the user then confirmed that the final chapters were previews on both Kindle and the same account's phone. [Verification](VERIFICATION.md#completeness-correction-after-the-physical-continuation) withdraws the whole-content conclusion and distinguishes persisted responses from authorized full text.

## Finite resource closeout

The user requested finite closeout without more full-domain discovery rounds.
The original two consecutive clean rounds were not achieved and are not
claimed. Necessary ordinary-flow and font corrections retain durable data,
old report proofs, drafts, and the deployed private trial.

The final controlled record passed complete library 1,481/0 with three ignored
manual benchmarks, Main 52/0, backend 65/0, Swash 15/0, and launcher 168/0.
Rust source/run/executable bindings remain at the original f8c1 checkpoint;
only a launcher-fixture correction changed the delivery identity to 58d8.
Exact unchanged-input comparisons allow that reuse without relabelling old
logs or resource samples. The original eight launcher failures are retained
with their strict fixture-window/precondition corrections.

The [active portrait implementation](ACTIVE_ORIENTATION.md) includes protected
startup/managed calibration, event-driven 0/180-degree holding, independent
manual application rotation, stale-contact cancellation, and a typed
written-frame/input gate. Its 28 cases passed in the actual complete receipts.
The matching static ARM/ZIP and current-source archive passed guarded remote
identity/content/CRC/permission checks. See [Verification](VERIFICATION.md#finite-resource-closeout-and-active-orientation)
for exact hashes, durations, resource trends, and local artifact links.

No current source was deployed or physically accepted in this resource task.
The existing `2fd3/799d` Kindle trial is unchanged. Private extreme-input
HTML/ZIP-footer/IVS overlays remain follow-up work, and product/device/license
acceptance remains incomplete independently of this bounded resource closeout.


## Resource review historical checkpoints

The chronology below records discovery-time source and verification states;
its pending-work statements are historical, not additional closeout gates.

R30 completed four fresh source domains with thirteen recommendations:
RL96-RL101 and seven existing-family consumer extensions. All six implementation
groups are source-frozen; the first remote compilation, session 18174, failed
with five App type/inventory/tuple bridge errors in 51.623 seconds. No test,
full suite, sample, backend, Swash, or changed launcher followed that failure.
The actual post-format source is `cc9d9559d134765d08223448220053846805408757f23ea864dbbf0dc80d4761`
over 524 leaves. Those narrow bridges were refrozen. Corrected session 33663
compiled in 131.559 seconds, but its library target passed 876 and failed seven
in 150.31 seconds, with two ignored and 561 filtered. Its source is
`a2e299bfadebc9c3f8f6ef58f58a544e770ba6a500db749269f94f5510820243`
over 524 leaves. Main and complete verification remained unexecuted. Exact
fixture/test-entry corrections are being refrozen without relaxing budgets or
assertions. Clean is zero; historical checkpoints do not establish a current
R30 pass or final ARM/bundle acceptance.

The completed RL01-RL85/extended-RL79 checkpoint passed 1,093 library cases and
23 Main cases on `test-env`, with three manual benchmarks ignored. Its same-run
533-case target and 20-case Main font target passed too. R27 then accepted eight
source recommendations: RL86-RL88 plus omitted consumers in five existing
families. Their isolated implementations are formally refrozen with 44 new
library and seven new launcher regressions. Two compilation checkpoints failed
on a private Snapshot Default and a test-only Path import; both mechanical
corrections are retained separately and refrozen. Corrected2 compiled in
114.61 seconds and confirmed all 44 new library names in its compiled test
list. It passed 577 library targets, 20 Main font targets, and complete Rust
suites of 1,137 library cases plus all 23 Main cases. Its actual post-format
source identity is
`eef2b4e0d112520a84d5be005239ff906e27012552a2c1e376087ffb9f7dcc3b`
over 277 leaves. The parent driver exited 1 only after its successful Rust suites
when a formal-case log matcher rejected interleaved diagnostics. A supplementary
same-log receipt confirms all 44 new names and results; no Rust run was repeated.
The changed 165-case launcher remains unexecuted. R28 completed four fresh source
domains and accepted ten further recommendations, RL89-RL92 plus six known-family
caller extensions. Their isolated fixes and producer regressions are being
refrozen. This review still needs two consecutive
complete clean rounds on one unchanged passing candidate; clean is zero and
no resource candidate has been deployed to Kindle.

Resource corrections RL01 through RL65 are integrated. That frozen checkpoint
passed 850 library cases and 23 binary-entry cases on `test-env`, with three
manual benchmarks ignored. Its target passed 279 library cases and 20 Main
font cases. Round 23 accepted RL66 through RL71 for NCX entity expansion,
image-anchor work, weighted progress, keyboard admission, operation-local
account snapshots, and Total chart output. Their first targeted run passed 342
cases and failed an existing historical-note view regression before Main,
complete suites, or sampling. Its narrow correction is refrozen with two new
callback cases. Round 24 accepted six further QR, response-output, XML,
downloaded-source, and checked-overflow recommendations (RL72-RL77). The ZIP
condition applies to the actual development/test build; the current release
already returns an error. Round 25 added RL78-RL83 with 84 prepared library
regressions. Its corrected2 compilation passed, but the target passed 496 cases
and failed three UI regressions before Main, complete suites, or sampling.
The whole-page hit admission and closed-TOC chapter-model consumer tails are
refrozen with a test-only drawing-token adaptation, retaining pixel, anchor,
and bounded ID-copy assertions. Corrected3 compiled and explicitly passed
those three cases; its target passed 498 cases and failed one bulk snapshot
identifier/stamp assertion before Main, complete suites, or sampling. The
clean count is zero. The stopping condition
requires two consecutive complete rounds without a new recommendation. The
unchanged launcher passed its complete 158-case remote regression. Final
ARM/package and source identity evidence will be recorded in
[Verification](VERIFICATION.md#resource-lifetime-candidate). The physical
`2fd3/799d` installation has not been replaced by this review.

R26's complete fresh source review added two mechanisms, RL84 Publications
filtering temporaries and RL85 local statistics daily expansion, plus an RL79
SVG-guard consumer extension. Three isolated implementation groups and the
shared BookAssets caller are formally refrozen with 34 new library regression
functions. They have not run and are not a new validation pass; the next unchanged candidate still needs full
remote verification and two clean reviews.

## Priority-1/2 download implementation

The requested priority-1/2 download work is implemented: Reader/JSON compression,
task TXT hints, two-shard reads with session-safe serial fallback, byte-bounded
prepared-image reuse, worker metadata parsing, atomic body/cursor/count updates,
terminal task compare-and-save, and reduced UI/cache work. Remote verification
passed 334 correctness cases and three explicit controlled benchmarks. ARM
`4befdb9a32b1` was installed on the physical Scribe and continued 25 remaining
entries in about 33 seconds; those responses were later identified as previews.
This is not a full-book comparison or evidence of complete chapter content.
[Performance](DOWNLOAD_PERFORMANCE.md) and
[Verification](VERIFICATION.md#priority-12-download-performance-implementation)
separate implementation, synthetic measurements, and pending physical gates.

## Content access correction

The requested source, domain, store, download, and UI correction is implemented.
Its unlock-only remote baseline passed 386 library tests, and the `2dd3`
resident baseline subsequently passed 418 root-library tests with zero failures
and three ignored benchmarks; the main target had zero tests. Formatting was
applied, and the earlier 462-key interface-resource check passed. The final
source also corrects the observed public SSR client-clock placeholder while
retaining strict server-clock and permission-evidence binding. It separates
`Unknown`, `Full`, `Preview`, and
`Locked` content states;
neither a nonempty response nor a complete image manifest is a full-text grant.
Legacy WeRead bodies remain readable but become unverified until checked,
and only confirmed full bodies count toward a completed full-text download.
Preview checkpoints do not advance the full-download cursor. `Preview` also
covers an unpurchased trial edition: a complete sample chapter is not evidence
of rights to the entire book.

The accepted interaction is to show the current content state, let the user
obtain the required rights in the phone application, then explicitly refresh
permissions and fetch full text. Replacement retains reading positions,
bookmarks, and notes, accepting only checked exact prefix extensions and
preserving old data on conflicts. Permission, unverified-content, and
content-change failures pause for explicit refresh without automatic network
retry or a login prompt. No automatic purchase or payment is part of
the correction. [Content access](CONTENT_ACCESS.md) records the implementation
contract and evidence limits. Four light-state captures from an earlier
desktop artifact were visually reviewed; they are not final-source or
dark-mode device evidence. [Verification](VERIFICATION.md#content-access-correction-remote-checks)
records the test log and separate render identity. The unlock-only release ARM
cross-build succeeded with SHA-256
`546efae824dbf9e74658f19ddd46520ff5531de2ed4ab6b61040472b0ea7ee3f`;
it is a stripped static ELF32 ARM EABI5 hard-float executable. SSH connectivity
has returned, and a predeployment check found the old `4bef` executable with
native UI restored. An early SCP failed during idle sleep, but the final
combined `2dd3/971e` candidate was subsequently installed after another wake;
the `546e` unlock-only artifact was not deployed. A real
phone-unlock-to-full-content test remains pending.

## Resident managed suspend

The user requested returning directly to the original Reader after wake.
Source inspection confirmed that the existing save/terminate/cold-start path
caused a new process and startup feedback. The resident replacement is
implemented: persist state and pause jobs/accounting, release the input grab
and inhibit drawing before acknowledgment, then stop the verified process.
Confirmed wake continues the same process and requests a full RGB redraw,
retaining its fonts, paginator, and Reader context.

The final combined source passed 418 root-library tests with three ignored
benchmarks, separately from the prior 414-test combined snapshot. The vendor
backend passed 47 tests and the launcher passed 65 fixtures in 277.698 seconds,
including the owned-orphan cleanup case. The four final source regressions
followed an anonymous public-page metadata check and did not change backend or
launcher source. They are not private-account entitlement evidence.

The 386 passing tests and `546e` ARM build belong to the preceding unlock-only
snapshot. The final combined release ARM build completed in 3 minutes
32 seconds with SHA-256
`2dd3bfc996d9c2161df155b26649e1fa6714dd9eb2c58068ef21e655c8e5d656`.
The launcher has SHA-256
`971efb417b4af5d6ec5111dcfa6b98bf4a0b032fd933303102433f3551c6c54c`.
The private trial archive is
`dist/weread-kindle-0.2.0-content-access-resident-trial.zip`, with SHA-256
`71f0d8b0a7488ad26974fd31e967b1d7892f07794e0b1b291e9da796d33d9201`.
[Verification](VERIFICATION.md#combined-arm-build-and-private-trial-bundle)
records immutable remote artifacts and the build log. That ELF and launcher
were installed on Scribe 2022 / firmware 5.19.5 with both device
hashes matching the build. Normal launch showed Library; an injected cover tap
opened Details. Schema 1-to-2 migration retained all 118 nonempty bodies and
both position rows, kept a cancelled task cancelled, and paused two legacy
completed tasks for revalidation. Note/bookmark tables were empty, so this
run does not establish their nonempty-record preservation.

The subsequent three cycles retained PID 28138, generation 1, and `/proc`
start time 703837374. Cycle 1 recovered Library; the user's cycle-2 power-button
check confirmed the same Reader page, no startup page, and working touch in
light mode. After night mode was explicitly enabled, cycle 3 retained chapter
105 / scalar 496 / dark mode 1; the user confirmed the same black/white Reader
page and normal page turns. The user then voluntarily exited and confirmed
native Library touch; process/lock/service checks and the owned-lease handoff
completed. These Reader/wake/exit checks are accepted for `2dd3/971e` on this
model/firmware. [Physical testing](PHYSICAL_TESTING.md#same-process-sleep-and-reader-recovery)
records the scopes and the earlier light-mode correction.

Live revalidation also replaced two stable-UID preview bodies with full text:
192 to 11,613 scalars and 181 to 9,058, with two completed tasks. No phone-side
unlock action was observed. After the user's confirmed logout and fresh QR
login, permission proof was invalidated and all 118 bodies were retained as
unverified; four tasks were paused. That account-operation result is not an
ordinary Exit defect. Nonempty note/bookmark retention and an actual phone
unlock transition remain pending. Historical cold-resume results retain their
earlier scope.

## Native night-mode handoff and Login close candidate

After the accepted exit, `prefs.dark_mode=1` remained stored, while native
`epdcMode` and `get-dynconf winmgr.colorinverse.pref` returned `Y8`. Thus the
verified `2dd3` Reader still used application-only night rendering.

The new requested candidate is implemented separately: synchronize native dark
mode only after successful ordinary exit, compensate native/application
inversion with XOR at launch and wake, and add a painted 48-by-48-pixel Login
close X. First use inherits a known native mode; stored preferences are
respected. Demo, unknown-mode, USB, and fault paths do not write the native
theme. Login close preserves stored account material. [Native theme continuity](NATIVE_THEME.md)
records accepted behavior and global inversion effects on other direct
framebuffer apps.

Its core suite passed 440 tests, zero failed, with three ignored benchmarks
and no main-target tests. Three targeted closing cases passed as an overlapping
subset; 463 translation keys passed their check. The launcher passed 86
fixtures with no failures or ignored cases in 359.502 seconds. Earlier
proxy/paint/host-shell diagnostics were repaired without removing assertions.
[Verification](VERIFICATION.md#native-night-mode-handoff-and-login-close-candidate)
records the logs and keeps the unchanged vendor's earlier 47-test scope
separate. The 418/47/65 remote results and `2dd3/971e` physical Reader/wake/exit
passes belong to the preceding implementation.

The release ARM build completed in 7 minutes 7 seconds, including a full
target-dependency rebuild. It is a stripped static ELF32 ARM EABI5 hard-float
executable with SHA-256
`2fd3d6ab2d648aec2671d0d2305a5e5ebe30fa6fd5e51f11705324d561317011`.
The launcher has SHA-256
`799d5bba5eb9e7c2b534437a1112e6810e5c85d73582742d770c96c6f72af234`.
The retrieved private trial archive is
`dist/weread-kindle-0.2.0-native-theme-login-trial.zip`, with SHA-256
`f6c3006f2d16b9663eedf11a7a167b0166a39ac3c3f92b4f1f84d01d12f3b5f4`.
[Verification](VERIFICATION.md#native-theme-arm-build-and-private-trial-bundle)
records immutable artifacts and the build log; build elapsed time is not a
device-speed measurement.

After the initial SSH timeout, `2fd3/799d` was installed on the identified
Scribe 2022 / firmware 5.19.5 with both device hashes matching the build.
Initial native property/configuration were `Y8` and saved application dark
mode was 1. The user confirmed that ordinary Exit changed the native Library
to dark; both native and persistent readback became `Y8INV`, with launcher
`persistence=confirmed`.

A normal reopening captured `Y8INV` in PID 11804 / generation 1 and displayed
correct startup and Library colors without double inversion. A managed sleep
and wake retained that PID and completed cycle-1 acknowledgments, with managed
native mode `Y8INV` and this Reader's saved chapter 1500 / scalar anchor 0 /
dark mode 1. The user confirmed the original dark page and working page turns.
The normal Login X also exited directly to a dark native Library with working
touch. Final process/lock/service checks and the owned-lease handoff completed;
BiliComics was explicitly told that global native mode was `Y8INV`.

These scoped native handoff, startup/wake compensation, and normal Login X
checks pass for `2fd3/799d`. [Physical testing](PHYSICAL_TESTING.md#native-theme-and-login-x-physical-acceptance)
records the backup, process snapshots, preserved counts, and boundaries.
Persistent configuration readback is verified; a whole-device reboot, a
cross-model matrix, and every busy/code/key/error Login state are not implied.
Those Login variants retain their controlled-test scope. USB and an observed
phone-side unlock transition keep their separate limits.

## Accepted implementation baseline

- Rust and project-owned Slint components; one application executable and local data directory.
- Separate presentation, application controllers, source adapter, SQLite/cache services, text layout, and Kindle platform responsibilities.
- The supplied grayscale and geometry tokens in `assets/design-tokens.json`. Kindle uses two physical pixels per logical unit. Desktop captures use one pixel per logical unit for inspection.
- Paperwhite and Scribe layouts, with additional `pw5` and `scribe2025` geometry profiles. Scribe uses more content slots rather than enlarging every control.
- Direct upstream integration and local persistence. No Jobman runtime, KOReader plugin, custom application server, PostgreSQL, or S3 service is required.

## Implemented code

| Area | Result |
| --- | --- |
| Application and navigation | Four primary tabs, independent search/detail/reader/article routes, return context, confirmation dialogs, fixed touch targets, pagination, and persistent preferences. |
| Login | QR generation and polling, four-digit verification, API Key entry, renewal, expiry handling, logout, and account-generation filtering for asynchronous results. The 48-by-48 close X cancels and fences work through ordinary exit, separately from logout. A normal Login state passed physical exit; busy/code/key/error/failure variants retain controlled coverage. QR success is never simulated by tapping the image. |
| Library | Cover and list modes, local cached metadata, sorting/filtering, selection, local additions/removals, acquisition progress, and explicit offline behavior. Local shelf overrides survive later upstream synchronization. |
| Search and input | Upstream catalog search when supported, local shelf search, query history, a real offline pinyin engine with 65,105 AOSP entries, candidate paging, English/numeric modes, and touch copy/paste. |
| Book details | Metadata, introduction, chapter pagination/selection, reviews, local notes, and acquisition dialogs. Timestamps alone do not claim new unread chapters. |
| Downloads | Serial durable tasks, per-account ownership, chapter checkpoints, cancellation, transient-error backoff, and interruption recovery. The content-access correction separates full content from previews and unverified bodies, pauses permission failures, and enables explicit revalidation. The `2dd3` candidate passed its 418-test root suite and live replacement of two preview caches by full text. An observed phone-side unlock transition remains pending. Unknown legacy task ownership is not silently assigned to another account. |
| Book images | Source-derived TAR/ZIP and remote image contracts, bounded acquisition, background raster processing, white-background compositing, 16-level grayscale PNGs, a separate pinned image cache, and text-bound image manifests. The UI acknowledges each completed local chapter before the worker continues. |
| Reader | Actual Unicode-aware pagination and font metrics, chapter headings, configurable font/weight/size/margins/spacing, character anchors across reflow, touch zones, horizontal swipes, bookmarks, local highlights/notes, exact quote overlays, and inline images. The notes panel is off by default, including on Scribe; the source updates a held drag selection at an 80 ms cadence. The current physical run confirmed held cross-line/reverse selection, the release menu, and saving a new highlight; details are recorded below. |
| Reading synchronization | Source-position mapping through stable chapter identities and word counts, fresh cloud progress, explicit conflict resolution, and stale-response rejection. |
| Reading time | Actual active Reader seconds, durable activity/outbox state, 60-second batches and final tails, account isolation, fresh progress checks, acknowledgement-based confirmation, and no automatic replay of uncertain delivery. |
| Publications | Cached subscriptions/articles, local unread state, article pagination, separate article typography, on-demand grayscale pictures, local favorites, and original-link phone QR codes. |
| Statistics | UTC+8 local sessions and source-derived cloud statistics, week/month/year/total views, ranking/calendar/preference rendering, and explicit unknown or unavailable cloud values. Local and cloud figures are not silently mixed. |
| Local import | UTF-8 TXT and unencrypted EPUB text, ordered spine/navigation handling, safe archive paths and decompression limits, and stable content identities. Rescanning the same content preserves local reading progress. |
| Kindle platform | Fixed-scale profiles, reviewed framebuffer backend, a post-render refresh queue, deferred keep-awake guards, bounded LIPC controls, validated battery values, and an application-memory clipboard. The `2fd3/799d` candidate passed native dark-mode handoff, persistent readback, next-start XOR compensation, and same-process Reader wake on Scribe 2022 / firmware 5.19.5. Whole-device reboot and broader model support remain unverified. |
| Launcher and bundle | A separate launcher and provisional package identity, exact profile/firmware allowlist, state preserved outside replaceable app files, and cleanup traps. The historical cold-resume baseline uses generation-bound sleep-save acknowledgments and restart at a stored Reader anchor; the implemented resident replacement has separate remote results and is now installed for physical checks. The product bundle no longer requires a Wi-Fi helper. Complete physical lifecycle acceptance remains open. |

The source adapter is pinned to the reference recorded in [API reference](API_REFERENCE.md). The hardware adapter retains its upstream licenses, exact base revision, and project patch notes. Source inspection and controlled tests establish implemented contracts; they are not live-account or physical-device acceptance.

## Product scope after user cancellation

The user removed the pull-down panel and application-owned Wi-Fi. They are no longer current product capabilities or pending deliveries, and their helper is no longer a product build/package dependency. Reader brightness/night controls, startup feedback, continuous selection, and sleep recovery remain. Product scripts build and archive the main Rust ELF, launcher, configuration, and existing notices. The restored baseline packaging tests cover missing-main, payload/permissions/digest, and runtime/private-input exclusion.

The cancellation version passed 245/245 core tests, 3/3 baseline packaging tests, and 133/133 legacy desktop UI checks. The desktop debug hash was `aecaa494c4099283574767014e76a28ec9546c3805ab4538396708297ee8d314`; ARM `6f6267a87dd2877bcd73860cc899d4dc2adad8e94fc60c30dcaf24b05dfe6d76` was a stripped static ARM ELF without a dynamic section. Fresh staging had no helper, and reused staging has a targeted obsolete-helper cleanup.

That ARM hash was checked, installed, and launched on Scribe 2022 / firmware 5.19.5 as `experience-28240`: the running executable matched, state was open / generation 1, geometry was 1860 x 2480 at scale 2, the log had no error, and two annotations remained. The canonical and three older helper files were archived outside the app under testlogs; OS network settings were unchanged. User confirmation of the new top-swipe behavior and a full retained hardware sleep regression is still absent, so overall acceptance remains unfinished. [Verification](VERIFICATION.md#product-scope-after-user-cancellation) and [Physical testing](PHYSICAL_TESTING.md#cancellation-version-physical-launch) hold the detailed record.

## Removed Wi-Fi helper experiment

The removed experiment used a separate project-owned C helper because the
static musl main ELF could not load the firmware's glibc `liblipc`. It used
bounded private binary pipes and fixed reviewed interoperability declarations.
This introduced no Jobman/KOReader runtime, bundled firmware libraries, or new
server. The experiment is historical; the Rust/Slint architecture and unselected
project license remain unchanged.

During the experiment, `zig cc` targeted ARM hard-float glibc 2.17 with
`-O2 -std=c11 -Wall -Wextra -Werror -ldl` and no test macro. The helper was staged
as executable and included in the archive's explicit inputs and digest sidecar.
Those requirements have been removed: current product building does not compile
or stage the helper, and current packaging does not require, include, mark, or
hash it. The recorded experiment source passed 283 core tests and three packaging
regressions before this removal; those results do not validate the modified
baseline packaging tests. Data, runtime state, and private research directories
remain excluded from packaging.

Helper-only evidence includes 17 synthetic checks and the earlier inspected
candidate's `GLIBC_2.4` requirement. Read-only inventory of the identified Scribe
2022 / firmware 5.19.5 confirmed glibc 2.35, `/lib/ld-linux-armhf.so.3`, and
`/usr/lib/liblipc.so`. The corrected helper identified as `6977` used the same
source with a different build path from the staged helper. Its real read-only
probe reported protocol 1; `available`, `status_ok`, `scan_ok`, `enabled`,
`connected`, and `has_ip` were true. It observed 23 cached records with
`cached_list` coverage and null status/scan diagnostics. This observes the
existing association and cache, not a connection established by the application.
No real `scan-start`, `connect`, profile creation, or global-enable operation has
been tested. Private network records are not included here.

The earlier `1d4e` helper rejected empty optional `enterprise` flags in open
network rows. Preserving these fields as `UNKNOWN` fixes response parsing; it
did not change network settings. The own-list/password/confirmation flow had
source and mock coverage before cancellation but was not accepted on hardware.
`095f` remains the preceding physical overlay candidate. Artifact hashes and
results below are retained as removed-experiment evidence, not current support.

<a id="current-iteration-and-evidence"></a>

## Recorded experiment and earlier baseline evidence

The following source/mock, helper, and VM results predate feature removal. They retain their original scope and are not post-removal verification or a pending Wi-Fi product delivery.

| Evidence | Observed result and limit |
| --- | --- |
| Recorded experiment source and packaging | 283 core tests and three packaging regressions passed on `test-env` before feature removal. The recorded 31 backend tests belong to the earlier backend baseline and are not added to this core total. |
| Legacy UI and compact Quick | 133 legacy UI checks and 54 compact checks passed with production SHA-256 prefix `633ac72` and private harness prefix `680583d`. These are earlier desktop identities, separate from the corrected Wi-Fi run. |
| Corrected Wi-Fi UI and privacy | 26 of 26 mock UI cases and 37 independent privacy checks passed with production `8b387e97da3b92f97aedfe178f04e4280b21063b82fa0e0fc8366cd8b7581e6a` and private harness `c35bc3f6d2aa5b3b948d233868256e4a06a442ad3224a66c2d6bf70c5a16e6c5`. The x/y-origin and named-fixture pre-shutdown observation fixes preceded this run. Earlier 13/12-failure diagnostics remain; assertions were not relaxed. Accessibility is source-reviewed only. |
| Removed experiment ARM/helper VM cases | Main `582b72345d863e918e403457cd32ef25c1efc8b8dce314027930df6f2fc0880d` and helper `3ec60abc4b335cfbbd23bfe4265aadae67fd52f55446a4da5eabe6c72f75f4da` completed the scoped Scribe 2022 / firmware 5.20.1 VM cases before cancellation. Fifteen overlay/list snapshots kept PID/generation and framebuffer/input counts stable with native panels unmapped; Reader ROI return was unchanged, dark persisted, and guest powerd wake restored anchor 228 in generation 2 with `position_confirmed=false` retained. Open-network confirmation was shown without clicking Connect. Managed cleanup completed, but its numeric exit code was not recorded. These artifacts were not deployed to the physical device and are not pending product delivery. |
| Recorded launcher fixtures | 50 of 50 isolated fixtures passed in 205.691 seconds with launcher SHA-256 `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`. Evidence is under `test-env:/tmp/weread-native-queries.iYZFwu/launcher-fixtures.log`. Normal, sleep, USB, generation, and native-query regressions are included. This is fixture evidence only, not verification of the new helper packaging. |
| Preceding launcher fixtures | The preceding revision passed 48 of 48 isolated fixtures in 199.782 seconds under `test-env:/tmp/weread-native-failure.pwWnI0`. This remains a separate historical result. |
| Desktop UI and clipboard | 133 UI checks and five clipboard checks passed with `e11d74c23e309f2aa4e1daa6c4e33fe3d611f747d7e5c904a40bd2537eb24b4c`. This binary predates the latest checkpoint regression fix; Reader source was unchanged by that fix. These runs do not establish the latest full lifecycle behavior or the new helper-backed Wi-Fi UI. |
| Earlier physical Scribe 2022 / firmware 5.19.5 | In demo/offline mode, the earlier `61fa9afd` candidate showed the library, Reader page turns and font-size changes, explicit exit, and one saved local note persisted to storage. Its old sleep path exited with 143 and restored stock UI touch control. This is not a same-position resume result. |
| Recorded physical `2b39140b/0226f613` | Both hashes matched on the Scribe 2022 / firmware 5.19.5, and the candidate ran in demo/offline mode. The user confirmed startup feedback, hidden notes sidebar, continuous cross-line/reverse finger selection, the release menu, saving a new highlight, and dark body/menu appearance. The default native Quick pull-down detoured through stock bookstore/Home before returning to the app; the user rejected that default UX. See [Physical testing](PHYSICAL_TESTING.md#current-physical-experience-with-2b39140b0226f613). |
| Later physical overlay `095f` | `experience-14640` reached the 900-second watchdog, exited 143, and restored Home. It was not manually stopped by the operator and was not a voluntary UI Exit. The user still requested simplification and reported Wi-Fi reaching Home; the experience was not full UX acceptance. |
| Intermediate physical candidate | The `9a13abe6` candidate introduced startup/default-body/sleep-recovery work, but the user found drag highlighting inadequate. It is not continuous drag-selection acceptance. |
| Native panel controls on the physical device | Independent LIPC toggles displayed and closed Wi-Fi and Quick Settings. The application's complete handoff and return were not accepted. |
| Recorded Scribe 2022 / firmware 5.20.1 VM | ARM `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` with launcher `0226f613` passed scoped startup/dark, continuous selection, Quick Settings return, saved-anchor wake, and Wi-Fi return cases. Reader/Settings content ROI differences were 0 for their respective returns; light/dark Reader text-mask agreement was 1.0. Explicit exit ended the app, removed the lock, and restored a running status bar; no process exit code was recorded. These are not results for final ARM `582b`. |
| Earlier VM attempt | Binary `daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3` produced selection captures, but its native-panel attempt did not complete a return. That incomplete attempt is retained as history and is not evidence that native QMP input is unsupported. |
| Recorded 2b ARM candidate | `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` built successfully. `file`/`readelf` identify a stripped static ELF32 ARM EABI5 executable with no dynamic section. It has run in the VM and on the physical device with launcher `0226f613`. These records do not establish acceptance of the later overlay or helper-backed Wi-Fi integration. |

The native observer, mapping timeout, own-UI flows, and helper results are historical after user cancellation. The experiment VM used one fake cached access point, separate from the physical helper probe's 23-record cache. No current Quick/Wi-Fi capability or remaining connection-acceptance task is claimed. Those older results did not accept retained Reader night/sleep behavior or voluntary UI exit; the later `2dd3/971e` physical checks above now accept those scoped paths. USB is still skipped. See [Physical testing](PHYSICAL_TESTING.md) and [Verification](VERIFICATION.md#final-arm-scoped-scribe-vm-cases).

The earlier `experience-10179` stop was a separate operator-managed TERM result for `2b`: exit 143, no watchdog trigger, running `statusbar`, `awesome` in state S, and no lock. It must not be used to explain the later `095f` watchdog stop. Neither accepts voluntary UI exit. The earlier saved highlight and dark-appearance observations remain valid in their own scope; private identifiers and text are omitted, and the later light preference is not treated as a persistence defect.

The removed experiment's VM work completed and the VM was returned to the operator. `582b/3ec60` is no longer queued for physical delivery. The retained mock report is `test-env:/root/weread-emulator/artifacts/wifi-final-wifi-ui-v3/run-20261104T085827Z-208619/report.json`, downloaded under `artifacts/wifi-final/`. Its VM report is `test-env:/root/weread-emulator/artifacts/wifi-app-native/run-20261104T090442Z/report.json`, with `preview.png` beside it and downloaded copies under `artifacts/wifi-final/native/run-20261104T090442Z/`. The recorded `passed_with_explicit_scope_limits` result and unverified real-network/physical boundaries remain historical, as detailed in [Verification](VERIFICATION.md#current-helper-backed-wi-fi-candidate).

VM Home recovered after a documented VM-only boot-environment change. The old six-case emulator result did not use this change. [The Scribe emulator follow-up](SCRIBE_EMULATOR.md#current-environment-recovery-after-the-six-case-run) records the notification-command adjustment and the subsequent `2b39140b` software cases. An early close tap at about five seconds had no effect; the same coordinates closed the fully drawn panel later without XInput, input-grab, or driver changes. This does not establish an input-recovery defect or a general QMP limitation. See [Verification](VERIFICATION.md) for generation transitions, saved anchor, ROI comparisons, and report paths.

## Use and verification

From Windows PowerShell, run `scripts/verify-remote.ps1` to copy only project sources and verify through `ssh test-env`. It performs no local application build or test. `-Kindle` also stages the ARM development bundle. The remote host needs Rust, C build tools, Xvfb, Pillow, and cargo-zigbuild/Zig for the Kindle build.

The application accepts `--device pw5|pw6|scribe|scribe2025`, `--data-dir`, `--demo`, `--offline`, `--screen`, `--capture`, `--exit-after-ms`, and `--actions`. The action file is an explicit developer fixture input; normal use is through the Chinese interface. Captures contain actual rendered pixels and geometry metadata.

Default data is stored in `%APPDATA%/weread` on Windows and `$XDG_DATA_HOME/weread` or `~/.local/share/weread` on Linux. `--demo` uses a separate `demo/` child. The demo contains public-domain texts and one locally generated illustration; it does not make authenticated upstream writes.

Put TXT or EPUB files in the active data directory's `imports/` folder and use the scan/import action. User TTF/OTF files in `fonts/` are discovered on restart. Bundled Sans faces are 400/500/700 and Serif faces are 300/400/500/700, with matching layout metrics for each actual face. WenKai remains unavailable until a suitable user font is present.

See [Verification](VERIFICATION.md) for artifact-specific commands, results, and their evidence boundaries, and [Kindle packaging](../packaging/kindle/README.md) for supervised device setup.

## Persistent data behavior

SQLite stores metadata, chapter text, positions, local annotations/bookmarks, history, preferences, and resumable work. Covers/article thumbnails have a 96 MiB evictable file budget. Explicit book pictures use a separate 512 MiB cache and are pinned while referenced by manifests. Cache failure never counts as completed acquisition.

Book-image anchors are bound to the exact canonical body digest. Catalog replacement follows stable chapter identities rather than trusting changed list indices. Removed chapter notes/bookmarks remain preserved as unresolved locations. Local shelf adds/removals, notes, favorites, and bookmarks are device-local; the inspected upstream did not establish all corresponding remote-write contracts.

Credentials are stored only in the chosen local account file. POSIX private-directory/file permissions are used where supported. Kindle FAT storage cannot enforce POSIX mode bits; this is local file storage, not an encrypted credential vault. Credentials, real content, personal data, and research archives are excluded from source and development bundles.

Reading-time accounting never substitutes application uptime, background-page time, or restart wall-clock time for observed reading. After 300 seconds without reading interaction, activity counting stops until interaction resumes. `report_reader_only` controls when delivery may start; counting always requires active Reader time. Because the inspected time endpoint also writes position, delivery requires permission to write cloud progress and sufficient current-account position evidence. A timeout after transmission can be ambiguous: such batches remain uncertain and are not automatically resent. Local reading history remains available independently.

## Remaining acceptance and intentionally unavailable features

- Real-account QR login, session renewal, persisted responses, and two live preview-to-full replacements have been observed. An actual phone-side unlock transition, whole-library completeness, synchronization, and reporting acceptance remain incomplete. The legacy 111/111 label was invalidated by the preview discovery and must not be used as full-content evidence.
- The pinned PW6 and Scribe 2022 firmware 5.20.1 experiments exercise native ARM startup, QMP-injected taps, reading navigation, and launcher restoration; see [PW6](EMULATOR.md) and [Scribe](SCRIBE_EMULATOR.md). The `582b/3ec60` Quick/Wi-Fi guest cases remain a removed experiment. Scribe 2022 / 5.19.5 `2dd3/971e` accepts same-process light/dark Reader recovery; `2fd3/799d` additionally passes native dark handoff/readback, next-start/wake compensation, and a normal Login X exit. These results do not establish whole-device reboot persistence or a cross-model matrix. USB remains explicitly deferred.
- EMR pen tip/eraser, multi-touch pinch, and landscape/two-page reading remain unavailable until their platform contracts and device evidence exist. Native inversion synchronization is accepted only in the recorded Scribe model/firmware and artifact scope. The UI does not report simulated hardware success.
- Native clipboard contents are process-local. Desktop builds use their desktop backend's clipboard; the Kindle adapter does not claim integration with a stock system clipboard.
- EPUB import is text-oriented; a general EPUB/CSS, PDF, or protected-file engine is not implemented. Upstream chapter acquisition is not a promise of portable EPUB/PDF export or stock-reader handoff.
- Audio/video playback and long review authoring are outside the supplied v1 scope. Publication media is opened on the phone through the actual original URL when available.
- Remote shelf mutation and personal annotation/bookmark writes remain unavailable where the inspected reference did not establish a contract. Their local operations remain functional and persistent.
- There is no selected project license, published release channel, verified KPM release package, or automatic updater. Third-party notices and the provisional package identity are concrete review inputs; publication remains a separate decision.
