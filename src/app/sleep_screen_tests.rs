//! Sleep presentations retain current-book pixels without power-path I/O.

use super::super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::{
    jobs::JobResult,
    store::{BookMetadataLimits, with_book_metadata_test_limits},
};
use slint::Model;

fn image_bytes(width: u32, height: u32) -> Result<Vec<u8>> {
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(width, height).write_to(&mut image, image::ImageFormat::Png)?;
    Ok(image.into_inner())
}

fn set_book_metadata_unprepared(app: &mut App, url: &str) -> Result<()> {
    let mut book = app.store.book(BOOK)?.unwrap();
    book.title = "Prepared sleep title".into();
    book.author = "Prepared sleep author".into();
    book.cover_url = url.into();
    app.store.upsert_book(&book)?;
    app.render_reader()
}

fn set_book_metadata(app: &mut App, url: &str) -> Result<()> {
    set_book_metadata_unprepared(app, url)?;
    app.drain_cover_preparation_for_test()
}

fn prepared_pixels_survive_cache_removal_without_reading_metadata() -> Result<()> {
    let fixture = Fixture::new("Reader body remains available.", vec![], false)?;
    {
        let mut app = fixture.app();
        let key = thumbnail_key(BOOK, "https://example.invalid/sleep-cover.png").unwrap();
        app.cache.put_cover(&key, &image_bytes(3, 5)?, 400, 560)?;
        set_book_metadata(&mut app, "https://example.invalid/sleep-cover.png")?;
        assert!(app.sleep_screen.cover.is_some());
        let pixels = app.sleep_screen.cover.as_ref().unwrap().size();
        assert!(pixels.width > 400);
        assert!(app.cache.remove(&key)?);
        app.reader.draft = PinyinInput::new("Retained draft");
        let route = app.route.clone();
        let anchor = app.reader.paginator.as_ref().unwrap().current_page().start;
        with_book_metadata_test_limits(
            BookMetadataLimits {
                bytes: 1,
                ..Default::default()
            },
            || -> Result<()> {
                app.store.reset_book_read_stats();
                app.show_sleep_screen()?;
                let state = fixture.ui().global::<SleepScreenModel>();
                assert!(state.get_active() && state.get_has_cover());
                assert_eq!(state.get_title(), "Prepared sleep title");
                assert_eq!(state.get_author(), "Prepared sleep author");
                assert_eq!(state.get_cover().size(), pixels);
                assert_eq!(app.store.book_read_stats().metadata_rows, 0);
                app.hide_sleep_screen()?;
                assert!(!state.get_active());
                let empty = state.get_cover().size();
                assert_eq!((empty.width, empty.height), (0, 0));
                Ok(())
            },
        )?;
        assert_eq!(app.route, route);
        assert_eq!(app.reader.draft.query, "Retained draft");
        assert_eq!(
            app.reader.paginator.as_ref().unwrap().current_page().start,
            anchor
        );
    }
    fixture.finish()
}

fn fallback_requires_the_current_reader_and_drops_account_data() -> Result<()> {
    let fixture = Fixture::new("Current Reader body.", vec![], false)?;
    {
        let mut app = fixture.app();
        set_book_metadata(&mut app, "")?;
        let route = app.route.clone();
        app.show_sleep_screen()?;
        let state = fixture.ui().global::<SleepScreenModel>();
        assert!(!state.get_has_cover());
        assert_eq!(state.get_title(), "Prepared sleep title");
        assert_eq!(state.get_author(), "Prepared sleep author");

        for invalid in [
            Route::Library,
            Route::Article("article-fixture".into()),
            Route::Reader("another-book".into(), app.reader.chapter_index),
            Route::Reader(BOOK.into(), app.reader.chapter_index + 1),
        ] {
            app.route = invalid;
            app.show_sleep_screen()?;
            assert!(!state.get_has_cover());
            assert_eq!(state.get_title(), "WeRead for Kindle");
            assert!(state.get_author().is_empty());
        }
        app.route = route;
        app.invalidate_account_work()?;
        app.show_sleep_screen()?;
        assert!(!state.get_has_cover());
        assert_eq!(state.get_title(), "WeRead for Kindle");
        assert!(state.get_author().is_empty());
        app.hide_sleep_screen()?;
    }
    fixture.finish()
}

