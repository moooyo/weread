use super::*;
use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
};

struct Host {
    supported: bool,
    closing: bool,
    state: Cell<ManagedSleepState>,
    events: RefCell<Vec<&'static str>>,
    prepare_fails: bool,
    resume_fails: bool,
    resume_action: Option<Box<dyn FnOnce() -> Result<()>>>,
    native_base: Cell<Option<bool>>,
    native_base_fails: bool,
    portrait_observations: RefCell<Vec<crate::platform::NativePortraitCalibration>>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            supported: true,
            closing: false,
            state: Cell::new(ManagedSleepState::Running),
            events: RefCell::new(Vec::new()),
            prepare_fails: false,
            resume_fails: false,
            resume_action: None,
            native_base: Cell::new(None),
            native_base_fails: false,
            portrait_observations: RefCell::new(Vec::new()),
        }
    }
}

impl ManagedSleepHost for Host {
    fn supported(&self) -> bool {
        self.supported
    }
    fn closing(&self) -> bool {
        self.closing
    }
    fn render_state(&self) -> Result<ManagedSleepState> {
        Ok(self.state.get())
    }
    fn set_sleeping(&self, sleeping: bool) -> Result<()> {
        self.events.borrow_mut().push(if sleeping {
            "request_sleep"
        } else {
            "request_wake"
        });
        self.state.set(if sleeping {
            ManagedSleepState::Suspending
        } else {
            ManagedSleepState::Resuming
        });
        Ok(())
    }
    fn native_base(&self) -> Option<bool> {
        self.native_base.get()
    }
    fn set_native_base(&mut self, inverted: bool) -> Result<()> {
        self.events.borrow_mut().push("native_base");
        ensure!(
            !self.native_base_fails,
            "The synthetic native compensation failed"
        );
        self.native_base.set(Some(inverted));
        Ok(())
    }
    fn prepare(&mut self) -> Result<()> {
        self.events.borrow_mut().push("prepare");
        ensure!(!self.prepare_fails, "The synthetic save failed");
        Ok(())
    }
    fn prepare_native_portrait_resume(
        &self,
        observation: crate::platform::NativePortraitCalibration,
    ) -> Result<()> {
        ensure!(
            self.state.get() == ManagedSleepState::Suspended,
            "The synthetic portrait preparation must precede input acquisition"
        );
        self.events.borrow_mut().push("native_portrait");
        self.portrait_observations.borrow_mut().push(observation);
        Ok(())
    }
    fn resume(&mut self) -> Result<()> {
        self.events.borrow_mut().push("resume");
        ensure!(!self.resume_fails, "The synthetic wake recovery failed");
        if let Some(action) = self.resume_action.take() {
            action()?;
        }
        Ok(())
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    controller: ManagedSleep,
    host: Host,
    now: Instant,
}

impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
        }
        let context = LauncherContext::managed_test_context(directory.path().to_path_buf())?;
        Ok(Self {
            directory,
            controller: ManagedSleep {
                initialized: true,
                context: Some(context),
                ..ManagedSleep::default()
            },
            host: Host::default(),
            now: Instant::now(),
        })
    }
    fn context(&self) -> &LauncherContext {
        self.controller.context.as_ref().unwrap()
    }
    fn poll(&mut self, sleep: bool, wake: bool) -> Result<bool> {
        self.controller.poll(&mut self.host, sleep, wake, self.now)
    }
    fn request_sleep(&mut self, cycle: u64) -> Result<()> {
        for name in [
            "managed-sleep-ready",
            "managed-wake-ready",
            "managed-wake-request",
            "resume-ready",
        ] {
            match fs::remove_file(self.path(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.context().write("sleep-request")?;
        self.context()
            .write_managed_cycle(ManagedMarker::SleepRequest, cycle)
    }
    fn sleep(&mut self, cycle: u64) -> Result<()> {
        self.request_sleep(cycle)?;
        assert!(self.poll(true, false)?);
        self.host.state.set(ManagedSleepState::Suspended);
        assert!(self.poll(false, false)?);
        assert_eq!(
            self.context().managed_cycle(ManagedMarker::SleepReady)?,
            Some(cycle)
        );
        assert!(self.context().matches("resume-ready")?);
        Ok(())
    }
    fn request_wake(&mut self, cycle: u64) -> Result<()> {
        self.context()
            .write_managed_cycle(ManagedMarker::WakeRequest, cycle)
    }
    fn wake(&mut self, cycle: u64) -> Result<()> {
        self.request_wake(cycle)?;
        assert!(self.poll(false, true)?);
        assert_eq!(
            self.context().managed_cycle(ManagedMarker::WakeReady)?,
            None
        );
        self.host.state.set(ManagedSleepState::Running);
        assert!(!self.poll(false, false)?);
        assert_eq!(
            self.context().managed_cycle(ManagedMarker::WakeReady)?,
            Some(cycle)
        );
        Ok(())
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join(name)
    }
}

fn write_private(path: &Path, value: &str) -> Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(value.as_bytes())?;
    Ok(())
}

#[test]
fn two_resident_cycles_require_renderer_release_and_restore_before_their_acknowledgements()
-> Result<()> {
    let mut fixture = Fixture::new()?;
    assert!(!fixture.poll(false, false)?);
    assert!(fixture.context().matches("managed-sleep-available")?);
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false)?);
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::SleepReady)?,
        None
    );
    assert!(!fixture.context().matches("resume-ready")?);
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep"]
    );
    assert!(
        fixture.poll(true, false)?,
        "Repeated signals must not repeat preparation"
    );
    assert_eq!(fixture.host.events.borrow().len(), 2);
    fixture.host.state.set(ManagedSleepState::Suspended);
    assert!(fixture.poll(false, false)?);
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::SleepReady)?,
        Some(1)
    );
    assert!(fixture.context().matches("resume-ready")?);
    fixture.request_wake(1)?;
    assert!(fixture.poll(false, true)?);
    assert!(fixture.poll(false, true)?);
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::WakeReady)?,
        None
    );
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep", "request_wake"]
    );
    fixture.host.state.set(ManagedSleepState::Running);
    assert!(!fixture.poll(false, false)?);
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::WakeReady)?,
        Some(1)
    );
    fs::remove_file(fixture.path("managed-sleep-ready"))?;
    assert!(
        !fixture.poll(true, false)?,
        "The same cycle must not start another handoff"
    );
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::SleepReady)?,
        None
    );
    assert_eq!(fixture.host.events.borrow().len(), 4);
    fs::remove_file(fixture.path("managed-wake-ready"))?;
    fixture.sleep(3)?;
    fixture.wake(3)?;
    assert_eq!(
        &*fixture.host.events.borrow(),
        &[
            "prepare",
            "request_sleep",
            "request_wake",
            "resume",
            "prepare",
            "request_sleep",
            "request_wake",
            "resume",
        ]
    );
    Ok(())
}

