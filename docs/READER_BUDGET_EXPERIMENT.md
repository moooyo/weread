# Reader budget and parser experiment

Recorded: 2026-11-11 (Asia/Shanghai).

## Authorization and current status

The user authorized relaxing the Reader response budget and parser concurrency
to investigate performance. Preparation, builds, and analysis run only on
`test-env`. Native physical execution is now complete on the identified Scribe
2022 / firmware 5.19.5. All eighteen cases passed. The exclusive lease was
released before analysis, and no experiment window remains reserved.

The production implementation and installed `dd8ac6c3` candidate retain the
accepted 16 MiB response-reservation pool and two parser permits. The configured
chapter-download default remains eight, with a supported range of one to
sixteen. The experiment uses a separate test-only source copy and executable;
its larger values are proposals, not accepted production settings.

## Question and limits of earlier evidence

The earlier native five-setting experiment changed chapter workers while
keeping the Reader pool at 16 MiB and parsers at two. Its approximately 1.35 MiB
Reader responses used two-MiB reservations. Sixteen chapter workers could
therefore obtain only eight Reader reservations. The observed four/eight
plateau describes that configuration; it does not establish a hardware limit
or prove that larger Reader or parser limits cannot help.

The production source obtains fresh chapter-specific Reader context and access
evidence before fetching content shards. This experiment retains that path.
It does not reuse authorization across chapters or relax entitlement checks.

## Executed comparison

The target is the explicitly identified first-generation Kindle Scribe 2022 /
firmware 5.19.5. The same native executable is used for every case. Each case
runs in a fresh process, warms one chapter, and measures 32 validated synthetic
chapters with the same approximately 1.35 MiB Reader HTML, 1,863-entry catalog,
gzip, and fixed 90 ms fixture delay. The loopback server runs on the Kindle and
shares its CPU. Tokio retains two asynchronous workers in every configuration.

| Case | Chapter workers | Reader pool | Parser permits |
| --- | ---: | ---: | ---: |
| A | 16 | 16 MiB | 2 |
| B | 16 | 32 MiB | 2 |
| C | 16 | 16 MiB | 4 |
| D | 16 | 32 MiB | 4 |
| E | 8 | 16 MiB | 2 |
| F | 8 | 32 MiB | 4 |

A through D isolate the two changed limits while holding chapter workers at
sixteen. E/F separately check the application's default chapter concurrency.
Each configuration has three repetitions. The fixed interleaved order is
`ABCDEF / FEDCBA / BDACFE`; all raw results, medians, and ranges must be retained.
This is not a randomized or fully balanced statistical study.

The initial barrier proves the eligible Reader HTTP peak is attained. It is not
evidence of a speedup. Report barrier-inclusive elapsed time, the barrier wait,
and the interval after release separately. Fixture preparation and gzip
construction are outside timed work. Actual content, fresh-context, request
count, and complete credit-release assertions must pass for every case.

## Resource measurements and stopping conditions

- Record process CPU over the timed interval, including the loopback server,
  runtime, parser work, and sampler. Report one-core percent and actual clock
  ticks; do not describe it as parser-only or whole-device CPU.
- Record timed RSS baseline, ending RSS, sampled peak, and sample count per
  fresh process. Process HWM includes preparation and warmup and has its own
  scope. These are not production UI/font memory measurements.
- Reader admission includes fetch, waiting, and parsing lifetime. Parser permit
  peaks can include blocking-pool queueing and do not prove simultaneous CPU
  execution. Reader and combined shard/Reader HTTP intervals remain distinct.
- A case has a 30-second internal deadline and an independent 35-second process
  watchdog. RSS above 256 MiB or missing RSS terminates the experiment process.
  The device supervisor stops a case after 40 seconds or the full batch after
  150 seconds. Failure does not trigger an automatic retry or larger workload.
- Stage only the temporary harness. Preserve the installed application, data,
  settings, launcher, native appearance, and firmware services. Remove scratch
  files, close device transports, and exactly release the lease before analysis.

The private constructor permits the larger experiment settings while normal
construction retains 16 MiB/two. Expanding the pool also requires review of the
legacy 24 MiB response allowance's exclusive behavior before any production
change. This small-page experiment does not exercise that boundary.

## Results and acceptance

