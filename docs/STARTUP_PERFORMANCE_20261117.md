# Kindle startup performance

Date: 2026-11-17 (Asia/Shanghai).
Status: implementation and scoped remote/device verification completed. The
regular physical installation was preserved; these results cover the isolated
Scribe startup comparison below, not general device qualification.

Five paired warm launches reduced software readiness from **10.82 s to 8.43 s**
and the first application-frame SEND from **12.25 s to 9.30 s**. The candidate
was faster in every pair. Sampled resident memory at that frame fell from
**216.4 MiB to 103.2 MiB**. Final software captures and the final paired native
framebuffer captures were pixel-identical.

## Scope and baseline

The operator requested adaptation of Aidoku's three startup optimization rounds.
The WeRead baseline is `3423e73205c048653675a881eeed5306a5d516eb`, including the
accepted penguin document cover. Both comparison variants include the same
opt-in monotonic timing instrumentation. Aidoku results are reference evidence,
not WeRead measurements.

The source references are Aidoku commits `6660e32`, `e141d44`, and
`4ce1fd156a7a4a75ca2fda5f4324991d6b02ce92`. The shared backend starts from this
project's reviewed revision and existing lifecycle, image, and orientation
patches. Only the applicable changes are adapted.

## Implementation

Seven canonical gzip font sources are expanded during the build. The same
uncompressed font bytes are embedded in the executable and shared with
Fontique. Text metrics borrow static faces instead of cloning complete bundled
font payloads. Custom fonts retain their owned bytes, validation, admission,
staged registration, and failure behavior. The existing conservative resource
prices remain unchanged. Font hashes, weights, family names, glyph admission,
advances, kerning, and fallback behavior must remain identical.

Input discovery first reads a bounded sysfs roster. Device major/minor,
filesystem, inode, names, and axis hints are checked before accepting opened
descriptors. Original evdev ioctls remain authoritative, and the complete roster
is observed again. Missing, malformed, changed, or rejected hints fall back to
the original bounded scan. Multiple validated portrait sources remain an
error. The selected touch descriptor transfers directly into its existing
owner, preserving the original grab and synchronization paths.

The first complete application scene carries the initial FULL display update.
The previous separate blank-paper submission is removed. An absent draw cannot
consume the pending first-frame request or release the presentation/input
barrier. Retained-frame preparation and managed sleep restoration remain.

Managed helper waits use 20 ms completion polls with proportionally increased
tick budgets. Their original nominal sleep budgets, ownership, process birth, nonce,
generation, watchdog, and recovery checks remain. The integer-sleep fallback
retains its previous second-based budgets.

## Already implemented or different

WeRead already reads process identity using shell builtins, including process
names with spaces and parentheses. Its native command gate already uses ten
10 ms polls before its existing coarse timeout ticks. These paths are retained.

WeRead uses a joined raw startup worker followed by synchronous application
attachment, rather than Aidoku's incremental attachment state machine. There
is no corresponding final progress-bar redraw or 4 ms attachment scheduler to
copy. Existing slow-wait feedback remains at its bounded cadence. The inherited
two-second document-launcher settling delay is retained pending separate
evidence about native launcher restoration.

Aidoku's later hidden-keyboard finding was also checked. WeRead already returns
from `PinyinInput::all_candidates` for an empty composition before accessing
`Dictionary::bundled()`. That reported empty-input initialization does not
apply to this baseline, and no dictionary change or corresponding gain is
claimed here.

## Verification and measurement contract

All builds, tests, and runtime checks run on `ssh test-env`, with six Cargo jobs.
Both ARM variants use the project's release profile: thin LTO, one codegen
unit, the Kindle feature, and `armv7-unknown-linux-musleabihf`. Instrumentation
is enabled only by `WEREAD_STARTUP_TIMING=1`; normal launches do not query the
timing clock or write these markers.

`software_ready` marks the shown, attached application before its event loop.
`first_application_frame_send` is emitted only after the driver accepts a
submission armed by an actual Slint application draw. It is not optical panel
completion. Launcher/native-request timings, startup stages, warm/cold context,
and final frame comparisons are recorded separately.

Physical measurements require a fresh shared lease after the artifacts,
fixtures, and collection tools are ready. Each batch must preserve the regular
installation and private data, use isolated offline fixtures, restore native
ownership and lighting, close owned transports, and release the exact lease.
Other applications' results do not qualify WeRead, another model, or firmware.

## Measured results

The target was the first-generation Scribe released in 2022, firmware 5.19.5.
The accepted batch used one warmup per variant followed by five AB/BA rounds,
the same boot, the same offline demonstration snapshot restored before each
case, and the same compiler/release settings. The native Scriptlet route kept
the production default FBInk behavior and the approved penguin Icon. Timing,
capture, isolated paths, and an automatic exit were measurement-only options.

| Metric | Baseline median | Candidate median | Reduction |
| --- | ---: | ---: | ---: |
| Native request to software ready | 10.82 s | 8.43 s | 2.39 s / 22.1% |
| Native request to first Library SEND | 12.25 s | 9.30 s | 2.95 s / 24.1% |
| Resident memory at first SEND | 221,632 KiB | 105,632 KiB | 116,000 KiB / 52.3% |
| Anonymous resident memory at first SEND | 148,172 KiB | 22,520 KiB | 125,652 KiB |

