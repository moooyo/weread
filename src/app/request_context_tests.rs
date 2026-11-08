//! Genuine view callbacks followed by typed, matching or late transport events.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use slint::Model;

const BODY: &str = "One Two.";

fn account(identity: &str) -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-context-key".into(),
        user_vid: identity.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), identity.into()),
            ("wr_skey".into(), "synthetic-context-session".into()),
        ]),
        ..Default::default()
    }
}

fn underlines() -> Vec<RemoteUnderline> {
    ["One", "Two"]
        .into_iter()
        .enumerate()
        .map(|(index, text)| RemoteUnderline {
            text: text.into(),
            range: format!("context-range-{index}"),
            count: 1,
            thoughts: 1,
        })
        .collect()
}

fn reviews(text: &str) -> Vec<RemoteReview> {
    vec![RemoteReview {
        id: "context-review".into(),
        author: "Fixture author".into(),
        text: text.into(),
        likes: 1,
        ..Default::default()
    }]
}

fn reader_event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event(action.into(), value.into());
}

fn detail_event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<DetailModel>()
        .invoke_event(action.into(), value.into());
}

fn thought_event(request: u64, epoch: u64, result: source::Result<Vec<RemoteReview>>) -> JobEvent {
    JobEvent::new(
        request,
        epoch,
        JobResult::Thoughts {
            book_id: BOOK.into(),
            chapter_id: CHAPTER.into(),
            result,
        },
    )
}

fn review_event(request: u64, epoch: u64, result: source::Result<Vec<RemoteReview>>) -> JobEvent {
    JobEvent::new(
        request,
        epoch,
        JobResult::Reviews {
            book_id: BOOK.into(),
            result,
        },
    )
}

fn thought_key(index: usize) -> String {
    format!("thoughts:{BOOK}:{CHAPTER}:remote:{index}")
}

fn retained(
    state: &App,
) -> (
    String,
    String,
    Option<(u32, u32)>,
    String,
    Option<source::AuthSession>,
    Route,
) {
    (
        state.reader.draft.query.clone(),
        state.reader.draft.composition.clone(),
        state.reader.selection,
        state.reader.body.clone(),
        state.jobs.session(),
        state.route.clone(),
    )
}

fn mine_fence() -> Result<()> {
    let fixture = Fixture::new(BODY, underlines(), true)?;
    {
        let state = fixture.app();
        state.jobs.replace_session(account("context-account-a"))?;
        state.store.save_annotation(&Annotation {
            id: "context-mine".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 3,
            quote: "One".into(),
            note: "The local note".into(),
            created_at: 1,
        })?;
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    let (request, epoch) = {
        let state = fixture.app();
        assert_eq!(
            state.requested_thought_cache_key(state.reader.thought_request, BOOK, CHAPTER),
            Some(thought_key(0).as_str())
        );
        (state.reader.thought_request, state.jobs.account_epoch())
    };
    reader_event(&fixture, "reader.thought", "mine:context-mine");
    {
        let mut state = fixture.app();
        let kept = retained(&state);
        assert!(state.reader.thought_context.is_none());
        assert!(!state.handle_job(thought_event(
            request,
            epoch,
            Ok(reviews("Late public note"))
        ))?);
        assert!(!state.handle_job(thought_event(
            request,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
        assert_eq!(state.reader.thought_id, "mine:context-mine");
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&thought_key(0))?
                .is_none()
        );
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&format!(
                    "thoughts:{BOOK}:{CHAPTER}:mine:context-mine"
                ))?
                .is_none()
        );
        assert_eq!(
            state
                .store
                .annotation_by_id(BOOK, "context-mine")?
                .unwrap()
                .note,
            "The local note"
        );
    }
    fixture.finish()
}

#[test]
fn late_remote_success_and_login_error_cannot_rebind_a_local_note_view() -> Result<()> {
    isolated(
        "app::request_context_tests::late_remote_success_and_login_error_cannot_rebind_a_local_note_view",
        mine_fence,
    )
}

