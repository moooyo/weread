# Indexed font A/B on Scribe 2022

On **Kindle Scribe 2022 / firmware 5.19.5**, enabling bundled font preflight
reduced median time from a programmatic native open of a temporary indexed
Library document to first Library frame submission from **9.964 s to 6.847 s
(31.3%)**. All six within-pair differences favored the candidate; the
median paired saving was **3.119 s**, range **2.870–3.279 s**.
[All samples, warmups and qualification results](indexed-font-ab-20261120.json)
are separate from the [CPU-only experiment](physical-20261120.md).

| Six warm pairs | Control | Preflight |
| --- | ---: | ---: |
| Request to first Library SEND, median | 9.964 s | 6.847 s |
| Request to first Library SEND, range | 9.788–10.100 s | 6.773–6.949 s |
| Window created to fonts registered, median | 3,116.272 ms | 38.489 ms |

The [research protocol](../../../tools/kindle-font-trial/README.md) held the
entry, launcher, native helpers, boot, public 15-cover seed, instrumentation
and restoration policy constant. Four independent physical picture runs
qualified Library and Reader before timing. The 1,860 × 2,480 software
snapshots matched each active Gray8 scanout, and both arms matched one
another, throughout the compared body (72 dynamic edge rows excluded on
each end). All timed runs' post-SEND thumbnail bodies matched the qualified
Library reference, not Reader. This did not measure a real tap, all covers
arriving, an optical refresh, or the pre-existing daily book's latency.

After one excluded warmup per arm, the actual order was `AB, BA, AB, BA,
AB, BA`. Direct device `CLOCK_MONOTONIC` timestamps were compared; no SSH
round trip, `/proc/uptime` subtraction, or later screenshot was used as the
endpoint. Both orientation probes timed out on all 12 measured launches
(splash median 507.523 ms; event loop 506.161 ms). This identifies a
separate possible improvement, not a saving from this release. Neither
orientation query nor the launcher wait was removed.

An initial read-only admission lost its SSH exec request before staging;
the bounded stdin transport was then rehearsed. A later admission correctly
stopped at a system executable below a symlinked parent: the peer owner
confirmed that its **leaf** must be regular and non-symlinked and match the
frozen bytes. The successful batch also checked that its resolved path did
not change. Neither rejected admission wrote to the Kindle or contributed
a timing sample. The final batch removed its own indexed item and sidecars,
confirmed the test path had zero remaining catalog rows, rechecked the
shared handler and peers, restored native state and lights and released
its lease at **17:29:44 Asia/Shanghai**.

**Decision:** enable `bundled-font-preflight` in the [Kindle package build](../../../scripts/build-kindle.sh).
Raw Cargo feature choices and the desktop default are unchanged. Only the
seven immutable embedded fonts qualify for receipts; user-provided fonts
continue through full runtime validation. Host Main passed 56/56 without
the feature and 58/58 with it; ten package contracts passed. The normal
installed build is a separate transaction: it uses the qualified candidate
ELF and uninstrumented production launcher, not the temporary entry, fixture
or research arguments.

## Normal installation

After the A/B lease was released, a fresh lease admitted the old public
document only if it still matched the [November 15 deployed source](https://github.com/moooyo/weread/blob/5a66fe7bf8962ee0c950d5bd4af6c0b45e553578/packaging/kindle/launch.sh).
The [gated rollout](../../../tools/kindle-font-trial/prepare_rollout.py)
used the candidate ARM (95,137,176 bytes), production frame guard/native-query
helpers, notices, production wrapper and supervisor. Only WeRead's public
files were updated. The existing private data directory was neither moved,
reset nor backed up; no shared registration was installed or changed.

The ordinary `file:///mnt/us/documents/weread.sh` URI had one matching
catalog entry and launched the deployed ELF with exactly `--kindle --device
scribe --data-dir /mnt/us/weread/data`. There was no demo/offline override,
test entry, capture or automatic exit deadline. The deployed process reached
its state-open milestone, stayed alive through a further four seconds, and
held both `/dev/fb0` and an evdev input descriptor. No private screen or book
was captured. The operator harness stopped only the birth- and token-checked
owned supervisor. Native foreground, statusbar, window manager, input roster
and lights returned to the checked state, peers matched, the transaction
stage was removed and the separate lease was released at **17:32:29
Asia/Shanghai**. The new installation was left in place. This smoke check
does not time the operator's private shelf or prove touch/optical latency.
