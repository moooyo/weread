# Active portrait orientation

The user authorized automatic upright/upside-down changes after the resource
review's finite closeout request. This implementation is part of the final
combined candidate. It does not introduce another complete resource-review
round or replace the private Kindle trial.

Automatic sensing changes only 0/180-degree holding. Manual application
rotation, including landscape, remains independent. No additional artificial
debounce is introduced.

## Ownership and calibration

Before taking display ownership, the launcher pairs stable native portrait
direction with read-only framebuffer metadata while the native window manager
and power state remain active. A private generation/nonce record binds this
calibration to the application launch. The hidden metadata command exits
before fonts, application data, framebuffer mapping, or touch ownership are
initialized. Missing, fixed, or rejected calibration retains the existing
fixed orientation behavior.

The initial scanout relationship remains immutable. A matching managed-wake
cycle permits fresh holding and framebuffer observations while input is still
suspended; it does not redefine that initial relationship. Raw framebuffer
rotation codes are interpreted only as a same-launch relative portrait delta.
No universal meaning is assigned to a kernel rotation number.

## Active changes

The backend observes one recognized pressure-only accelerometer with a
read-only nonblocking descriptor. It does not grab the sensor or create a
sensor thread or periodic orientation timer. Complete reports supply holding
changes; dropped or partial reports require resynchronization. Control events
and sensor observations precede touch dispatch.

An unavailable optional framebuffer observation or unsupported relative
landscape delta preserves the installed mapping and retires the observer until
the next trusted calibration or resume. Actual stream/protocol and full-frame
update failures retain the explicit recovery path.

Rendering compensates the difference between holding direction and scanout.
Raw touch uses holding direction plus the separate application rotation.
Changing native holding therefore preserves the user's application rotation
setting. A transition cancels the old gesture, invalidates its poll generation,
drains stale packets, rewrites the scene, and requires acceptance of a complete
GC16 update before accepting fresh mapped touch. The existing panel-completion
ioctl remains best effort; acceptance of that ioctl is not a physical-display
measurement.

Active portrait changes retain the Reader, page anchor, drafts, and display
ownership. They do not simulate sleep or show native Home. Managed and RTC
suspend close the observer; resume reopens it with fresh metadata. Existing
dark-mode compensation and ordinary-exit recovery remain in place.

## Verification boundary

The exact final source, executable, test, and bundle records are listed in
[Verification](VERIFICATION.md) and [Resource lifetime review](RESOURCE_LEAK_REVIEW.md).
Controlled Linux tests passed for sensor reports, relative mapping, native pointer
cancellation, startup records, managed-cycle staging, and launcher calibration
and fallback. The 28 new cases are resolved in actual full logs and qualified
execution receipts; the complete backend passed 65 cases, Main 52, library
1,481 with three ignored manual benchmarks, and launcher 168. Matching ARM/ZIP
and explicit source-origin records are in the final delivery directory. Source
declarations were not used as passing evidence.

This combined candidate has not been deployed or tested on a Kindle. The
earlier `2fd3/799d` first-generation Scribe / firmware 5.19.5 trial remains
installed with its own scoped Reader, touch, and theme observations. Peer
calibration evidence and the reviewed KOReader source contract inform this
implementation; they are not WeRead physical acceptance. Other device and
firmware combinations remain unverified.
