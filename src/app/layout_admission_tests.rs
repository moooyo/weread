//! Real App refusals retain receivers, model ownership, bookmarks, and queues.

use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use crate::reader::layout_budget::with_layout_test_limits;
use crate::reader::{LayoutLimitError, LayoutLimits};
use crate::store::{StoreInventoryLimits, with_store_inventory_test_limits};
use slint::Model;

fn reader_refusal() -> Result<()> {
    let fixture = Fixture::new("甲e\u{301}\n乙\n", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.store.upsert_chapter(&Chapter {
            id: "underline-fixture-chapter-1".into(),
            book_id: BOOK.into(),
            index: 1,
            title: "A small line-heavy chapter".into(),
            body: Some("x\n".repeat(8)),
            cached: true,
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        state.reader.selection = Some((0, 2));
        assert!(
            state
                .reader
                .draft
                .set_query_for_purpose("Keep this draft", crate::input::InputPurpose::Thought)
        );
        state.store.set_setting(
            &format!("progress.pending:{BOOK}"),
            &source::ReadingPosition {
                chapter_uid: "underline-fixture-chapter-1".into(),
                chapter_index: 1,
                chapter_offset: 1,
                ..Default::default()
            },
        )?;
        let global = fixture.ui().global::<ReaderModel>();
        let rows = Rc::new(slint::VecModel::from(
            global.get_lines().iter().collect::<Vec<_>>(),
        ));
        let weak_rows = Rc::downgrade(&rows);
        global.set_lines(rows.clone().into());
        drop(rows);
        let old_body = state.reader.body.clone();
        let old_page = state.reader.paginator.as_ref().unwrap().pages.as_ptr();
        let old_route = state.route.clone();
        let old_selection = state.reader.selection;
        let old_session = state.reader.session_started;
        let old_font = state.reader.preferences.font.clone();
        let old_size = state.reader.preferences.font_size;
        let error = with_layout_test_limits(
            LayoutLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            || state.load_reader(BOOK, 1, None),
        )
        .unwrap_err();
        assert!(error.downcast_ref::<LayoutLimitError>().is_some());
        assert_eq!(state.reader.body, old_body);
        assert_eq!(
            state.reader.paginator.as_ref().unwrap().pages.as_ptr(),
            old_page
        );
        assert_eq!(state.route, old_route);
        assert_eq!(state.reader.selection, old_selection);
        assert_eq!(state.reader.session_started, old_session);
        assert_eq!(state.reader.draft.query, "Keep this draft");
        assert!(
            state
                .store
                .setting::<source::ReadingPosition>(&format!("progress.pending:{BOOK}"))?
                .is_some()
        );
        assert!(weak_rows.upgrade().is_some());
        assert!(
            with_layout_test_limits(
                LayoutLimits {
                    metadata_bytes: 0,
                    ..Default::default()
                },
                || state.reader_action("font-size", "1")
            )
            .is_err()
        );
        assert_eq!(state.reader.preferences.font, old_font);
        assert_eq!(state.reader.preferences.font_size, old_size);
        state.load_reader(BOOK, 1, None)?;
        state.render_reader()?;
        assert!(weak_rows.upgrade().is_none());
        assert_eq!(state.reader.paginator.as_ref().unwrap().char_count, 16);
    }
    fixture.finish()
}

#[test]
fn real_reader_layout_refusal_retains_draft_pending_choice_and_rows() -> Result<()> {
    isolated(
        "app::layout_admission_tests::real_reader_layout_refusal_retains_draft_pending_choice_and_rows",
        reader_refusal,
    )
}

fn article_refusal() -> Result<()> {
    const ID: &str = "layout-current-article";
    let fixture = Fixture::new("Unrelated Reader body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let book = format!("article:{ID}");
        state.store.upsert_book(&Book {
            source: "article".into(),
            on_shelf: false,
            ..Book::new(&book, "Retained Article")
        })?;
        state.store.upsert_chapter(&Chapter {
            id: ID.into(),
            book_id: book.clone(),
            body: Some("Old text".into()),
            cached: true,
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        state
            .store
            .set_setting(&format!("article.html:{ID}"), &"<p>Old text</p>")?;
        state.navigate(Route::Article(ID.into()), true)?;
        state.render_article()?;
        state.pending_articles.insert(ID.into());
        let global = fixture.ui().global::<ArticleModel>();
        let rows = Rc::new(slint::VecModel::from(
            global.get_blocks().iter().collect::<Vec<_>>(),
        ));
        let weak_rows = Rc::downgrade(&rows);
        global.set_blocks(rows.clone().into());
        drop(rows);
        let page = state.article.pages.as_ptr();
        let route = state.route.clone();
        let event = JobEvent::new(
            state.request(),
            state.jobs.account_epoch(),
            JobResult::Article {
                article_id: ID.into(),
                result: Ok(source::ChapterContent {
                    html: "<p>x</p><p>x</p><p>x</p>".into(),
                    plain_text: "x\nx\nx\n".into(),
                    style: None,
                    format: source::ContentFormat::Xhtml,
                    content_status: ContentStatus::Full,
                }),
            },
        );
        assert!(!with_layout_test_limits(
            LayoutLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            || state.handle_job(event)
        )?);
        assert_eq!(state.route, route);
        assert_eq!(state.article.body, "Old text");
        assert_eq!(state.article.html, "<p>Old text</p>");
        assert_eq!(state.article.pages.as_ptr(), page);
        assert!(state.pending_articles.contains(ID));
        assert_eq!(
            state.store.chapter_body(&book, 0)?.as_deref(),
            Some("Old text")
        );
        assert!(weak_rows.upgrade().is_some());
        state.store.upsert_chapter(&Chapter {
            id: ID.into(),
            book_id: book,
            body: Some("x\nx\n".into()),
            cached: true,
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        state
            .store
            .set_setting(&format!("article.html:{ID}"), &"<p>x</p><p>x</p>")?;
        state.load_article(ID)?;
        state.render_article()?;
        assert!(weak_rows.upgrade().is_none());
    }
    fixture.finish()
}

#[test]
fn real_article_layout_refusal_precedes_raw_sql_and_receiver_retirement() -> Result<()> {
    isolated(
        "app::layout_admission_tests::real_article_layout_refusal_precedes_raw_sql_and_receiver_retirement",
        article_refusal,
    )
}

fn bookmark_projection() -> Result<()> {
    let fixture = Fixture::new("Reader bookmark fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        for (id, offset, created) in [("older", 0, 1), ("newer", 2, 2), ("third", 12, 3)] {
            state.store.save_bookmark(&Bookmark {
                id: id.into(),
                book_id: BOOK.into(),
                chapter_index: 0,
                char_offset: offset,
                label: "A retained label".into(),
                created_at: created,
            })?;
        }
        crate::store::reset_bookmark_test_read_stats();
        state.render_reader()?;
        assert_eq!(crate::store::bookmark_test_read_stats().id_copies, 0);
        assert_eq!(crate::store::bookmark_test_read_stats().label_copies, 0);
        assert_eq!(fixture.ui().global::<ReaderModel>().get_mark_count(), 3);
        state.reader_action("bookmark", "")?;
        assert_eq!(crate::store::bookmark_test_read_stats().id_copies, 1);
        assert_eq!(crate::store::bookmark_test_read_stats().label_copies, 0);
        assert_eq!(state.store.bookmark_count(BOOK)?, 2);
        crate::store::reset_bookmark_test_read_stats();
        state.reader.toc_tab = "notes".into();
        fixture.ui().global::<ReaderModel>().set_toc_open(true);
        state.render_reader()?;
        assert_eq!(crate::store::bookmark_test_read_stats().id_copies, 0);
        assert_eq!(crate::store::bookmark_test_read_stats().label_copies, 0);
    }
    fixture.finish()
}

#[test]
fn real_reader_bookmark_projection_copies_only_the_selected_canonical_id() -> Result<()> {
    isolated(
        "app::layout_admission_tests::real_reader_bookmark_projection_copies_only_the_selected_canonical_id",
        bookmark_projection,
    )
}

fn task_inventory_refusal() -> Result<()> {
    let fixture = Fixture::new("Reader task fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.jobs.replace_session(source::AuthSession {
            user_vid: "layout-task-owner".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "synthetic-key".into())]),
            ..Default::default()
        })?;
        let owner = state.chapter_content_owner().unwrap();
        let first = DownloadTask {
            id: "layout-task-first".into(),
            book_id: BOOK.into(),
            account_key: owner.clone(),
            chapters: vec![0, 1],
            status: DownloadStatus::Paused,
            next_index: 1,
            ..Default::default()
        };
        let second = DownloadTask {
            id: "layout-task-second".into(),
            next_index: 0,
            status: DownloadStatus::Pending,
            ..first.clone()
        };
        state.store.save_task(&first)?;
        state.store.save_task(&second)?;
        state.downloads = VecDeque::from([first.clone(), second.clone()]);
        state.current_download = Some(first.clone());
        let session = state.jobs.session();
        crate::store::reset_inventory_test_read_stats();
        assert!(!with_store_inventory_test_limits(
            StoreInventoryLimits {
                rows: 0,
                ..Default::default()
            },
            || state.reload_pending_downloads(true)
        )?);
        assert_eq!(
            state.downloads.iter().collect::<Vec<_>>(),
            [&first, &second]
        );
        assert_eq!(state.current_download.as_ref(), Some(&first));
        assert_eq!(state.jobs.session(), session);
        assert_eq!(
            crate::store::inventory_test_read_stats().task_deserializations,
            0
        );
        assert_eq!(state.store.task(&first.id)?.unwrap().next_index, 1);
        crate::store::reset_inventory_test_read_stats();
        state.current_download = None;
        state.demo = false;
        state.force_offline = false;
        state.download_retry_at = Instant::now() - Duration::from_secs(1);
        state.retry_pending_downloads()?;
        assert_eq!(
            crate::store::inventory_test_read_stats().task_deserializations,
            0
        );
        assert_eq!(
            state.downloads.iter().collect::<Vec<_>>(),
            [&first, &second]
        );
        state.force_offline = true;
    }
    fixture.finish()
}

