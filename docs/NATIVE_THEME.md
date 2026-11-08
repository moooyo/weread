# Native theme continuity and login exit

Status: implemented and verified on the user-identified Scribe 2022 / firmware
5.19.5 with executable `2fd3d6ab2d64` and launcher `799d5bba5eb9`. Native exit
synchronization, the next startup, Reader wake, and login close passed the
scoped physical checks. See [Verification](VERIFICATION.md) and
[Physical testing](PHYSICAL_TESTING.md) for artifact-specific results.

## Accepted behavior

The user requested that leaving a dark WeRead page should also leave the native
Kindle interface dark. The earlier `2dd3` candidate intentionally used only
application-local grayscale inversion. Its saved dark preference remained true
after exit, while the native system remained in `Y8`; it did not erase the
application preference. The new behavior extends that earlier boundary.

A successful, explicit application exit may synchronize the selected mode to
the native interface after application input and rendering have ended. Sleep,
USB, external termination, crashes, demo mode, unsupported firmware, and an
unknown native mode do not authorize a system theme write.

The login screen provides an always-available close icon. Closing cancels and
fences login work, then uses the ordinary application-exit path. It preserves
stored account material and is distinct from signing out.

## Reviewed native interface

The official first-generation Scribe 5.20.1 filesystem registers the string
property `com.lab126.winmgr.epdcMode` in
`/etc/xdg/awesome/lab126_ligl.lua`. The reviewed grayscale values are `Y8` and
`Y8INV`. Its successful listener updates EPDC grayscale inversion, persists
`winmgr.colorinverse.pref`, refreshes the native interface, and publishes the
theme-change event. A successful command exit alone is insufficient: the
listener can restore the old property value when the driver rejects a change.

Initial read-only observation on the identified Scribe 2022 / firmware 5.19.5
confirmed that both the property and persisted configuration returned `Y8`.
The subsequent `2fd3` physical run changed them to `Y8INV` after explicit exit;
readback and the user confirmed the dark native interface. This does not
establish support for other device/firmware combinations.

The property producer runs in `awesome`, which the supervisor pauses while
WeRead owns the panel. Therefore the application does not issue theme IPC
during normal rendering. The supervisor samples the actual mode before taking
native control and attempts an authorized exit update only after continuing
the window manager it paused. Setter/readback and available persistent-config
checks use bounded private helper process groups. Failure must still return
native input and display ownership.

## Output compensation

Native EPDC inversion affects direct framebuffer applications. Retaining the
old software inversion unchanged would invert the next session twice. The
requested preference remains unchanged; only the software output transform
compensates for the actual native mode.

| Application target | Native EPDC inversion | Software inversion |
| --- | --- | --- |
| Dark | Off (`Y8`) | On |
| Dark | On (`Y8INV`) | Off |
| Light | On (`Y8INV`) | On |
| Light | Off (`Y8`) | Off |

This applies to startup feedback, the initial panel fill, and subsequent RGB
output. With no saved application preference, first use inherits the known
native mode. An explicit saved application preference remains authoritative.

Startup mode records bind the private supervisor generation and nonce; wake
records also bind the current sleep cycle. Wake updates compensation before
restoring touch and the complete retained frame. A known mode cannot silently
reuse an invalid or missing cycle snapshot.

Global inversion also affects other direct framebuffer applications. Device
handoffs must communicate a confirmed mode change rather than assuming every
application uses the same software transform.

## Verification boundaries

Controlled tests cover output truth tables, early startup ancestry and record
checks, same-cycle wake ordering, ordinary-exit intent, IPC acceptance and
readback, helper cleanup, failure recovery, and login close interaction.
Successful mock tests do not establish native panel appearance or persistence
across a real device restart.

The normal-entry physical run verified native dark appearance after exit, the
next startup without double inversion, Reader screen-saver sleep/wake with the
same PID and page, and the login close icon. Busy/code/key/error close states
remain covered by controlled tests rather than separate physical observations.
Reading back the persisted setting is not a full-device-reboot test. The
existing user-deferred USB gate is unchanged.
