//! Actual published image scenes own processing until a complete replacement.

use super::super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::{
    cache::InventoryReadStats,
    platform::{ImageRegion, PlatformError, RenderSink},
    reader::layout_budget::LayoutLimits,
    store::{BookMetadataLimits, with_book_metadata_test_limits},
};
use slint::Model;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

struct ImageSink {
    available: AtomicBool,
    requests: Mutex<Vec<Vec<ImageRegion>>>,
}

impl ImageSink {
    fn new() -> Self {
        Self {
            available: AtomicBool::new(true),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<Vec<ImageRegion>> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

impl RenderSink for ImageSink {
    fn set_dark_mode(&self, _: bool) -> std::result::Result<(), PlatformError> {
        Ok(())
    }
    fn hardware_y4_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }
    fn set_hardware_y4_for_frame(
        &self,
        regions: &[ImageRegion],
    ) -> std::result::Result<(), PlatformError> {
        self.requests.lock().unwrap().push(regions.to_vec());
        Ok(())
    }
}

fn image(value: u8) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(2, 2);
    buffer.make_mut_slice().fill(slint::Rgb8Pixel {
        r: value,
        g: value,
        b: value,
    });
    slint::Image::from_rgb8(buffer)
}

fn rows(value: u8) -> ModelRc<ReaderImage> {
    model(vec![ReaderImage {
        id: "published-picture".into(),
        loaded: true,
        picture: image(value),
        width: 2.0,
        height: 26.0,
        y: 0.0,
        caption: "Retained caption".into(),
    }])
}

fn region() -> ImageRegion {
    ImageRegion {
        x: 32,
        y: 164,
        width: 2,
        height: 2,
    }
}

fn reader_pixel(ui: &AppWindow) -> u8 {
    ui.global::<ReaderModel>()
        .get_images()
        .row_data(0)
        .unwrap()
        .picture
        .to_rgb8()
        .expect("The fixture owns decoded RGB pixels")
        .as_bytes()[0]
}

fn article_pixel(ui: &AppWindow) -> u8 {
    ui.global::<ArticleModel>()
        .get_blocks()
        .row_data(0)
        .unwrap()
        .picture
        .to_rgb8()
        .expect("The fixture owns decoded RGB pixels")
        .as_bytes()[0]
}

fn active_reader(fixture: &Fixture) -> Result<Arc<ImageSink>> {
    let sink = Arc::new(ImageSink::new());
    fixture.app().platform.set_render_sink(sink.clone())?;
    let shell = fixture.ui().global::<ShellModel>();
    shell.set_route("reader".into());
    shell.set_toast("".into());
    shell.set_dialog_kind("".into());
    let reader = fixture.ui().global::<ReaderModel>();
    reader.set_menu(false);
    reader.set_toc_open(false);
    reader.set_selection_open(false);
    reader.set_thought_open(false);
    reader.set_writing_thought(false);
    reader.set_sync_conflict(false);
    reader.set_access_notice(false);
    fixture.ui().global::<SleepScreenModel>().set_active(false);
    sink.take();
    Ok(sink)
}

fn prepared(
    app: &App,
    source: &str,
    effective: ImageDithering,
    raw: u8,
) -> Result<PreparedReaderImages> {
    let primary = rows(raw);
    let software = rows(if effective == ImageDithering::Hardware {
        85
    } else {
        raw
    });
    app.prepared_reader_scene(
        primary,
        software,
        ImageDithering::Hardware,
        effective,
        if effective == ImageDithering::Hardware {
            vec![region()]
        } else {
            Vec::new()
        },
        source.to_owned(),
        &LayoutBudget::default(),
    )
}

fn pending_targets_do_not_change_the_displayed_scene() -> Result<()> {
    let fixture = Fixture::new("Published image body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        app.prefs.image_dithering = ImageDithering::Hardware;
        let first = prepared(&app, "source-A:hardware", ImageDithering::Hardware, 81)?;
        app.publish_reader_images(first)?;
        assert_eq!(sink.take(), vec![vec![region()]]);
        assert_eq!(reader_pixel(fixture.ui()), 81);
        app.prefs.image_dithering = ImageDithering::Off;
        let replacement = prepared(&app, "source-B:software", ImageDithering::Software, 68)?;
        app.sync_presented_image_processing()?;
        assert!(sink.take().is_empty());
        assert_eq!(reader_pixel(fixture.ui()), 81);
        let retained = app.image_processing.scene.as_ref().unwrap();
        assert_eq!(retained._identity, "source-A:hardware");
        assert_eq!(retained.requested, ImageDithering::Hardware);
        assert_eq!(retained.effective, ImageDithering::Hardware);
        app.publish_reader_images(replacement)?;
        assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
        assert_eq!(reader_pixel(fixture.ui()), 68);
        assert_eq!(
            app.image_processing.scene.as_ref().unwrap()._identity,
            "source-B:software"
        );
    }
    fixture.finish()
}

fn rejected_preparation_cannot_retire_the_displayed_model() -> Result<()> {
    let fixture = Fixture::new("Image admission body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        let current = prepared(&app, "retained-source", ImageDithering::Hardware, 81)?;
        app.publish_reader_images(current)?;
        sink.take();
        let old_storage = app.model_layout.live_metadata();
        let exhausted = LayoutBudget::with_limits(LayoutLimits {
            metadata_bytes: 0,
            allocation_work: 0,
        });
        let rejected = app.prepared_reader_scene(
            rows(68),
            rows(68),
            ImageDithering::Software,
            ImageDithering::Software,
            Vec::new(),
            "rejected-source".into(),
            &exhausted,
        );
        assert!(rejected.is_err());
        app.sync_presented_image_processing()?;
        assert!(sink.take().is_empty());
        assert_eq!(reader_pixel(fixture.ui()), 81);
        assert_eq!(app.model_layout.live_metadata(), old_storage);
        assert_eq!(
            app.image_processing.scene.as_ref().unwrap()._identity,
            "retained-source"
        );
    }
    fixture.finish()
}

fn overlay(ui: &AppWindow, name: &str, enabled: bool) {
    let reader = ui.global::<ReaderModel>();
    match name {
        "menu" => reader.set_menu(enabled),
        "contents" => reader.set_toc_open(enabled),
        "selection" => reader.set_selection_open(enabled),
        "thought" => reader.set_thought_open(enabled),
        "writing" => reader.set_writing_thought(enabled),
        "conflict" => reader.set_sync_conflict(enabled),
        "access" => reader.set_access_notice(enabled),
        "toast" => ui
            .global::<ShellModel>()
            .set_toast(if enabled { "Fixture notice" } else { "" }.into()),
        "dialog" => ui
            .global::<ShellModel>()
            .set_dialog_kind(if enabled { "more" } else { "" }.into()),
        _ => unreachable!("The overlay fixture names an explicit UI state"),
    }
}

fn overlays_select_prepared_software_without_observing_pending_preferences() -> Result<()> {
    let fixture = Fixture::new("Image overlay body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        let current = prepared(
            &app,
            "overlay-retained-source",
            ImageDithering::Hardware,
            81,
        )?;
        app.publish_reader_images(current)?;
        sink.take();
        app.prefs.image_dithering = ImageDithering::Off;
        for name in [
            "menu",
            "contents",
            "selection",
            "thought",
            "writing",
            "conflict",
            "access",
            "toast",
            "dialog",
        ] {
            overlay(fixture.ui(), name, true);
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()], "{name}");
            assert_eq!(reader_pixel(fixture.ui()), 85, "{name}");
            assert!(!app.image_processing.scene.as_ref().unwrap().using_primary);
            app.sync_presented_image_processing()?;
            assert!(
                sink.take().is_empty(),
                "Idle {name} repeated a display request"
            );
            overlay(fixture.ui(), name, false);
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![vec![region()]], "{name}");
            assert_eq!(reader_pixel(fixture.ui()), 81, "{name}");
        }
        assert_eq!(app.prefs.image_dithering, ImageDithering::Off);
        assert_eq!(
            app.image_processing.scene.as_ref().unwrap()._identity,
            "overlay-retained-source"
        );
    }
    fixture.finish()
}

