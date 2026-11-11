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

fn set_book_metadata(app: &mut App, url: &str) -> Result<()> {
    let mut book = app.store.book(BOOK)?.unwrap();
    book.title = "Prepared sleep title".into();
    book.author = "Prepared sleep author".into();
    book.cover_url = url.into();
    app.store.upsert_book(&book)?;
    app.render_reader()
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
                app.hide_sleep_screen();
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
        app.hide_sleep_screen();
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
        let key = app.sleep_screen.source_key.clone().unwrap();
        assert!(app.cache.get(&key)?.is_some());
        app.show_sleep_screen()?;
        let state = fixture.ui().global::<SleepScreenModel>();
        assert!(state.get_has_cover());
        let pixels = state.get_cover().size();
        app.hide_sleep_screen();

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
        app.hide_sleep_screen();
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
        app.hide_sleep_screen();
    }
    fixture.finish()
}

fn sleep_cover_setting_is_enabled_saved_and_refreshes_preparation() -> Result<()> {
    let fixture = Fixture::new("Sleep preference body.", vec![], false)?;
    {
        let mut app = fixture.app();
        set_book_metadata(&mut app, "")?;
        app.settings_subpage = "device".into();
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
        app.hide_sleep_screen();
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