fn cover_results_bind_to_the_current_book_and_account_epoch() -> Result<()> {
    let fixture = Fixture::new("Current cover result body.", vec![], false)?;
    {
        let mut app = fixture.app();
        set_book_metadata(&mut app, "https://example.invalid/current-cover.png")?;
        let epoch = app.jobs.account_epoch();
        app.register_cover_request(1, BOOK, "https://example.invalid/current-cover.png");
        assert!(!app.handle_job(JobEvent::new(
            1,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(3, 5)?)
            },
        ))?);
        app.drain_cover_preparation_for_test()?;
        let key = app.sleep_screen.source_key.clone().unwrap();
        assert!(app.cache.get(&key)?.is_some());
        app.show_sleep_screen()?;
        let state = fixture.ui().global::<SleepScreenModel>();
        assert!(state.get_has_cover());
        let pixels = state.get_cover().size();
        app.hide_sleep_screen()?;

        assert!(!app.handle_job(JobEvent::new(
            2,
            epoch,
            JobResult::Cover {
                book_id: "old-book".into(),
                result: Ok(image_bytes(5, 3)?)
            },
        ))?);
        app.register_cover_request(3, BOOK, "https://example.invalid/current-cover.png");
        app.jobs.cancel_all();
        app.clear_pending_cover_requests();
        assert!(!app.handle_job(JobEvent::new(
            3,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(5, 3)?)
            },
        ))?);
        app.show_sleep_screen()?;
        assert!(state.get_has_cover());
        assert_eq!(state.get_cover().size(), pixels);
        assert_eq!(state.get_title(), "Prepared sleep title");
        app.hide_sleep_screen()?;
    }
    fixture.finish()
}

fn reader_admits_a_sharper_cover_even_with_a_cached_thumbnail() -> Result<()> {
    let fixture = Fixture::new("Cover admission body.", vec![], false)?;
    {
        let mut app = fixture.app();
        let thumbnail = thumbnail_key(BOOK, "https://example.invalid/reader-cover.png").unwrap();
        app.cache
            .put_cover(&thumbnail, &image_bytes(3, 5)?, 400, 560)?;
        set_book_metadata(&mut app, "https://example.invalid/reader-cover.png")?;
        app.demo = false;
        app.force_offline = false;
        app.online = true;
        app.schedule_visible_covers();
        assert!(app.pending_covers.contains(BOOK));
        let request = app.next_id;
        let epoch = app.jobs.account_epoch();
        assert!(!app.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(3, 5)?)
            },
        ))?);
        app.drain_cover_preparation_for_test()?;
        assert!(!app.pending_covers.contains(BOOK));
        app.schedule_visible_covers();
        assert!(!app.pending_covers.contains(BOOK));
        app.prefs.sleep_cover = false;
        assert!(
            app.cache
                .remove(app.sleep_screen.source_key.as_deref().unwrap())?
        );
        app.schedule_visible_covers();
        assert!(!app.pending_covers.contains(BOOK));
        app.force_offline = true;
        app.jobs.cancel_all();
    }
    fixture.finish()
}

