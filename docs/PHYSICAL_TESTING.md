# Physical Kindle testing

Development is continuing. The observations in this document do not complete acceptance of the latest source or establish a supported model/firmware matrix. Historical artifact identifiers and dates remain attached to their own results.

## Identified physical environment

- Device: first-generation Kindle Scribe released in 2022.
- Firmware: 5.19.5.
- Application mode: earlier isolated offline demonstrations, followed by a normal-mode private daily trial with a real account.
- Scope: supervised device iterations and private daily-trial observations, separate from the Scribe 2022 firmware 5.20.1 VM experiments.

No device address, credential, serial number, private note text, or account content is part of this record. Offline demonstration results do not establish live-account authentication, content entitlements, or upstream writes.

## Clean current-layout installation

On 2026-11-11 (Asia/Shanghai), the user authorized deleting this project's Kindle installation, application data, and related test residue without backups, then installing the newly verified build. Reauthentication and redownloading were explicitly accepted. The remote software gates in [Verification](VERIFICATION.md#pre-release-persistence-cleanup) completed before device access. The coordinator waited for the other project's natural lease release and acquired a fresh exclusive lease only for the actual installation window.

The target was the identified Scribe 2022 / firmware 5.19.5. The precheck found no active reading application or display lock, with native Library, running statusbar, and an unpaused window manager. Inventory found only `/mnt/us/weread` and the normal `/mnt/us/documents/weread.sh` entry in this project's application/document scope, with no matching scratch roots. The old application root occupied 98,376 KiB. That entire root, including the database, account file, downloaded content, previous programs, and logs, and the old document entry were deleted. No backup or private-content archive was created. The separate KOReader installation was outside this project's reset.

The first installation transfer exhausted `/var/tmp`'s 64 MiB memory-backed `tmpfs` before any old application data was deleted. The incomplete transfer was removed immediately. The verified payload was then staged on `/mnt/us`, which had about 20 GiB available. Both temporary transfer paths were absent at completion.

All 36 installed public files matched the remotely verified staging manifest. Installed ARM SHA-256 is `1a4bab8bd652e38e1bdff15de793628e3a202eb24a0a3941f7fda4d5bced0110`; launcher `a5fec72f` and frame guard `a235b41c` retain their existing identities. Fresh local configuration contains the exact `scribe` / `5.19.5` experiment pair and no compatibility override for another device.

The normal document entry launched exactly `--kindle --device scribe --data-dir /mnt/us/weread/data`, without demo, offline, scripted-action, capture, or exit-deadline arguments. The running `/proc` executable hash matched the new ARM. Startup reached `state-open`, created the current unversioned layout (`PRAGMA user_version=0`), and had zero books, download tasks, and chapter-state rows, with no account file. This proves clean initialization; it does not claim successful reauthentication, a completed download, finger-touch acceptance, or a new firmware support matrix.

The coordinator then stopped the launcher through managed TERM, verified all WeRead and frame-guard processes and display locks were absent, and removed the installation payload. Native Library, running statusbar, and the original running window-manager identity were confirmed. Both native EPDC metadata and the persisted native setting remained `Y8INV`. All device commands had exited before the owner/token-checked lease release at **2026-11-11 02:26:42.352999 +08:00**, followed by an independent lease-absence check. The application is installed but stopped after verification, ready for a later ordinary user launch.

Private command receipts are under `test-env:/root/weread-persistence-device-20261111-r1/evidence/`; they contain no old-data backup. Public build and software verification identities remain separate from any later authenticated use.

## Temporary-memory cleanup

The user's follow-up requested inspection of `/var/tmp` and removal of residue from the related development sessions. A fresh exclusive lease was acquired for this separate device-only window. `/tmp` was confirmed to be a symlink to `/var/tmp`; both use the same 64 MiB memory-backed `/var` filesystem.

Inventory distinguished seven old BiliComics barrier/pose probe scripts and result directories from native service files, logs, network state, X11 sockets, and the `chroot` tree. It also found the 7,547,448-byte `slint-kindle-default.ttf` runtime font. Reviewed backend source writes that font at every startup, so it can be removed while all reading applications are stopped and will be regenerated when needed.

Before deletion, all eight targets passed canonical-path, ownership, symlink, nested-mount, process-executable/cwd/root/file-descriptor, and memory-mapping checks. No reading application or display lock was active. Only those admitted targets were deleted. The measured free-space change was **8,508 KiB**, leaving **44,948 KiB free** at that sample; unrelated system state and the `chroot` tree were retained. No backup was created.

The native Library, original running window manager, running statusbar, and unchanged `Y8INV` metadata were confirmed. Device transports completed before the owner/token-checked release at **2026-11-11 02:32:38.277812 +08:00**. Private inventory and deletion receipts remain outside the repository at `test-env:/root/weread-tmp-cleanup-20261111-r2/evidence/`.

## Relaxed Reader budget comparison

The 2026-11-11 user-authorized comparison ran a separate native ARM harness on
the identified Scribe 2022 / firmware 5.19.5. All eighteen cases passed: three
fresh processes for each of six pool/parser/chapter-worker combinations. Each
used the same 32 synthetic chapters, approximately 1.35 MiB Reader pages, gzip,
and 90 ms fixture delay, with the loopback server on the Kindle itself.

At sixteen chapter workers, the 16 MiB/two-parser baseline took a median
2.569 s; 32 MiB/two took 2.567 s, with overlapping ranges. The larger pool
actually reached sixteen Reader HTTP requests, but fixture-process sampled peak
RSS median increased from 25.23 to 36.84 MiB. Four parser permits increased
median elapsed by approximately 5.5-6.9%. The default-eight comparison showed
no reliable gain. The [full record](READER_BUDGET_EXPERIMENT.md) preserves raw
repetitions, the first-wave barrier qualification, CPU/RSS scope, and limits.

