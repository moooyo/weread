# Event-loop pose reconciliation: controlled Linux comparison

This is a measurement of the event-loop orientation decision on **one Linux
worker**, not a Scribe startup A/B. The measured interval covers the real
production resolver, including polling and retiring its fallback child when
called. Framebuffer and sensor values at the decision boundary are synthetic.
The child is a real `/bin/sleep 5` (silent) or `/bin/echo U` (responsive),
called through the backend's normal query/timeout mechanism. Neither the
500 ms deadline nor the child time is simulated. No Kindle, sensor ioctl,
screen update, native startup gesture, wake, or app launch is timed here.

## Arms and correctness

The comparator is **not** the older build with all orientation reuse disabled.
These three test binaries use the same [fixture](reconcile-host.rs):

- `published_exact`: backend source from `074a4d2`, compiled with the shipped
  `startup-orientation-reuse` feature. The fixture calls its actual
  `confirmed_launch_controller` and `resolve_launch_orientation` entry points.
- `exact`: the proposed source, with only `startup-orientation-reuse` selected.
- `reconcile`: **the same source** as `exact`, with
  `startup-orientation-reconcile` selected (which implies exact reuse). Both
  new-source lanes call `confirmed_event_loop_launch` through
  `resolve_launch_orientation_path`. Only the real compiled implementation
  decides whether to call the child. Timing markers are enabled in every lane.

The 18 input cases per lane assert marker, query count, observation counts,
return of the original observer descriptor and separate render/touch offsets.
The two exact lanes must agree on every non-timing observation. In the stable
`U:3` and `D:1` cases, all lanes read the framebuffer twice, read the initial
pose once, and issue no legacy query. For an opposite grip with unchanged
scanout, the candidate takes three framebuffer reads and two matching sensor
observations on the same descriptor, rechecks the initial request, and
emits `reconciled`. Both exact lanes take the fallback. `D→U` is render 180°,
touch 0°; `U→D` is render 180°, touch 180° in the candidate fixture.
Changing or losing a reading or request, using a non-initial generation, or
providing an absent/inconsistent anchor must not qualify the shortcut.
The legacy default controller inspected here is the immediate fallback result,
**not** the final corrected mapping after the rest of the event loop runs.

All **54/54** branch observations (18 per binary) passed. The 26 ordered
pairs are in the [public nanosecond samples](reconcile-pairs.csv). A always
means exact reuse and B means reconciliation enabled. Medians below are in
milliseconds; the last column is the median of the paired differences, not
the difference of arm medians.

| Paired condition | Pairs | A queries / B queries | A median | B median | Median A − B |
| --- | ---: | ---: | ---: | ---: | ---: |
| Opposite grip, silent child; published A | 6 | 6 / 0 | 500.427 | 0.063 | 500.374 |
| Opposite grip, silent child; matched-source A | 6 | 6 / 0 | 500.427 | 0.067 | 500.363 |
| Opposite grip, responsive child; matched-source A | 6 | 6 / 0 | 20.335 | 0.112 | 20.221 |
| Stable grip, both already reuse | 4 | 0 / 0 | 0.053 | 0.055 | −0.000871 |
| Missing confirmation or changed generation, responsive child | 4 | 4 / 4 | 20.370 | 20.338 | 0.029504 |

The timed path avoided all the silent timeouts (and all six responsive
children) for qualified opposite grips; every prescribed fallback still
ran. The stable pairs range from −0.022 to +0.047 ms and the rejected pairs
from −0.007 to +0.307 ms. These small host samples are not an equivalence
test or a measurement of the added real evdev/GETVAR cost. The responsive
case includes an actual child and the production polling interval; it is
not a Scribe winmgr latency estimate. Complete per-visit observations,
production marker intervals and build logs are retained at
`test-env:/root/weread-reconcile-matrix-20261121.fPctWLFg/`.

Actual pipe/Slint contact dispatch, active cancellation and changed-pose wake
are checked by `platform::pose_acceptance_tests` outside the stopwatch.
`gyro::tests::quiet_confirmation` additionally checks the actual poll path:
queued data on either side of confirmation stays available to the observer.
Those tests have synthetic inputs, not physical fingers or a Kindle kernel.
A match at two observation boundaries does not prove that no motion occurred
between them or after the final check. In particular, the
[physical draw-boundary visit](first-draw-boundary.md) did not record whether
the app's own descriptor was quiet during that fallback. Do not subtract a
host saving from that visit, or interpret the test-case mix as device hit rate.

## Reproduction

On `test-env`, make a private scratch workspace. Place the new-source backend
at `stage/`, the `074a4d2:vendor/slint-backend-kindle` subtree at `baseline/`,
and the root `Cargo.lock` and project `vendor/{i-slint-core,swash,zeno}` under
`src/`. Copy that lock to both backend directories. Place the review Python
and Rust files in `review/` in the workspace. Configure each backend copy's
`.cargo/config.toml` as follows so the dependencies are the project's copies:

```toml
[patch.crates-io]
i-slint-core = { path = "/work/src/vendor/i-slint-core" }
swash = { path = "/work/src/vendor/swash" }
zeno = { path = "/work/src/vendor/zeno" }
```

Run `/opt/kindle-toolchain/ubuntu-run --build "$workspace" python3 -B
/work/review/run-reconcile-host.py` (one shell command). The
[driver](run-reconcile-host.py) injects the fixture only into those disposable
copies, checks the standalone locks are subsets of the project lock, requires
three distinct feature/source builds and invokes each sample in a fresh test
process. Both timed arms use the worker's monotonic clock. The paired numbers
use the outer decision interval; the production begin/end marker interval is
retained alongside it. The test-process start, builds and matrix assertions
are outside both timers. The one warm-up is reported but not included.