#[test]
fn foreign_or_malformed_cycle_requests_cannot_prepare_or_acknowledge_sleep() -> Result<()> {
    for record in [
        "7:ffffffffffffffffffffffffffffffff:1\n",
        "6:0123456789abcdef0123456789abcdef:1\n",
        "7:0123456789abcdef0123456789abcdef:0\n",
        "7:0123456789abcdef0123456789abcdef:01\n",
        "7:0123456789abcdef0123456789abcdef:-1\n",
        "7:0123456789abcdef0123456789abcdef:18446744073709551616\n",
        "7:0123456789abcdef0123456789abcdef:1:2\n",
    ] {
        let mut fixture = Fixture::new()?;
        fixture.context().write("sleep-request")?;
        write_private(&fixture.path("managed-sleep-request"), record)?;
        assert!(!fixture.poll(true, false)?);
        assert!(fixture.host.events.borrow().is_empty());
        assert_eq!(
            fixture.context().managed_cycle(ManagedMarker::SleepReady)?,
            None
        );
    }
    Ok(())
}

#[test]
fn an_owner_or_generation_replacement_invalidates_a_loaded_protocol_context() -> Result<()> {
    for (name, value) in [
        ("nonce", "ffffffffffffffffffffffffffffffff\n"),
        ("generation", "8:0123456789abcdef0123456789abcdef\n"),
        ("application-pid", "0\n"),
    ] {
        let mut fixture = Fixture::new()?;
        assert!(!fixture.poll(false, false)?);
        fixture.request_sleep(1)?;
        write_private(&fixture.path(name), value)?;
        assert!(fixture.poll(true, false).is_err());
        assert!(fixture.host.events.borrow().is_empty());
        assert!(!fixture.path("managed-sleep-ready").exists());
    }
    Ok(())
}

