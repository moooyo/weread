# Indexed orientation timing on the shared Scribe

Compare two builds of the **same source** on a Kindle Scribe 2022 running
5.19.5. Both use `kindle,bundled-font-preflight`; only B also enables
`startup-orientation-reuse`. Keep the same indexed temporary entry, launcher
(including `sleep 2`), helpers, one boot, restored public 15-cover offline
shelf, instrumentation and restoration policy. Do not edit the installed
daily entry, shared handler or private data. Preparing the experiment does
not reserve a device; a frozen plan is not a measurement or an installation.

**Do not run the font `freeze.py`/`run.py` for this experiment.** They
select different font arms and the font runner invokes its own analyzer.
The original `device.sh` also requires `/mnt/us/weread/launch.sh` to be
absent. It is now the installed daily supervisor. Never remove it, weaken
the old admission guard, or reuse that collector to force a new trial. The
[orientation collector](orientation-device.sh) protects the November 21
WeRead, Aidoku R3 and lowercase BiliComics installation expectations. If a
peer installation has since changed, stop and get a new handoff rather than
restoring the old expectation. The collector's temporary document,
stage and native request helper use their own orientation namespace. Keep
failures and interrupted attempts separately; never fill a missing row from
a retry or the font trial.

## Prepare without holding the device

The November 21 worker workspace is
`/root/weread-orientation-20261121.WR7ofkGL` on `test-env`. Its
`orientation-inputs/` contains the public 15-cover seed, production helpers
and a separate measurement helper built with `WEREAD_ORIENTATION_TRIAL`.
Both application binaries come from the retained `arm-pair` build receipts.
On the **worker**, freeze a fresh pilot only once; use `verify` on an existing
pilot instead of recreating it:

```sh
W=/root/weread-orientation-20261121.WR7ofkGL
tool=/work/src/tools/kindle-font-trial/prepare_orientation.py
/opt/kindle-toolchain/ubuntu-run "$W" python3 -B "$tool" freeze --work /work \
  --run /work/orientation-pilot --seed /work/orientation-inputs/seed.tar.gz \
  --helpers /work/orientation-inputs/helpers --pilot
python3 -B "$W/src/tools/kindle-font-trial/prepare_orientation.py" verify \
  --work "$W" --run "$W/orientation-pilot"
```

The pilot contains only `A-library, B-library`, not timed pairs. It requires
no assumed holding or alignment. When the current owner has confirmed safe
restoration and release, and the payload, authentication and recovery
command are ready, run the following on the **worker host**, not inside
`ubuntu-run`. `OWNER` is this session's identifier and `LEASE_TOOL` is the
reviewed worker lease utility. Supply authentication via private terminal
input, not a command-line argument, file in this repository or chat message.

```sh
runner="$W/src/tools/kindle-font-trial/run_orientation.py"
python3 -B "$runner" --work "$W" --plan "$W/orientation-pilot" \
  --owner "$OWNER" --lease-tool "$LEASE_TOOL"
```

There is no reserve/wait mode. A pre-existing lease is not permission to
connect, regardless of its age. A rejected read-only admission releases
immediately. A started trial first restores its own changes and verifies
stage removal and native state, then releases before offline analysis or
code changes. A failed transfer does not excuse leaving a recoverable stage
behind. An unconfirmed physical close is **not** a release: use the same
command and owner with `--recover "$ATTEMPT"` for cleanup only, never a new
run or deletion of another owner's lock. No other session takes over recovery.

Only a successful, released pilot can produce `pilot-reference.json`. A's
independent Library picture sets the scanout alignment; both images and both
candidate reuse markers must agree. Each case records only the native
idle portrait, the current generation's launch calibration checked after
SEND, and the restored native portrait. A fixed `U:n` or `D:n` must agree
across these reads. This is native evidence, not proof of a user's physical
grip. If the branch misses or the picture differs, stop here; do not time or
promote a fallback as a successful optimization.

