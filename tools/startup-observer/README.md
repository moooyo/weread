# Normal Library card: startup observation

The [startup contract](../../docs/PHYSICAL_TESTING.md#library-first-frame-startup) starts
at release of the **ordinary WeRead tile** in the Kindle Library. Stop at a
completed or submitted Library frame from the same launch: navigation, shelf
layout, and entries or the legitimate empty state. A cover may be a placeholder.
No sort action, filming, manual step or optical endpoint is involved.

The current external observer is for **Scribe 2022 / 5.19.5 only**. It does not
replace the installed document, launcher or app, edit user data, or copy screen
images or titles off the device. It cannot turn on timing inside an already
installed GUI via an environment variable in an unrelated SSH session.

| Record | What it establishes |
| --- | --- |
| `up_before_ns`, `up_after_ns` | Device `CLOCK_MONOTONIC` brackets around the XTest ButtonRelease and server confirmation on the verified stock card. Synthetic release, not a physical finger. |
| `first_complete_scene_ns` | Upper bound on when the observer found a matching shelf in framebuffer **memory**, subsequently confirmed in the same GUI. Not a render, submission or ink timestamp. |
| `submit_upper_bound_ns` | Optional witness of the bound GUI in a framebuffer wait while the shelf is present. Not the exact return of a frame-specific `SEND`; zero means no witness. |
| A qualified, directly logged Library submission | If available in a measurement build, subtract the release bracket endpoints from its device-clock timestamp. Report the resulting interval, qualified build and software boundary. |

The unchanged installed-build runner always reports `primary=not_timed`
without a frame-specific completion event. Do not combine a file-open result,
a different launch's log, or the later pixel observation to manufacture one.

## Worker workflow

1. Build and verify the [stock tile helper](native_tile.md) and render observer
   on `test-env`. Replay the public fixtures before requesting hardware.
2. `prepare_daily_render.py prepare` packages the qualified tile helpers,
   current shell and static `render-observer` into a fresh private `install.tgz`.
   In the checkout layout on the worker, `run_daily_render.py --check` validates
   inputs without acquiring a lease or connecting to the device.
3. Invoke `run_daily_render.py --work WORK --lease-tool LEASE_TOOL --owner OWNER`
   only when ready. The host checks its pinned SSH configuration, acquires the
   shared physical lease without waiting, and stages only this run's tools.
   The observer must arm before the helper may release the button. The GUI
   identity used to accept a result must match the launch being closed.
4. Stop the owned session, restore the original native view and pointer,
   clear only the owned stage, confirm absence, close transports, then release
   the exact lease. A refused endpoint is not a reason to keep the device.
   If cleanup itself is unconfirmed, use the same frozen inputs and identity
   with `--recover ORIGINAL_ATTEMPT`. Recovery never clicks or starts an app.

The worker's `operations.json` contains the public timings and receipts, not
the private authentication input or lease token. Use host fixtures for failure
paths; do not experiment with gate changes while holding the physical device.