fn late_source_a_cannot_publish_or_retire_source_b() -> Result<()> {
    let fixture = Fixture::new("Source-bound Reader body.", vec![], false)?;
    {
        let mut app = fixture.app();
        let first = "https://example.invalid/first-cover.png";
        let second = "https://example.invalid/second-cover.png";
        let first_thumbnail = thumbnail_key(BOOK, first).unwrap();
        app.cache
            .put_cover(&first_thumbnail, &image_bytes(3, 5)?, 400, 560)?;
        set_book_metadata(&mut app, first)?;
        assert!(app.sleep_screen.cover.is_some());
        app.register_cover_request(10, BOOK, first);
        set_book_metadata(&mut app, second)?;
        assert!(app.sleep_screen.cover.is_none());
        let epoch = app.jobs.account_epoch();
        assert!(!app.handle_job(JobEvent::new(
            10,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(3, 5)?)
            },
        ))?);
        assert!(!app.pending_covers.contains(BOOK));
        assert!(
            app.cache
                .get(&thumbnail_key(BOOK, second).unwrap())?
                .is_none()
        );
        assert!(app.cache.get(&cover_key(BOOK, second).unwrap())?.is_none());
        app.register_cover_request(11, BOOK, second);
        assert!(!app.handle_job(JobEvent::new(
            10,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Err(source::SourceError::Network)
            },
        ))?);
        assert!(app.pending_covers.contains(BOOK));
        assert!(
            app.store
                .setting::<i64>(&format!("cover.failed:{BOOK}"))?
                .is_none()
        );
        assert!(!app.handle_job(JobEvent::new(
            11,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(5, 3)?)
            },
        ))?);
        app.drain_cover_preparation_for_test()?;
        assert!(!app.pending_covers.contains(BOOK));
        assert!(
            app.cache
                .get(&thumbnail_key(BOOK, second).unwrap())?
                .is_some()
        );
        assert!(app.cache.get(&cover_key(BOOK, second).unwrap())?.is_some());
        assert_eq!(app.sleep_screen.source_key, cover_key(BOOK, second));
        app.show_sleep_screen()?;
        assert!(fixture.ui().global::<SleepScreenModel>().get_has_cover());
        app.hide_sleep_screen()?;
    }
    fixture.finish()
}

fn sleep_cover_setting_is_enabled_saved_and_refreshes_preparation() -> Result<()> {
    let fixture = Fixture::new("Sleep preference body.", vec![], false)?;
    {
        let mut app = fixture.app();
        set_book_metadata(&mut app, "")?;
        app.settings_subpage = "display".into();
        app.render_settings()?;
        let settings = fixture.ui().global::<AuxiliaryModel>();
        let row = settings
            .get_settings_rows()
            .iter()
            .find(|row| row.id == "sleep_cover")
            .unwrap();
        assert!(row.enabled && row.checked);
        app.auxiliary_action("settings.toggle", "sleep_cover")?;
        assert!(!app.sleep_cover_enabled());
        assert!(app.sleep_screen.book_id.is_none());
        assert!(
            !app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .sleep_cover
        );
        app.render_settings()?;
        let row = settings
            .get_settings_rows()
            .iter()
            .find(|row| row.id == "sleep_cover")
            .unwrap();
        assert!(row.enabled && !row.checked);
        app.auxiliary_action("settings.toggle", "sleep_cover")?;
        assert!(app.sleep_cover_enabled());
        assert_eq!(app.sleep_screen.book_id.as_deref(), Some(BOOK));
        assert!(
            app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .sleep_cover
        );
        app.show_sleep_screen()?;
        assert_eq!(
            fixture.ui().global::<SleepScreenModel>().get_title(),
            "Prepared sleep title"
        );
        app.hide_sleep_screen()?;
    }
    fixture.finish()
}

#[test]
fn sleep_cover_is_predecoded_before_the_power_event() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_cover_is_predecoded_before_the_power_event",
        prepared_pixels_survive_cache_removal_without_reading_metadata,
    )
}

#[test]
fn sleep_fallback_never_uses_residual_reader_or_account_state() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_fallback_never_uses_residual_reader_or_account_state",
        fallback_requires_the_current_reader_and_drops_account_data,
    )
}

#[test]
fn sleep_cover_arrivals_reject_another_book_and_retired_account_epoch() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_cover_arrivals_reject_another_book_and_retired_account_epoch",
        cover_results_bind_to_the_current_book_and_account_epoch,
    )
}

#[test]
fn reader_cover_preparation_has_its_own_bounded_cache_target() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::reader_cover_preparation_has_its_own_bounded_cache_target",
        reader_admits_a_sharper_cover_even_with_a_cached_thumbnail,
    )
}

#[test]
fn sleep_cover_source_changes_reject_late_old_requests_and_cached_pixels() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_cover_source_changes_reject_late_old_requests_and_cached_pixels",
        late_source_a_cannot_publish_or_retire_source_b,
    )
}

#[test]
fn sleep_cover_preference_controls_the_enabled_settings_row() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_cover_preference_controls_the_enabled_settings_row",
        sleep_cover_setting_is_enabled_saved_and_refreshes_preparation,
    )
}