Only a scratch executable and synthetic logs were staged. The installed
`dd8ac6c3` application, launcher, data, and settings were retained. Native
metadata matched, owned processes and scratch were removed, transports closed,
and exact lease release occurred at 01:20:25.081063 +08:00, followed by an
independent absence check. These source-fixture results do not establish full
application/UI memory, live-service throughput, or a hardware performance limit.

## Bounded independent-chapter follow-up

The 2026-11-10/11 follow-up used this same Scribe 2022 / firmware 5.19.5.
The native ARM Reader fixture compared one fixed-order sample at 1/2/4/8/16
chapter workers: 3,620/2,004/1,330/1,340/1,402 ms for sixteen synthetic chapters.
Actual Reader HTTP peaks were 1/2/4/8/8. Four and eight differ by only 10 ms;
this is not a statistical or production optimum. The fixture excludes the
application UI, images, persistence, and real upstream conditions.

After a fresh coherent private backup, production ARM `dd8ac6c3` replaced
`985efefc`; launcher `a5fec72f` and frame guard `a235b41c` remained unchanged.
The first ten-second startup attempt still observed schema 2 and its
stop script exposed the device's integer-only `sleep` utility. An immediate
identity-bound recovery corrected that script and verified native restoration
and exact lease release at 2026-11-11 00:04:30.682944 +08:00.

A separate fresh window confirmed schema 3 and captured 24.36 seconds at eight
workers. One bound task advanced from 288 to 309 complete selected chapters:
21 durable completions, 0.8621 chapters/second. Sampled RSS peaked at 241,632 KiB;
the process-lifetime HWM was 246,812 KiB. CPU was 82.88% of one core. Reader
reservations reached 16 MiB/eight admitted pages, with two parser workers.
Admitted pages include fetch/wait/parse lifetime; actual Reader-only HTTP
concurrency is unavailable in production logs. The download's HTTP and chapter
read lifetime peaks were eight.

The exact window contained 194 Reader-session HTTP-426 log events and 31
`-10102` occurrences. At t1, 31 chapters were waiting and 187 were blocked, with
the task still Running and ready work retained. Those are event/state counts,
not an upstream error explanation or a full-book completion claim. No sampled
completion count exceeded its contiguous prefix, so this window does not
independently establish physical out-of-order publication. The controlled
source/controller regressions remain the evidence for that case.

Four-worker sampling qualification did not complete. The initial follow-up
checked for a new download record before the new process had started work;
the corrected attempt later failed its full process/display identity gate.
These failures remain separate from application download evidence and cannot
be used to rank production concurrency. The last attempt's exact new log
segment still records a four-worker start, 30 checkpoints over a 29,643 ms task
lifetime, and cold progress from 309 to 339 complete chapters. It does not
provide the planned fixed-window process measurements or establish startup
failure. Original concurrency eight and native
dark metadata were restored. The later windows exactly released their leases
at 00:10:03.618297 and 00:14:03.544744 +08:00, with independent absence checks.

[Independent chapter downloads](INDEPENDENT_CHAPTER_DOWNLOADS.md) retains
artifact identities, anonymous numeric receipts, and the open physical gates.
Raw logs, task/account bindings, database copies, and backups remain private on
the designated worker. This source does not inherit the earlier user visual
and touch acceptance, nor does the bounded run establish sustained throughput,
automatic-retry completion, a statistically optimal setting, or other devices.

## Current native reading corrections

The 2026-11-09 isolated resource-review run confirmed active portrait rotation
and inverted page-turn input on the identified device. Its subsequent sleep
attempt exposed a display-release failure caused by the missing SSH `DISPLAY`
environment. The TOC alignment, immediate system-theme transaction, and default
display correction have their own current remote and physical gates in
[Native reading corrections](NATIVE_FIX_ACCEPTANCE.md). Those later changes do
not inherit acceptance from earlier daily-trial or resource-review artifacts.

The subsequent v10 isolated manual trial passed the scoped visual, sleep, and
UI-exit checks on this same device and firmware. Its matching normal-mode
installation was independently verified against all 36 public file hashes and
sizes. The user opened the existing document entry, confirmed the original
Library and books, then exited to a working dark native UI. Full normal
arguments, backup provenance, filesystem-mode observations, remaining
approximately five-second theme response, and final lease return are recorded
in [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md). No new content
entitlement, download, reboot, USB, or other-device claim follows from this
installation check.

## Live resource authentication and image aliases

### Performance-build physical continuation

After BiliComics explicitly returned the device and removed its shared lease,
WeRead acquired a new exclusive lease. The native precheck found both application
locks absent, no WeRead/BiliComics/Jobman GUI process, a running status bar, and
the native `com.lab126.KPPMainApp` foreground. Firmware remained 5.19.5.

ARM `4befdb9a32b1f1279d671cb54b76f3afd2568a6ea4d25d80483c647f8a55f57f`
was installed after backing up the account, metadata database, prior log, and
previous executable. The running process hash matched, with only
`--kindle --device scribe --data-dir /mnt/us/weread/data`. No demo, offline,
scripted-action, capture, or application exit deadline was enabled.

The previously failed illustrated request resumed from cursor 86. The log
recorded catalog HTTP 200 / `source.api.-2012`, followed by acknowledged renewal.
The legacy task then reported **111 of 111 selected entries**, with no terminal
error. Read-only aggregate checks found 111 nonempty bodies, 111 complete image
manifests, zero unresolved image sources, and legacy book cached/total counters
111/111. These counts establish persisted responses and resolved image bindings,
not full-text authorization or complete chapter content.
The explicitly cancelled request remained cancelled at its prior cursor, and
the separate single-chapter completed request was unchanged.

Credential comparison retained only booleans: account and Skill key unchanged,
Cookie material changed, and a web-session key still present. This establishes
that the acknowledged renewal was persisted without recording its values.

