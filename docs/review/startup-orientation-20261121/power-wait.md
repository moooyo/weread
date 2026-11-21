# Can the launcher shorten its two-second power wait?

**Decision for now: keep `sleep 2`.** Six earlier warm launches on Scribe 2022,
firmware 5.19.5, spent a median **2.020 s** (2.010–2.020 s) between the observer
and settle markers. That [measurement](../library-startup-latency-20261119/README.md#where-the-time-goes)
shows the cost of the delay, not when it becomes safe to proceed. The later
[font A/B](../library-startup-latency-20261119/indexed-font-ab-20261120.md) kept
the delay in both arms. No shorter wait was tested in either batch.

## What the ready record proves

1. The [supervisor](../../../packaging/kindle/launch.sh#L1808) captures the
   supervisor, relay and subscriber identities, then grants permission tied to
   their process IDs, birth times and the launch token.
2. The [subscriber](../../../packaging/kindle/launch.sh#L1597) has its output
   FIFO open. It validates permission and atomically publishes
   `observer-stream-ready` **before** it executes `lipc-wait-event`. This proves
   local stream setup, not that `powerd` accepted the subscriptions. A live
   PID, an executable name, or a read of the current power state does not
   establish that the subscriber will receive the next event.
3. The [startup sequence](../../../packaging/kindle/launch.sh#L2786) waits two
   seconds, configures native settings and sleep covers, takes the display and
   starts the application. The relay can queue received events; the supervisor
   reads its event FIFO **after** application startup. A FIFO cannot recover an
   event that never reached the relay. Elapsed time is not an acknowledgement
   of either subscription or safe native display takeover.

## Independent experiment, not a rollout

- The [Linux fixtures](../../../packaging/tests/launcher_test.py) distinguish
  two gates. `test_power_observer_stream_ready_precedes_subscriber_exec` holds
  the subscriber before execution, observes the supervisor accepting its stream
  record, then interrupts the supervisor and checks cleanup.
- `test_power_observer_delayed_subscription_delivers_usb_after_application_start`
  runs the subscriber fixture and withholds the mock service's acknowledgement
  after receiving its exact subscription request. In the unchanged startup
  order, the simulated status bar stops and the application opens *before* that
  acknowledgement. Only then does the service acknowledge, wait for the
  subscriber's reply and send USB via the subscriber and the actual relay. The
  supervisor exits with status 143 without a restart; the native status bar and
  refresh are restored in the fixture, and the frame guard and observers retire.
  These assertions establish event order, not the duration of the wait, real
  firmware protocol semantics, or that an earlier USB event would have been lost.
  Both cases passed separately on `test-env` on November 21; the final
  `power_observer` run passed 14/14, including both. Delayed or failed
  registration during sleep/wake remains a separate case.
- Consider a fast path only if that firmware exposes a verifiable registration
  acknowledgement for **this** subscriber and event set. Establish separately
  whether native display/input handoff requires settling. An absent, stale or
  late acknowledgement must not shorten the baseline wait; observer failure
  must still abort. Do not generate a power transition as a readiness probe.
- After a fresh device window, qualify ordinary exit, interrupted startup and
  sleep/wake separately from timing. Then compare only the wait policy: same
  indexed entry, build, boot, public shelf and orientation policy; one warmup
  per arm, six balanced AB/BA pairs, device-monotonic request to first Library
  SEND. Keep failed and slower runs. A missed event or failed restoration rules
  out promotion regardless of the median. Until then there is no measured saving.