#[test]
fn sleep_cover_sources_and_prepared_text_have_explicit_limits() {
    assert!(cover_key("book", "").is_none());
    assert!(cover_key("book", &"x".repeat(MAX_SLEEP_COVER_URL_BYTES + 1)).is_none());
    assert_ne!(cover_key("a", "bc"), cover_key("ab", "c"));
    assert_ne!(cover_key("book", "first"), cover_key("book", "second"));
    assert_eq!(
        bounded_text(&"\u{4e2d}".repeat(500), MAX_SLEEP_TITLE_SCALARS)
            .chars()
            .count(),
        MAX_SLEEP_TITLE_SCALARS
    );
}

fn settle_reader_images(app: &mut App) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.image_preparation_busy() {
        let _ = app.poll_image_preparation()?;
        anyhow::ensure!(
            Instant::now() < deadline,
            "Reader fixture preparation did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn wait_cover_worker(app: &App) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app
        .sleep_screen
        .preparation
        .active
        .as_ref()
        .is_some_and(|active| !active.handle.is_finished())
    {
        anyhow::ensure!(
            Instant::now() < deadline,
            "Cover fixture worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn idle_cover_preserves_reader_and_reference_pixels() -> Result<()> {
    let fixture = Fixture::new(
        "Reader lines remain unchanged while a cover prepares.",
        vec![],
        false,
    )?;
    {
        let mut app = fixture.app();
        let url = "https://example.invalid/idle-cover.png";
        let key = cover_key(BOOK, url).unwrap();
        let source = image::GrayImage::from_fn(11, 17, |x, y| {
            image::Luma([((x * 19 + y * 31) % 256) as u8])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(source).write_to(&mut encoded, image::ImageFormat::Png)?;
        let geometry = app.sleep_cover_geometry();
        let cached = app
            .cache
            .put_cover(&key, encoded.get_ref(), geometry.0, geometry.1)?;
        set_book_metadata_unprepared(&mut app, url)?;
        settle_reader_images(&mut app)?;
        let reader = fixture.ui().global::<ReaderModel>();
        let lines = reader.get_lines();
        let pictures = reader.get_images();
        assert!(app.sleep_screen.cover.is_none());
        app.poll_cover_preparation()?;
        assert_eq!(app.sleep_screen.preparation.started, 0);
        assert!(app.sleep_screen.preparation.active.is_none());
        app.note_cover_reading_activity();
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_none());
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.full_resolution);
        assert_eq!(reader.get_lines(), lines);
        assert_eq!(reader.get_images(), pictures);
        let actual = app.sleep_screen.cover.as_ref().unwrap().to_rgb8().unwrap();
        let reference = image::ImageReader::open(cached)?
            .decode()?
            .resize(
                geometry.0,
                geometry.1,
                image::imageops::FilterType::Triangle,
            )
            .into_luma8();
        assert_eq!((actual.width(), actual.height()), reference.dimensions());
        for (actual, expected) in actual.as_bytes().chunks_exact(3).zip(reference.as_raw()) {
            let expected = (((u16::from(*expected) + 8) / 17).min(15) * 17) as u8;
            assert_eq!(actual, [expected; 3]);
        }
    }
    fixture.finish()
}

fn reading_cancellation_retains_slot_preview_and_idle_deadline() -> Result<()> {
    let fixture = Fixture::new(
        "Reader cancellation preserves a prepared preview.",
        vec![],
        false,
    )?;
    {
        let mut app = fixture.app();
        let url = "https://example.invalid/cancel-cover.png";
        set_book_metadata_unprepared(&mut app, url)?;
        settle_reader_images(&mut app)?;
        app.sleep_screen.reload_needed = false;
        app.sleep_screen.cover = Some(slint::Image::from_rgb8(CoverPixels::new(3, 5)));
        let preview = app.sleep_screen.cover.as_ref().unwrap().size();
        let request = app.request();
        let work = CoverWork {
            identity: CoverIdentity {
                request,
                account_epoch: app.jobs.account_epoch(),
                book_id: BOOK.into(),
                url: url.into(),
                geometry: app.sleep_cover_geometry(),
                route: app.route.clone(),
            },
            input: CoverInput::Thumbnail(Arc::new(CoverSource {
                bytes: image_bytes(3, 5)?,
                _credits: None,
            })),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(5));
            let _ = sender.send(CoverCompletion {
                request,
                elapsed_us: 0,
                result: Err(crate::cache::PreparationCancelled.into()),
                _source_guard: None,
            });
        });
        app.sleep_screen.preparation.active = Some(ActiveCoverWork {
            work,
            cancelled,
            receiver,
            handle,
        });
        app.note_cover_reading_activity();
        let deadline = app.sleep_screen.preparation.reader_idle_until;
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_some());
        assert_eq!(app.sleep_screen.cover.as_ref().unwrap().size(), preview);
        release.send(())?;
        wait_cover_worker(&app)?;
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_none());
        assert_eq!(app.sleep_screen.preparation.queued.len(), 1);
        assert_eq!(app.sleep_screen.preparation.reader_idle_until, deadline);
        assert_eq!(app.sleep_screen.cover.as_ref().unwrap().size(), preview);
        assert!(
            app.store
                .setting::<i64>(&format!("cover.failed:{BOOK}"))?
                .is_none()
        );
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_none());
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.full_resolution);
    }
    fixture.finish()
}

