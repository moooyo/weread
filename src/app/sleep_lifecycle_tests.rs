//! Exercise the real application sleep overlay through controlled display completion.

use super::super::{
    managed_sleep::ManagedSleep,
    session_restore::{LauncherContext, ManagedMarker},
};
use super::*;

std::thread_local! {
    // RenderSink is Send + Sync; the actual Slint handle stays on its UI thread.
    static SLEEP_UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
}

fn sleep_overlay_active() -> bool {
    SLEEP_UI.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(slint::Weak::upgrade)
            .expect("The sleep lifecycle fixture must retain its Slint window")
            .global::<SleepScreenModel>()
            .get_active()
    })
}

#[derive(Default)]
struct SleepDisplayState {
    backend: ManagedSleepState,
    requested: u64,
    completed: u64,
    sleep_calls: Vec<(bool, bool)>,
}

#[derive(Default)]
struct SleepLifecycleDisplay(Mutex<SleepDisplayState>);

impl SleepLifecycleDisplay {
    fn complete_presentation(&self) {
        let mut state = self.0.lock().unwrap();
        state.completed = state.requested;
    }

    fn set_state(&self, backend: ManagedSleepState) {
        self.0.lock().unwrap().backend = backend;
    }

    fn sleep_calls(&self) -> Vec<(bool, bool)> {
        self.0.lock().unwrap().sleep_calls.clone()
    }
}

impl RenderSink for SleepLifecycleDisplay {
    fn set_dark_mode(&self, _: bool) -> std::result::Result<(), PlatformError> {
        Ok(())
    }

    fn managed_sleep_supported(&self) -> bool {
        true
    }

    fn observes_native_output_on_resume(&self) -> bool {
        true
    }

    fn set_managed_sleep(&self, sleeping: bool) -> std::result::Result<(), PlatformError> {
        let overlay_active = sleep_overlay_active();
        let mut state = self.0.lock().unwrap();
        state.sleep_calls.push((sleeping, overlay_active));
        state.backend = if sleeping {
            ManagedSleepState::Suspending
        } else {
            ManagedSleepState::Resuming
        };
        Ok(())
    }

    fn managed_sleep_state(&self) -> std::result::Result<ManagedSleepState, PlatformError> {
        Ok(self.0.lock().unwrap().backend)
    }

    fn request_sleep_presentation(&self) -> std::result::Result<u64, PlatformError> {
        assert!(sleep_overlay_active());
        let mut state = self.0.lock().unwrap();
        state.requested += 1;
        Ok(state.requested)
    }

    fn sleep_presentation_completed(&self, token: u64) -> std::result::Result<bool, PlatformError> {
        let state = self.0.lock().unwrap();
        if token == 0 || token > state.requested {
            return Err(PlatformError::InvalidValue(
                "unknown fixture presentation token",
            ));
        }
        Ok(token <= state.completed)
    }
}

