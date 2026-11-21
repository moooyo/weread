# Single stock-card visit on the verification worker

`run_stock_ab_visit.py` operates one **already frozen** Scribe 2022 / 5.19.5 visit. It cannot freeze inputs, build an application, deploy a permanent replacement, queue for the device or continue to the next slot. Before any live use, finish the combined device-shell/fixture checks, verify the frozen schedule, and coordinate exclusive access. A successful host fixture or `--preflight` is not device acceptance.

The first slots are warmups `00-A`, `01-B`; the remaining frozen slots represent six AB/BA pairs. Run only the next approved slot. On Windows, the wrapper uses `ssh test-env` via the configured agent. Supply a `SecureString` already held by the calling session. It is not an argument to SSH or Python, not written to disk, and must never be included in a transcript. Each invocation exits independently.

```powershell
$visitTool = '.\tools\startup-observer\Invoke-StockAbVisit.ps1'
& $visitTool -Worker $Worker -Visit '00-A' -Preflight
& $visitTool -Worker $Worker -Visit '00-A' -Owner $Owner -DevicePassword $DevicePassword
```

Equivalent worker commands (live authentication is one line on private stdin; do not use a password in a command, SSH argument or shell history):

```sh
runner="$W/src/tools/startup-observer/run_stock_ab_visit.py"
plan="$W/stock-library-ab/visits/00-A"
python3 -B "$runner" --work "$W" --plan "$plan" --preflight
python3 -B "$runner" --work "$W" --plan "$plan" --owner "$OWNER" --lease-tool "$LEASE_TOOL"
```

`--preflight` checks the frozen visit, the install script and the two original-production reference files without reading a password, opening a Kindle connection or requesting a lease. A busy device returns immediately; an atomic acquisition race cannot cause a device connection. After a real attempt, the same slot is never launched again. The public synthetic shelf is the only permitted test data.

Before acquiring the lease the live runner prepares a private stream containing the frozen shell header followed immediately by the unmodified archive. SSH receives a short command, not the shell body in its exec request. A bounded `dd bs=1 count=N` reads only the header; `sh -c` receives the remaining bytes as its standard input. A lone diagnostic sentinel means the transport consumed the header, **not** that the installer accepted the archive. A failure report keeps the local SSH return code when available and whether that sentinel was observed, not raw stderr. A timeout does not invent an SSH return code.

The `request` SSH channel emits a monotonic arm receipt, keeps its original request file pinned during the independent stock-tile click and GUI ownership checks, then emits the submitted Library marker and the original tile receipt. Do not substitute a screenshot, full cover set, other launch, UTC timestamp, `Ready` log or manual input. The endpoint is the same GUI's accepted qualifying Library frame submission, not optical completion. For this populated fixture an empty shelf is not a timed result.

`attempt/operations.json` contains the bounded operation/guard receipts. `primary=qualified` is set **after** the independent original-file identity and byte checks, native restoration, close, absence and exact owner/token release. The interval is `[submitted - up_after, submitted - up_before]` on the device's `CLOCK_MONOTONIC`. A stage that vanishes after navigation or selector activation without a confirmed restoration does not authorize release. A failed visit which is safely returned remains `not_timed`.

The stock catalog may touch the unchanged document. Its before/after identity therefore checks device, inode, size, mode, UID, GID and link count, not timestamps. Both independent byte audits still compare the full document and supervisor against their retained originals. Supervisor and application identities also check the real modification time; the runner does not fake or restore timestamps.

| Exit | Meaning and next action |
| --- | --- |
| `0` | Preflight, a qualified and returned visit, or an explicitly requested close-only recovery succeeded. Read the action/receipt to distinguish these. |
| `1` | Validation failed before admission, or the returned visit had no qualified sample. Do not retry an operated slot. |
| `2` | Recovery/handback is unconfirmed or a fatal worker/transport condition needs audit. Check the attempt and coordinator; do not start another slot from this result. |
| `3` | Device or this slot is busy; no reservation and no device action by this invocation. |

Recover only the original slot/token, using the original owner. The worker retains the immutable document and supervisor reference bytes in the private attempt; a missing upload package is not an excuse to activate again. Recovery never opens a request, taps, creates a sample, or rewrites the original operations report. Its independent receipt is `attempt/recovery.json`. If no owned token exists or release is ambiguous, stop rather than borrowing or deleting another owner's lease.

```powershell
& $visitTool -Worker $Worker -Visit '00-A' -Owner $Owner -DevicePassword $DevicePassword -Recover
```

```sh
python3 -B "$runner" --work "$W" --plan "$plan" --owner "$OWNER" --lease-tool "$LEASE_TOOL" --recover "$plan/attempt"
```
