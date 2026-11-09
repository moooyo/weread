# KOReader night-mode alignment

Recorded: 2026-11-10 (Asia/Shanghai).

## Decision and evidence status

The user explicitly chose to align the night-mode implementation with KOReader.
This supersedes immediate native SYSTEM setting synchronization and the earlier
native day/night illumination-preset policy for that path. The application
remains an independent Rust and Slint product, with a replaceable Kindle backend
and a separate C frame guard for real stock sleep.

The renderer-owned candidate passed the scoped remote software gates and has a
matching ARM build and development package, recorded below. It subsequently ran
in the isolated R1 physical window on Scribe 2022 / firmware 5.19.5. Those scoped
observations, isolated preference reopen, and the subsequent normal-entry update
close this task's scoped gates. The implementation is recorded in commit
`f154c9e`. The normal trial now uses launcher `a5fec72f`,
application `d221d65d`, and helper `a235b41c`. The replaced v10 `15aab68c` /
`8010a217` SYSTEM-path evidence remains historical in
[Native reading corrections](NATIVE_FIX_ACCEPTANCE.md).

## Pinned implementation comparison

The reviewed sources are KOReader frontend
[`9040116701c888889bd35233b23b83283be66745`](https://github.com/koreader/koreader/tree/9040116701c888889bd35233b23b83283be66745)
and koreader-base
[`1c9682506757d5fd30cfa01c5698929dcc35340f`](https://github.com/koreader/koreader-base/tree/1c9682506757d5fd30cfa01c5698929dcc35340f).
They establish source behavior, not this project's device or performance result.

| Reviewed KOReader function | Candidate contract |
| --- | --- |
| [`fb:toggleNightMode`](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer.lua#L476-L490) chooses hardware inversion when available and software inversion otherwise. | The backend owns the desired appearance; unsupported or unapproved hardware uses software compensation. |
| [`framebuffer:setHWNightmode` and `getHWNightmode`](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_linux.lua#L157-L200) use the 8bpp grayscale 1/2 flag and `FBIOPUT_VSCREENINFO`. | Only the explicit Scribe 2022 / firmware 5.19.5 target may write grayscale; actual 8bpp, known grayscale, and valid unchanged layout are also required. |
| [`Device:init`](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/generic/device.lua#L352-L356) retains original hardware inversion and reads the saved application preference; [`Device:exit`](https://github.com/koreader/koreader/blob/9040116701c888889bd35233b23b83283be66745/frontend/device/generic/device.lua#L679-L690) restores the original hardware state. | Save the application preference independently. Restore startup hardware state on ordinary exit while display mode is app-owned; a completed native yield prevents a later stale cleanup write. |
| The [MTK waveform definitions](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/mxcfb_kindle_h.lua#L134-L153) and [night refresh selection](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L433-L455), with the [Kindle MTK mapping](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L1072-L1100), select GCK16 (8) for full night refresh. | The current candidate narrowly maps MTK full GC16 (2) to GCK16 (8) while explicitly enabled hardware inversion is actually observed. Day, software-only, and non-MTK paths retain their existing waveform selection. |

The Rust backend adds fresh GETVAR/GETFIX layout and readback checks around a
grayscale-only write. It preserves framebuffer rotation and mapping. An uncertain
write fails rather than being reported as applied. The model/firmware opt-in is
an implementation boundary with scoped R1 evidence, not a general support declaration.

## Current candidate behavior

- Application startup and wake retain the saved application preference. Initial
  splash output uses software compensation for actual native kernel inversion.
  The backend retains canonical light RGB and owns exactly one
  `desired_dark XOR observed_kernel_inverse` conversion for every pixel path.
- An ordinary toggle submits a checked full redraw through the existing pipeline.
  It does not write SYSTEM `epdcMode` or dynconf, continue native WM, send
  `SIGSTOP`, quiesce application jobs, release touch, or invoke the frame guard.
- Manual brightness and warmth controls retain their existing behavior. An
  application toggle does not copy or restore light profiles or change ambient
  automatic brightness. No explicit light setter is present in that toggle;
  R1's stable-illumination report is limited to its confirmed transitions and
  does not establish zero frontlight transient from every framebuffer ioctl.
- Real stock sleep retains the existing ownership protocol. Before acknowledging
  Suspended, restore the latest native baseline and rewrite coherent compensated
  RGB for frame-guard capture. On wake, observe actual kernel state, reapply the
  app target, and finish the full-frame/input acknowledgment.
- Active ordinary exit restores the original startup hardware state. Exit while
  already yielded to native performs no further mode write. Native day/night
  settings remain unchanged, and the original native appearance returns.

Automatic portrait 0/180-degree behavior remains accepted; manual rotation is
disabled and USB remains deferred. No project license, dependency freeze, or
additional device compatibility decision is introduced.

## Remote software and artifacts

All execution in this checkpoint ran on `test-env`. The recorded results are:

| Check | Result |
| --- | --- |
| Library | 1,492 passed; three manual benchmarks ignored |
| Main | 52 of 52 passed |
| Kindle backend | 82 of 82 passed |
| Package contracts | Four of four passed |
| Translations | 463 keys checked |
| Initial launcher execution | 194 of 195 passed; one failure involving exit 143 retained |
| Launcher after the minimal grace fix | 10 of 10 targeted and 198 of 198 full cases passed |

The shell correction did not change the frozen Rust/UI/assets/vendor/lock
inputs, preserving the scope of their recorded results. No second Rust execution
is claimed. These are build-time frozen fingerprints; later documentation edits
do not assert that every current checkout leaf still has the same hash.

| Frozen input or artifact | SHA-256 | Size in bytes |
| --- | --- | ---: |
| Final build source fingerprint | `72be3373d9585e2ae03d416903d6831de97c8b6a4ce0973d3833c9471a7645b2` | - |
| Rust/UI/assets/vendor/lock inputs | `25d32976c9ce37b5b0291a4b03fa5a711a995cbc9526a3b19c8890322d2d4a75` | - |
| Static ARM application | `d221d65d823a9171757e3f97dc6eee2636574ded28a6e4c6b6805ded9ab0679a` | 84,306,920 |
| Production launcher | `a5fec72fc23a4527333af76a6b524391d8be40923e9c48d50c4319e6f75ff256` | - |
| Development ZIP | `dae7dd5243355bd8c4333d644a504912c03a9c43ce9da5b2b533c18604b81a5c` | 68,060,346 |
| Verification manifest | `dbbca090995085e6c1da760d728264e2289f9e71d749144ac2ba09fe2302733d` | - |
| Archive-verification receipt | `452b9ffa21e1173eaca6d08d598477f55ee6703b223eb80bc0627e83f02b1b5c` | - |

These software and artifact results do not establish direct ioctl appearance,
responsiveness, frontlight behavior, or physical sleep/exit restoration.

## R1 isolated physical continuation

The fresh R1 device window began at 2026-11-10 04:21:51.305430 +08:00
(Asia/Shanghai; receipt time `2026-11-09T20:21:51.305430Z`). It used
`/mnt/us/weread-ko-night-20261110-r1` on the first-generation Scribe (2022),
firmware 5.19.5. It did not replace the normal installation. The original native
manager was PID 3495, birth 1888, in state S; its native preference was `Y8INV`,
kernel grayscale was 2, and raw framebuffer rotation was 3.

The `short15` operator-terminated run returned real exit 143. Every owned
process-family identity ended, both display locks and the actual guard window
were absent, and native grayscale 2 was restored. This is an operator-TERM
cleanup result, not a voluntary UI-exit result.

The manual window retained application PID 28847, birth 738431113, generation 1.
Fresh application appearance was light, with persisted app preference false and
kernel grayscale 1 while native preference remained `Y8INV`. The first reported
click was initially described as taking at most one second, but its sample was
still light; the user then clarified that the current page was white. That
attempt is inconclusive and is not counted as a successful transition.

After a second explicit click, the user confirmed a black page; the saved app
preference was night and kernel grayscale was 2. Turning it off then produced a
white page, saved light preference, and kernel grayscale 1. The user estimated
each confirmed transition at at most one second, without native Home or startup
exposure, and reported stable illumination. These are scoped visual and timing
observations, not a measured latency distribution or a universal frontlight claim.

A real five-second stock power-button cycle retained the same application.
During sleep it was in state T with grayscale 2 and a matching sleep acknowledgment
for cycle 1. The sample two seconds after wake still showed T and grayscale 2;
it is not reported as completed wake. A later sample showed the same application
in state S with grayscale 1 and matching sleep/wake acknowledgments for cycle 1.
The user confirmed direct return to the original white page, no native Home or
startup page, and working touch.

An inverted-holding stage observed raw framebuffer rotation 1. The user confirmed
automatic orientation and a right-side page turn. After returning upright, the
user exited through the UI; the launcher returned real status zero, and the user
confirmed dark native appearance and working touch. These raw rotation codes
remain relative observations for this device, not a universal direction mapping.

The immediate operator native-after guard reported `native-awesome-not-running`
without a process state. A later read-only observation found the same original
native manager, PID 3495 / birth 1888, in state S, active power, `KPPMainApp`
foreground, native `Y8INV`, grayscale 2, rotation 3, and status bar PID 32058.
All owned process-family identities, including the stock helper, had ended;
both locks and the actual guard window were absent. The initial guard failure
and its unproven cause remain recorded; the later healthy sample does not turn
it into an immediate first-pass result.

Receipts remain private in the operator envelope under `logs/evidence/private/`.
Private logs, captures, and runtime state are not copied into the repository.
The original `native-after.failed` receipt was retained. Independent root
re-verification under the old manual window's `root-reverification/` repeated
strict owned-family/window/lock absence and a fresh native baseline before the
original 3,600-second supervisor reopened `ko-r1-manual-restart`. The new
application was PID 554, birth 738485072. Against the native `Y8INV` / grayscale 2
baseline, it reopened with saved light preference and grayscale 1; the user
confirmed white startup feedback and Library. Voluntary UI exit returned real
status zero and the user confirmed dark native appearance with working touch.
This run's first native-after check succeeded: original native manager PID 3495,
birth 1888, state S, `Y8INV`, grayscale 2, rotation 3, and status bar PID 869.
All owned process-family identities, locks, and the actual guard window were
absent. The isolated preference-reopen gate is closed; the earlier failed check
and its unproven cause remain retained.

## R1 normal-entry update and closeout

The public-file update payload had SHA-256
`7143023ea8bf8b768d2980752840f4118764a75292caa135c22c8173007e914e`.
It used only the 35 R1-verified public App leaves and the exact production
launcher, replacing 36 public leaves. Each of the 36 old public files was retained
for rollback under `/mnt/us/weread-ko-normal-update-20261110-r1`; that backup
contains no real data directory. Normal `library.sqlite` and WAL/SHM hashes were
identical before and after the public-file update. Account material and
downloaded books were not copied.

| Installed component | SHA-256 |
| --- | --- |
| Normal document launcher | `a5fec72fc23a4527333af76a6b524391d8be40923e9c48d50c4319e6f75ff256` |
| Static ARM application | `d221d65d823a9171757e3f97dc6eee2636574ded28a6e4c6b6805ded9ab0679a` |
| Native frame guard | `a235b41c80a21a35c99f8798f366a4f2e921b971a606c83f3f8d69d576676806` |

The user opened **WeRead for Kindle** from native Library and confirmed the
original Library and books, normal display, and working touch. Live arguments
were exactly:

```text
/mnt/us/weread/app/weread --kindle --device scribe --data-dir /mnt/us/weread/data
```

There was no demo/offline option or product exit timer. The user exited through
the UI and confirmed dark native presentation with working touch. This normal
launch had no external supervisor and no observed launcher exit code; its UI
exit and cleanup are not restamped as exit zero. The two isolated UI exits retain
their real zero results.

All original normal-launch identities had ended: owner `1938/738534301`,
application `2334/738534798`, runner `2329/738534797`, guard `2273/738534739`, and
watcher `2275/738534739` (PID/birth). The actual guard XID `0x4a00003` and both
display locks were absent. Fresh native observation found original manager
`3495/1888` in state S, `KPPMainApp` foreground, active power, native `Y8INV`,
grayscale 2, rotation 3, and status bar PID 2469.

The private device-log archive contains 77 entries, is 84,480 bytes, and has
SHA-256 `c2de0a80919b2f7e406ae26f523deb0731af2ecc4b41b918848badd30cbac99d`.
It and the private R1 envelope remain outside the repository. After archival,
the isolated home was removed and the normal launcher/application/helper hashes
were reconfirmed unchanged. The R1 lease was exactly released at
2026-11-10 04:45:25.056069 +08:00 (Asia/Shanghai; receipt
`2026-11-09T20:45:25.056069Z`), and independent checks confirmed that the global
lock path was absent and was not a symlink. All Kindle transports were closed
before the coordinator was told that a fresh window was available.

A private physical manifest binds 26 output receipts and the scoped observations
above. Its SHA-256 is
`05d7cc34ec43ccefc97d2b4c782b589bec1e9ca67f76ea00802851132af18fcf`;
the manifest and raw device records are not redistributed with the product.

## Scoped acceptance closeout

| Gate | Status and required evidence |
| --- | --- |
| Remote software verification | **PASS**, scoped to the frozen inputs and executions above, including the corrected launcher run. |
| ARM and development package | **PASS**, scoped to the recorded ARM application, production launcher, development ZIP, and package-contract results. |
| Physical night transitions | **CLOSED** for the R1 scope: both directions, matching saved preference/kernel state, native-setting stability, no native-UI detour, and user-estimated at-most-one-second response with stable illumination. The initial inconclusive click remains excluded. |
| Startup, stock sleep, and exit | **CLOSED** for the observed R1 baseline/cycle: native-dark/app-light startup, same-process stock return to the original page, automatic inverted holding, and UI exit. The immediate native-after failure remains retained with its later independent convergence. |
| Isolated preference reopen | **CLOSED**, scoped to `ko-r1-manual-restart`: saved light startup/Library, real UI-exit zero, and this run's first native-after check passed. The earlier failed receipt remains retained. |
| Normal-entry update | **CLOSED**: exact public replacement and rollback backup, unchanged database/WAL/SHM hashes, user-confirmed original Library/books and UI exit, and independent cleanup. No normal launcher exit code is claimed. |

The old approximately five-second SYSTEM response remains a separate historical
measurement; R1's timing is a scoped user estimate, not an instrumented absolute
performance guarantee. Partial-night GLKW16 (9), broader waveform tuning, and
update-wait changes remain **OPEN** as separate future proposals; they are not
completion gates for this finished task. Other device/firmware combinations,
USB, and power-loss behavior were not tested by R1. This closeout does not
declare general compatibility or complete overall product-release acceptance.