fn download_credit_follows_the_original_into_its_sleep_derivative() -> Result<()> {
    let fixture = Fixture::new(
        "Retained source credit follows optional cover preparation.",
        vec![],
        false,
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (sender, events) = mpsc::channel();
    let jobs = crate::jobs::Jobs::new(runtime.handle().clone(), sender, None)?;
    {
        let mut app = fixture.app();
        let url = "file:///credit-cover.png";
        set_book_metadata_unprepared(&mut app, url)?;
        settle_reader_images(&mut app)?;
        let request = app.request();
        assert!(jobs.cover(request, BOOK.into(), url.into()).is_none());
        let mut event = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(event) = events.try_recv() {
                        return event;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
        })?;
        assert!(event.credits.is_some());
        assert_eq!(jobs.cover_capacity(), 1);
        event.account_epoch = app.jobs.account_epoch();
        event.result = JobResult::Cover {
            book_id: BOOK.into(),
            result: Ok(image_bytes(3, 5)?),
        };
        app.register_cover_request(request, BOOK, url);
        app.handle_job(event)?;
        assert_eq!(jobs.cover_capacity(), 1);
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_some());
        wait_cover_worker(&app)?;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now() + READER_COVER_IDLE);
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.cover.is_some());
        assert!(!app.sleep_screen.full_resolution);
        assert!(app.sleep_screen.preparation.queued.iter().any(|work|
            matches!(&work.input, CoverInput::SleepOriginal(source) if source._credits.is_some())));
        assert_eq!(jobs.cover_capacity(), 1);
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.full_resolution);
        assert_eq!(jobs.cover_capacity(), 2);
    }
    drop(jobs);
    runtime.shutdown_timeout(Duration::from_secs(1));
    fixture.finish()
}

fn cached_cover_survives_quiesce_and_local_eviction_uses_the_thumbnail() -> Result<()> {
    let fixture = Fixture::new(
        "The same Reader resumes cached cover intent.",
        vec![],
        false,
    )?;
    {
        let mut app = fixture.app();
        let url = "https://example.invalid/resume-cover.png";
        let full = cover_key(BOOK, url).unwrap();
        let thumbnail = thumbnail_key(BOOK, url).unwrap();
        let bytes = image_bytes(3, 5)?;
        let geometry = app.sleep_cover_geometry();
        app.cache.put_cover(&full, &bytes, geometry.0, geometry.1)?;
        app.cache.put_cover(&thumbnail, &bytes, 400, 560)?;
        set_book_metadata_unprepared(&mut app, url)?;
        settle_reader_images(&mut app)?;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.queue_cached_sleep_cover();
        assert!(!app.sleep_screen.reload_needed);
        assert_eq!(app.sleep_screen.preparation.queued.len(), 1);
        app.quiesce_cover_preparation(Duration::from_secs(1))?;
        assert!(app.sleep_screen.reload_needed);
        app.note_cover_reading_activity();
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_none());
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.full_resolution);

        app.sleep_screen.cover = None;
        app.sleep_screen.full_resolution = false;
        app.sleep_screen.reload_needed = true;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.poll_cover_preparation()?;
        assert!(app.sleep_screen.preparation.active.is_some());
        wait_cover_worker(&app)?;
        app.quiesce_cover_preparation(Duration::from_secs(1))?;
        assert!(app.sleep_screen.reload_needed);
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.full_resolution);

        app.sleep_screen.cover = None;
        app.sleep_screen.full_resolution = false;
        app.sleep_screen.reload_needed = true;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.queue_cached_sleep_cover();
        assert!(app.cache.remove(&full)?);
        app.drain_cover_preparation_for_test()?;
        assert!(app.sleep_screen.cover.is_some());
        assert!(!app.sleep_screen.full_resolution);
        assert!(
            app.store
                .setting::<i64>(&format!("cover.failed:{BOOK}"))?
                .is_none()
        );
        assert!(app.sleep_screen.skip_full_cache);
    }
    fixture.finish()
}

