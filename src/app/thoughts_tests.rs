//! Genuine App card/corpus operations in the existing bounded UI isolation.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use slint::Model;

fn underline(text: &str) -> RemoteUnderline {
    RemoteUnderline {
        text: text.into(),
        range: format!("range-{text}"),
        count: 1,
        thoughts: 1,
    }
}

fn preview_card_paging() -> Result<()> {
    let tokens = (0..100)
        .map(|index| format!("W{index:03}"))
        .collect::<Vec<_>>();
    let body = tokens.join(" ");
    let fixture = Fixture::new_for_profile(
        &body,
        tokens.iter().map(|text| underline(text)).collect(),
        true,
        DeviceProfile::Scribe,
    )?;
    {
        let mut state = fixture.app();
        for index in 0..100 {
            state.store.set_setting(
                &format!("thoughts:{BOOK}:{CHAPTER}:remote:{index}"),
                &vec![RemoteReview {
                    text: format!("Preview {index}"),
                    likes: 3,
                    ..Default::default()
                }],
            )?;
        }
        let reads = state.reader.thought_preview_reads;
        let layouts = state.reader.thought_quote_layouts;
        for _ in 0..16 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.thought_preview_reads, reads);
        assert_eq!(state.reader.thought_quote_layouts, layouts);
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_thought_count(),
            100
        );
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_count(),
            0
        );
        state.reader.notes_panel = true;
        state.reflow_reader()?;
        state.render_reader()?;
        let visible = fixture
            .ui()
            .global::<ReaderModel>()
            .get_thoughts()
            .row_count();
        assert!(visible > 0 && visible < 100 && visible <= super::thoughts::MAX_VISIBLE_CARDS);
        assert_eq!(state.reader.thought_preview_reads - reads, visible);
        assert_eq!(state.reader.thought_quote_layouts - layouts, visible);
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            "Preview 0"
        );
        let old_reads = state.reader.thought_preview_reads;
        let old_layouts = state.reader.thought_quote_layouts;
        state.reader.thought_page = 1;
        state.render_reader()?;
        let next = fixture
            .ui()
            .global::<ReaderModel>()
            .get_thoughts()
            .row_count();
        assert_eq!(state.reader.thought_preview_reads - old_reads, next);
        assert_eq!(state.reader.thought_quote_layouts - old_layouts, next);
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            format!("Preview {visible}")
        );
        state.reader.notes_panel = false;
        state.reflow_reader()?;
        let old_reads = state.reader.thought_preview_reads;
        let old_layouts = state.reader.thought_quote_layouts;
        for _ in 0..16 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.thought_preview_reads, old_reads);
        assert_eq!(state.reader.thought_quote_layouts, old_layouts);
    }
    fixture.finish()
}

fn cached_view_limits() -> Result<()> {
    let fixture =
        Fixture::new_for_profile("x body", vec![underline("x")], true, DeviceProfile::Scribe)?;
    {
        let mut state = fixture.app();
        let key = format!("thoughts:{BOOK}:{CHAPTER}:remote:0");
        let oversized = "z".repeat(super::thoughts::CACHE_BYTES + 1);
        state.store.set_setting(
            &key,
            &vec![RemoteReview {
                text: oversized.clone(),
                ..Default::default()
            }],
        )?;
        state.reader.draft.set_query("Keep pending local draft");
        state.reader.selection = Some((0, 1));
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.reader.notes_panel = true;
        state.reflow_reader()?;
        state.render_reader()?;
        let cards = fixture.ui().global::<ReaderModel>().get_thoughts();
        assert_eq!(cards.row_data(0).unwrap().body, tr("api_error"));
        assert_eq!(state.reader.draft.query, "Keep pending local draft");
        assert_eq!(state.reader.selection, Some((0, 1)));
        assert_eq!(
            state.store.setting::<Vec<RemoteReview>>(&key)?.unwrap()[0].text,
            oversized
        );
        let too_long_preview = "p".repeat(super::thoughts::PREVIEW_BYTES + 1);
        state.store.set_setting(
            &key,
            &vec![RemoteReview {
                text: too_long_preview.clone(),
                likes: 10,
                ..Default::default()
            }],
        )?;
        state.render_reader()?;
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            tr("api_error")
        );
        assert_eq!(
            state.store.setting::<Vec<RemoteReview>>(&key)?.unwrap()[0].text,
            too_long_preview
        );
        let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        connection.execute("UPDATE settings SET value=x'ff' WHERE key=?1", [&key])?;
        state.render_reader()?;
        assert_eq!(state.reader.draft.query, "Keep pending local draft");
        assert_eq!(state.reader.selection, Some((0, 1)));
        let kind: String = connection.query_row(
            "SELECT typeof(value) FROM settings WHERE key=?1",
            [&key],
            |row| row.get(0),
        )?;
        assert_eq!(kind, "blob");
        assert!(state.jobs.session().is_none());
    }
    fixture.finish()
}