#[test]
fn real_app_inventory_refusal_keeps_fifo_cursor_session_and_stops_retry() -> Result<()> {
    isolated(
        "app::layout_admission_tests::real_app_inventory_refusal_keeps_fifo_cursor_session_and_stops_retry",
        task_inventory_refusal,
    )
}

fn annotation_body_refusal() -> Result<()> {
    let fixture = Fixture::new("甲e\u{301} Reader", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.reader.selection = Some((0, 3));
        assert!(
            state
                .reader
                .draft
                .set_query_for_purpose("Preserved draft", crate::input::InputPurpose::Thought)
        );
        let body = state.reader.body.clone();
        let selection = state.reader.selection;
        let account = state.jobs.session();
        assert!(
            with_store_inventory_test_limits(
                StoreInventoryLimits {
                    work_bytes: 0,
                    ..Default::default()
                },
                || state.reader_action("highlight", "selection")
            )
            .is_err()
        );
        assert_eq!(state.reader.selection, selection);
        assert_eq!(state.reader.draft.query, "Preserved draft");
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.store.annotation_count(BOOK)?, 0);
        state.reader_action("highlight", "selection")?;
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
        assert_eq!(state.store.annotations(BOOK)?[0].quote, "甲e\u{301}");
    }
    fixture.finish()
}

#[test]
fn real_annotation_scalar_refusal_retains_selection_and_original_unicode() -> Result<()> {
    isolated(
        "app::layout_admission_tests::real_annotation_scalar_refusal_retains_selection_and_original_unicode",
        annotation_body_refusal,
    )
}
