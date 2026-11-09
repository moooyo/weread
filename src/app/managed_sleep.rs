//! Nonce-bound cooperative handoff to the launcher's resident sleep supervisor.

use super::session_restore::{LauncherContext, ManagedMarker};
use super::*;
use crate::platform::ManagedSleepState;
use anyhow::{bail, ensure};

const SUSPEND_TIMEOUT: Duration = Duration::from_secs(7);
const RESUME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
enum Phase {
    #[default]
    Running,
    Suspending {
        cycle: u64,
        started: Instant,
    },
    Suspended {
        cycle: u64,
    },
    Resuming {
        cycle: u64,
        started: Instant,
    },
    Failed,
}

impl Phase {
    fn cycle(&self) -> Option<u64> {
        match self {
            Self::Suspending { cycle, .. }
            | Self::Suspended { cycle }
            | Self::Resuming { cycle, .. } => Some(*cycle),
            Self::Running | Self::Failed => None,
        }
    }
}

#[derive(Default)]
pub(super) struct ManagedSleep {
    initialized: bool,
    context: Option<LauncherContext>,
    advertised: bool,
    phase: Phase,
    last_cycle: u64,
    pending_wake: bool,
}

/// Keeps the protocol testable without a framebuffer, signals, or an account.
trait ManagedSleepHost {
    fn supported(&self) -> bool;
    fn closing(&self) -> bool;
    fn render_state(&self) -> Result<ManagedSleepState>;
    fn set_sleeping(&self, sleeping: bool) -> Result<()>;
    fn native_base(&self) -> Option<bool> {
        None
    }
    fn set_native_base(&mut self, _: bool) -> Result<()> {
        bail!("The resident renderer cannot compensate a native output mode")
    }
    fn prepare_native_portrait_resume(
        &self,
        _: crate::platform::NativePortraitCalibration,
    ) -> Result<()> {
        Ok(())
    }
    fn prepare(&mut self) -> Result<()>;
    fn resume(&mut self) -> Result<()>;
}

impl ManagedSleepHost for App {
    fn supported(&self) -> bool {
        self.platform.managed_sleep_supported()
    }

    fn closing(&self) -> bool {
        self.closing.get()
    }

    fn render_state(&self) -> Result<ManagedSleepState> {
        Ok(self.platform.managed_sleep_state()?)
    }

    fn set_sleeping(&self, sleeping: bool) -> Result<()> {
        Ok(self.platform.set_managed_sleep(sleeping)?)
    }

    fn native_base(&self) -> Option<bool> {
        self.platform.native_output_inverted()
    }

    fn set_native_base(&mut self, inverted: bool) -> Result<()> {
        self.synchronize_native_dark_mode(inverted)
    }
    fn prepare_native_portrait_resume(
        &self,
        observation: crate::platform::NativePortraitCalibration,
    ) -> Result<()> {
        Ok(self.platform.prepare_native_portrait_resume(observation)?)
    }

    fn prepare(&mut self) -> Result<()> {
        self.prepare_resident_sleep()
    }

    fn resume(&mut self) -> Result<()> {
        self.resume_resident_sleep()
    }
}

impl App {
    /// Return true while ordinary UI actions and application ticks must be held.
    /// The launcher's SIGSTOP freezes all threads only after this protocol has
    /// acknowledged durable state and the renderer's input/display release.
    pub fn poll_managed_sleep(
        &mut self,
        suspend_requested: bool,
        resume_requested: bool,
    ) -> Result<bool> {
        let mut controller = std::mem::take(&mut self.managed_sleep);
        let result = controller.poll(self, suspend_requested, resume_requested, Instant::now());
        self.managed_sleep = controller;
        result
    }
}

impl ManagedSleep {
    fn poll(
        &mut self,
        host: &mut impl ManagedSleepHost,
        suspend_requested: bool,
        resume_requested: bool,
        now: Instant,
    ) -> Result<bool> {
        let result = self.poll_inner(host, suspend_requested, resume_requested, now);
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }

