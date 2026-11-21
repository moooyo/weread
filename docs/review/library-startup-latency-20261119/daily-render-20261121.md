# Ordinary Library tile: first-frame retest

**Target:** the normal installed WeRead entry on Kindle Scribe 2022,
firmware 5.19.5. Start at the native card's synthetic button release; end
at the same launch's first complete Library render or its accepted submission.
See the [endpoint contract](../../PHYSICAL_TESTING.md#library-first-frame-startup).
The public 15-cover orientation experiment and the earlier manual functional
observation are not samples of this interval.

## Result on the unchanged installation

One automatic ordinary-card launch completed on the identified Scribe. By
**8.258 s after synthetic release** (rounded up), a complete grid Library
scene was recognizable in framebuffer memory. This is a **single digital
scene-observation upper bound**, not the time the renderer finished or
the display driver accepted the first Library frame. No application submission
log was available, and no qualifying driver witness was observed. The primary
release-to-render/submission result is **`not_timed`**. Optical completion,
interactivity and cover completion were not tested.

The [public timing receipt](daily-render-20261121.json) records the successful
attempt `2f27a43e4e0c949f`. With the firmware's shell and private lock identity
recognized, the ordinary GUI was positively bound to this launch. The
same-process observer recognized shelf navigation, layout and an entry, and
confirmed the same scene again. It did not read or export book titles or
save private images. The warm running device and ordinary installed data
were left as found; there was no cache reset, demo seed or reboot.

| Device `CLOCK_MONOTONIC` event | Nanoseconds |
| --- | ---: |
| Before / after synthetic release | 289607540966336 / 289607542342105 |
| Start of the last nonmatching framebuffer sample | 289615784787567 |
| End of the first matching framebuffer sample | 289615798054567 |
| End of the later confirmation | 289617605710875 |

The release bracket was 1.376 ms wide. Subtracting the earliest possible
release from the first matching sample gives **8,257,088,231 ns**. The later
confirmation is not included in the bound. The sequence from admission through
ordinary card tap, claim, observation, witness, stop, native view/pointer
restore, owned stage removal and independent absence all succeeded. The exact
lease was released at **12:28:06 Asia/Shanghai**. There is no reason to repeat
this unchanged configuration to obtain a missing in-app log: a future
explicitly deployed [instrumented build](../../../tools/startup-observer/LIBRARY-FRAME.md)
is required for that measurement path. Do not mix the 8.258 s bound with the
[isolated A/B](../startup-orientation-20261121/physical-balanced.md).

## First admission

At 12:07:13 Asia/Shanghai on November 21, the read-only admission for
`3864caa8e989482e` reported `active_view_unavailable`. The LIPC property
did not provide a value accepted by the collector. This does **not** show
that the device was outside the native UI or that another application was
active. No device stage was created, the card was not clicked, and no
app frame was observed. The exact lease was released at 12:07:13. No
duration can be assigned to this attempt.

## First confirmed stock-card activation

The next attempt, `4c24666ae6bd34c0`, treated the optional navigation
property as supplemental and independently captured the original Home or
Library tab and pointer before navigation. The observer armed, the unique
WeRead card was identified on the stock Library, and the synthetic release
was acknowledged between **288918488923987** and **288918489812680 ns**
on the device's `CLOCK_MONOTONIC` (bracket width 0.889 ms). This is only a
start record, not a startup duration.

The original claim rejected the launcher: its root-private directory was
`02700` (setgid, no group or other access), not the assumed `0700`. The
launcher used the system `/bin/sh`, not `sh` resolved via the SSH session's
search path. The collector cancelled observation instead of attributing a
late frame to the run. During close-only recovery the original owner
identified the stock waiting entry, its supervising shell and runner, the
normal GUI executable/arguments and birth, and the same open launcher
generation. The app was stopped through its supervisor, the original tab
and pointer restored, the stage removed and its absence verified. The exact
lease was released at **12:24:18**. No second tap occurred during recovery.
Result: **not timed**; an open GUI does not establish a completed Library frame.