fn sleep_and_wake_use_only_prepared_models_and_retained_processing() -> Result<()> {
    let fixture = Fixture::new("Sleep image ownership body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        let current = prepared(&app, "sleep-retained-source", ImageDithering::Hardware, 81)?;
        app.publish_reader_images(current)?;
        sink.take();
        app.reader.draft = PinyinInput::new("Retained image draft");
        let route = app.route.clone();
        let chapter = app.reader.chapter_index;
        let anchor = app.reader.paginator.as_ref().unwrap().current_page().start;
        app.prefs.image_dithering = ImageDithering::Off;
        app.cache.clear_unpinned()?;
        app.cache.reset_inventory_read_stats();
        app.store.reset_book_read_stats();
        with_book_metadata_test_limits(
            BookMetadataLimits {
                bytes: 1,
                ..Default::default()
            },
            || -> Result<()> {
                app.suspend_image_processing_for_sleep()?;
                assert_eq!(reader_pixel(fixture.ui()), 85);
                assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
                fixture.ui().global::<SleepScreenModel>().set_active(true);
                app.restore_image_processing_after_sleep()?;
                assert_eq!(reader_pixel(fixture.ui()), 85);
                assert!(sink.take().is_empty());
                fixture.ui().global::<SleepScreenModel>().set_active(false);
                app.sync_presented_image_processing()?;
                assert_eq!(reader_pixel(fixture.ui()), 81);
                assert_eq!(sink.take(), vec![vec![region()]]);
                app.sync_presented_image_processing()?;
                assert!(sink.take().is_empty());
                assert_eq!(app.store.book_read_stats().metadata_rows, 0);
                assert_eq!(
                    app.cache.inventory_read_stats(),
                    InventoryReadStats::default()
                );
                Ok(())
            },
        )?;
        assert_eq!(app.route, route);
        assert_eq!(app.reader.book_id, BOOK);
        assert_eq!(app.reader.chapter_index, chapter);
        assert_eq!(
            app.reader.paginator.as_ref().unwrap().current_page().start,
            anchor
        );
        assert_eq!(app.reader.draft.query, "Retained image draft");
    }
    fixture.finish()
}