For a separate full comparison, `freeze` a fresh run without `--pilot`, then
`seal --work "$W" --run "$FULL_PLAN" --reference "$ATTEMPT/pilot-reference.json"`
using `prepare_orientation.py` on the host. The full plan must match the pilot
inputs. Use the same host runner with this new plan and a fresh lease only
after a new handoff. A failed gate does not change the daily installation.

## Recording contract

- The plan JSON must declare `purpose: "startup-orientation-reuse"`,
  `device: "Kindle Scribe 2022"`, `firmware: "5.19.5"`, `clock:
  "CLOCK_MONOTONIC"`, `launcher_wait_seconds: 2`, `control_features:
  ["kindle", "bundled-font-preflight"]` and `candidate_features:
  ["kindle", "bundled-font-preflight", "startup-orientation-reuse"]`.
  Freeze `holding` (`up` or `down`), `raw_rotation` (0 through 3) and
  `expected_panel_alignment_degrees` (0 or 180) **before** timing, from
  the independently closed control reference. Do not select whichever
  candidate orientation makes the comparison pass.
- Preserve the raw [`measure.c`](measure.c) request format and the application's
  `WEREAD_STARTUP` markers in `out/<case>/request` and `markers`. The request
  immediately preceding the native file-open operation and the first Library
  driver `SEND` must use the same device `CLOCK_MONOTONIC`. SSH, a screenshot
  timestamp, `state-open`, and an input descriptor are not endpoints.
- Both arms emit `splash_orientation_begin` and `event_loop_orientation_begin`,
  each followed by exactly one outcome for its site. Legacy queries produce
  `success`, `timeout`, `unavailable` or `failed`; only B can emit `reused`.
  Keep the original outcome when B falls back. Do not compare only its hits.
- Use one public trial `label` (16 lowercase hex characters, **not** a lease
  token or device ID) in both the frozen plan and the final operations receipt.
  The collector must place that label plus one newline in `out/orientation-batch`
  in the raw capture archive from its own stage. Use fresh output locations.
- For `schedule` use `A-library, B-library, A-reader, B-reader` for independent
  pictures, then `A0, B0` warmups, then `A1, B1, B2, A2, A3, B3, B4, A4,
  A5, B5, B6, A6`. The qualified software PNGs, Gray8 scanouts and per-run
  thumbnails use the [indexed protocol](README.md). A half-turn that differs
  from the predeclared alignment is a failure, even if rotating the image
  could make the content match. Keep the four independent picture runs'
  raw request and stage records too: both candidate pictures must exercise
  reuse at both sites. Those durations never enter the timing medians.
  The operations receipt must show admission,
  staging, the planned cases, collection after picture qualification, then
  restoration, final collection, close and release without failures.

On `test-env`, analyze the matching offline files. No credentials, device
commands or network access are used by the analyzer:

```sh
python3 -B tools/kindle-font-trial/test_orientation.py
python3 -B tools/kindle-font-trial/analyze_orientation.py plan.json operations.json results.tgz report.json
```

The report keeps the two warmups, raw request nanoseconds and every relative
marker, per-site outcomes/durations, six measured pairs including slower
ones, and both variant medians. A timing pass requires a positive median
paired request-to-SEND saving, at least five faster pairs, and both candidate
sites reusing calibration in all six **fixed-pose** measured runs. Authentic
fallbacks remain valid observations, not proof the fast path worked. A
completed batch missing the timing gate writes its report and exits nonzero;
invalid or incomplete inputs cannot produce a new successful report.

## Separate physical acceptance

Use separate, explicitly authorized qualification runs for normal and inverted
holding, changes between splash and first frame, missing/stale calibration,
and changed pose across sleep/wake. Verify the startup artwork and Library
are upright in the user's view, touch the same visible button and book in
both poses, check the selected action and Reader layout, and verify native
input/display recovery after exit. Record visible outcomes, not just device
nodes or a route name. Film a real tap and the panel together if reporting
tap-to-stable-ink; report first effective input separately. Do not store
private screens. Passing this analysis proves neither those interactions
nor the all-covers endpoint and never enables a feature or deploys a build.