Every paired candidate improved software readiness by 2.11-2.99 s and first
Library SEND by 2.71-3.54 s. The ELF grows from 84,930,720 to 95,168,304 bytes,
an increase of 10,237,584 bytes, because the canonical fonts now occupy static
uncompressed storage rather than runtime heap copies.

| Surrounding stage interval | Baseline median | Candidate median |
| --- | ---: | ---: |
| Font payload loading | 1,760.492 ms | 1.671 ms |
| Native portrait calibration | 1,190 ms | 660 ms |
| Event-loop entry to first application SEND | 1,411.778 ms | 871.506 ms |
| Font validation, registration, and metrics | 2,879.861 ms | 3,071.040 ms |
| Application attachment | 166.469 ms | 133.789 ms |

These are surrounding intervals, not isolated syscall measurements. Independently
computed medians must not be added as one execution timeline. Font validation
and registration did not become faster in this batch; their approximately
3.07 s remains a major cost. The 4.55 s pre-exec interval includes native
Scriptlet behavior, the retained two-second settling delay, and protected
native queries. These are future profiling candidates, not authorization to
remove their safety checks.

The [complete measurements](review/startup-performance-20261117-3423e73/startup-ab.json)
include both warmups, all ten measured cases, paired reductions, stage events,
memory samples, and image hashes. Device-node names and private account data are
omitted. Actual selected input paths agreed across all twelve launches.

## Clock correction and limits

Request and collector-receipt timestamps both use the device's `/proc/uptime`,
with 10 ms resolution and ordinary pipe/scheduling delay. Internal stage
durations use `CLOCK_MONOTONIC` only. No duration subtracts one clock from the
other. These values do not measure physical tap latency, optical completion,
or the point at which the full-frame input barrier finishes draining.

An initial twelve-launch diagnostic batch restored native state and produced
matching images, but its collector mixed the two clock domains for end-to-end
calculations and disabled the Scriptlet's default FBInk wrapper. Its total
times were rejected. After releasing its lease, the collector was corrected and
the complete comparison rerun with the production metadata behavior. Only that
second batch supplies the headline numbers above. One intervening acquisition
attempt found another task's lease; it performed no device command and did not
replace that lease.

The accepted batch restored native service/display ownership and lighting,
preserved the regular application, launcher, configuration, and database hashes,
removed its owned staging, closed its transports, and released its exact lease.
No daily-use binary was replaced. Physical touch gestures, rotation, sleep/wake,
cold boots, other devices, and authenticated use are not newly qualified here.

## Verification

| Check | Result |
| --- | --- |
| Application library | 1,699 passed; four existing benchmarks ignored |
| Main, including static font integrity and metrics | 55 passed |
| Kindle backend, including discovery and descriptor ownership | 147 passed |
| Launcher lifecycle/ownership fixtures | 211 passed |
| Desktop UI/persistence checks | 133 passed |
| Development archive contracts | Four passed |
| Real development archive | 36 explicit members, CRC/modes/content passed; penguin cover preserved |
| Formatting and ARM build | Passed |
| Accepted physical startup batch | Twelve successful launches; five paired candidates faster |
| Final paired native and software images | Identical across the entire 1860 x 2480 image |

The first compile found an old test caller still using the owned-font helper;
the first completed Main run then identified missing per-thread Slint setup in
one new static-font test. Both test adaptations were corrected. The final
targeted Main rerun passed all 55 cases; the completed library results were
retained because its runtime source did not change. There was no ignored new
failure. The ordinary-exit fixture's sleep stub was updated to preserve the
real 20 ms tick and the existing nominal sleep budget.

The [source record](review/startup-performance-20261117-3423e73/candidate-source.json)
binds 530 runtime/build/public-asset files to the reviewed checkout. Forty-six
text assets/vendor metadata files differ only by Git CRLF/LF normalization;
the raw tested hashes and canonical identity are both retained. All changed
Rust sources were formatted remotely and copied back before this comparison.
The compiler was rustc/cargo 1.99.1 with Zig 0.15.2; this records the experiment,
not a new dependency or release-version freeze.

| Artifact | SHA-256 |
| --- | --- |
| Instrumented baseline ARM | `00a763f9372bf0b03650d456959b903cb6ea1241bb5966097a2a3742f7f4be67` |
| Candidate ARM | `5271918d104eeb3d8dad630b8bb0cfa33525970201d2b891da6d1fc709fc453f` |
| Candidate launcher | `60d7def6c9f35eceb89b647b388b5306ea0a1ee09c7fe8fa3e9d03037710b3ca` |
| Shared frame guard | `689ef367eabe7f780ec51a20a033961badceb6c1ff56524e51cad30a4e44372d` |
| Final development ZIP | `c7dfb005b9532d45cc996da1ee8555311cd3d05996b50f7881be651281fa81fe` |

The final ZIP is `weread-kindle-0.2.0-startup-20261117.zip`. Its only member
change from the measured runtime package is the public packaging README;
application, helper, launcher, cover, modes, and the 36-member contract are
unchanged. This is a development handoff, not a published release or installation.

The [package record](review/startup-performance-20261117-3423e73/package-verification.json)
and [verification summary](review/startup-performance-20261117-3423e73/verification.json)
retain their scopes. Private raw records and collection tools remain under
`test-env:/root/weread-startup-20261117-r1/`. The public
[candidate Library capture](review/startup-performance-20261117-3423e73/candidate-library.png)
contains only the isolated demonstration fixture.