fn inactive_routes_and_changed_capability_cannot_reuse_raw_ownership() -> Result<()> {
    let fixture = Fixture::new("Image capability body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        app.prefs.image_dithering = ImageDithering::Hardware;
        sink.available.store(false, Ordering::Release);
        let (requested, effective) = app.image_processing_choices();
        assert_eq!(
            (requested, effective),
            (ImageDithering::Hardware, ImageDithering::Hardware)
        );
        let current = app.prepared_reader_scene(
            rows(81),
            rows(85),
            requested,
            effective,
            vec![region()],
            "capability-retained-source".into(),
            &LayoutBudget::default(),
        )?;
        app.publish_reader_images(current)?;
        assert_eq!(reader_pixel(fixture.ui()), 85);
        assert!(sink.take().iter().all(Vec::is_empty));
        app.cache.clear_unpinned()?;
        app.cache.reset_inventory_read_stats();
        app.store.reset_book_read_stats();
        with_book_metadata_test_limits(
            BookMetadataLimits {
                bytes: 1,
                ..Default::default()
            },
            || -> Result<()> {
                sink.available.store(true, Ordering::Release);
                app.sync_presented_image_processing()?;
                assert_eq!(reader_pixel(fixture.ui()), 81);
                assert_eq!(sink.take(), vec![vec![region()]]);
                sink.available.store(false, Ordering::Release);
                app.sync_presented_image_processing()?;
                assert_eq!(reader_pixel(fixture.ui()), 85);
                assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
                assert_eq!(
                    app.image_processing.scene.as_ref().unwrap()._identity,
                    "capability-retained-source"
                );
                assert_eq!(
                    app.image_processing.scene.as_ref().unwrap().effective,
                    ImageDithering::Hardware
                );
                assert_eq!(app.store.book_read_stats().metadata_rows, 0);
                assert_eq!(
                    app.cache.inventory_read_stats(),
                    InventoryReadStats::default()
                );
                Ok(())
            },
        )?;
        sink.available.store(true, Ordering::Release);
        app.sync_presented_image_processing()?;
        assert_eq!(reader_pixel(fixture.ui()), 81);
        assert_eq!(sink.take(), vec![vec![region()]]);
        fixture
            .ui()
            .global::<ShellModel>()
            .set_route("library".into());
        app.sync_presented_image_processing()?;
        assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
        assert_eq!(reader_pixel(fixture.ui()), 85);
        app.sync_presented_image_processing()?;
        assert!(sink.take().is_empty());
    }
    fixture.finish()
}

