use super::*;
use crate::{
    preferences::ImageDithering,
    reader::layout_budget::{LayoutBudget, LayoutModelRegistry},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, sync::atomic::AtomicUsize};

fn request(key: &str, budget: &LayoutBudget) -> SceneRequest {
    SceneRequest {
        key: key.into(),
        viewport: (32, 32),
        pictures: Vec::new(),
        storage: budget.hold(64).unwrap(),
        registration: LayoutModelRegistry::default().register(64).unwrap(),
        notify: None,
    }
}

fn drain(state: &mut ImagePreparationState) {
    // Controlled workers have already been released through their channels.
    // No elapsed-time threshold is used as a scheduling or performance oracle.
    while state.busy() {
        state.poll().unwrap();
        thread::yield_now();
    }
}

fn blocked(
    state: &mut ImagePreparationState,
    request: SceneRequest,
) -> (mpsc::SyncSender<()>, mpsc::Receiver<()>) {
    let (entered_sender, entered) = mpsc::sync_channel(0);
    let (release, released) = mpsc::sync_channel(0);
    let (exited_sender, exited) = mpsc::sync_channel(1);
    assert!(
        state
            .request_with(request, move |_, _, _| {
                entered_sender.send(()).unwrap();
                released.recv().unwrap();
                exited_sender.send(()).unwrap();
                Ok(Vec::new())
            })
            .unwrap()
            .is_none()
    );
    entered.recv().unwrap();
    (release, exited)
}

#[test]
fn ready_scene_reuses_the_same_result_and_pixel_allocation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let gray = image::GrayImage::from_fn(16, 16, |x, y| image::Luma([(x + y * 16) as u8]));
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(gray).write_to(&mut encoded, image::ImageFormat::Png)?;
    let bytes = encoded.into_inner();
    let name = format!(
        "{}-{}.png",
        "a".repeat(64),
        hex::encode(Sha256::digest(&bytes))
    );
    let path = directory.path().join(name);
    std::fs::write(&path, bytes)?;
    let budget = LayoutBudget::default();
    let registry = LayoutModelRegistry::default();
    let mut initial = request("ready", &budget);
    initial.registration = registry.register(64)?;
    initial.pictures.push(PictureRequest {
        path,
        frame: (16, 16),
        fit: image_raster::PictureFit::Contain,
        method: ImageDithering::Hardware,
    });
    let mut state = ImagePreparationState::default();
    assert!(state.request(initial)?.is_none());
    drain(&mut state);
    assert!(state.ready());
    let first = state.request(request("ready", &budget))?.unwrap();
    state.cancel_pending();
    let second = state.request(request("ready", &budget))?.unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].storage_pointer(), second[0].storage_pointer());
    assert_eq!(state.stats().started, 1);
    assert_eq!(state.stats().completed, 1);
    assert_eq!(state.stats().reused, 2);
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    state.cancel();
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    drop(first);
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    drop(second);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(registry.live_metadata(), 0);
    Ok(())
}

#[test]
fn cancellation_keeps_the_worker_slot_and_admission_until_drain_and_join() {
    let budget = LayoutBudget::default();
    let registry = LayoutModelRegistry::default();
    let mut state = ImagePreparationState::default();
    let mut initial = request("old", &budget);
    initial.registration = registry.register(64).unwrap();
    let (release, exited) = blocked(&mut state, initial);
    assert!(
        state
            .request_with(request("old", &budget), |_, _, _| {
                panic!("An active matching key must reuse its worker")
            })
            .unwrap()
            .is_none()
    );
    assert!(
        !state
            .active
            .as_ref()
            .unwrap()
            .cancelled
            .load(Ordering::Acquire)
    );
    state.cancel();
    assert!(state.busy());
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    assert!(
        state
            .request_with(request("replacement", &budget), |_, _, _| {
                panic!("A cancelled worker must retain its slot")
            })
            .unwrap()
            .is_none()
    );
    assert_eq!(state.stats().started, 1);
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    release.send(()).unwrap();
    exited.recv().unwrap();
    drain(&mut state);
    assert_eq!(state.stats().cancelled, 1);
    assert_eq!(state.stats().stale, 1);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(registry.live_metadata(), 0);
    assert!(state.ready.is_none());
}

