# One-shot Library submission witness

This contract requires a build that includes the witness. The existing daily
ELF and frozen orientation A/B pair do not produce it. It cannot retroactively
time them. It does not launch an application, inject input, read the shelf from
another process, or install software.

With a device lease and an otherwise idle ordinary installation, a collector
may create `/var/tmp/weread-library-frame` (root-owned, mode `0700`) and a
regular single-link `request` inside it (root-owned, mode `0600`). Do not reuse
an outstanding request or overwrite an existing diagnostic directory. Use a
fresh **public** 16-digit lowercase hexadecimal attempt label, not a credential,
and a decimal device `CLOCK_MONOTONIC` nanosecond deadline. The caller should
retain ownership of cleanup until restore. A setgid parent can give a new
directory mode `02700`: clear inherited setgid on **only the newly created
diagnostic directory**, then verify `0700`. Do not change the launcher's own
private lock directory. The entire input is one ASCII line:

```text
WEREAD_LIBRARY_REQUEST <label> <deadline_ns>\n
```

Here `\n` denotes one LF byte. At claim, `0 < deadline - now <= 60 s` is
required. Without the directory the Kindle app does one unsuccessful directory
open, queries no diagnostic clock, installs no observer and writes nothing.
It accepts only the daily program, data path and exact launcher arguments for
the approved `scribe` or `pw6` profile, with neither resume nor native return
and no separate startup timing environment. The launcher must still own its
root-private generation-one, nonce and process ancestry records. No display
calibration record or new launcher option is needed. A valid claim locks and
empties the original file. An empty file is **not** a successful measurement.

On a naturally requested Slint draw, the app checks the ordinary Library route,
selected Library navigation, successful shelf projection, first page, list or
cover mode, and no selection, dialog, toast or sleep overlay. Recognizable
entries have a nonblank title and an ID in the first projected position. No
cover is required. A legitimate empty state requires an unfiltered, completely
projected empty model with neither pending sync nor failure/status placeholder.
An in-progress empty shelf or transient overlay can wait for a later natural
draw within the same request; the observer creates no polling timer or redraw.
An ineligible route, an application action, observed touch input while the app
owns input, or a power handoff abandons it. It never follows a later visit to
the shelf. A skipped draw cannot produce or consume a receipt.

For the first qualifying draw only, the renderer uses `NewBuffer`, writes the
entire physical panel, confirms that the same Library class still holds and
submits a checked full refresh. Immediately **after SEND succeeds and before
WAIT**, the application samples device `CLOCK_MONOTONIC`. It checks the original
file identity, empty state, deadline and launcher ownership again. Only then
does it write the original descriptor:

```text
WEREAD_LIBRARY_SUBMITTED <label> <submitted_ns> <entries|empty>\n
```

This bounds driver acceptance from userspace; it is not render completion,
driver WAIT completion or an optical observation. An already complete opening
frame would have used FULL anyway. If data arrives after the opening frame,
the instrumented eligible frame may use a complete redraw/FULL where an
uninstrumented launch would have used a partial refresh: compare like-for-like
instrumented runs and record this limitation. Clock and file checks also run
only in an armed attempt. No book, account, device identifier, process ID or
launcher token enters this record. No extra application/bootstrap log is used.

Correlate the single exact record to the same ordinary tile release and attempt
label on the same clock, without another launch, input or ownership transition.
If the synthetic release has only before/after bounds, retain a latency
interval, not a point. Missing or malformed records, rejected or ambiguous
launches, and missed/expired frames are `not_timed`. A user-mode ARM smoke test
or host fixture is not a physical Kindle or firmware-emulator result.

`library-submit.sh` is the read side, not an installation or device runner.
Its caller must keep the newly created directory
and original request open on file descriptors 6 and 7 in the **same shell**,
provide the ordinary transaction's stage and identity checks, and own cleanup.
Before the card input, call `library_submit_arm DEADLINE_NS` directly (not in a
subshell); it checks the live request and records device `CLOCK_MONOTONIC` via
`render-observer --monotonic-ns`. After the bound GUI and observer witness, call
`library_submit_collect` in that shell before stopping the GUI. Preserve both
JSON objects as the `arm` and `library_submit` operation receipts. The qualifier
compares the latter's saved tile release with the actual `tap` receipt. The
unchanged external daily runner cannot keep these descriptors or select an
instrumented app; those prerequisites are not implemented by this component.