fn altered_ui_geometry_cannot_reuse_a_retained_raw_rectangle() -> Result<()> {
    let fixture = Fixture::new("Geometry ownership body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        let current = prepared(
            &app,
            "geometry-retained-source",
            ImageDithering::Hardware,
            81,
        )?;
        app.publish_reader_images(current)?;
        sink.take();
        let reader = fixture.ui().global::<ReaderModel>();
        let margin = reader.get_margin();
        let heading = reader.get_show_heading();
        let notes = reader.get_notes_panel();
        for field in ["margin", "heading", "notes"] {
            match field {
                "margin" => reader.set_margin(margin + 1),
                "heading" => reader.set_show_heading(!heading),
                _ => reader.set_notes_panel(!notes),
            }
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()], "{field}");
            assert_eq!(reader_pixel(fixture.ui()), 85, "{field}");
            reader.set_margin(margin);
            reader.set_show_heading(heading);
            reader.set_notes_panel(notes);
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![vec![region()]], "{field}");
            assert_eq!(reader_pixel(fixture.ui()), 81, "{field}");
        }
    }
    fixture.finish()
}

fn an_article_scene_replaces_reader_models_and_respects_article_overlays() -> Result<()> {
    let fixture = Fixture::new("Article image ownership body.", vec![], false)?;
    let sink = active_reader(&fixture)?;
    {
        let mut app = fixture.app();
        let current = prepared(&app, "reader-before-article", ImageDithering::Hardware, 81)?;
        app.publish_reader_images(current)?;
        sink.take();
        let blocks = |value| {
            model(vec![ArticleBlock {
                id: "article-picture".into(),
                kind: "image".into(),
                picture: image(value),
                loaded: true,
                height: 26,
                frame_height: 2.0,
                ..Default::default()
            }])
        };
        let primary = blocks(73);
        let software = blocks(68);
        let scene = app.image_scene(
            SceneModels::Article {
                primary: primary.clone(),
                software,
            },
            ImageDithering::Hardware,
            ImageDithering::Hardware,
            vec![region()],
            "article-source".into(),
            &LayoutBudget::default(),
        )?;
        fixture
            .ui()
            .global::<ShellModel>()
            .set_route("article".into());
        app.publish_article_images(primary, Some(scene))?;
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_images()
                .row_count(),
            0
        );
        assert_eq!(article_pixel(fixture.ui()), 73);
        assert_eq!(
            app.image_processing.scene.as_ref().unwrap()._identity,
            "article-source"
        );
        sink.take();
        let article = fixture.ui().global::<ArticleModel>();
        for name in ["menu", "list", "qr"] {
            match name {
                "menu" => article.set_menu(true),
                "list" => article.set_list_open(true),
                _ => article.set_qr_open(true),
            }
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
            assert_eq!(article_pixel(fixture.ui()), 68);
            article.set_menu(false);
            article.set_list_open(false);
            article.set_qr_open(false);
            app.sync_presented_image_processing()?;
            assert_eq!(sink.take(), vec![vec![region()]]);
            assert_eq!(article_pixel(fixture.ui()), 73);
        }
        app.clear_presented_image_processing()?;
        assert_eq!(sink.take(), vec![Vec::<ImageRegion>::new()]);
        assert_eq!(article.get_blocks().row_count(), 0);
    }
    fixture.finish()
}

