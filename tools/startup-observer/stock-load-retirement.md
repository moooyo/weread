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

The archive receiver uses the device's `timeout -s KILL` for a 150-second
hard receive deadline. The 5.19.5 Scribe BusyBox rejects `-k`, so the same
explicit signal is used on all supported applets; there is no version branch
or unbounded fallback. Before opening the archive the script checks that the
applet can run a reader and stop a blocked command. The supervised shell
replaces itself with the one `head` process that copies archive bytes. The
reader must exit successfully after consuming at most the frozen length plus
one byte; the file must have exactly the declared length before the
manifests or unpack directory are created. Exact length without EOF remains
bounded. Short, extra, or timed-out input is not promoted. An SSH disconnect
can deliver ordinary EOF to the remote process: the bytes might be valid, but
this does not qualify an install without the original closed receipts.

The waiting install shell owns the stage lock. BusyBox can leave its timer
alive after a successful read; the timer inherits neither the stage lock nor
an open archive descriptor. The supervised reader opens the archive without
creating it, takes its file lock, and checks that the name still
identifies the pinned inode. Only then can it write through that descriptor.
The installer must take the same lock before validating the file or reporting
retirement. On a bounded refusal with no active writer, it revokes the name
while holding the lock; a delayed reader cannot open a new file or write the
unlinked inode. A busy archive lock or an unexpected supervisor status still
leaves retirement unconfirmed. This is not a wall-clock bound on SSH,
program-header input, archive validation, or the whole install; the worker's
separate 240-second install SSH budget still applies. No process discovered
through a PID scan is signaled. Normal native restore, stage removal and
original-file byte readback are still required after a confirmed load
retirement, including an early refusal before navigation.

These checks apply equally to both arms, before card release or during
handback. This is test tooling, not part of the production Kindle package or
an application timing endpoint. Freeze fresh visit inputs before use;
previous load-only evidence does not exercise this transport. Firmware-side
applet behavior and both no-navigation handbacks must be qualified on the
representative Kindle before using the tool for a new timed comparison.