fn run_cover_sleep_lifecycle(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    path: &Path,
    _: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let native = install_retired_native_control(&mut state, display)?;
    native.blocked.store(false, Ordering::SeqCst);
    let sink = Arc::new(SleepLifecycleDisplay::default());
    state.platform.set_render_sink(sink.clone())?;
    SLEEP_UI.with(|slot| *slot.borrow_mut() = Some(ui.as_weak()));

    state.prefs.sleep_cover = true;
    state.reader.draft = PinyinInput::new("A thought survives the cover sleep cycle.");
    ui.global::<ReaderModel>().set_writing_thought(true);
    let mut book = state.store.book("resident-reader")?.unwrap();
    book.author = "Retained fixture author".into();
    book.cover_url = "https://example.invalid/resident-sleep-cover.png".into();
    state.store.upsert_book(&book)?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(3, 5).write_to(&mut encoded, image::ImageFormat::Png)?;
    let thumbnail_key = super::super::sleep_screen::thumbnail_key(&book.id, &book.cover_url)
        .expect("The fixture cover source must have a thumbnail key");
    state
        .cache
        .put_cover(&thumbnail_key, encoded.get_ref(), 400, 560)?;
    state.render_reader()?;

    // The current-thread runtime is never driven: public-image work can be
    // admitted and cancelled here without performing a network operation.
    state.force_offline = false;
    state.online = true;
    state.schedule_visible_covers();
    assert!(state.pending_covers.contains(&book.id));

    let route = state.route.clone();
    let page = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .clone();
    let buffers = resident_buffer_addresses(&state);
    let draft = state.reader.draft.query.clone();
    let epoch = state.jobs.account_epoch();
    let reading_started = state.reader.session_started;
    let saved_before = state
        .store
        .setting::<serde_json::Value>("reader.resume.v1")?;
    let launcher_directory = tempfile::tempdir_in(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            launcher_directory.path(),
            std::fs::Permissions::from_mode(0o700),
        )?;
    }
    let context = LauncherContext::managed_test_context(launcher_directory.path().to_path_buf())?;
    state.managed_sleep = ManagedSleep::with_test_context(context.clone());
    context.write("sleep-request")?;
    context.write_managed_cycle(ManagedMarker::SleepRequest, 1)?;
    context.write_managed_cycle(ManagedMarker::CoverRequest, 1)?;

    assert!(state.poll_managed_sleep(true, false)?);
    let overlay = ui.global::<SleepScreenModel>();
    assert!(overlay.get_active() && overlay.get_has_cover());
    assert_eq!(overlay.get_title().as_str(), book.title.as_str());
    assert_eq!(overlay.get_author().as_str(), book.author.as_str());
    assert!(state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert_eq!(state.reader.session_started, reading_started);
    assert_eq!(state.reader.session_seconds, 23);
    assert_eq!(state.jobs.account_epoch(), epoch);
    assert_eq!(native.checks.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .store
            .setting::<serde_json::Value>("reader.resume.v1")?,
        saved_before
    );
    assert!(sink.sleep_calls().is_empty());
    assert!(state.poll_managed_sleep(false, false)?);
    assert!(sink.sleep_calls().is_empty());
    assert!(!context.matches("resume-ready")?);

    sink.complete_presentation();
    assert!(state.poll_managed_sleep(false, false)?);
    assert!(overlay.get_active());
    assert!(state.resident_lifecycle.is_prepared());
    assert_eq!(native.checks.load(Ordering::SeqCst), 1);
    assert_eq!(state.jobs.account_epoch(), epoch + 1);
    assert!(state.pending_covers.is_empty());
    assert_eq!(sink.sleep_calls(), vec![(true, true)]);
    assert_eq!(
        state.platform.managed_sleep_state()?,
        ManagedSleepState::Suspending
    );
    let saved = state
        .store
        .setting::<serde_json::Value>("reader.resume.v1")?
        .unwrap();
    assert_eq!(saved["anchor"].as_u64(), Some(page.start as u64));
    assert_eq!(context.managed_cycle(ManagedMarker::CoverReady)?, None);
    assert_eq!(context.managed_cycle(ManagedMarker::SleepReady)?, None);
    assert!(!context.matches("resume-ready")?);
    assert!(state.poll_managed_sleep(false, false)?);
    assert_eq!(context.managed_cycle(ManagedMarker::SleepReady)?, None);

    sink.set_state(ManagedSleepState::Suspended);
    assert!(state.poll_managed_sleep(false, false)?);
    assert_eq!(context.managed_cycle(ManagedMarker::CoverReady)?, Some(1));
    assert_eq!(context.managed_cycle(ManagedMarker::SleepReady)?, Some(1));
    assert!(context.matches("resume-ready")?);

    context.write_managed_cycle(ManagedMarker::WakeRequest, 1)?;
    assert!(state.poll_managed_sleep(false, true)?);
    assert!(!overlay.get_active());
    // The sink sampled the actual Slint model inside set_managed_sleep(false).
    assert_eq!(sink.sleep_calls(), vec![(true, true), (false, false)]);
    assert_eq!(
        state.platform.managed_sleep_state()?,
        ManagedSleepState::Resuming
    );
    assert!(state.managed_sleep_blocked);
    assert!(state.resident_lifecycle.is_prepared());
    assert!(state.pending_covers.is_empty());
    assert_eq!(context.managed_cycle(ManagedMarker::WakeReady)?, None);

    sink.set_state(ManagedSleepState::Running);
    assert!(!state.poll_managed_sleep(false, false)?);
    assert!(!state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert_eq!(context.managed_cycle(ManagedMarker::WakeReady)?, Some(1));
    assert_eq!(state.route, route);
    assert_eq!(
        state.reader.paginator.as_ref().unwrap().current_page(),
        &page
    );
    assert_eq!(resident_buffer_addresses(&state), buffers);
    assert_eq!(state.reader.draft.query, draft);
    assert!(ui.global::<ReaderModel>().get_writing_thought());
    assert!(state.pending_covers.contains(&book.id));

    state.force_offline = true;
    state.jobs.cancel_all();
    SLEEP_UI.with(|slot| *slot.borrow_mut() = None);
    Ok(())
}

#[test]
fn real_reader_sleep_waits_for_presentation_and_restores_the_original_frame() -> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::sleep_lifecycle_tests::real_reader_sleep_waits_for_presentation_and_restores_the_original_frame";
    isolated_lifecycle_resource_case(CASE, run_cover_sleep_lifecycle, false)
}
