# Book-cover sleep screen

Recorded: 2026-11-12 (Asia/Shanghai).

## Accepted objective and current implementation

The operator requested a book-cover sleep screen and a shorter interval between
pressing the power button and seeing sleep feedback. A brief e-ink flash is
acceptable. The application therefore prepares the current Reader's cover while
awake and presents it before synchronous sleep persistence and worker shutdown.
The existing `sleep_cover` preference controls this behavior and defaults to on.

This is an application-owned presentation, not an Amazon book-cover setting or
a KOReader runtime dependency. The system `powerd` still controls physical sleep.
No `preventScreenSaver` write, passcode removal, advertisement change, service
replacement, or new kernel suspend policy is part of this feature.

## Reviewed KOReader behavior

The source reference is KOReader frontend revision
[`eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2`](https://github.com/koreader/koreader/tree/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2),
dated 2026-11-11. These are source observations, not this project's hardware
verification:

- The [Kindle startup script](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/platform/kindle/koreader.sh#L259-L303)
  pauses the native window manager on applicable firmware and stops the native
  status-bar job while KOReader owns the display.
- The [Kindle sleep handler](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/frontend/device/kindle/device.lua#L758-L835)
  manages its own screensaver on supported, non-Special-Offers devices. Its
  stock-screensaver branch resumes the native window manager instead. The
  [power adapter](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/frontend/device/kindle/powerd.lua#L242-L247)
  requests sleep through `powerd`; a custom image is not a keep-awake mechanism.
- Its [cover selection](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/frontend/ui/screensaver.lua#L378-L445)
  uses a local current/last document and has image fallbacks. This application
  instead uses its own account-scoped WeRead cover cache and current Reader.
- Its [image presentation](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/frontend/ui/screensaver.lua#L492-L503)
  can pre-clear the panel to reduce ghosting. A cover does not imply flash-free
  operation. Also, [showing the widget](https://github.com/koreader/koreader/blob/eb8fe6a3e4b0a8aefce4f8875083ae922963c4e2/frontend/ui/uimanager.lua#L229-L259)
  schedules painting; that call alone does not establish physical presentation
  before later settings flushes. This project adds an explicit completion gate.

The startup script's passcode-removal behavior is not reused. Configured native
locks remain authoritative.

## Application and platform boundaries

The application prepares one bounded sleep presentation during ordinary Reader
work. The current route, Reader book identity, and account must agree. Merely
retaining `reader.book_id` after leaving Reader does not select a sleep cover.

The cover uses anonymous public-image acquisition through the existing source
adapter. Existing public-image concurrency and transport limits remain in
force. A current-book grayscale derivative, at most 1536 by 2048 pixels, shares
the existing bounded local file cache. The existing book-list thumbnail remains
a fallback. The selected image is decoded before sleep and retained as one
bounded in-memory image; sleep presentation does not read SQL, load a file,
decode an image, or start a network request.

A book without a ready image uses its prepared title and author. A non-Reader
route uses a neutral application sleep screen. The image is displayed with its
aspect ratio preserved and no animation. The route, paginator, notes, and drafts
remain behind the temporary overlay.

The renderer receives a presentation token, not a Book, source URL, or account
credential. It acknowledges that token only after rendering a complete frame,
submitting a checked full update, and successfully waiting for that update. A
queued property change or redraw request is not an acknowledgement.

The reviewed `koreader-base` revision is
[`1c9682506757d5fd30cfa01c5698929dcc35340f`](https://github.com/koreader/koreader-base/tree/1c9682506757d5fd30cfa01c5698929dcc35340f).
Its [independent update markers](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L61-L69)
and [preceding-update ordering](https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L380-L405)
inform the project's checked presentation boundary. Failed submissions consume
their marker without replacing the last successful submission; a sleep cover
waits for the preceding update before rewriting mapped pixels and waits for its
own marker before acknowledgement.

## Sleep and wake ordering

1. The supervisor receives `goingToScreenSaver`. If a controlled, stopped native
   window manager and a known non-advertisement configuration permit custom
   sleep, it publishes a generation/nonce/cycle-bound `managed-cover-request`
   beside the existing sleep request. A configured passcode retains stock sleep.
2. The application blocks ordinary actions, shows its prepared overlay, requests
   a renderer presentation token, and returns to the event loop. It has not yet
   run synchronous persistence or requested renderer suspension.
3. After the renderer confirms the token, the application performs its existing
   state saves, report reconciliation, cancellation, and hardware-worker joins.
   The cover remains visible during this work.
4. The backend preserves its existing native-grayscale normalization, releases
   touch, and acknowledges suspension. Only then can the application publish
   matching cover-ready and durable sleep-ready records.
5. After verifying those records and the stopped application identity, the
   supervisor retains display ownership. It does not resume the native window
   manager, restart the status bar, upload a frame-guard snapshot, or invoke
   `xrefresh` for this successful custom-sleep branch. `powerd` can suspend normally.
6. A current wake event and confirmed active power state authorize continuing the
   same process. The application hides the overlay before requesting backend
   resume. The first complete wake render therefore contains the retained Reader
   or previous route. Existing input draining precedes ordinary work.

Unknown capability, Special Offers, configured passcode, a disabled preference,
or a missing matching cover acknowledgement retain the stock path. Presentation
or protocol failure must not manufacture a save acknowledgement or leave a
stopped application owning the display. Existing bounded cold recovery and
native cleanup remain available. No unsupported database is upgraded or deleted.

## Verification boundaries

The intended latency improvement removes persistence, joins, and native handoff
from the path to the first visible sleep image. It does not establish a numeric
latency reduction or an earlier transition into kernel low-power state. Physical
key-to-`goingToScreenSaver` delay and panel response require device measurements.

The bounded application diagnostic records the elapsed interval from application
sleep handling to its observed presentation completion. It includes a timer
observation interval and is not a measurement from the physical power-button
press. It contains no book, account, or device identity.

Remote software results and artifact identities are recorded below. An initial
test build exposed incorrect draft/size constructors in new test assertions;
those assertions were corrected. The first complete application run exposed a
test helper that had accidentally cleared a deferred cover-refill intent while
releasing synthetic capacity. Its original assertions and production behavior
were retained, and the helper now clears only the pending ownership it retires.

| Check | Result | Scope |
| --- | --- | --- |
| Formatting and translations | Passed for all three manifests; 472 translation keys complete | Run on `test-env`; no local application verification. |
| Kindle backend | 95 passed | Controlled rendering, driver packet/marker ordering, completion errors, input, and orientation; no device framebuffer opened. |
| Launcher | 211 passed; the focused cover subset passed 13 | Isolated Linux firmware/service fixtures, including current-cycle cover ownership, stock fallback, cleanup, and wake. |
| Packaging | Four contract tests passed; all 36 archive members matched staged bytes, modes, and CRCs | Three executable members; no account or book-content data packaged. |
| ARM development build | Completed | Stripped static ARM ELF32 EABI5 main executable; not device execution. |
| Application | 1,619 library tests and 52 Main tests passed; four benchmarks ignored | Final integrated `--features kindle --all-targets` run, including real App/Slint presentation-before-save and wake-before-work cases. |
| Sleep-screen UI | 16 passed | Eight offline captures plus eight assertions across PW6 and Scribe; complete cover, proportional fit, text fallback, and unchanged Reader body after hiding. |
| General desktop UI | 133 passed | Ordinary Library, Reader, settings, clipboard, import, and restart/persistence flows on the final desktop binary. |

The development main executable is
`d8b3df1f9461191b3a79698e3567bc6fe85021448c1ef24aa8c6dd446b88c115`;
the unchanged native frame guard is
`a235b41c80a21a35c99f8798f366a4f2e921b971a606c83f3f8d69d576676806`.
The matching launcher is
`713167110d4839f14505cd03c6b30753476772c729fe92890a86fbd0085fe6be`.
The development ZIP is
`8e0c632d4dbfbbc823776d245d39a4c8d7cfff1a5d871e9fc6c4f79f322b964e`.
The build follows the final production edits; later constructor assertions and
the deferred-capacity helper correction are test-only changes, with no change
to the compiled application behavior. The subsequent scoped physical update is
recorded below and does not change those build identities.

Logs and disposable sources are under
`test-env:/tmp/weread-cover-sleep-20261112-r1/`.
The focused UI report is
`source/artifacts/sleep-screen/run-20261111T180230Z-905543/report.json` under that
directory; its UTC run identifier is preserved. These fixtures use synthetic
cover pixels and offline demo text, not private account data.
The general UI report is
`source/artifacts/general-ui/run-20261111T180817Z-907081/report.json` under that
same directory. Its tested desktop executable has SHA-256
`b29133b517b2cc157c1e3fea3198ffaec6c8d03128e664818834021382748a28`.
The package was refreshed with the final README and backend notices after the
software run; this changed the ZIP bytes without changing the ARM executables.

Software closeout is complete. The scoped physical follow-up below remains
separate from those software results.

## Scoped physical follow-up

On 2026-11-12 (Asia/Shanghai), the operator authorized deployment and observation
on the identified Scribe 2022 / firmware 5.19.5. The matching main executable and
launcher above replaced only the 36 public installation files. Private rollback
copies stayed outside the repository. All installed public bytes matched the
development bundle, and the existing SQLite file was unchanged across public
replacement before application startup. No application-data reset was performed.

The operator selected the built-in offline sample Reader and chose to inspect
its title/author fallback. An initial current-run sleep/wake observation recorded
753 ms from application sleep handling to observed presentation completion. A
controlled stock power-event cycle recorded 855 ms. During that controlled
cycle, `powerd` reported `screenSaver`, the application and controlled window
manager were stopped, the status bar stayed stopped, and matching current-cycle
cover/sleep acknowledgements were present. Wake returned `powerd` to `active`
with the same application process, start time, and generation, and matching wake
acknowledgements. These internal durations include the timer observation
interval; neither is a physical key-to-visible measurement.

The operator confirmed the effect was very good and requested no further tests.
Testing stopped at that point. Managed TERM restored native foreground, running
statusbar, an unpaused window manager, and the original inverse grayscale
baseline. The task-owned on-device staging/capture directory was removed. All
device commands had ended before owner/token-checked lease release at
2026-11-12 02:53:27 +08:00, followed by an independent lease-absence check.
The new normal-entry candidate remains installed and stopped.

This establishes the scoped title/author presentation and resident recovery
observations on the named device, together with the operator's qualitative
feedback. The sample had no bitmap cover. Authenticated bitmap-cover display,
numeric physical-button latency, long sleep, rapid reversals, native-lock
fallback, exhaustive panel quality, power consumption, and other model/firmware
combinations remain unverified. No broader support matrix is declared.

Private command receipts are under
`test-env:/tmp/weread-cover-sleep-physical-20261112-r1/evidence/`; no account
contents, device address, unique device identity, or lease token is included in
this repository record.

The neutral non-Reader fallback is the current implementation choice. Showing
the most recently read book outside Reader is a separate optional product
choice; this feature does not create a recent-book persistence snapshot.