#[test]
fn a_wake_signal_cannot_cross_sleep_cycles_or_implicitly_resume_an_ordinary_launch() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.request_wake(1)?;
    assert!(!fixture.poll(false, true)?);
    assert!(fixture.host.events.borrow().is_empty());
    fixture.sleep(1)?;
    fixture.request_wake(2)?;
    assert!(fixture.poll(false, true)?);
    assert_eq!(fixture.host.state.get(), ManagedSleepState::Suspended);
    assert_eq!(fixture.host.events.borrow().len(), 2);
    fixture.request_wake(1)?;
    assert!(
        fixture.poll(false, false)?,
        "An invalid wake signal cannot be reused later"
    );
    assert_eq!(fixture.host.events.borrow().len(), 2);
    fixture.wake(1)?;
    Ok(())
}

#[test]
fn simultaneous_start_signals_cannot_skip_native_sleep_ownership() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.request_sleep(1)?;
    fixture.request_wake(1)?;
    assert!(fixture.poll(true, true)?);
    fixture.host.state.set(ManagedSleepState::Suspended);
    assert!(fixture.poll(false, false)?);
    assert_eq!(fixture.host.events.borrow().len(), 2);
    assert!(fixture.poll(false, false)?);
    assert_eq!(fixture.host.state.get(), ManagedSleepState::Suspended);
    fixture.wake(1)?;
    Ok(())
}

#[test]
fn unsupported_unmanaged_and_explicit_exit_paths_never_advertise_or_acknowledge_sleep() -> Result<()>
{
    let mut fixture = Fixture::new()?;
    fixture.host.supported = false;
    assert!(!fixture.poll(true, true)?);
    assert!(!fixture.path("managed-sleep-available").exists());
    assert!(fixture.host.events.borrow().is_empty());

    let mut host = Host::default();
    let mut unmanaged = ManagedSleep {
        initialized: true,
        ..ManagedSleep::default()
    };
    assert!(!unmanaged.poll(&mut host, true, true, Instant::now())?);
    assert!(host.events.borrow().is_empty());

    let mut fixture = Fixture::new()?;
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false)?);
    fixture.host.state.set(ManagedSleepState::Suspended);
    fixture.context().write("ordinary-exit")?;
    assert!(fixture.poll(false, false)?);
    assert!(!fixture.path("managed-sleep-ready").exists());
    assert!(!fixture.context().matches("resume-ready")?);
    fixture.host.closing = true;
    assert!(fixture.poll(true, true)?);
    assert_eq!(fixture.host.events.borrow().len(), 2);
    Ok(())
}

#[test]
fn preparation_failure_and_renderer_loss_never_produce_a_sleep_acknowledgement() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.host.prepare_fails = true;
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false).is_err());
    assert_eq!(&*fixture.host.events.borrow(), &["prepare"]);
    assert!(!fixture.path("managed-sleep-ready").exists());
    assert!(!fixture.context().matches("resume-ready")?);
    assert!(fixture.poll(false, false).is_err());

    for state in [ManagedSleepState::Terminated] {
        let mut fixture = Fixture::new()?;
        fixture.request_sleep(1)?;
        assert!(fixture.poll(true, false)?);
        fixture.host.state.set(state);
        assert!(fixture.poll(false, false).is_err());
        assert!(!fixture.path("managed-sleep-ready").exists());
    }
    let mut fixture = Fixture::new()?;
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false)?);
    fixture.host.supported = false;
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-sleep-ready").exists());
    Ok(())
}

#[test]
fn a_replaced_sleep_intent_cannot_reuse_a_late_renderer_release_acknowledgement() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false)?);
    fixture
        .context()
        .write_managed_cycle(ManagedMarker::SleepRequest, 2)?;
    fixture.host.state.set(ManagedSleepState::Suspended);
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.context().matches("resume-ready")?);
    assert!(!fixture.path("managed-sleep-ready").exists());
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep"]
    );
    Ok(())
}