fn offline_other_remote() -> Result<()> {
    let fixture = Fixture::new(BODY, underlines(), true)?;
    {
        let state = fixture.app();
        state.jobs.replace_session(account("context-account-a"))?;
        state
            .store
            .set_setting(&thought_key(0), &reviews("Retained historical first view"))?;
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    let (old_request, epoch) = {
        let state = fixture.app();
        (state.reader.thought_request, state.jobs.account_epoch())
    };
    {
        fixture.app().online = false;
    }
    reader_event(&fixture, "reader.thought", "remote:1");
    {
        let mut state = fixture.app();
        let request = state.reader.thought_request;
        assert_ne!(request, old_request);
        let kept = retained(&state);
        assert!(!state.handle_job(thought_event(
            old_request,
            epoch,
            Ok(reviews("Wrong first-view batch"))
        ))?);
        assert!(!state.handle_job(thought_event(
            old_request,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
        assert_eq!(state.reader.thought_id, "remote:1");
        assert!(state.reader.remote_thoughts.is_empty());
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&thought_key(1))?
                .is_none()
        );
        assert!(state.handle_job(thought_event(
            request,
            epoch,
            Ok(reviews("Matching second-view batch"))
        ))?);
        assert_eq!(
            state
                .store
                .setting::<Vec<RemoteReview>>(&thought_key(0))?
                .unwrap()[0]
                .text,
            "Retained historical first view"
        );
        assert_eq!(
            state
                .store
                .setting::<Vec<RemoteReview>>(&thought_key(1))?
                .unwrap()[0]
                .text,
            "Matching second-view batch"
        );
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            "Matching second-view batch"
        );
    }
    fixture.finish()
}

#[test]
fn offline_view_change_uses_a_new_token_and_an_immutable_remote_cache_key() -> Result<()> {
    isolated(
        "app::request_context_tests::offline_view_change_uses_a_new_token_and_an_immutable_remote_cache_key",
        offline_other_remote,
    )
}

fn close_document_account() -> Result<()> {
    let fixture = Fixture::new(BODY, underlines(), true)?;
    fixture
        .app()
        .jobs
        .replace_session(account("context-account-a"))?;
    reader_event(&fixture, "reader.thought", "remote:0");
    let (first, epoch) = {
        let state = fixture.app();
        (state.reader.thought_request, state.jobs.account_epoch())
    };
    reader_event(&fixture, "reader.thought-close", "");
    {
        let mut state = fixture.app();
        let kept = retained(&state);
        assert!(state.reader.thought_context.is_none());
        assert!(!state.handle_job(thought_event(
            first,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    let second = fixture.app().reader.thought_request;
    {
        let mut state = fixture.app();
        assert_ne!(first, second);
        assert!(!state.handle_job(thought_event(
            first,
            epoch,
            Ok(reviews("Closed first view"))
        ))?);
        state.load_local_reader(BOOK, 1, None)?;
        state.render_reader()?;
        let body = state.reader.body.clone();
        assert!(state.reader.thought_context.is_none());
        assert!(!state.handle_job(thought_event(
            second,
            epoch,
            Ok(reviews("Previous document"))
        ))?);
        assert_eq!(state.reader.body, body);
        state.load_local_reader(BOOK, 0, None)?;
        state.render_reader()?;
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    {
        let mut state = fixture.app();
        let request = state.reader.thought_request;
        state.invalidate_account_work()?;
        let replacement = account("context-account-b");
        state.jobs.replace_session(replacement.clone())?;
        let kept = retained(&state);
        assert!(!state.handle_job(thought_event(
            request,
            epoch,
            Ok(reviews("Previous account"))
        ))?);
        assert!(!state.handle_job(thought_event(
            request,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
        assert_eq!(state.jobs.session(), Some(replacement));
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&thought_key(0))?
                .is_none()
        );
    }
    fixture.finish()
}

#[test]
fn close_reopen_document_and_account_changes_retire_the_old_thought_context() -> Result<()> {
    isolated(
        "app::request_context_tests::close_reopen_document_and_account_changes_retire_the_old_thought_context",
        close_document_account,
    )
}

fn cookie_rotation() -> Result<()> {
    let fixture = Fixture::new(BODY, underlines(), true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = source::WeReadClient::new(account("context-account-a"))?;
    {
        let mut state = fixture.app();
        let (sender, events) = mpsc::channel();
        state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client.clone()))?;
        state.events = events;
        state.demo = false;
        assert!(state.force_offline);
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    {
        let mut state = fixture.app();
        let epoch = state.jobs.account_epoch();
        let request = state.reader.thought_request;
        let mut updated = client.session();
        updated
            .cookies
            .insert("wr_skey".into(), "synthetic-context-rotated-session".into());
        client.replace_session(updated.clone())?;
        assert_eq!(state.jobs.account_epoch(), epoch);
        assert!(state.handle_job(thought_event(
            request,
            epoch,
            Ok(reviews("Matching rotated-session result"))
        ))?);
        state.persist_session_updates()?;
        assert_eq!(state.jobs.session(), Some(updated.clone()));
        assert_eq!(state.session_file.load()?, Some(updated));
        assert_eq!(
            state
                .store
                .setting::<Vec<RemoteReview>>(&thought_key(0))?
                .unwrap()[0]
                .text,
            "Matching rotated-session result"
        );
    }
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn shared_client_cookie_rotation_keeps_a_matching_view_and_persistence_live() -> Result<()> {
    isolated(
        "app::request_context_tests::shared_client_cookie_rotation_keeps_a_matching_view_and_persistence_live",
        cookie_rotation,
    )
}

fn review_order_fence() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    fixture
        .app()
        .jobs
        .replace_session(account("context-account-a"))?;
    reader_event(&fixture, "reader.detail", "");
    detail_event(&fixture, "tab", "reviews");
    let (hot, epoch) = {
        let mut state = fixture.app();
        let request = state.review_request;
        let epoch = state.jobs.account_epoch();
        assert!(state.handle_job(review_event(
            request,
            epoch,
            Ok(reviews("Current hot batch"))
        ))?);
        // A parallel BookInfo error is an actual producer of the offline state.
        let book_request = state.request();
        state.book_requests.insert(BOOK.into(), book_request);
        assert!(state.handle_job(JobEvent::new(
            book_request,
            epoch,
            JobResult::BookInfo {
                book_id: BOOK.into(),
                result: Err(source::SourceError::Network)
            }
        ))?);
        assert!(!state.online);
        (request, epoch)
    };
    detail_event(&fixture, "review-order", "latest");
    {
        let mut state = fixture.app();
        let latest = state.review_request;
        assert_ne!(hot, latest);
        assert!(state.reviews.is_empty());
        let kept = retained(&state);
        assert!(!state.handle_job(review_event(hot, epoch, Ok(reviews("Late hot batch"))))?);
        assert!(!state.handle_job(review_event(
            hot,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
        assert!(state.reviews.is_empty());
        assert_eq!(
            fixture.ui().global::<DetailModel>().get_review_order(),
            "latest"
        );
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&format!("reviews:{BOOK}:latest"))?
                .is_none()
        );
        assert!(state.handle_job(review_event(
            latest,
            epoch,
            Ok(reviews("Matching latest batch"))
        ))?);
        assert_eq!(
            state
                .store
                .setting::<Vec<RemoteReview>>(&format!("reviews:{BOOK}:hot"))?
                .unwrap()[0]
                .text,
            "Current hot batch"
        );
        assert_eq!(
            state
                .store
                .setting::<Vec<RemoteReview>>(&format!("reviews:{BOOK}:latest"))?
                .unwrap()[0]
                .text,
            "Matching latest batch"
        );
    }
    fixture.finish()
}

#[test]
fn offline_review_order_change_neither_displays_nor_caches_a_late_hot_batch_as_latest() -> Result<()>
{
    isolated(
        "app::request_context_tests::offline_review_order_change_neither_displays_nor_caches_a_late_hot_batch_as_latest",
        review_order_fence,
    )
}

fn review_tab_route_fence() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    fixture
        .app()
        .jobs
        .replace_session(account("context-account-a"))?;
    reader_event(&fixture, "reader.detail", "");
    detail_event(&fixture, "tab", "reviews");
    let (request, epoch) = {
        let state = fixture.app();
        (state.review_request, state.jobs.account_epoch())
    };
    detail_event(&fixture, "tab", "notes");
    {
        let mut state = fixture.app();
        let kept = retained(&state);
        assert!(!state.handle_job(review_event(
            request,
            epoch,
            Err(source::SourceError::Http(401))
        ))?);
        assert_eq!(retained(&state), kept);
    }
    detail_event(&fixture, "tab", "reviews");
    {
        let mut state = fixture.app();
        let current = state.review_request;
        assert_ne!(request, current);
        assert!(!state.handle_job(review_event(request, epoch, Ok(reviews("Previous tab"))))?);
        state.navigate(Route::Library, true)?;
        let kept = retained(&state);
        assert!(state.review_context.is_none());
        assert!(!state.handle_job(review_event(current, epoch, Ok(reviews("Previous route"))))?);
        assert_eq!(retained(&state), kept);
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&format!("reviews:{BOOK}:hot"))?
                .is_none()
        );
    }
    fixture.finish()
}

#[test]
fn review_tab_and_route_changes_drop_old_success_and_error_events() -> Result<()> {
    isolated(
        "app::request_context_tests::review_tab_and_route_changes_drop_old_success_and_error_events",
        review_tab_route_fence,
    )
}

fn current_budget_error() -> Result<()> {
    let fixture = Fixture::new(BODY, underlines(), true)?;
    {
        let state = fixture.app();
        state.jobs.replace_session(account("context-account-a"))?;
        state
            .store
            .set_setting(&thought_key(0), &reviews("Admitted current cached result"))?;
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    {
        let mut state = fixture.app();
        assert!(state.reader.draft.set_query("Retained current draft"));
        let kept = retained(&state);
        let request = state.reader.thought_request;
        let epoch = state.jobs.account_epoch();
        let stored = state.store.setting::<serde_json::Value>(&thought_key(0))?;
        assert!(state.handle_job(thought_event(
            request,
            epoch,
            Err(source::SourceError::ResponseTooLarge)
        ))?);
        assert_eq!(retained(&state), kept);
        assert_eq!(
            state.store.setting::<serde_json::Value>(&thought_key(0))?,
            stored
        );
        assert_eq!(
            state.reader.remote_thoughts[0].text,
            "Admitted current cached result"
        );
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("api_error")
        );
    }
    fixture.finish()
}

#[test]
fn a_matching_response_budget_error_preserves_current_cache_editor_and_account() -> Result<()> {
    isolated(
        "app::request_context_tests::a_matching_response_budget_error_preserves_current_cache_editor_and_account",
        current_budget_error,
    )
}