#[test]
fn pausing_pending_work_keeps_its_desired_key_for_a_later_fresh_request() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("paused", &budget));
    state.cancel_pending();
    assert_eq!(state.desired.as_deref(), Some("paused"));
    assert!(state.busy());
    release.send(())?;
    exited.recv()?;
    drain(&mut state);
    assert!(!state.ready());
    assert_eq!(state.desired.as_deref(), Some("paused"));
    state.request(request("paused", &budget))?;
    drain(&mut state);
    assert!(state.ready());
    assert_eq!(state.stats().started, 2);
    assert_eq!(state.stats().cancelled, 1);
    assert_eq!(state.stats().completed, 1);
    Ok(())
}

#[test]
fn quiesce_keeps_a_ready_scene_for_shared_publication_without_another_worker() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    state.request(request("retained", &budget))?;
    drain(&mut state);
    let first = state.request(request("retained", &budget))?.unwrap();
    state.quiesce(Duration::ZERO)?;
    assert!(state.ready());
    assert!(!state.restart_needed());
    let second = state.request(request("retained", &budget))?.unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    assert_eq!(state.stats().started, 1);
    assert_eq!(state.stats().completed, 1);
    Ok(())
}

#[test]
fn paused_lookup_only_reuses_ready_pixels_or_schedules_one_future_render() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    assert!(state.ready_while_paused("paused").is_none());
    assert_eq!(state.stats().started, 0);
    assert!(!state.busy());
    assert!(state.restart_needed());
    assert!(state.poll()?);
    assert!(!state.poll()?);
    state.request(request("paused", &budget))?;
    drain(&mut state);
    let first = state.request(request("paused", &budget))?.unwrap();
    let second = state.ready_while_paused("paused").unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    assert_eq!(state.stats().started, 1);
    assert_eq!(state.stats().completed, 1);
    assert!(!state.restart_needed());
    Ok(())
}

#[test]
fn paused_lookup_does_not_drain_a_finished_old_worker_or_start_its_replacement() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("before-pause", &budget));
    release.send(())?;
    exited.recv()?;
    while !state.active.as_ref().unwrap().thread.is_finished() {
        thread::yield_now();
    }
    assert!(state.ready_while_paused("after-pause").is_none());
    assert!(state.busy());
    assert_eq!(state.desired.as_deref(), Some("after-pause"));
    assert_eq!(state.stats().started, 1);
    assert_eq!(state.stats().cancelled, 0);
    assert!(state.restart_needed());
    assert!(state.poll()?);
    assert!(!state.busy());
    assert!(!state.poll()?);
    assert_eq!(state.stats().started, 1);
    assert_eq!(state.stats().cancelled, 1);
    state.request(request("after-pause", &budget))?;
    drain(&mut state);
    assert!(state.ready());
    assert_eq!(state.stats().started, 2);
    Ok(())
}

#[test]
fn quiesce_preserves_one_restart_notification_after_cancelling_a_pending_scene() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("resume", &budget));
    state.cancel_pending();
    assert!(state.restart_needed());
    release.send(())?;
    exited.recv()?;
    // Let the controlled worker finish before using a zero quiesce deadline.
    while !state.active.as_ref().unwrap().thread.is_finished() {
        thread::yield_now();
    }
    state.quiesce(Duration::ZERO)?;
    assert!(!state.busy());
    assert!(!state.ready());
    assert!(state.restart_needed());
    assert!(state.poll()?);
    assert!(!state.restart_needed());
    assert!(!state.poll()?);
    state.request(request("resume", &budget))?;
    drain(&mut state);
    assert!(state.ready());
    assert!(!state.restart_needed());
    assert_eq!(state.stats().started, 2);
    assert_eq!(state.stats().completed, 1);
    assert_eq!(state.stats().cancelled, 1);
    Ok(())
}