fn cover_input_is_moved_and_bounded_before_any_decode() -> Result<()> {
    let fixture = Fixture::new("Cover source bytes have bounded ownership.", vec![], false)?;
    {
        let mut app = fixture.app();
        let epoch = app.jobs.account_epoch();
        let first = vec![0_u8; MAX_COVER_INPUT_BYTES];
        let pointer = first.as_ptr();
        assert!(app.queue_downloaded_cover(800, epoch, BOOK, "", first, None)?);
        let CoverInput::Thumbnail(retained) = &app.sleep_screen.preparation.queued[0].input else {
            panic!("Wrong cover work");
        };
        assert_eq!(retained.bytes.as_ptr(), pointer);
        assert!(app.queue_downloaded_cover(
            801,
            epoch,
            BOOK,
            "",
            vec![0; MAX_COVER_INPUT_BYTES],
            None
        )?);
        assert_eq!(
            app.retained_cover_input_bytes(),
            MAX_RETAINED_COVER_INPUT_BYTES
        );
        assert!(!app.queue_downloaded_cover(802, epoch, BOOK, "", vec![0], None)?);
        assert!(
            app.store
                .setting::<i64>(&format!("cover.failed:{BOOK}"))?
                .is_none()
        );
        app.cancel_cover_preparation();
        assert_eq!(app.retained_cover_input_bytes(), 0);
        assert!(
            app.queue_downloaded_cover(
                803,
                epoch,
                BOOK,
                "",
                vec![0; MAX_COVER_INPUT_BYTES + 1],
                None
            )
            .is_err()
        );
    }
    fixture.finish()
}

#[test]
fn sleep_cover_waits_for_reader_idle_and_preserves_pixels_and_models() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_cover_waits_for_reader_idle_and_preserves_pixels_and_models",
        idle_cover_preserves_reader_and_reference_pixels,
    )
}

#[test]
fn cancelled_cover_holds_its_slot_and_resumes_only_after_idle() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::cancelled_cover_holds_its_slot_and_resumes_only_after_idle",
        reading_cancellation_retains_slot_preview_and_idle_deadline,
    )
}

#[test]
fn sleep_derivative_retains_real_public_image_admission() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::sleep_derivative_retains_real_public_image_admission",
        download_credit_follows_the_original_into_its_sleep_derivative,
    )
}

#[test]
fn quiesced_cached_cover_rearms_and_eviction_is_not_a_source_failure() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::quiesced_cached_cover_rearms_and_eviction_is_not_a_source_failure",
        cached_cover_survives_quiesce_and_local_eviction_uses_the_thumbnail,
    )
}

#[test]
fn cover_input_handoff_has_no_large_copy_and_enforces_its_byte_budget() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::cover_input_handoff_has_no_large_copy_and_enforces_its_byte_budget",
        cover_input_is_moved_and_bounded_before_any_decode,
    )
}

