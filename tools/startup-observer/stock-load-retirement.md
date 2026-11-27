# Stock A/B load retirement

The host persists a pending `stage` or `install` operation before sending it.
The frozen device program emits one visit-bound terminal record on stderr
only after the corresponding foreground work has returned. The host requires
that final record on a normally closed SSH command (exit 0 for success or exit
1 for refusal). The existing stdout receipts and install header are separate;
the header only confirms that the command body was received. Missing, foreign,
duplicate, or incomplete terminal evidence, and a lost or timed-out SSH
connection, leave the original lease pending. Do not retry the write, clear
the pending journal, infer retirement from directory absence, or dispatch a
new visit. The ordinary recovery entry point also refuses an unresolved
pending load; its owner must separately establish the remote outcome. Older
frozen operations and journals are not accepted as retirement evidence.

The archive receiver uses the device's `timeout -k 1` for a 150-second
receive window and one second to escalate if needed. Before opening the
archive the script checks that the applet can run a reader and terminate a
blocked command. The reader must exit successfully after consuming at most
the frozen length plus one byte; the on-device file must have exactly the
declared length before the manifests or unpack directory are created. Exact length
without EOF remains bounded. A short file, an extra byte, or a timed-out
reader is not promoted. An SSH disconnect can deliver ordinary EOF to the
remote process: the bytes might then be valid, but this is not a qualified
install or permission to advance without the original closed receipts.

The waiting install shell owns the stage lock. BusyBox can leave its timer
alive after a successful read; that timer inherits neither the stage lock
nor an open archive descriptor. The supervised reader verifies the pinned
archive identity and takes a separate archive-file lock before writing.
After the supervisor returns, the installer must acquire that same lock
before processing the archive or confirming retirement; an ambiguous
supervisor failure is not a child exit receipt. This is not a wall-clock
bound on SSH, program-header input, archive validation, or the whole install;
the worker's separate 240-second install SSH budget still applies. No
process discovered through a PID scan is signaled. After a confirmed load retirement, the normal native restore,
stage removal and original-file byte readback are still required before
release, including an early load refusal that never reached navigation.

These checks apply equally to both arms, before card release or during
handback. This is test tooling, not part of the production Kindle package or
an application timing endpoint. Freeze fresh visit inputs before use;
previous load-only evidence does not exercise this transport. Firmware-side
applet behavior and both no-navigation handbacks must be qualified on the
representative Kindle before using the tool for a new timed comparison.