#[test]
fn suspend_and_wake_deadlines_fail_closed_and_never_report_success() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.request_sleep(1)?;
    assert!(fixture.poll(true, false)?);
    fixture.now += SUSPEND_TIMEOUT;
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-sleep-ready").exists());

    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    assert!(fixture.poll(false, true)?);
    fixture.now += RESUME_TIMEOUT;
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-wake-ready").exists());
    assert!(!fixture.host.events.borrow().contains(&"resume"));
    Ok(())
}

#[test]
fn lifecycle_restore_failure_preserves_the_unacknowledged_wake_for_fallback() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    assert!(fixture.poll(false, true)?);
    fixture.host.state.set(ManagedSleepState::Running);
    fixture.host.resume_fails = true;
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-wake-ready").exists());
    assert!(fixture.context().matches("resume-ready")?);
    assert!(fixture.poll(false, false).is_err());
    assert_eq!(
        fixture
            .host
            .events
            .borrow()
            .iter()
            .filter(|event| **event == "resume")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn a_replaced_wake_intent_cannot_acknowledge_or_resume_the_old_cycle() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    assert!(fixture.poll(false, true)?);
    fixture.request_wake(2)?;
    fixture.host.state.set(ManagedSleepState::Running);
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-wake-ready").exists());
    assert!(!fixture.host.events.borrow().contains(&"resume"));
    Ok(())
}

#[test]
fn a_wake_intent_changed_during_lifecycle_recovery_is_checked_again_before_acknowledgement()
-> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    assert!(fixture.poll(false, true)?);
    fixture.host.state.set(ManagedSleepState::Running);
    let marker = fixture.path("managed-wake-request");
    fixture.host.resume_action = Some(Box::new(move || {
        write_private(&marker, "7:0123456789abcdef0123456789abcdef:2\n")
    }));
    assert!(fixture.poll(false, false).is_err());
    assert!(!fixture.path("managed-wake-ready").exists());
    assert_eq!(
        fixture
            .host
            .events
            .borrow()
            .iter()
            .filter(|event| **event == "resume")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn native_output_snapshot_is_applied_before_the_resident_framebuffer_can_resume() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.host.native_base.set(Some(false));
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    write_private(
        &fixture.path("managed-native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:1:Y8INV\n",
    )?;
    assert!(fixture.poll(false, true)?);
    assert_eq!(fixture.host.native_base.get(), Some(true));
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep", "native_base", "request_wake"]
    );
    assert!(!fixture.path("managed-wake-ready").exists());
    fixture.host.state.set(ManagedSleepState::Running);
    assert!(!fixture.poll(false, false)?);
    assert_eq!(
        fixture.context().managed_cycle(ManagedMarker::WakeReady)?,
        Some(1)
    );

    fixture.sleep(3)?;
    fixture.request_wake(3)?;
    write_private(
        &fixture.path("managed-native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:3:Y8\n",
    )?;
    assert!(fixture.poll(false, true)?);
    assert_eq!(fixture.host.native_base.get(), Some(false));
    assert_eq!(
        &*fixture.host.events.borrow(),
        &[
            "prepare",
            "request_sleep",
            "native_base",
            "request_wake",
            "resume",
            "prepare",
            "request_sleep",
            "native_base",
            "request_wake",
        ]
    );
    Ok(())
}

#[test]
fn current_portrait_metadata_is_staged_while_input_remains_suspended() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    write_private(
        &fixture.path("managed-native-portrait-calibration"),
        "7:0123456789abcdef0123456789abcdef:1:D:1\n",
    )?;
    assert!(fixture.poll(false, true)?);
    assert_eq!(
        &*fixture.host.events.borrow(),
        &[
            "prepare",
            "request_sleep",
            "native_portrait",
            "request_wake"
        ]
    );
    assert_eq!(
        &*fixture.host.portrait_observations.borrow(),
        &[crate::platform::NativePortraitCalibration {
            direction: crate::platform::NativePortraitDirection::Down,
            framebuffer_rotation: 1
        }]
    );
    assert_eq!(fixture.host.state.get(), ManagedSleepState::Resuming);
    assert!(!fixture.path("managed-wake-ready").exists());
    Ok(())
}

