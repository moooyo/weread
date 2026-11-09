# Native theme continuity and login exit

Updated: 2026-11-10 (Asia/Shanghai).

The latest accepted decision aligns application night mode with the reviewed
KOReader framebuffer implementation. The renderer-owned candidate passed its
scoped remote software gates and has a matching ARM application `d221d65d`,
development ZIP `dae7dd52`, and production launcher `a5fec72f`. Its isolated R1
Scribe 2022 / firmware 5.19.5 trial observed two confirmed theme transitions,
same-process stock sleep/wake, automatic portrait orientation, and UI exit. The
user estimated both transitions at at most one second with stable illumination.
The inconclusive first click and immediate native-restoration guard failure
remain recorded. An isolated reopen confirmed the saved light startup and clean
native-dark UI exit, closing the isolated preference-reopen gate. The normal-entry
update and broader physical gates remain **OPEN**.
[KOReader night-mode alignment](KOREADER_NIGHT_ALIGNMENT.md) records the current
contract, pinned source comparison, software failures, exact build identities,
and the bounded R1 physical observations.

The installed normal-mode private trial remains the earlier v10 SYSTEM path:
launcher `15aab68c`, application `8010a217`, and native frame guard `a235b41c`.
Its scoped Scribe 2022 / firmware 5.19.5 visual, sleep, and UI-exit checks passed.
The earlier R3 native-Library exposure was subsequently closed by v8/R9 and v10
observations; the approximately five-second SYSTEM response remains historical
open evidence. [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md) preserves
the full hashes, failures, later acceptance, and normal-mode deployment.

## Accepted current behavior

Application night mode has its own persisted preference, independent of the
native day/night setting. A new launch uses that preference. Native observations
describe the actual output transform; they do not overwrite the application
preference. Desktop night mode remains local.

An ordinary toggle stays in the renderer's existing pipeline, keeps touch
ownership, rewrites every visible pixel, and submits one checked full redraw.
It does not issue SYSTEM `epdcMode` or dynconf writes, continue the native window
manager, suspend the application, quiesce background work, or release touch.
The separate C frame guard is retained for real stock sleep handoff.

Hardware inversion is an explicit implementation opt-in for Scribe 2022 /
firmware 5.19.5, conditional on actual 8bpp, grayscale 1/2, and valid framebuffer
layout. Other targets use software inversion. This allowlist is not a released
compatibility claim. A failed hardware write or readback is an error, not a
successful fallback claim. The hardware-night MTK full-update candidate uses
the reviewed GCK16 waveform. R1 supplies scoped visible light/dark observations,
not comprehensive waveform or other-device acceptance.

Manual brightness and device-supported warmth remain unchanged. Ordinary night
mode has no explicit light setter, application-owned pair of light profiles,
light-value snapshot/restore, or ambient automatic-brightness action. The prior
decision to retain native illumination presets applied to the old SYSTEM path
and is superseded for application toggles. R1's reported stable illumination is
a scoped user observation; it does not prove that the direct ioctl avoids every
hardware frontlight transient.

The login screen retains its always-available close icon. Closing cancels and
fences login work, then uses ordinary application exit. It preserves stored
account material and is distinct from signing out.

## Output and lifecycle ownership

The backend retains canonical light RGB and applies exactly one software
transform: `desired_dark XOR observed_kernel_inverse`. Actual kernel inversion
is read from framebuffer metadata rather than inferred from a saved preference
or a successful setter exit.

| Application target | Observed kernel inversion | Software inversion |
| --- | --- | --- |
| Dark | Off | On |
| Dark | On | Off |
| Light | On | On |
| Light | Off | Off |

Startup feedback compensates in software for the actual native kernel state
before backend ownership begins. The backend then applies the application's
saved target. Text, images, overlays, and later dirty regions use the same
conversion, avoiding separate application and backend inversions.

Real stock screensaver entry retains the managed lifecycle fence. Before
acknowledging Suspended, the backend restores the latest observed native
baseline and rewrites retained RGB with matching compensation. The frame guard
therefore captures a coherent frame. Wake reobserves actual kernel state,
reapplies the retained application target, performs the complete frame rewrite,
and only then acknowledges input readiness.

Ordinary exit while the application still owns display mode restores the
original startup hardware inversion. The native day/night preference is never
changed by this path, so the original native appearance returns. Exit after a
completed native yield issues no further mode write and preserves native changes
made while yielded. The startup restoration value remains separate from the
fresh sleep/wake baseline. Uncatchable termination cannot be treated as a proven
cleanup result.

## Historical system-setting evidence

The reviewed first-generation Scribe 5.20.1 native filesystem exposes
`com.lab126.winmgr.epdcMode` values `Y8` and `Y8INV`. Its listener can update the
driver, persist `winmgr.colorinverse.pref`, refresh native UI, and publish a
theme event; command exit alone does not establish a committed setting. This
interface supported the older SYSTEM implementation and is not an ordinary
night-mode setter in the current candidate.

Executable `2fd3d6ab2d64` and launcher `799d5bba5eb9` established scoped native
dark appearance after explicit exit, compensated next startup, same-process
Reader wake, and login close on the identified Scribe 2022 / firmware 5.19.5.
Controlled busy/code/key/error close cases were additional software evidence,
not separate physical observations. Later v8/v10 tests and deployment retain
their own identities in [Native reading corrections](NATIVE_FIX_ACCEPTANCE.md).
None of these results verifies the new renderer-owned implementation.

The remote software and ARM/package gates have passed for their recorded frozen
inputs. R1 used an isolated installation, leaving the normal v10 entry unchanged.
It observed native dark restoration after UI exit, but an immediate operator
guard reported `native-awesome-not-running` without a process state. A later
read-only observation found the same original native manager running normally;
the initial failure and its unproven cause are preserved. A subsequent isolated
reopen passed its first native-after check and closed the scoped preference-reopen
gate; it does not erase the earlier failure. The normal-entry update and remaining
physical scope stay open. Reading a
persisted setting is not a full-device-reboot test.
Automatic 0/180-degree orientation remains the accepted scope; manual rotation
is disabled, USB is deferred, and no new model or firmware compatibility is
claimed. Overall acceptance remains incomplete.