#[test]
fn rapid_targets_keep_only_the_latest_key_and_cannot_queue_workers() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("first", &budget));
    let invoked = Arc::new(AtomicUsize::new(0));
    for key in ["second", "third", "last"] {
        let invoked = invoked.clone();
        assert!(
            state
                .request_with(request(key, &budget), move |_, _, _| {
                    invoked.fetch_add(1, Ordering::SeqCst);
                    Ok(Vec::new())
                })?
                .is_none()
        );
    }
    assert_eq!(state.desired.as_deref(), Some("last"));
    assert_eq!(state.stats().started, 1);
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert_eq!(budget.usage().live_metadata, 64);
    release.send(())?;
    exited.recv()?;
    drain(&mut state);
    assert!(state.ready.is_none());
    assert_eq!(state.stats().cancelled, 1);
    assert_eq!(state.stats().stale, 1);
    assert!(state.request(request("last", &budget))?.is_none());
    drain(&mut state);
    let ready = state.request(request("last", &budget))?.unwrap();
    assert!(ready.is_empty());
    assert_eq!(state.ready.as_ref().unwrap().key, "last");
    assert_eq!(state.stats().started, 2);
    assert_eq!(state.stats().completed, 1);
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn raster_failure_keeps_its_type_is_reported_once_and_does_not_restart() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let limit = RasterLimitError {
        resource: "scene pixels",
        limit: 1,
        requested: 2,
    };
    state.request_with(
        request("limited", &budget),
        move |_, _, _| Err(limit.into()),
    )?;
    drain(&mut state);
    let error = state.request(request("limited", &budget)).unwrap_err();
    assert_eq!(error.downcast_ref::<RasterLimitError>(), Some(&limit));
    assert!(
        state
            .request_with(request("limited", &budget), |_, _, _| {
                panic!("An unchanged failure must not restart")
            })?
            .is_none()
    );
    assert!(!state.busy());
    assert_eq!(state.stats().started, 1);
    assert_eq!(budget.usage().live_metadata, 0);
    state.request(request("different", &budget))?;
    drain(&mut state);
    assert!(state.request(request("different", &budget))?.is_some());
    assert_eq!(state.stats().started, 2);
    Ok(())
}

#[test]
fn ordinary_failure_is_reported_once_and_explicit_cancel_allows_a_retry() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    state.request_with(request("failed", &budget), |_, _, _| {
        Err(anyhow!("controlled decoder failure"))
    })?;
    drain(&mut state);
    let error = state.request(request("failed", &budget)).unwrap_err();
    assert!(error.to_string().contains("controlled decoder failure"));
    assert!(state.request(request("failed", &budget))?.is_none());
    assert_eq!(state.stats().started, 1);
    state.cancel();
    state.request(request("failed", &budget))?;
    drain(&mut state);
    assert!(state.request(request("failed", &budget))?.is_some());
    assert_eq!(state.stats().started, 2);
    Ok(())
}

#[test]
fn bounded_quiesce_retains_a_blocked_slot_until_a_later_drain() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("blocked", &budget));
    assert!(state.quiesce(Duration::ZERO).is_err());
    assert!(state.busy());
    assert_eq!(budget.usage().live_metadata, 64);
    release.send(())?;
    exited.recv()?;
    drain(&mut state);
    state.quiesce(Duration::ZERO)?;
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(state.stats().cancelled, 1);
    Ok(())
}

#[test]
fn a_panicked_worker_is_joined_and_latched_without_restarting() -> Result<()> {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    state.request_with(request("panic", &budget), |_, _, _| {
        panic!("controlled worker panic")
    })?;
    drain(&mut state);
    assert!(
        state
            .request(request("panic", &budget))
            .unwrap_err()
            .to_string()
            .contains("panicked")
    );
    assert!(state.request(request("panic", &budget))?.is_none());
    assert_eq!(state.stats().started, 1);
    assert_eq!(budget.usage().live_metadata, 0);
    Ok(())
}