#[test]
fn unavailable_or_stale_portrait_metadata_preserves_the_existing_resident_wake() -> Result<()> {
    for record in [
        None,
        Some("7:0123456789abcdef0123456789abcdef:2:D:1\n"),
        Some("6:0123456789abcdef0123456789abcdef:1:U:3\n"),
        Some("7:ffffffffffffffffffffffffffffffff:1:U:3\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:FIXED:3\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:U:4\n"),
    ] {
        let mut fixture = Fixture::new()?;
        fixture.sleep(1)?;
        fixture.request_wake(1)?;
        if let Some(record) = record {
            write_private(&fixture.path("managed-native-portrait-calibration"), record)?;
        }
        assert!(fixture.poll(false, true)?);
        assert!(fixture.host.portrait_observations.borrow().is_empty());
        assert_eq!(
            &*fixture.host.events.borrow(),
            &["prepare", "request_sleep", "request_wake"]
        );
        assert_eq!(fixture.host.state.get(), ManagedSleepState::Resuming);
    }
    Ok(())
}

#[test]
fn a_known_native_base_requires_a_valid_current_wake_snapshot() -> Result<()> {
    for snapshot in [
        None,
        Some("7:0123456789abcdef0123456789abcdef:2:Y8INV\n"),
        Some("7:ffffffffffffffffffffffffffffffff:1:Y8INV\n"),
        Some("6:0123456789abcdef0123456789abcdef:1:Y8INV\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:UNKNOWN\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:Y8:Y8INV\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:1\n"),
        Some("7:0123456789abcdef0123456789abcdef:1:y8inv\n"),
    ] {
        let mut fixture = Fixture::new()?;
        fixture.host.native_base.set(Some(false));
        fixture.sleep(1)?;
        fixture.request_wake(1)?;
        if let Some(snapshot) = snapshot {
            write_private(&fixture.path("managed-native-epdc-mode"), snapshot)?;
        }
        assert!(fixture.poll(false, true).is_err());
        assert_eq!(fixture.host.state.get(), ManagedSleepState::Suspended);
        assert_eq!(
            &*fixture.host.events.borrow(),
            &["prepare", "request_sleep"]
        );
        assert!(!fixture.path("managed-wake-ready").exists());
    }
    Ok(())
}

#[test]
fn unknown_native_base_remains_legacy_without_a_snapshot_but_can_adopt_a_verified_one() -> Result<()>
{
    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.wake(1)?;
    assert_eq!(fixture.host.native_base.get(), None);
    assert!(!fixture.host.events.borrow().contains(&"native_base"));

    let mut fixture = Fixture::new()?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    write_private(
        &fixture.path("managed-native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:1:Y8INV\n",
    )?;
    assert!(fixture.poll(false, true)?);
    assert_eq!(fixture.host.native_base.get(), Some(true));
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep", "native_base", "request_wake"]
    );
    Ok(())
}

#[test]
fn failed_native_compensation_cannot_release_the_framebuffer_resume_gate() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.host.native_base.set(Some(false));
    fixture.host.native_base_fails = true;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    write_private(
        &fixture.path("managed-native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:1:Y8INV\n",
    )?;
    assert!(fixture.poll(false, true).is_err());
    assert_eq!(fixture.host.native_base.get(), Some(false));
    assert_eq!(fixture.host.state.get(), ManagedSleepState::Suspended);
    assert_eq!(
        &*fixture.host.events.borrow(),
        &["prepare", "request_sleep", "native_base"]
    );
    assert!(!fixture.path("managed-wake-ready").exists());
    Ok(())
}