fn opened_review_paging() -> Result<()> {
    let fixture =
        Fixture::new_for_profile("x body", vec![underline("x")], true, DeviceProfile::Scribe)?;
    {
        let mut state = fixture.app();
        state.reader_action("thought", "remote:0")?;
        state.reader.remote_thoughts = (0..100)
            .map(|index| RemoteReview {
                id: format!("review-{index}"),
                author: "Fixture author".into(),
                text: format!("Review {index}"),
                ..Default::default()
            })
            .collect();
        state.render_reader()?;
        let cards = fixture.ui().global::<ReaderModel>().get_thoughts();
        assert!(cards.row_count() > 0 && cards.row_count() < 100);
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_thought_count(),
            100
        );
        assert_eq!(cards.row_data(0).unwrap().body, "Review 0");
        let size = cards.row_count();
        state.reader.thought_page = 1;
        state.render_reader()?;
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            format!("Review {size}")
        );
        assert_eq!(state.reader.thought_quote_layouts, 0);
        assert_eq!(state.reader.thought_preview_reads, 0);
        let note = "n".repeat(super::thoughts::OPEN_TEXT_BYTES + 1);
        state.store.save_annotation(&Annotation {
            id: "large-local-note".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 1,
            quote: "x".into(),
            note: note.clone(),
            created_at: 0,
        })?;
        state
            .reader
            .draft
            .set_query("Do not replace this local draft");
        state.reader_action("thought", "mine:large-local-note")?;
        state.render_reader()?;
        assert_eq!(state.reader.draft.query, "Do not replace this local draft");
        assert_eq!(
            state
                .store
                .annotations(BOOK)?
                .iter()
                .find(|item| item.id == "large-local-note")
                .unwrap()
                .note,
            note
        );
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            tr("api_error")
        );
    }
    fixture.finish()
}

fn catalog_owner_preflight() -> Result<()> {
    let fixture = Fixture::new("x body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let before_bytes = state.store.materialized_chapter_bytes();
        let before_body = state.reader.body.clone();
        let before_chapter = state.reader.chapter_index;
        state.detail_id = BOOK.into();
        let request = state.request();
        state.catalog_requests.insert(BOOK.into(), request);
        let remote = source::RemoteChapter {
            uid: CHAPTER.into(),
            title: "Duplicate".into(),
            word_count: 1,
            ..Default::default()
        };
        let epoch = state.jobs.account_epoch();
        state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Catalog {
                book_id: BOOK.into(),
                result: Ok(vec![remote.clone(), remote]),
            },
        ))?;
        assert_eq!(state.store.materialized_chapter_bytes(), before_bytes);
        assert_eq!(state.reader.body, before_body);
        assert_eq!(state.reader.chapter_index, before_chapter);
        assert_eq!(state.store.chapter_uid(BOOK, 0)?.as_deref(), Some(CHAPTER));
        assert!(state.jobs.session().is_none());
        for index in 2..18 {
            state.store.upsert_chapter(&Chapter {
                id: format!("corpus-{index}"),
                book_id: BOOK.into(),
                index,
                title: "Corpus fixture".into(),
                body: Some("c".repeat(128 * 1024)),
                cached: true,
                content_status: ContentStatus::Full,
                ..Default::default()
            })?;
        }
        let headers = state.store.chapter_headers(BOOK)?;
        let current = headers
            .iter()
            .find(|chapter| chapter.id == CHAPTER)
            .unwrap();
        assert!(current.cached && current.body.is_none());
        let rows = headers
            .iter()
            .rev()
            .enumerate()
            .map(|(index, chapter)| source::RemoteChapter {
                uid: chapter.id.clone(),
                title: format!("New title {index}"),
                index: index as u32,
                word_count: 10,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let request = state.request();
        state.catalog_requests.insert(BOOK.into(), request);
        state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Catalog {
                book_id: BOOK.into(),
                result: Ok(rows),
            },
        ))?;
        assert_eq!(state.store.materialized_chapter_bytes(), before_bytes);
        assert_eq!(state.reader.body, before_body);
        assert_eq!(state.reader.chapter_index, 17);
        assert_eq!(
            state.store.chapter_body(BOOK, 17)?.as_deref(),
            Some(before_body.as_str())
        );
        assert_eq!(
            state.store.chapter_body(BOOK, 0)?.unwrap().len(),
            128 * 1024
        );
    }
    fixture.finish()
}

