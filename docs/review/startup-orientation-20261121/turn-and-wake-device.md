# Physical turn and resume tests

These are correctness visits on a Kindle Scribe 2022 / 5.19.5 on November 21,
2026 (all wall times Asia/Shanghai). They use the isolated candidate and a
public offline book or Library, not the daily application. Popup readiness,
an action cue, a human observation, and a machine qualification are distinct.
No visit measures ordinary tile-release latency, optical settling or deep system suspend.

## Stable opposite portrait: passed

The native-only window detected the move from `D:1` to two successive `U:3`
references and released at 16:25:41. A fresh visit (`4f72fd29f2a42749`)
reconfirmed `U:3` before upload, before launch, in the launch record and
on return; all ten independent prelaunch axis reads were `15`. Both startup
orientation queries reused the calibration. The digital observer recognized
a populated grid, the final route was `settings` (Profile), and the operator
[confirmed the upright shelf and single physical tap](opposite-operator.json).
The label-bound analyzer reports `confirmed_in_scope`. Restore, close and
independent absence passed; the lease was released at 16:29:23. No input was
injected for this tap.

## Turning during startup

Two initial collectors rejected the transaction (`flip_overlap` at 16:33:19,
`send_launch_pose` at 16:41:47). Their archives contained only the batch
label; neither establishes when or whether a turn occurred. The replacement
binds the sensor and GUI on the same boot and device clock, preserves a
401-sample checkpoint independently of optional launch calibration and later
UI capture, and never substitutes another process or visit for missing data.

In visit `1c1d4856e2f09dc8`, the axis changed once, from down to up (107 down
then 294 up observations). Native reference at entry was `D:1`; it was `U:3`
on return. Relative to `startup_feedback_ready`:

| Event | Time (ms) |
| --- | ---: |
| `portrait_opened` (before the first business draw) | 795.207 |
| Independent sensor transition, outer bounds | 829.118–879.682 |
| `first_application_frame_send` (recorded after driver acceptance) | 1172.643 |

The entire transition was **after `portrait_opened` and before the first
submission log**. It therefore did not qualify the predefined pre-draw window;
there is no finer draw or driver-call timestamp to resolve what orientation
the first frame used. Both queries reused the starting calibration. The app
exited on `library`. The operator [separately confirmed](startup-turn-operator.json)
seeing upright Library text after the turn; this was not an identification
of the very first frame.
The digital classifier did not recognize a populated shelf in its later
sample (`none`, not a finding of an upside-down or empty shelf). The complete
visit restored and released at 16:52:41. Do not repeat this human motion to
turn a near miss into a pass.

## Turning in the screen saver

- `dac13f89a000e4a3`: reached the public Reader in `D:1`, but did not obtain
  the initial guarded panel observation. No power or turn prompt was issued,
  no sleep was attempted, and release was verified at 16:38:10.
- `05a0d1e0bc091a2a`: confirmed the initial Reader image and `U:3`, then
  confirmed the stopped resident GUI in the stock screen saver. Two pre-turn
  screen-saver reads returned `15`/scan `3`. The operator confirmed turning
  half a turn while the saver was visible and holding until wake. The guarded
  observer did not record the opposite value while asleep. It sent a recovery
  wake cue, not the qualified-wake cue; no post-wake panel was captured.
  Native return was `U:3`; the device was restored and released at 16:47:48.
  Neither the operator report nor the recovery command establishes a
  device-measured sleep turn or same-page resume in this visit.

**Final visit `5264fc288149f050` (16:56:01–16:59:42).** The operator
[confirmed](sleep-witness-operator.json) seeing the screen saver, turning the
device half a turn, holding it until the display woke and reading the same
page afterward. The desktop bridge issued only the sleep and turn prompts:
the Reader resumed before the planned wake prompt, so no further power-key
press was requested. The records do not identify what triggered the wake.

| Same-visit device evidence | Result |
| --- | --- |
| Reader and power state | The same application process and generation stopped in the stock screen saver, resumed and exited on the `reader` route. Both startup query sites had reused calibration. |
| Direction | Before sleep: up (`15`, fb rotation `3`). All eight stopped-state samples were also `15`/`3`. Both post-wake samples were down (`16`, fb rotation `3`); stock returned `D:1`. The machine confirms different awake endpoints, not when the movement occurred. |
| Page content | The sensor/scanout data predict a 180-degree relative change. Comparing the two raw frames at that angle gives a body MAE of **0.00437 / 255** and **162 different pixels out of 4,344,960**. Without rotation, MAE is **17.14324** and 441,131 pixels differ. This supports the same page at a relative half turn, not pixel-for-pixel identity or absolute optical orientation. |
| Device return | Case, restore, collection, close and an independent absence check succeeded. The host verified the original owner and token at release. No daily installation changed. |

The original image check called **both** frames `opposite_alignment`. Its
reference, `final.png`, was captured after event-loop exit by Slint's software
renderer, which retains the last rendering rotation. It was not a fixed 0°
logical reference: the pre-sleep raw frame matched it at 180° (MAE 0.00406)
and the post-wake raw frame at 0° (MAE 0.00329). The apparent double inversion
is an analyzer error, not evidence that the user saw an upside-down page.
The original machine-only and operator-inclusive reports remain unchanged.
`batch-sorggeth/relative-replay.json` is the corrected offline replay: the
relative match is `approximately_matching_aligned_body`, not an exact match
or a retrospective pass of the original gate. No visit measured deep
suspend, optical update timing or touch mapping after wake.

## Default decision

Leave `startup-orientation-reuse` off in the normal Kindle package. Both fixed
grips passed the Library and Profile tap, and the screen-saver case resumed
with the same readable content. The earlier turn above did not identify
the first frame. A [separately instrumented follow-up](first-draw-boundary.md#physical-result)
closed that **software mapping** question: the grip changed before draw, and
the first submitted frame used the new 180° angle instead of the old 0°.
The event loop fell back rather than reusing the initial calibration. Neither
that measurement nor the operator's view of the Library measures first-frame
ink or ordinary-entry latency. No repeat of the turn is needed.

Worker receipts: `weread-assisted-shelf-20261121.BcM2UfxL` holds the
opposite-portrait and screen-saver visits, `weread-startup-turn-checkpoint-20261121.boU0XHJQ`
the completed startup checkpoint. Each private worker under `/root` on
`test-env` retains its per-visit `operations.json`, allowlisted raw results
and analysis; do not combine observations from different labels.
The seven completed plans' upload archives (489,041,436 bytes in total)
were removed after checking their return receipts; their public results,
plans and operator records were retained.

The final `test-env` collector suite passed **271/271** tests across 28 modules.
Both production helpers also compiled as static ARMv7 binaries; Cortex-A8
user-mode QEMU verified the sensor policy, four rotations, eleven sensor
rejections and four finish-helper refusals. These checks are not new Scribe
visits. The complete Linux log is `discovery-relative-scanout-v2-final.log`
under `/root/weread-combined-qa-20261121.8Jh3NUQp/evidence` on the worker.