#[test]
fn image_processing_changes_only_with_published_models() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_processing_changes_only_with_published_models",
        pending_targets_do_not_change_the_displayed_scene,
    )
}

#[test]
fn image_processing_admission_failure_retains_the_current_scene() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_processing_admission_failure_retains_the_current_scene",
        rejected_preparation_cannot_retire_the_displayed_model,
    )
}

#[test]
fn image_overlays_switch_prepared_models_and_restore_the_retained_scene() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_overlays_switch_prepared_models_and_restore_the_retained_scene",
        overlays_select_prepared_software_without_observing_pending_preferences,
    )
}

#[test]
fn image_sleep_restoration_performs_no_metadata_or_cache_reads() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_sleep_restoration_performs_no_metadata_or_cache_reads",
        sleep_and_wake_use_only_prepared_models_and_retained_processing,
    )
}

#[test]
fn image_raw_ownership_follows_route_and_actual_capability() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_raw_ownership_follows_route_and_actual_capability",
        inactive_routes_and_changed_capability_cannot_reuse_raw_ownership,
    )
}

#[test]
fn image_raw_regions_require_the_retained_display_geometry() -> Result<()> {
    isolated(
        "app::image_processing::tests::image_raw_regions_require_the_retained_display_geometry",
        altered_ui_geometry_cannot_reuse_a_retained_raw_rectangle,
    )
}

#[test]
fn article_image_models_keep_independent_processing_ownership() -> Result<()> {
    isolated(
        "app::image_processing::tests::article_image_models_keep_independent_processing_ownership",
        an_article_scene_replaces_reader_models_and_respects_article_overlays,
    )
}

#[test]
fn article_image_cache_identity_binds_every_source_field() {
    let key = article_cache_key("article", "picture", "https://example.invalid/a.png");
    assert!(key.starts_with("article-image:gray8-v2:"));
    assert_ne!(
        key,
        article_cache_key("article", "picture", "https://example.invalid/b.png")
    );
    assert_ne!(
        key,
        article_cache_key(
            "another-article",
            "picture",
            "https://example.invalid/a.png"
        )
    );
    assert_ne!(
        article_cache_key("a", "bc", "d"),
        article_cache_key("ab", "c", "d")
    );
}

#[test]
fn raw_regions_require_unclipped_integral_physical_edges() {
    let clip = (0.0, 0.0, 10.0, 10.0);
    assert_eq!(
        clean_region((0.5, 1.5, 2.5, 3.5), clip, 2.0),
        Some(ImageRegion {
            x: 1,
            y: 3,
            width: 5,
            height: 7
        })
    );
    assert_eq!(
        clean_region((0.5, 0.5, 0.5, 0.5), clip, 2.0),
        Some(ImageRegion {
            x: 1,
            y: 1,
            width: 1,
            height: 1
        })
    );
    for candidate in [
        (0.25, 0.5, 1.0, 1.0),
        (0.5, 0.5, 0.25, 1.0),
        (-0.5, 0.5, 1.0, 1.0),
        (9.5, 0.5, 1.0, 1.0),
        (0.5, 0.5, 0.0, 1.0),
        (f64::NAN, 0.5, 1.0, 1.0),
    ] {
        assert!(clean_region(candidate, clip, 2.0).is_none());
    }
    assert!(clean_region((0.5, 0.5, 1.0, 1.0), clip, 0.0).is_none());
}