The interval from normal-entry launch to the legacy task's completion timestamp was about
33 seconds, including startup, catalog renewal, and validation of the cached
prefix. It covers only the remaining 25 chapters; it is not a cold full-book or
before/after speed comparison. A post-completion process snapshot reported
`VmHWM=242476 KiB` and `VmRSS=185080 KiB`, scoped to this startup/continuation run,
not a general device-memory ceiling. The 25 responses in this continuation were
later identified as previews, so this interval is not a measurement of 25 full
chapter downloads.

An operator-only, PID/birth/cmdline-bound five-minute restoration guard ended
the initial observation window normally. The user had not yet performed the
requested last-chapter check; this exit is not a product auto-exit defect or a
voluntary UI-exit result. At the user's request the application was restarted
through the same normal Library entry with a longer operator observation guard.
Generation 1, recovery disabled, and state-open were checked again. The ordinary
document entry itself remains untimed. User reading/exit confirmation and final
native restoration are recorded separately when completed.

### Preview discovery and corrected completeness conclusion

In the subsequent reading check, the user reported that the final downloaded
chapters contained only a short opening excerpt. Read-only length comparisons
found that zero-based chapter indices 86 through 110 contained 151 to 208 Unicode
scalars each, all ending in `...`. The decoded source HTML was already short;
the truncation was present before pagination. Sampled earlier chapters had long
bodies consistent with their catalog word counts, but those samples do not
prove every earlier chapter is complete.

The user confirmed that the same account on the phone could also only preview
the final chapters or was prompted to unlock them. This establishes a current
reading-rights restriction for the observed chapters. It does not identify a
Kindle pagination defect or authorize bypassing the source's entitlement gate.

The earlier whole-content conclusion is withdrawn: `full_content_verified=false`
for the `4bef` continuation. Its valid shard checksums, nonempty bodies, image
manifests, and legacy `111/111` counters did not distinguish a legitimate preview
response from full text. The application incorrectly presented those responses
as a completed download. No private book title, identifier, or excerpt is
included here.