#[test]
fn destruction_does_not_wait_for_a_worker_blocked_inside_preparation() {
    let budget = LayoutBudget::default();
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, request("detach", &budget));
    // The test releases the worker only after Drop returns. An implicit join
    // would deadlock this controlled sequence rather than merely run slowly.
    drop(state);
    assert_eq!(budget.usage().live_metadata, 0);
    release.send(()).unwrap();
    exited.recv().unwrap();
}

#[test]
fn notification_runs_after_enqueue_without_releasing_an_unfinished_worker_slot() -> Result<()> {
    let budget = LayoutBudget::default();
    let registry = LayoutModelRegistry::default();
    let (notify_sender, notified) = mpsc::sync_channel(0);
    let (release_notification, notification_released) = mpsc::sync_channel(0);
    let mut scene = request("notifying", &budget);
    scene.registration = registry.register(64)?;
    scene.notify = Some(Box::new(move || {
        notify_sender.send(()).unwrap();
        notification_released.recv().unwrap();
    }));
    let mut state = ImagePreparationState::default();
    state.request_with(scene, |_, _, _| Ok(Vec::new()))?;
    notified.recv()?;
    assert!(state.poll()?);
    let active = state.active.as_ref().unwrap();
    assert!(active.reply.is_some());
    assert!(!active.thread.is_finished());
    assert!(state.busy());
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    assert!(
        state
            .request_with(request("replacement", &budget), |_, _, _| {
                panic!("A notifying worker still owns the only slot")
            })?
            .is_none()
    );
    assert_eq!(state.stats().started, 1);
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    release_notification.send(())?;
    drain(&mut state);
    assert_eq!(state.stats().cancelled, 1);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(registry.live_metadata(), 0);
    Ok(())
}

#[test]
fn a_cancelled_preparation_still_notifies_exactly_once() -> Result<()> {
    let budget = LayoutBudget::default();
    let notifications = Arc::new(AtomicUsize::new(0));
    let worker_notifications = notifications.clone();
    let (notify_sender, notified) = mpsc::sync_channel(1);
    let mut scene = request("cancel-notify", &budget);
    scene.notify = Some(Box::new(move || {
        worker_notifications.fetch_add(1, Ordering::SeqCst);
        notify_sender.send(()).unwrap();
    }));
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, scene);
    state.cancel_pending();
    release.send(())?;
    exited.recv()?;
    drain(&mut state);
    notified.recv()?;
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    assert!(matches!(
        notified.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
    assert_eq!(state.stats().cancelled, 1);
    assert!(!state.ready());
    assert!(!state.poll()?);
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn a_closed_result_receiver_still_notifies_and_does_not_panic_the_worker() -> Result<()> {
    let budget = LayoutBudget::default();
    let registry = LayoutModelRegistry::default();
    let notifications = Arc::new(AtomicUsize::new(0));
    let worker_notifications = notifications.clone();
    let (notify_sender, notified) = mpsc::sync_channel(1);
    let mut scene = request("closed-receiver", &budget);
    scene.registration = registry.register(64)?;
    scene.notify = Some(Box::new(move || {
        worker_notifications.fetch_add(1, Ordering::SeqCst);
        notify_sender.send(()).unwrap();
    }));
    let mut state = ImagePreparationState::default();
    let (release, exited) = blocked(&mut state, scene);
    let ActiveScene {
        replies,
        thread,
        storage,
        registration,
        ..
    } = state.active.take().unwrap();
    drop(replies);
    assert_eq!(budget.usage().live_metadata, 64);
    assert_eq!(registry.live_metadata(), 64);
    release.send(())?;
    exited.recv()?;
    // The capacity-one notification cannot block this join. Joining first also
    // makes an accidental unwrap of the failed result send fail deterministically.
    assert!(thread.join().is_ok());
    notified.recv()?;
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    assert!(matches!(
        notified.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
    drop(storage);
    drop(registration);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(registry.live_metadata(), 0);
    Ok(())
}