#[test]
fn native_mode_records_require_the_exact_owner_fields_and_reject_an_ordinary_exit() -> Result<()> {
    let mut fixture = Fixture::new()?;
    assert_eq!(fixture.context().native_output_mode()?, None);
    for (mode, expected) in [("Y8", false), ("Y8INV", true)] {
        write_private(
            &fixture.path("native-epdc-mode"),
            &format!("7:0123456789abcdef0123456789abcdef:{mode}\n"),
        )?;
        assert_eq!(fixture.context().native_output_mode()?, Some(expected));
    }
    for snapshot in [
        "7:ffffffffffffffffffffffffffffffff:Y8\n",
        "6:0123456789abcdef0123456789abcdef:Y8\n",
        "7:0123456789abcdef0123456789abcdef:1:Y8\n",
        "7:0123456789abcdef0123456789abcdef:UNKNOWN\n",
        "7:0123456789abcdef0123456789abcdef:1\n",
    ] {
        write_private(&fixture.path("native-epdc-mode"), snapshot)?;
        assert_eq!(fixture.context().native_output_mode()?, None);
    }
    write_private(
        &fixture.path("native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:Y8INV\n",
    )?;
    fixture.sleep(1)?;
    fixture.request_wake(1)?;
    write_private(
        &fixture.path("managed-native-epdc-mode"),
        "7:0123456789abcdef0123456789abcdef:1:Y8INV\n",
    )?;
    assert_eq!(fixture.context().managed_native_output_mode(1)?, Some(true));
    fixture.context().write("ordinary-exit")?;
    assert_eq!(fixture.context().native_output_mode()?, None);
    assert_eq!(fixture.context().managed_native_output_mode(1)?, None);
    Ok(())
}

#[test]
fn explicit_exit_records_retain_legacy_targets_without_manufacturing_a_new_target() -> Result<()> {
    for target in [None, Some(false), Some(true)] {
        let fixture = Fixture::new()?;
        fixture.context().mark_explicit_exit(target)?;
        assert_eq!(fixture.context().native_dark_target()?, target);
        assert!(fixture.context().matches("ordinary-exit")?);
    }
    let fixture = Fixture::new()?;
    fixture.context().mark_explicit_exit(None)?;
    assert!(fixture.context().matches("ordinary-exit")?);
    assert!(!fixture.path("native-dark-target").exists());
    fixture.context().clear_explicit_exit()?;
    for desired in [true, false] {
        fixture.context().mark_explicit_exit(Some(desired))?;
        assert_eq!(fixture.context().native_dark_target()?, Some(desired));
        assert!(fixture.context().matches("ordinary-exit")?);
        fixture.context().clear_explicit_exit()?;
        assert!(!fixture.context().matches("ordinary-exit")?);
        assert!(!fixture.path("native-dark-target").exists());
    }
    write_private(
        &fixture.path("native-dark-target"),
        "7:0123456789abcdef0123456789abcdef:1\n",
    )?;
    fixture.context().mark_explicit_exit(None)?;
    assert!(fixture.context().matches("ordinary-exit")?);
    assert!(
        !fixture.path("native-dark-target").exists(),
        "A failed older attempt cannot lend a stale target to unknown native state"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_exit_record_failures_cannot_publish_an_ordinary_native_appearance_acknowledgement()
-> Result<()> {
    use std::os::unix::fs::symlink;
    for failing in ["native-dark-target.pending", "ordinary-exit.pending"] {
        let fixture = Fixture::new()?;
        let unrelated = fixture.path("unrelated-private-data");
        write_private(&unrelated, "Unrelated synthetic data\n")?;
        symlink(&unrelated, fixture.path(failing))?;
        assert!(fixture.context().mark_explicit_exit(Some(true)).is_err());
        assert!(!fixture.context().matches("ordinary-exit")?);
        assert!(!fixture.path("native-dark-target").exists());
        assert_eq!(fs::read_to_string(unrelated)?, "Unrelated synthetic data\n");
    }
    Ok(())
}

#[test]
fn a_failed_ordinary_exit_cleanup_still_clears_its_owned_native_target() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.context().mark_explicit_exit(Some(true))?;
    fs::remove_file(fixture.path("ordinary-exit"))?;
    fs::create_dir(fixture.path("ordinary-exit"))?;
    assert!(fixture.context().clear_explicit_exit().is_err());
    assert!(!fixture.path("native-dark-target").exists());
    assert!(
        fixture.path("ordinary-exit").is_dir(),
        "Unrecognized records cannot be deleted"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlinked_cycle_record_is_rejected_without_touching_its_target() -> Result<()> {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new()?;
    fixture.context().write("sleep-request")?;
    let target = fixture.path("unrelated-private-record");
    write_private(&target, "Unrelated synthetic data\n")?;
    symlink(&target, fixture.path("managed-sleep-request"))?;
    assert!(fixture.poll(true, false).is_err());
    assert_eq!(fs::read_to_string(target)?, "Unrelated synthetic data\n");
    assert!(fixture.host.events.borrow().is_empty());
    assert!(!fixture.path("managed-sleep-ready").exists());
    Ok(())
}
