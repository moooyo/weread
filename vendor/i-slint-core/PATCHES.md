# Native text-edit admission

This directory retains the published `i-slint-core` 1.17.1 source. The original
crate checksum, reviewed upstream revision, and three modified-file identities
are recorded in `SOURCE_PROVENANCE.json` and `UPSTREAM_REVISION`. Original
licensing declarations and notices remain unchanged; no project-license choice
or dependency upgrade is made.

The patch adds a window-owned admission callback for the actual borrowed native
text, replacement range, insertion, and composition snapshot. Registered
callbacks use a reentry lease and weak root identity. Admission precedes owned
Undo retention and mutation on the existing key, clipboard, IME, and Undo/Redo
paths. An absent callback preserves upstream behavior.

The same planned-range admission now precedes Cut and Delete mutation. The
application also applies the numeric Jump editor's independent prospective-byte
and logical-work limits; authentication inputs retain their ordinary policy.

Middle-click insertion rechecks the sealed native state after the existing
cursor callbacks. If they replace the text, selection, flags, focus, or root,
the old admission is not consumed and their new state is not overwritten.
The lease controls registered native reentry and root lifetime; it does not
constitute a public, reusable proof that arbitrary callbacks are pure.

Two doc-hidden reexports expose existing key-event types to the application's
native IME integration fixtures. No compiler/builtin schema is changed.
Current project editors use the main AppWindow; other native windows require
explicit registration. Baseline Android focus bookkeeping is retained without
an Android-device acceptance claim.

The original fixed deltas passed limited source review. This source-integration
record is not an application/native execution receipt. Complete verification
and final artifact identity are recorded separately for the matching canonical
source candidate.
