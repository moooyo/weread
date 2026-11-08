//! Bounded LIPC replies and owned direct-child retirement without reader threads.

use super::PlatformError;
use std::{
    io::{self, Read},
    os::fd::{AsRawFd, RawFd},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_OUTPUT_BYTES: usize = 4096;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const MAX_COMMAND_SLOTS: usize = 8;

static COMMANDS: OnceLock<CommandRunner> = OnceLock::new();

enum CommandSlot {
    Free,
    Active,
    Retired(Child),
}

struct CommandRunner {
    slots: Mutex<[CommandSlot; MAX_COMMAND_SLOTS]>,
}

impl Default for CommandRunner {
    fn default() -> Self {
        Self {
            slots: Mutex::new(std::array::from_fn(|_| CommandSlot::Free)),
        }
    }
}

impl CommandRunner {
    fn reserve(&self) -> Result<CommandLease<'_>, PlatformError> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reap_retired(&mut slots);
        let index = slots
            .iter()
            .position(|slot| matches!(slot, CommandSlot::Free))
            .ok_or_else(|| operation_error("LIPC child cleanup capacity is exhausted"))?;
        slots[index] = CommandSlot::Active;
        Ok(CommandLease {
            runner: self,
            slot: Some(index),
            child: None,
        })
    }

    /// Call after stopping producers. This observes existing ownership without
    /// launching work or treating a joined caller as proof its child exited.
    fn quiesce(&self, timeout: Duration) -> Result<(), PlatformError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| operation_error("LIPC quiescence deadline is invalid"))?;
        loop {
            let quiescent = match self.slots.try_lock() {
                Ok(mut slots) => {
                    reap_retired(&mut slots);
                    slots.iter().all(|slot| matches!(slot, CommandSlot::Free))
                }
                Err(std::sync::TryLockError::Poisoned(error)) => {
                    let mut slots = error.into_inner();
                    reap_retired(&mut slots);
                    slots.iter().all(|slot| matches!(slot, CommandSlot::Free))
                }
                Err(std::sync::TryLockError::WouldBlock) => false,
            };
            if quiescent {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(operation_error(
                    "LIPC command ownership is still active or unconfirmed",
                ));
            }
            // One deadline covers all slots; never spend a timeout per child.
            std::thread::sleep(remaining.min(POLL_INTERVAL));
        }
    }

    fn run_with_timeout(
        &self,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<String, PlatformError> {
        // Reply framing and direct-child completion share one userspace
        // deadline. Kernel spawn/kill operations cannot be preempted by it.
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| operation_error("LIPC operation deadline is invalid"))?;
        let mut lease = self.reserve()?;
        if Instant::now() >= deadline {
            return Err(operation_error("LIPC operation timed out"));
        }
        lease.child = Some(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let result = read_reply(lease.child.as_mut().unwrap(), deadline);
        let cleanup = lease.finish();
        match (result, cleanup) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(operation_error(&format!(
                "{error}; direct-child cleanup remains pending: {cleanup}"
            ))),
        }
    }

    #[cfg(test)]
    fn occupied_slots(&self) -> usize {
        self.slots
            .lock()
            .unwrap()
            .iter()
            .filter(|slot| !matches!(slot, CommandSlot::Free))
            .count()
    }
}

struct CommandLease<'a> {
    runner: &'a CommandRunner,
    slot: Option<usize>,
    child: Option<Child>,
}

impl CommandLease<'_> {
    fn finish(&mut self) -> Result<(), PlatformError> {
        let Some(index) = self.slot.take() else {
            return Ok(());
        };
        let mut replacement = CommandSlot::Free;
        if let Some(mut child) = self.child.take() {
            // read_reply normally owns stdout. Also close it here if unwinding
            // reached cleanup before reply parsing took ownership.
            drop(child.stdin.take());
            drop(child.stdout.take());
            drop(child.stderr.take());
            // Retained children never retain a parent-side pipe or reader thread.
            if !retire_direct_child(&mut child, CLEANUP_TIMEOUT) {
                replacement = CommandSlot::Retired(child);
            }
        }
        let pending = matches!(replacement, CommandSlot::Retired(_));
        self.runner
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index] = replacement;
        if pending {
            return Err(operation_error(
                "LIPC direct child did not exit during bounded cleanup",
            ));
        }
        Ok(())
    }
}

impl Drop for CommandLease<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            crate::diagnostic!("Retire LIPC direct child failed: {error}");
        }
    }
}

pub(super) fn run_command(program: &Path, arguments: &[&str]) -> Result<String, PlatformError> {
    COMMANDS
        .get_or_init(CommandRunner::default)
        .run_with_timeout(Command::new(program).args(arguments), OPERATION_TIMEOUT)
}

pub(super) fn quiesce_commands(timeout: Duration) -> Result<(), PlatformError> {
    match COMMANDS.get() {
        Some(commands) => commands.quiesce(timeout),
        None => Ok(()),
    }
}

fn reap_retired(slots: &mut [CommandSlot; MAX_COMMAND_SLOTS]) {
    for slot in slots.iter_mut() {
        if let CommandSlot::Retired(child) = slot {
            // Reap only a confirmed exited direct child. Unknown/live children
            // keep their original slot; no descendant or numeric PID is signalled.
            if matches!(child.try_wait(), Ok(Some(_))) {
                *slot = CommandSlot::Free;
            }
        }
    }
}

fn read_reply(child: &mut Child, deadline: Instant) -> Result<String, PlatformError> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| operation_error("LIPC stdout pipe is unavailable"))?;
    set_nonblocking(stdout.as_raw_fd())?;
    let mut output = Vec::with_capacity(MAX_OUTPUT_BYTES + 1);
    let mut fragment = [0_u8; 1024];
    let mut eof = false;
    let mut status = None;
    loop {
        if Instant::now() >= deadline {
            return Err(operation_error("LIPC operation timed out"));
        }
        if !eof {
            let maximum = fragment.len().min(MAX_OUTPUT_BYTES + 1 - output.len());
            match stdout.read(&mut fragment[..maximum]) {
                Ok(0) => eof = true,
                Ok(count) => {
                    output.extend_from_slice(&fragment[..count]);
                    if output.len() > MAX_OUTPUT_BYTES {
                        return Err(operation_error("LIPC output exceeds its 4096-byte budget"));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if let Some(status) = status {
            if !status.success() {
                return Err(operation_error("LIPC operation was rejected"));
            }
            if eof {
                return String::from_utf8(output)
                    .map_err(|_| operation_error("LIPC output is not valid UTF-8"));
            }
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(POLL_INTERVAL),
        );
    }
}

fn set_nonblocking(descriptor: RawFd) -> io::Result<()> {
    // SAFETY: this is the owned child stdout descriptor and fcntl takes integer
    // flags. Ownership remains with ChildStdout for every success/error path.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn retire_direct_child(child: &mut Child, timeout: Duration) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => return true,
        Ok(None) => {
            // try_wait confirmed the original direct child is still owned.
            // Production never signals a descendant or a process group.
            let _ = child.kill();
        }
        Err(_) => return false,
    }
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(POLL_INTERVAL),
        );
    }
}

fn operation_error(message: &str) -> PlatformError {
    PlatformError::Operation(message.into())
}

#[cfg(test)]
#[path = "native_command_tests.rs"]
mod tests;