`stock-library-request.sh` supplies that descriptor lifetime for the isolated
stock-card transaction. The `request` action owns a fresh root-private request,
publishes it only if the fixed path is absent, and holds the original directory
and file open across the stock tap and GUI ownership check. On that same channel
send `collect` followed by LF and EOF for the application receipt, or `cancel`
followed by LF and EOF for cleanup only. Bare EOF also cancels. No frame match
or screen image from an external observer is needed. An occupied path, a wrong
inode or a lost GUI generation is not a sample. The recovery action may remove
only recorded original objects; final absence and the launcher's separate
restoration must still be verified. The `isolated_stock_launch` qualifier uses
the fresh arm receipt rather than the ordinary launch fence, and accepts only
the temporary measurement build and disposable public fixture, not a timing of
the untouched installation. An offline fixture pass is not a Kindle result.

## November 22 ordinary-card A/B preparation

Two ARM measurement builds from `ac6d63f` can emit the same submission
record. A includes the bundled-font optimization and no orientation reuse
(95,224,032 bytes); B adds the normal exact-pose reuse (95,228,264 bytes).
Neither includes the experimental opposite-pose path. The worker retains the
binaries and their Cargo records in
`test-env:/root/weread-ordinary-library-pair-20261122.icCGKPHm/pair.json`.
With no account or network and no forced route, the ordinary desktop app
opened Library from the public 15-book data directory and Login from an empty
one. This verifies a suitable public fixture, not Kindle startup latency.

**No ordinary-card A/B ran.** The existing stock document starts the installed
program. Changing the program or data path causes the current submission
witness to decline the request, and the installed daily program lacks this
witness. Do not replace the shared handler, cover the installed program or
data with a system-wide mount, or open the private shelf to manufacture a
result. The [daily-entry routing decision](../../docs/review/library-startup-latency-20261119/physical-20261120.md)
is distinct from the approved temporary indexed test entry.

One option, not an approved deployment, is a temporary, restorable change
only to WeRead's own launcher: the normal stock card would select a fixed
measurement build and disposable public data for that visit. Both arms
would first need the same narrowly scoped submission witness for that
launcher and its process; keep the installed witness unchanged. Compare
identical release-to-submit intervals and label the result as a stock-card
launch of an isolated measurement build, not of the untouched installation.

The observer tools passed 61 `test_*.py` tests and five non-overlapping
`*_test.py` cases under Xvfb on `test-env`. The 12 focused witness tests
also passed under BusyBox. Native and static ARM helpers built with
`-Wall -Wextra -Werror`; the native binary and Cortex-A8 QEMU each returned
clock values inside the surrounding monotonic interval and refused expired
deadlines. These are clock and collector checks, not on-device samples.

## November 21 worker verification

On `test-env`, formatting passed for the changed Rust files. The Kindle-feature
host platform group passed 67 tests, the isolated Library projection case
passed, the application binary passed all 56 tests, and the standalone
backend passed all 153 tests. One backend case used a real Slint software
window: a skipped draw left the observer armed; a subsequent placeholder draw
waited; a model update caused a natural 64-by-64 complete redraw. The send
was a test callback, not a Kindle driver ioctl. The full application library suite
was not rerun here.

The packaging feature selection at the time (`kindle,bundled-font-preflight`,
without orientation reuse) built a 95,222,992-byte release application for
`armv7-unknown-linux-musleabihf`. `qemu-arm -cpu cortex-a8` returned
`weread 0.2.0`; a separate ARM test executable linked the actual request and
launcher-validation modules and passed six focused cases under the same CPU.
Those filesystem/clock fixtures did not exercise a Kindle framebuffer.
The worker retains the candidate and logs under
`/root/weread-library-frame-20261121.0tNiPgC9`. The documented Scribe/PW6
guest runners were not present in this worker environment. No physical device,
installation, lease, or ordinary-tile timing was used to verify this change.