    fn poll_inner(
        &mut self,
        host: &mut impl ManagedSleepHost,
        suspend_requested: bool,
        resume_requested: bool,
        now: Instant,
    ) -> Result<bool> {
        if matches!(self.phase, Phase::Failed) {
            bail!("The resident sleep protocol has already failed");
        }
        if host.closing() {
            return Ok(true);
        }
        if !host.supported() {
            ensure!(
                matches!(self.phase, Phase::Running),
                "Managed rendering became unavailable during a sleep handoff"
            );
            return Ok(false);
        }
        if !self.initialized {
            self.context = LauncherContext::from_environment()?;
            self.initialized = true;
        }
        let Some(context) = self.context.as_ref() else {
            // A direct or ordinary desktop launch cannot manufacture a trusted
            // supervisor merely by receiving a process signal.
            return Ok(false);
        };
        if context.matches("ordinary-exit")? {
            return Ok(true);
        }

        let state = host.render_state()?;
        ensure!(
            state != ManagedSleepState::Terminated,
            "The resident renderer terminated during a sleep handoff"
        );
        if !self.advertised {
            if state != ManagedSleepState::Running {
                return Ok(false);
            }
            context.publish_managed_sleep()?;
            self.advertised = true;
        }
        if resume_requested {
            if let Some(cycle) = self.phase.cycle() {
                // Signals are hints. Only the matching private cycle record can
                // authorize display/input acquisition after native dismissal.
                if context.managed_cycle(ManagedMarker::WakeRequest)? == Some(cycle) {
                    self.pending_wake = true;
                }
            }
        }

        match self.phase {
            Phase::Running => {
                ensure!(
                    state == ManagedSleepState::Running,
                    "The renderer changed ownership without a managed sleep request"
                );
                if !suspend_requested {
                    return Ok(false);
                }
                let Some(cycle) = context.managed_cycle(ManagedMarker::SleepRequest)? else {
                    return Ok(false);
                };
                if cycle <= self.last_cycle {
                    return Ok(false);
                }
                ensure!(
                    context.matches("sleep-request")?,
                    "The managed sleep request has no cold-resume checkpoint intent"
                );
                self.last_cycle = cycle;
                self.pending_wake = false;
                self.phase = Phase::Suspending {
                    cycle,
                    started: now,
                };
                host.prepare()?;
                host.set_sleeping(true)?;
                Ok(true)
            }
            Phase::Suspending { cycle, started } => {
                ensure!(
                    now.saturating_duration_since(started) < SUSPEND_TIMEOUT,
                    "The resident renderer did not release display and input in time"
                );
                match state {
                    ManagedSleepState::Running | ManagedSleepState::Suspending => {}
                    ManagedSleepState::Suspended => {
                        context.acknowledge_managed_sleep(cycle)?;
                        self.phase = Phase::Suspended { cycle };
                    }
                    ManagedSleepState::Resuming | ManagedSleepState::Terminated => {
                        bail!("The renderer could not complete its managed sleep release");
                    }
                }
                Ok(true)
            }
            Phase::Suspended { cycle } => {
                ensure!(
                    state == ManagedSleepState::Suspended,
                    "The suspended renderer changed ownership without a wake request"
                );
                if self.pending_wake {
                    ensure!(
                        context.managed_cycle(ManagedMarker::WakeRequest)? == Some(cycle),
                        "The resident wake request is no longer current"
                    );
                    self.pending_wake = false;
                    let native_base = context.managed_native_output_mode(cycle)?;
                    ensure!(
                        host.native_base().is_none() || native_base.is_some(),
                        "The native output mode is unavailable for the current wake cycle"
                    );
                    if let Some(inverted) = native_base {
                        // Synchronize the confirmed system mode while display
                        // and input remain suspended, before the full restore.
                        host.set_native_base(inverted)?;
                    }
                    if let Some(observation) = context.managed_native_portrait_calibration(cycle)? {
                        // Fresh holding/scanout metadata is staged while input
                        // and display are still suspended, before the full draw.
                        host.prepare_native_portrait_resume(observation)?;
                    }
                    host.set_sleeping(false)?;
                    self.phase = Phase::Resuming {
                        cycle,
                        started: now,
                    };
                }
                Ok(true)
            }
            Phase::Resuming { cycle, started } => {
                ensure!(
                    now.saturating_duration_since(started) < RESUME_TIMEOUT,
                    "The resident renderer did not restore display and input in time"
                );
                match state {
                    ManagedSleepState::Suspended | ManagedSleepState::Resuming => Ok(true),
                    ManagedSleepState::Running => {
                        ensure!(
                            context.managed_cycle(ManagedMarker::WakeRequest)? == Some(cycle)
                                && !context.matches("ordinary-exit")?,
                            "The resident wake request changed before completion"
                        );
                        host.resume()?;
                        context.write_managed_cycle(ManagedMarker::WakeReady, cycle)?;
                        self.phase = Phase::Running;
                        self.pending_wake = false;
                        Ok(false)
                    }
                    ManagedSleepState::Suspending | ManagedSleepState::Terminated => {
                        bail!("The renderer could not complete its managed wake restoration")
                    }
                }
            }
            Phase::Failed => unreachable!("A failed protocol is rejected before polling"),
        }
    }
}

#[cfg(test)]
#[path = "managed_sleep_tests.rs"]
mod tests;