fn completed_cover_rejects_replaced_sources_and_accounts() -> Result<()> {
    let fixture = Fixture::new(
        "Completed work remains fenced until publication.",
        vec![],
        false,
    )?;
    {
        let mut app = fixture.app();
        let first = "https://example.invalid/worker-first.png";
        let second = "https://example.invalid/worker-second.png";
        set_book_metadata_unprepared(&mut app, first)?;
        settle_reader_images(&mut app)?;
        let request = app.request();
        let epoch = app.jobs.account_epoch();
        app.register_cover_request(request, BOOK, first);
        app.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(3, 5)?),
            },
        ))?;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.poll_cover_preparation()?;
        wait_cover_worker(&app)?;
        set_book_metadata_unprepared(&mut app, second)?;
        app.drain_cover_preparation_for_test()?;
        assert!(
            app.cache
                .get(&thumbnail_key(BOOK, first).unwrap())?
                .is_none()
        );
        assert!(app.sleep_screen.cover.is_none());
        let stale = app.sleep_screen.preparation.stale;
        assert!(stale > 0);

        let request = app.request();
        let epoch = app.jobs.account_epoch();
        app.register_cover_request(request, BOOK, second);
        app.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Cover {
                book_id: BOOK.into(),
                result: Ok(image_bytes(5, 3)?),
            },
        ))?;
        app.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
        app.poll_cover_preparation()?;
        wait_cover_worker(&app)?;
        app.jobs.cancel_all();
        app.clear_pending_cover_requests();
        assert!(app.sleep_screen.preparation.active.is_some());
        app.drain_cover_preparation_for_test()?;
        assert!(
            app.cache
                .get(&thumbnail_key(BOOK, second).unwrap())?
                .is_none()
        );
        assert!(app.sleep_screen.cover.is_none());
        assert!(app.sleep_screen.preparation.stale > stale);
    }
    fixture.finish()
}

#[test]
fn completed_cover_work_cannot_publish_into_replaced_sources_or_accounts() -> Result<()> {
    isolated(
        "app::sleep_screen::tests::completed_cover_work_cannot_publish_into_replaced_sources_or_accounts",
        completed_cover_rejects_replaced_sources_and_accounts,
    )
}

#[cfg(target_os = "linux")]
#[test]
fn cached_cover_worker_rejects_links_and_nonregular_objects() -> Result<()> {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.png");
    std::fs::write(&source, image_bytes(3, 5)?)?;
    let link = directory.path().join("linked.png");
    symlink(&source, &link)?;
    let cancelled = AtomicBool::new(false);
    assert!(decode_cached_cover(&link, (400, 560), &cancelled).is_err());
    let fifo = directory.path().join("input.pipe");
    let name = CString::new(fifo.as_os_str().as_bytes())?;
    // The test owns this temporary path and never opens a device node.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(decode_cached_cover(&fifo, (400, 560), &cancelled).is_err());
    Ok(())
}

#[test]
fn dropped_cover_owner_keeps_admission_with_the_detached_payload() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (sender, events) = mpsc::channel();
    let jobs = crate::jobs::Jobs::new(runtime.handle().clone(), sender, None)?;
    assert!(
        jobs.cover(900, "drop-cover".into(), "file:///drop-cover.png".into())
            .is_none()
    );
    let event = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(event) = events.try_recv() {
                    return event;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
    })?;
    let source = Arc::new(CoverSource {
        bytes: image_bytes(3, 5)?,
        _credits: event.credits,
    });
    let work = CoverWork {
        identity: CoverIdentity {
            request: 900,
            account_epoch: jobs.account_epoch(),
            book_id: "drop-cover".into(),
            url: "file:///drop-cover.png".into(),
            geometry: (400, 560),
            route: Route::Library,
        },
        input: CoverInput::Thumbnail(source),
    };
    let input = work.input.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::channel::<()>();
    let (finished, completion) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let source_guard = input.source_guard();
        let _ = wait.recv_timeout(Duration::from_secs(5));
        let result = prepare_cover_work(input, (400, 560), &worker_cancelled);
        let _ = sender.send(CoverCompletion {
            request: 900,
            elapsed_us: 0,
            result,
            _source_guard: source_guard,
        });
        let _ = finished.send(());
    });
    let mut state = CoverPreparationState::default();
    state.active = Some(ActiveCoverWork {
        work,
        cancelled,
        receiver,
        handle,
    });
    assert_eq!(jobs.cover_capacity(), 1);
    drop(state);
    assert_eq!(jobs.cover_capacity(), 1);
    release.send(())?;
    completion.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(jobs.cover_capacity(), 2);
    drop(jobs);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}