fn aligned_reader_images() -> Result<()> {
    let fixture = Fixture::new("x", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([255])))
            .write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        let id = {
            use sha2::Digest;
            hex::encode(sha2::Sha256::digest(&bytes))
        };
        let packet = source::images::DownloadedChapterImages {
            assets: vec![source::images::ImageAsset {
                id: id.clone(),
                media_type: "image/png".into(),
                extension: "png".into(),
                source_names: vec!["shared.png".into()],
                bytes,
            }],
            source_to_asset: BTreeMap::from([("shared.png".into(), id)]),
        };
        let manifest = state.book_assets.commit_chapter(
            &state.store,
            BOOK,
            CHAPTER,
            "<p>x</p><img src='shared.png'><img src='shared.png'>",
            "x",
            Some(&packet),
        )?;
        assert_eq!(manifest.images.len(), 2);
        assert_eq!(manifest.images[0].cache_key, manifest.images[1].cache_key);
        state.load_local_reader(BOOK, 0, None)?;
        let snapshot = state.reader_image_snapshot()?.unwrap();
        let layout = state.reader.paginator.as_ref().unwrap().layout;
        let images = state.reader_inline_images(&layout, Some(&snapshot))?;
        assert_eq!(images.len(), 2);
        assert_ne!(images[0].id, images[1].id);
        drop(snapshot);
        state.render_reader_images()?;
        assert_eq!(state.reader.body, "x");
        state.reflow_reader()?;
        assert_eq!(state.reader.body, "x");
    }
    fixture.finish()
}

fn catalog_long_owner_preflight() -> Result<()> {
    let fixture = Fixture::new("x body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = "opaque-owner-".repeat(400);
        let reads = state.store.chapter_header_read_count();
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        let request = state.request();
        state.catalog_requests.insert(owner.clone(), request);
        let row = source::RemoteChapter {
            uid: "uid".into(),
            title: "Fixture".into(),
            word_count: 1,
            ..Default::default()
        };
        let epoch = state.jobs.account_epoch();
        state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Catalog {
                book_id: owner,
                result: Ok(vec![
                    row.clone(),
                    source::RemoteChapter {
                        uid: "other-uid".into(),
                        ..row
                    },
                ]),
            },
        ))?;
        assert_eq!(state.store.chapter_header_read_count(), reads);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(
            state.store.chapter_body(BOOK, 0)?.as_deref(),
            Some(body.as_str())
        );
    }
    fixture.finish()
}

#[test]
fn thought_cards_read_and_layout_only_the_visible_page() -> Result<()> {
    isolated(
        "app::thoughts_tests::thought_cards_read_and_layout_only_the_visible_page",
        preview_card_paging,
    )
}

#[test]
fn thought_cache_limits_and_corruption_preserve_data_draft_and_selection() -> Result<()> {
    isolated(
        "app::thoughts_tests::thought_cache_limits_and_corruption_preserve_data_draft_and_selection",
        cached_view_limits,
    )
}

#[test]
fn opened_thought_reviews_page_before_copying_and_keep_oversized_local_notes() -> Result<()> {
    isolated(
        "app::thoughts_tests::opened_thought_reviews_page_before_copying_and_keep_oversized_local_notes",
        opened_review_paging,
    )
}

#[test]
fn catalog_owner_preflight_and_large_body_sql_reorder_preserve_the_reader() -> Result<()> {
    isolated(
        "app::thoughts_tests::catalog_owner_preflight_and_large_body_sql_reorder_preserve_the_reader",
        catalog_owner_preflight,
    )
}

#[test]
fn reader_images_use_an_operation_aligned_bulk_snapshot() -> Result<()> {
    isolated(
        "app::thoughts_tests::reader_images_use_an_operation_aligned_bulk_snapshot",
        aligned_reader_images,
    )
}

#[test]
fn catalog_long_owner_admission_precedes_any_old_header_read() -> Result<()> {
    isolated(
        "app::thoughts_tests::catalog_long_owner_admission_precedes_any_old_header_read",
        catalog_long_owner_preflight,
    )
}