Source review also found that the worker could retain any nonempty old body and
that the checkpoint protected that body against replacement. Together with the
completed-cache filter, this could keep a preview after the user later obtains
full reading rights. The corrective source, domain, store, download, and UI
changes are implemented. Their unlock-only remote all-targets baseline passed 386 library
tests with zero failures and three ignored benchmarks, separately from the
`4bef` results. [Content access](CONTENT_ACCESS.md) records the state and
revalidation contract, and [Verification](VERIFICATION.md#content-access-correction-remote-checks)
binds the tests and earlier light-state renders to their own evidence. The
unlock-only release ARM cross-build succeeded with SHA-256
`546efae824dbf9e74658f19ddd46520ff5531de2ed4ab6b61040472b0ea7ee3f`.
It is a stripped static ELF32 ARM EABI5 hard-float executable. The 386 tests and
this build predate the resident-suspend work below. The light desktop captures
do not establish physical display behavior or dark-mode operation.

After the user woke the device, SSH connectivity returned. The predeployment
check confirmed firmware 5.19.5, the old `4bef` executable, no active
application supervisor, the finished operator guard, a running status bar,
`awesome` in state S, and the native `com.lab126.KPPMainApp` foreground. This
does not establish fresh touch behavior. A later backup completed at
`/mnt/us/weread/backups/content-access-546efae824db`, but SCP failed when the
idle device slept again. The installed `4bef` executable was not replaced in
that attempt. The `546e` unlock-only artifact was never deployed; the later
successful combined installation is recorded below.

Actual phone-side unlocking followed by full-text replacement on the physical
Scribe remains unverified. No purchase, payment, membership subscription, or
automatic entitlement-changing request was performed by this project.

### Resident managed suspend iteration

The user then requested that waking the device return directly to the original
Reader. Source inspection confirmed the cause of the existing startup detour:
sleep saved state and terminated the application, and wake cold-started a new
process with startup feedback.

The resident replacement is implemented. It persists state, pauses
jobs and reading accounting, releases the backend input grab, and inhibits
drawing before acknowledgment. The supervisor then stops the verified ELF;
confirmed wake continues the same process and triggers a full RGB redraw. The
intended behavior keeps the same PID, loaded fonts, paginator, and Reader
context.

The final combined source passed 418 root-library tests with zero failures
and three ignored benchmarks. The backend passed 47 tests and the launcher
passed 65 fixtures, including owned-orphan cleanup, with no failures or
ignored cases. These are remote controlled results, not a physical sleep/wake
run. [Verification](VERIFICATION.md#combined-source-remote-checks) records the
separate prior 414-test snapshot and the final four source regressions prompted
by an anonymous public SSR metadata check. That check did not validate private
account entitlements or fetch chapter content.

The final combined release ARM build and private trial bundle are complete.
The ELF SHA-256 is
`2dd3bfc996d9c2161df155b26649e1fa6714dd9eb2c58068ef21e655c8e5d656`,
and the launcher SHA-256 is
`971efb417b4af5d6ec5111dcfa6b98bf4a0b032fd933303102433f3551c6c54c`.
[Verification](VERIFICATION.md#combined-arm-build-and-private-trial-bundle)
records their immutable remote copies and the private ZIP identity. The
386-test unlock baseline and `546e` build are earlier artifacts. Historical
cold-resume observations remain attached to their own candidates.

### Combined installation and migration observation

After the user woke the device again, the final `2dd3` ELF and `971e` launcher
were installed on the Scribe 2022 / firmware 5.19.5. A new backup is at
`/mnt/us/weread/backups/combined-2dd3bfc996d9`. Both installed files matched
their full build SHA-256 values. The ordinary document entry launched without
demo, offline, or automatic-exit arguments. PID 28138 was in state S with
`VmRSS=232884 KiB`; this snapshot is not a general memory ceiling. The
`managed-sleep-available` and generation metadata matched at startup. The later
completed suspend/wake observations are separated below.

SQLite migrated from `user_version=1` to 2 while retaining all 118 nonempty
chapter bodies. Stable ordered row digests for positions, annotations, and
bookmarks were identical before and after; their row counts were two, zero,
and zero respectively. One cancelled task remained cancelled, while two
legacy complete tasks became paused. All 1,974 catalog rows were `Unknown`,
including the 118 with bodies. These are scoped migration results. The empty
note/bookmark tables do not establish physical preservation of nonempty
annotation or bookmark records.

A private framebuffer capture, kept outside the repository, showed Library.
An injected ordinary cover tap initially opened Details rather than Reader.
The device naturally slept and SSH disconnected at that point. The subsequent
cycles below add Reader and wake evidence; that initial tap alone did not.
No private book title, identifier, content, or capture is included here.

### Same-process sleep and Reader recovery

Across three completed cycles, application PID 28138, generation 1, and
`/proc` start-time value 703837374 were unchanged. Sleep/wake acknowledgments
advanced through cycles 1, 2, and 3.

| Cycle | Trigger and observation | Accepted scope |
| --- | --- | --- |
| 1 | Natural sleep disconnected SSH; reconnection found the same process and completed cycle-1 acknowledgments, with Library restored. | Same-process Library recovery, not Reader-position or dark-mode evidence. |
| 2 | The user briefly pressed the power button while in Reader. The same process and cycle-2 acknowledgments were retained. The user confirmed the same page, no startup page, and normal touch. | Light-mode Reader recovery only: the recorded `dark_mode` was 0, and the user subsequently confirmed the white background. |
| 3 | The user explicitly enabled night mode and closed the menu. The operator used stock `powerd_test -p`, waited three seconds, then used it again to wake. During sleep, `powerd` reported `screenSaver` and PID 28138 was in state T. After wake, it reported `active`, with the same process and cycle-3 acknowledgments. | Before sleep and after saved Reader state both held chapter index 105, scalar offset 496, and dark mode 1. The user confirmed the same page, black background/white text, and working page turns. |

These observations accept the `2dd3/971e` resident Reader path for this device
and firmware, including the explicitly enabled dark Reader case. They are not
results for later native night-mode handoff changes or a wider support matrix.

### Full-text revalidation and account-operation scope

A read-only comparison matched stable chapter UIDs between the preupgrade
backup and the current database. At index 105, a 192-scalar preview became
11,613 scalars classified as full; at index 109, 181 scalars became 9,058.
Two download tasks reached complete. The corresponding catalog word counts
were 11,869 and 9,286; their similar scale is not a word-for-word checksum or
an independent completeness proof. No phone purchase or unlock action was
observed, so this is successful live full-text revalidation and preview-cache
replacement, not a verified phone-unlock transition.

The user also confirmed logging out and scanning a fresh login QR code. After
those account operations, the two full proofs were invalidated, all 118
retained bodies were unverified, and the two complete tasks contributed to
four paused tasks. All 118 nonempty bodies remained present. This state change
is associated with account operations and must not be reported as a normal
Exit defect.

### Voluntary exit and device handoff

The user voluntarily exited and confirmed normal native Library touch. The
operator then found no WeRead application process, no launcher lock, a running
status bar, `awesome` in state S, and native `com.lab126.KPPMainApp` foreground.
SSH was closed. After verifying ownership and its own lease token, WeRead
removed the host lease and directly notified BiliComics of the handoff. No
lease value or private account data is included here.

This completes the scoped `2dd3/971e` Reader sleep/wake and voluntary-exit
checks. Nonempty note/bookmark retention and an actual phone-side unlock
transition remain separately unverified. USB remains explicitly deferred.

### Native night-mode handoff and Login close candidate

After the accepted exit, `prefs.dark_mode` remained 1, but native `epdcMode`
and `get-dynconf winmgr.colorinverse.pref` both returned `Y8`. This confirms
that `2dd3` retained application-only dark rendering; it did not synchronize
the native system's night state on exit.

The subsequent candidate is implemented separately: synchronize native dark
mode only after successful ordinary exit, compensate application rendering
against native inversion with XOR at launch and wake, and add a painted
48-by-48-pixel Login close X. First use inherits a known native mode, while
stored application preferences remain authoritative. Demo, unknown-mode,
USB, and fault paths do not write the system theme. [Native theme continuity](NATIVE_THEME.md)
records these boundaries and the effect on other direct framebuffer apps.

Its remote core suite passed 440 tests with zero failures and three ignored
benchmarks, targeted closing tests passed three overlapping cases, and 463
translation keys passed their check. The launcher passed 86 fixtures in
359.502 seconds with zero failures or ignored cases. [Verification](VERIFICATION.md#native-night-mode-handoff-and-login-close-candidate)
retains logs, intermediate diagnostic failures, and the unchanged older vendor
scope; these are controlled remote results.

The native-theme ARM build completed in 7 minutes 7 seconds, including a full
target-dependency rebuild; that duration is not a device-speed result. The
stripped static ELF32 ARM EABI5 hard-float candidate has SHA-256
`2fd3d6ab2d648aec2671d0d2305a5e5ebe30fa6fd5e51f11705324d561317011`;
its launcher has SHA-256
`799d5bba5eb9e7c2b534437a1112e6810e5c85d73582742d770c96c6f72af234`.
[Verification](VERIFICATION.md#native-theme-arm-build-and-private-trial-bundle)
records immutable copies, the private ZIP identity, and the build log.

A fresh exclusive lease was acquired for this round. The initial SSH timeout
delayed transfer, but the subsequent user wake allowed installation and the
new physical checks below. Earlier `2dd3/971e` passes remain attached to that
baseline rather than being reassigned to the new candidate.

### Native theme and Login X physical acceptance

The `2fd3` ELF and `799d` launcher were installed on the Scribe 2022 /
firmware 5.19.5; device SHA-256 values matched both complete build hashes.
Backup: `/mnt/us/weread/backups/native-theme-2fd3d6ab2d64`. The ordinary
entry had no demo, offline, or automatic-exit arguments.

Initial native property and persistent configuration were `Y8`, with
application dark mode 1. First process PID 11212 captured `Y8`; the user
confirmed the dark application and then, after ordinary Exit, a dark native
Library. Native property and `winmgr.colorinverse.pref` both read `Y8INV`,
and the launcher recorded `persistence=confirmed`. The application and lock
were absent, with the status bar and native foreground restored.

A normal reopening created PID 11804 with native snapshot `Y8INV`, generation
1, and saved dark mode 1. The user confirmed correct startup and Library
colors without double inversion, then entered Reader. The operator used
stock `powerd_test -p`, waited three seconds, observed `screenSaver` and the
application in state T, then toggled wake and waited four seconds. The
device returned to `active` with the same PID and cycle-1 sleep/wake
acknowledgments. The managed native-mode snapshot was `Y8INV`, and saved
Reader state was chapter 1500 / scalar anchor 0 with dark mode 1. The user
confirmed the original dark page and working page turns. A different saved
position was not treated as this cycle's retained Reader anchor.

The user next opened a normal Login page and clicked the upper-right X
without scanning again. It exited directly; the user confirmed that native
Library remained dark and touch worked. This accepts the real normal Login
state. Busy, code, key, error, and failure variants were tested in controlled
cases rather than all being exercised on the physical device.

Final checks found no WeRead process or launcher lock, running `statusbar`
PID 13207, `awesome` in state S, and native `com.lab126.KPPMainApp`
foreground. Application dark mode 1, schema 2, all 118 nonempty bodies, and
the single cancelled task remained. Native property and persistent
configuration both read `Y8INV`. SSH was closed, the matching owned host lease
was released after token verification, and BiliComics was directly informed
of final global `Y8INV` mode rather than an assumed `Y8` state.

The `2fd3/799d` native-exit, next-start compensation, resident-wake compensation,
and normal Login X exit checks pass on this identified model/firmware. Native
configuration readback was verified, but a full device reboot was not tested.
This does not establish a cross-model support matrix or a phone-side unlock
event. The earlier preview-to-full revalidation keeps its separate scope;
nonempty note/bookmark preservation remains limited by the recorded data.
USB remains explicitly user-deferred.

### Earlier resource and session repair iterations

The user reported that a download stopped after several chapters and displayed credential expiry. The original `6f6267` trial had five committed chapter checkpoints before storing `source.http.401`. Session capability flags were present, and the device and designated test environment had the same Unix clock. No account values, book identifiers, resource URLs, or content were copied into this record.

The intermediate `4fb8c13c046a2a43e093925528e345240eacb1f5bdfdb891c1631b2088238843` candidate added rotating-session persistence, bounded read-only renewal, redacted operation diagnostics, and separate resource HTTP errors. Its scoped log identified the failing operation as `chapter_resource`; the task retained its cursor and recorded `source.resource_http.401` instead of forcing account login.

The failed archive's origin was `res.weread.qq.com`. A controlled comparison of the same archive returned 401 without a Cookie, 302 with the existing Cookie, and 200 at the external download redirect without a Cookie. The pinned upstream permits authenticated WeRead subdomains. The project narrowed the correction to the exact primary and reviewed resource origins, with permanent cookie removal after leaving that set. Resource-domain responses cannot change the active account, and no Skill Bearer key or reader authorization header is sent there.

Candidate `7e4a5b568e8d9082eb342a212bf17a5ab87ff46b6c4e5bb1ed3c526530f8a79e` was hash-checked and run with normal arguments. The resource 401 disappeared, and a separate user-selected one-chapter task with images completed. The original request then reached `missing_chapter_image`: its chapter referenced a short PNG name while the one-image flat archive used the same book's `CB_{book_id}_` prefix. A single-chapter diagnostic confirmed the structure and exact namespace ownership without changing the download cursor or posting reading reports. The probe saved any rotated session before normal application restart.

Namespace candidate `89269f457450bc09c967201d74e530823f7dab7438984514fb2354d14408beb6` was hash-checked and installed with the standard untimed, online arguments. The affected request resumed at cursor five and advanced to 55 verified checkpoints with images. A later source integrity failure did not advance that cursor. A fresh single-chapter diagnostic passed the unchanged chapter checksum checks; it had a remote image and no archive-CRC path. A controlled retry then advanced the same request to 86 checkpoints before another integrity failure. These observations establish recovery past the resource and alias defects, not completed acquisition of the affected request.

The subsequent read-hardening candidate `383a5f9b8248017ab4a84ac6fb46e26b3b5c3a26c47ea1ee02593e3e47d9a272` has remote coverage for JSON error-envelope classification and two bounded whole-chapter read retries with new Reader state. Its physical validation is separate. Neither the checksum rules nor the commit gate was relaxed. Explicitly cancelled tasks remain excluded from automatic resumption. Overall device, authenticated-write, and retained lifecycle acceptance remains separate; USB is still skipped.

### Exclusive cross-project handoff

Physical tests use the shared atomic directory `test-env:/tmp/codex-kindle-physical-test.lock`. Its `owner.json` identifies the project, owner thread, and an unguessable run token. An owner releases only its own matching token after normal application shutdown and native display/input restoration; the next project requires an explicit handoff and a new atomic acquisition. No account data or lease token is part of this repository record.

Before yielding the device to BiliComics, the WeRead application and diagnostic process count was zero, its on-device display lock was absent, `statusbar` was running, `awesome` was unpaused, and the native foreground was `com.lab126.KPPMainApp`. The WeRead SSH session was closed and its host lease removed after owner/token verification. The peer explicitly acknowledged acquiring its own lease. WeRead does not operate the device while that lease is held. This process/handle evidence does not substitute for a fresh finger-touch observation.

## Normal-mode private trial deployment

After approving the Library-entry change, the user requested retirement of the supervised test installation and deployment for daily trial. At that deployment checkpoint, release ARM executable `6f6267a87dd2877bcd73860cc899d4dc2adad8e94fc60c30dcaf24b05dfe6d76` was installed as `/mnt/us/weread/app/weread`, with launcher `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` at `/mnt/us/documents/weread.sh`. The device and firmware remain the identified Scribe 2022 / 5.19.5 combination.

The running executable and arguments were checked: only `--kindle --device scribe --data-dir /mnt/us/weread/data` were present. Generation 1 had recovery disabled, reached state-open, and reported nominal 1860 x 2480 geometry without a logged error. No demo data directory was created. There was no smoke wrapper, outer watchdog, offline restriction, or application exit deadline in this launch. The catalog contained **WeRead for Kindle** as a visible shell-script entry, and the user confirmed the login QR/prompt. Authenticated-service operations were still unverified at that checkpoint; the later resource-authentication investigation above adds separate live evidence.

The old owned supervisor was stopped through managed TERM, recording exit 143, restored native services, and a released display lock. The former demo data, progress, notes, scripts, notices, and logs were archived outside the repository on both the operator's computer and `test-env`; matching archive digests and remote archive readability were checked before cleanup. The old test directory and its six historical executables, the device-side backup, installer ZIP, and empty staging directories were removed. The device retains only the normal application root and document entry. Former demo records were not merged into the new real-account library. This is a private direct installation, not a published KPM release or a completed support-matrix acceptance.

## Fresh launches enter Library

The user confirmed Library as the entry for a new application launch. The application already selects Library for a configured session, local books, or demo data, with Login reserved for first use without these. The physical `experience` wrapper had overridden this with `--screen reader`; that override was removed. Managed sleep recovery still takes precedence over the fresh-launch screen.

The corrected wrapper has SHA-256 `a010b3e5ea9e2f4a3df6381435db4d25617a8e0cf885d0365e2f56a93ad13f75`. Remote shell syntax and three existing launcher regressions passed on `test-env`: inherited recovery intent is cleared on an ordinary start, sleep/wake resumes a saved Reader, and explicit exit wins over pending wake. The log is `test-env:/root/weread-emulator/artifacts/startup-library/launcher.log`.

The wrapper was hash-checked and installed on the identified physical device without rebuilding or replacing ARM `6f6267a87dd2`. Run `experience-29557` reached state-open in generation 1 with `WEREAD_RESUME_SESSION=0` and `--screen library`; the running executable was the retained ARM candidate and the log contained nominal 1860 x 2480 geometry without an error. This confirms the deployed entry parameters and state initialization. User visual confirmation and a new physical sleep cycle are not claimed.

## Product scope after user cancellation

The user removed the pull-down panel and application-owned Wi-Fi. Their helper, UI, and connection flows are no longer product capabilities or pending deliveries. Product building and packaging no longer require the helper. Reader brightness/night controls, startup feedback, continuous selection, and sleep recovery remain in scope; overall physical acceptance of the retained application is still incomplete.

### Cancellation-version physical launch

ARM `6f6267a87dd2877bcd73860cc899d4dc2adad8e94fc60c30dcaf24b05dfe6d76` was hash-checked, installed, and launched on the physical Scribe 2022 / firmware 5.19.5 as `experience-28240`. The running executable was confirmed as the installed `6f6267` candidate. The observed state was open / generation 1, native geometry was 1860 x 2480 at scale 2, the log had no error, and the two existing annotations were preserved.

The canonical helper and three older helper files were moved out of the app into a testlogs archive, without changing OS network configuration. Fresh staging contained no helper, and reused staging has a targeted obsolete-helper cleanup. The cancellation version passed 245/245 core, 3/3 baseline packaging, and 133/133 legacy desktop UI checks; [Verification](VERIFICATION.md#product-scope-after-user-cancellation) records the debug/build identities and report path.

This records actual launch plus source/UI removal evidence. The user has not confirmed the new top-swipe interaction or a complete retained hardware sleep regression. It is not full physical or project acceptance; the removed Quick/Wi-Fi feature is not a pending delivery.

## Removed Quick/Wi-Fi experiment

Experiment ARM main `582b72345d863e918e403457cd32ef25c1efc8b8dce314027930df6f2fc0880d` and helper `3ec60abc4b335cfbbd23bfe4265aadae67fd52f55446a4da5eabe6c72f75f4da` were built, staged, and run in the Scribe 2022 firmware 5.20.1 VM, but were not installed on the physical Scribe. They are retained experiment artifacts, not a pending Wi-Fi product release.

The removed experiment had source, mock, and scoped VM coverage for its own Quick and Wi-Fi list/password/confirmation UI. Its recorded source passed 283 core tests and three packaging regressions; desktop identities and separate 133/54 and 26/37 scopes remain in [Verification](VERIFICATION.md#current-helper-backed-wi-fi-candidate). These are not test results for the post-removal application.

In the final VM run, 15 running-state snapshots of Quick and cached Wi-Fi operations kept one application PID/generation, framebuffer/input counts 1/1, and native-panel mapped counts 0. Back/up/outside return left 0 changed pixels in the 3,073,200-pixel Reader ROI. Dark false-to-true persisted in the database and survived close/reopen within that PID. Actual guest powerd sleep/wake then restored anchor 228 in generation 2, with ROI difference 0, dark true, and `position_confirmed=false` retained. This is guest event/save/restart evidence, not physical suspend or stronger position confirmation.

The VM helper observed one fake cached access point, with `connected=false` and no IP; it is separate from the physical helper probe's 23-record cache below. Only the application's open-network confirmation UI was opened; Connect was not clicked, and no profile, global-enable, or disconnect request was sent. Early taps had no visible transition, but a later ordinary tap after reopening succeeded; no input-order root cause is asserted. Managed TERM after launcher identity verification left no app or lock, a running status bar, `awesome` in state S, and no native panel mapped; no numeric exit code was recorded. The VM was returned to the operator. The final `passed_with_explicit_scope_limits` report and `preview.png` are under `test-env:/root/weread-emulator/artifacts/wifi-app-native/run-20261104T090442Z/`, downloaded to `artifacts/wifi-final/native/run-20261104T090442Z/`. [Verification](VERIFICATION.md#final-arm-scoped-scribe-vm-cases) records the query timeout and the unverified real-WPA/fresh-scan/association, physical suspend, touch, and compatibility boundaries.

### Read-only Wi-Fi helper observation

The helper source passed 17 synthetic checks. A same-source helper identified as `6977`, built through a different path from the staged helper above, completed a read-only probe on the physical Scribe 2022 / firmware 5.19.5. Protocol was 1; `available`, `status_ok`, `scan_ok`, `enabled`, `connected`, and `has_ip` were true. It observed 23 records from the existing cached list, with `cached_list` coverage and null status/scan diagnostics. This is observation of an existing association and cache, not a connection established by the application.

Read-only inventory confirmed glibc 2.35, `/lib/ld-linux-armhf.so.3`, and `/usr/lib/liblipc.so`. The earlier `1d4e` helper rejected empty optional `enterprise` values in open-network rows; the corrected parser preserves these as `UNKNOWN`. This repair did not change network settings. No real `scan-start`, `connect`, profile creation, or global-enable operation has been tested. Private SSIDs, IDs, passwords, and actual addresses are omitted.

### Preceding physical overlay experience with 095f

The preceding `095f` overlay candidate ran as `experience-14640`. Its 900-second watchdog requested the stop, producing exit 143 and restoring native Home. This was not an operator-issued manual TERM or a voluntary application UI Exit. The user still requested UI simplification and reported Wi-Fi reaching Home, so neither the default UX nor the later own-list/password/confirmation flow is accepted by this run. It remains separate from the earlier `2b` managed stop below. USB is still skipped for this round.

## Earlier physical observations by candidate

| Candidate or operation | Observed result | Acceptance boundary |
| --- | --- | --- |
| ARM `61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90` | The user observed the library, Reader page turns and font-size changes, and explicit application exit. One user-authored local note was saved and persisted to storage. | These are limited results for this candidate in demo/offline mode. They do not accept later selection, display, or lifecycle changes. |
| Old sleep interruption with the `61fa9afd` candidate | Exit 143 restored stock UI and touch control. | This was interruption and restoration of native control, not automatic same-position Reader resume. |
| Intermediate ARM candidate, SHA-256 prefix `9a13abe6` | Startup, default-body, and sleep-recovery work was introduced. The user reported that drag highlighting remained inadequate. | Continuous press/hold/move selection was not accepted; implementation presence does not establish successful physical resume. |
| Independent native Wi-Fi and Quick Settings LIPC toggles | The user confirmed that the panels appeared and closed normally on the physical device. | This does not establish the application's complete input/display handoff and return, or preservation of application context. |
| USB transitions | The user explicitly skipped them in this round. | No USB acceptance result is recorded. |

## Current physical experience with 2b39140b/0226f613

ARM `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` and launcher `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` were installed and run on the identified physical Scribe 2022 / firmware 5.19.5. Both hashes matched. The run used demo data with application networking disabled.

| Observation | Confirmed result and limit |
| --- | --- |
| Startup and default body view | The user confirmed the project-name/loading feedback and the notes sidebar hidden by default. |
| Continuous finger selection | Chinese long-press selection updated during held cross-line dragging and reverse retraction. The menu after release also behaved normally. |
| Highlight save and local integrity | The user clicked the highlight save action and confirmed retention. One annotation was added, changing the total from one to two. The half-open range `100..128` matched a quote length of 28 Unicode scalars, and the original note-bearing entry remained. Identifiers, quote text, and private note content are omitted. |
| Dark body and menus | The user confirmed normal visual appearance. Dark preference persistence across returns remains a separate check; the user may have toggled the setting again, so a later stored preference of 0 does not establish a persistence defect. |
| Default top-edge native Quick flow | Pull-down first detoured through stock bookstore/Home, then returned to the app when the panel closed. The user rejected that default transition experience. This is not an observed native-input defect or a failure of every return path. |
| Managed stop for the next UX revision | The operator intentionally stopped `experience-10179` through managed TERM. The result was 143, with no watchdog trigger. `after.txt` recorded `statusbar` running, `awesome` PID 3495 in state S, and the display lock absent. This does not cover voluntary UI exit. |

At this recorded `2b` checkpoint, an application-owned Quick overlay with a proposed native advanced entry was still being developed. The whole pull-down and application-owned Wi-Fi feature was later removed by the user. Those UI plans are historical; Reader sleep/night behavior and voluntary UI exit were separate physical gates at that checkpoint. The later `2dd3/971e` observations above complete those scoped checks. USB remains skipped.

## Current source and pending acceptance

Current source contains the following work:

- A project-name startup screen before font loading, with local loading dots at 0.8 Hz.
- The Reader notes panel off by default, including on Scribe, so ordinary reading starts with the body view.
- Continuous held-selection updates at an 80 ms cadence.
- Software dark mode at the RGB-to-grayscale boundary. Hardware inversion remains unavailable.
- A supervisor sleep lifecycle that saves state, exits the application to release framebuffer/input, and restarts at the stored Reader anchor after a valid wake and native dismissal/unlock.
- Reader brightness and night controls remain available independently of the removed panel.
- Reader lifecycle, return-context, and editing/selection protection. Historical native-panel and helper-backed Wi-Fi records do not advertise current product support.

These are retained implementation statements, not completed acceptance of the whole application. The earlier physical `2b` run confirmed Reader selection/save and dark appearance. Quick/Wi-Fi feedback and experiment results remain historical after cancellation; no connection-acceptance task remains for those removed features. That earlier evidence did not accept Reader night persistence, same-position sleep/resume, or voluntary UI exit; the newer `2dd3/971e` run above provides those scoped physical passes without accepting later native night-mode handoff work.

The recorded `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609` ARM build and `file`/`readelf` inspection passed: stripped static ELF32 ARM EABI5, with no dynamic section. It ran with launcher `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4` in the recovered VM and on the physical device. Those historical observations are not results for final main `582b` or staged helper `3ec6`.

## Related remote and VM evidence

Recorded experiment source, packaging, and mock results are summarized above and in [Verification](VERIFICATION.md#current-helper-backed-wi-fi-candidate). The earlier application baseline passed 235 core tests and 31 backend tests. The native-observer query-ordering revision passed 50 of 50 isolated launcher fixtures in 205.691 seconds; evidence is at `test-env:/tmp/weread-native-queries.iYZFwu/launcher-fixtures.log`, with launcher SHA-256 `0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`. Those fixtures include normal, sleep, USB, generation, and native-query regressions. A passing USB fixture is separate from the physical USB check that the user skipped.

The preceding launcher revision passed 48 of 48 isolated fixtures in 199.782 seconds under `test-env:/tmp/weread-native-failure.pwWnI0`. That result remains attached to its earlier revision.

Desktop binary `e11d74c23e309f2aa4e1daa6c4e33fe3d611f747d7e5c904a40bd2537eb24b4c` passed 133 UI checks and five clipboard checks. It predates the latest checkpoint regression fix; Reader source was unchanged by that fix. These results are not reassigned to the latest complete lifecycle implementation.

Recorded Scribe 2022 firmware 5.20.1 VM binary `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609`, with launcher `0226f613`, produced its own QMP press/hold/move captures with these measurements:

| Capture state | changed-region bounding-box width (px) |
| --- | --- |
| Successive held movement | 49, 341, 827 |
| Movement across lines | 1,604 |
| Reversed movement | 438 |

Cross-line growth was evidenced separately by an increase in bounding-box height. The tabulated horizontal extents are not underlined-pixel counts or total underline lengths. Light/dark Reader body text-mask agreement was 1.0. This supports the captured VM selection and text-geometry observations; it does not measure physical panel response, ghosting, latency, or touch feel.

That `2b` guest also completed these scoped software cases without changing XInput, input-grab, or driver code:

- The `startup-06` capture records the startup page using persisted dark mode. Switching to light through the UI produced Reader body text-mask agreement of 1.0 with dark.
- Reader to native Quick Settings and QMP close advanced application generation 1 to 2. Reader body ROI pixel difference was 0, and dark background values were 0 before and after the return.
- At font size 38, wide margins, and page 2/2 of the `Lan Ting Ji Xu` demo text, an actual guest `powerButton=1` transition entered `screenSaver`. The save acknowledgement matched generation 2 with `chapter_index=1` and Unicode anchor 150. `wakeUp=1` produced native wake events and generation 3, with original text ROI pixel difference 0.
- Settings Wi-Fi opened the native list; QMP Done returned generation 3 to 4 with Settings content ROI pixel difference 0. No network settings were changed.
- Settings Exit and confirmation ended the application, cleared the lock, and left `statusbar` running. The observed `awesome` process was in state S and the native view was `KPP_HOME` under `com.lab126.KPPMainApp`. The application log had four geometry lines and no display-feedback error. No process exit code was recorded.

An early close tap at about five seconds had no effect; after the panel was fully drawn, the same-point click closed it normally. This does not establish a general native QMP input limitation or an input-restoration defect. Reports and captures are under `test-env:/root/weread-emulator/artifacts/ux-extended-final/`; the reports are `quick-return-report.json`, `wake-anchor-report.json`, `wifi-return-report.json`, `drag-report.json`, and `explicit-exit-report.json`. Downloaded copies are in `artifacts/ux-extended-final/`, with the aggregate record at `artifacts/ux-extended-acceptance/extended-acceptance-summary.json`. These generated artifacts remain ignored by Git. [Verification](VERIFICATION.md#current-extended-scribe-vm-cases) records the detailed software case results.

The recorded `2b` VM run was left at native Home with the application ended and `awesome` in state S. The later `095f` physical run stopped through its 900-second watchdog as described above. Neither historical stop accepts the final helper-backed physical application. The final `582b/3ec60` scoped VM cases are now complete, while physical deployment and feedback remain pending.

The earlier `daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3` attempt remains separate history: it produced its own selection/rendering captures but did not complete a native return, with no native XInput events observed then and an empty `activeView` after navigation. The current `2b39140b` cases are new evidence, not reassigned old captures. The [VM-only boot-recovery delta](SCRIBE_EMULATOR.md#current-environment-recovery-after-the-six-case-run) remains in place; none of these guest results accepts the physical firmware 5.19.5 device.

The earlier six-case Scribe 5.20.1 emulator record remains in [Scribe 2022 firmware emulator](SCRIBE_EMULATOR.md). [Verification](VERIFICATION.md) preserves the other historical binaries, hashes, dates, and test counts.

## Remaining physical gates

- Isolate dark preference persistence across returns and restarts. Dark body/menu appearance and startup feedback were confirmed in the current run; a later preference of 0 is not a standalone bug result.
- Sleep from the current Reader, dismiss native lock/screensaver state, and verify restoration of the same book, chapter, and anchor. Explicit exit must remain final.
- Verify voluntary UI exit and confirmation on the retained application; neither the earlier managed TERM nor the later watchdog stop covered this interaction.
- Revisit USB only when the user requests that deferred check.

Panel quality, battery behavior, pen/eraser input, multi-touch, live-account behavior, other devices/firmware, release packaging, and project licensing retain their separate acceptance or decision gates.