Every cell below describes three independent native processes. Elapsed time
includes the initial first-wave barrier. RSS is the group median of each
process's 100 ms sampled peak during timed work, not the full application's
UI/font memory. Ranges retain the minimum and maximum of all three samples.

| Case | Chapter workers | Pool / parser permits | Median elapsed | Elapsed range | Sampled peak RSS median |
| --- | ---: | --- | ---: | --- | ---: |
| A | 16 | 16 MiB / 2 | 2.569 s | 2.554-2.582 s | 25.23 MiB |
| B | 16 | 32 MiB / 2 | 2.567 s | 2.561-2.587 s | 36.84 MiB |
| C | 16 | 16 MiB / 4 | 2.710 s | 2.652-2.861 s | 29.36 MiB |
| D | 16 | 32 MiB / 4 | 2.746 s | 2.683-2.942 s | 42.09 MiB |
| E | 8 | 16 MiB / 2 | 2.601 s | 2.592-2.727 s | 24.84 MiB |
| F | 8 | 32 MiB / 4 | 2.637 s | 2.614-2.895 s | 27.76 MiB |

Increasing only the pool (A/B) admitted sixteen Reader HTTP requests instead
of eight, so the larger budget was exercised. Median total elapsed improved
by only 2.405 ms, or 0.094%, with overlapping ranges. Sampled peak RSS median
increased by 11.61 MiB, approximately 46%. No reliable total-time benefit was
observed in this bounded comparison.

The barrier requires more arrivals in B. Its wait was roughly 73-75 ms, versus
38-43 ms in A. Median time after barrier release was 2.492 s in B versus 2.532 s
in A, about 1.58% shorter. This does not justify claiming that every phase had
zero benefit, or that different network conditions cannot benefit. It also
does not establish a statistically significant improvement from three samples.

Increasing only parser permits (A/C) increased median elapsed by 5.47%; raising
both limits (A/D) increased it by 6.86%. Their observed elapsed ranges were
entirely slower than A. Corresponding median throughput decreased by 5.18% and
6.42%. These percentages describe different metrics and must not be interchanged.

The default-eight comparison (E/F) increased median elapsed by 1.39%, with
overlapping ranges. F still reserved at most 16 MiB and reached eight Reader
HTTP requests because chapter workers remained eight. E/F changes both limits
and does not separately attribute either change; it provides no demonstrated
default-configuration speedup.

Median process CPU was approximately 175-178% of one core across the groups.
That includes the on-device loopback server, runtime, and samplers. It is not
whole-device utilization or a parser-only profile and does not establish a
hardware limit. Parser permit peaks were two or four as configured; they are
permit observations, not proof of simultaneous CPU execution.

### Disposition and evidence

The accepted production pool remains 16 MiB with two parser permits, and the
default chapter concurrency remains eight. The installed application and
development ZIP are unchanged. This experiment provides no reason to adopt
larger defaults for this workload; it does not rule out improvements for other
page sizes, delays, or real upstream behavior. Results exclude UI, images,
persistence, complete-book throughput, and new user visual/touch acceptance.

The native binary is 58,231,296 bytes with SHA-256
`580bb9e8aac85b315ba4fc76885a20f75666a60d7ea7288856e9440a00949b7a`.
Its source fixture has SHA-256
`d2ca76b8ff678730627c1a28c22a6fc7449f6b9a412d5934bc607ddb752f964d`.
Cross-build completed in 5m41s on `test-env`; static inspection found ARM32,
EABI5 hard-float, static linkage, and no interpreter or dynamic section. The
harness was not executed on the host or in QEMU.

The physical window was acquired at 2026-11-11 01:17:22.019449 +08:00 and
released at 01:20:25.081063 +08:00. All eighteen exact test invocations and
their process-exit checks passed within the 150-second batch execution limit.
Only the scratch harness and synthetic output files were staged. Scratch files
were removed, native metadata matched, device transports were closed, and
owner/token-checked release was followed by an independent absence check.
Installed application SHA-256 remained
`dd8ac6c3e11aec6b9b850125865653f6a877f3bdcf062f9c5ea093a8a4530057`.

The anonymous local receipt is
`artifacts/reader-budget-experiment-20261111/results.json`. It retains every
case, comparison, timing scope, CPU observation, and memory sample aggregate.
Build identities and static reports are under the same directory's
`build-tools/reports/`. Private device execution and release receipts remain at
`test-env:/root/weread-reader-budget-physical-20261111-r1/evidence/`.
