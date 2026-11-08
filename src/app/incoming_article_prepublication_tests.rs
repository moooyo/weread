//! Incoming Article capacity is admitted before retiring the current receiver.

use super::auxiliary::{ArticleProjectionLimits, with_article_projection_limits};
use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::article::{StyleTestLimits, style_budget_tests::ObservationScope};
use crate::jobs::JobResult;
use crate::source::report::{AllocationHttpFixture, AllocationReply};

const ID: &str = "incoming-current";
const OTHER: &str = "incoming-other";
const OLD: &str = "Old content.";
const NEW: &str = "New content.";
const OLD_HTML: &str =
    "<p style='color:blue'>Old content.</p><img src='https://example.invalid/held.png'>";
const NEW_HTML: &str = "<p style='color:blue'>New content.</p>";

fn seed(state: &mut App) -> Result<()> {
    state.store.upsert_book(&Book {
        source: "article".into(),
        on_shelf: false,
        intro: "Preserved incoming introduction".into(),
        ..Book::new(format!("article:{ID}"), "Preserved Article")
    })?;
    state.store.upsert_chapter(&Chapter {
        id: ID.into(),
        book_id: format!("article:{ID}"),
        body: Some(OLD.into()),
        content_status: ContentStatus::Full,
        ..Default::default()
    })?;
    state
        .store
        .set_setting(&format!("article.html:{ID}"), &OLD_HTML)?;
    state.mp_articles = BTreeMap::from([(
        "MP_WXS_incoming".into(),
        vec![MpArticle {
            id: ID.into(),
            account_id: "MP_WXS_incoming".into(),
            title: "Preserved Article".into(),
            author: "Preserved author".into(),
            ..Default::default()
        }],
    )]);
    state.navigate(Route::Article(ID.into()), true)?;
    state.pending_articles.extend([ID.into(), OTHER.into()]);
    state
        .pending_article_images
        .insert("preserved-incoming-image-request".into());
    state
        .reader
        .draft
        .set_query("Preserved Reader draft during Article delivery");
    Ok(())
}

fn event(state: &mut App) -> Result<JobEvent> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = AllocationHttpFixture::new(|request| {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path(), "/web/mp/content");
        assert_eq!(request.target, format!("/web/mp/content?reviewId={ID}"));
        AllocationReply::text(NEW_HTML)
    });
    let client = {
        let _entered = runtime.enter();
        http.client(source::AuthSession {
            api_key: "synthetic-article-key".into(),
            user_vid: "synthetic-article-owner".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "synthetic-article-session".into())]),
            ..Default::default()
        })
    };
    let (sender, events) = mpsc::channel();
    state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
    state.events = events;
    let request = state.request();
    state.pending_article_requests.insert(ID.into(), request);
    assert!(state.jobs.article(request, ID.into()).is_none());
    let event = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match state.events.try_recv() {
                    Ok(event) => break Ok(event),
                    Err(mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(1)).await
                    }
                    Err(error) => {
                        break Err(anyhow::anyhow!(
                            "The actual Article mailbox closed: {error}"
                        ));
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("The actual Article source timed out"))?
    })?;
    assert_eq!(event.request, request);
    assert!(event.credits.is_some());
    assert!(
        matches!(&event.result, JobResult::Article { article_id, result: Ok(content) }
        if article_id == ID && content.html == NEW_HTML && content.plain_text == NEW)
    );
    assert_eq!(http.finish().len(), 1);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(event)
}

struct Retained {
    route: Route,
    body: String,
    html: String,
    pages: *const Vec<crate::article::PageBlock>,
    page_count: usize,
    page: usize,
    pending: BTreeSet<String>,
    images: BTreeSet<String>,
    ready: bool,
    draft: String,
    account: Option<source::AuthSession>,
}

impl Retained {
    fn capture(state: &App) -> Self {
        Self {
            route: state.route.clone(),
            body: state.article.body.clone(),
            html: state.article.html.clone(),
            pages: state.article.pages.as_ptr(),
            page_count: state.article.pages.len(),
            page: state.article.page,
            pending: state.pending_articles.clone(),
            images: state.pending_article_images.clone(),
            ready: state.article.image_work.is_ready_for(ID),
            draft: state.reader.draft.query.clone(),
            account: state.jobs.session(),
        }
    }
    fn assert_same(&self, state: &App) {
        assert_eq!(state.route, self.route);
        assert_eq!(state.article.id, ID);
        assert_eq!(state.article.body, self.body);
        assert_eq!(state.article.html, self.html);
        assert_eq!(state.article.pages.as_ptr(), self.pages);
        assert_eq!(state.article.pages.len(), self.page_count);
        assert_eq!(state.article.page, self.page);
        let mut pending = self.pending.clone();
        pending.remove(ID);
        assert_eq!(state.pending_articles, pending);
        assert!(!state.pending_article_requests.contains_key(ID));
        assert_eq!(state.pending_article_images, self.images);
        assert_eq!(state.article.image_work.is_ready_for(ID), self.ready);
        assert_eq!(state.reader.draft.query, self.draft);
        assert_eq!(state.jobs.session(), self.account);
    }
}

fn rejected_projection() -> Result<()> {
    let fixture = Fixture::new("Unrelated preserved Reader body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state)?;
        let incoming = event(&mut state)?;
        let retained = Retained::capture(&state);
        assert!(!with_article_projection_limits(
            ArticleProjectionLimits {
                work_bytes: 0,
                ..Default::default()
            },
            || state.handle_job(incoming)
        )?);
        retained.assert_same(&state);
        assert_eq!(
            state
                .store
                .chapter_body(&format!("article:{ID}"), 0)?
                .as_deref(),
            Some(OLD)
        );
        assert_eq!(
            state
                .store
                .setting::<String>(&format!("article.html:{ID}"))?
                .as_deref(),
            Some(OLD_HTML)
        );
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("api_error")
        );
    }
    fixture.finish()
}

#[test]
fn incoming_projection_quota_preserves_old_pages_body_images_and_request_flags() -> Result<()> {
    isolated(
        "app::incoming_article_prepublication_tests::incoming_projection_quota_preserves_old_pages_body_images_and_request_flags",
        rejected_projection,
    )
}

fn rejected_style() -> Result<()> {
    let fixture = Fixture::new("Unrelated preserved Reader body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state)?;
        let incoming = event(&mut state)?;
        let retained = Retained::capture(&state);
        let observation = ObservationScope::start(StyleTestLimits {
            work_bytes: 0,
            ..Default::default()
        });
        assert!(!state.handle_job(incoming)?);
        retained.assert_same(&state);
        assert_eq!(
            state
                .store
                .chapter_body(&format!("article:{ID}"), 0)?
                .as_deref(),
            Some(OLD)
        );
        drop(observation);
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("api_error")
        );
    }
    fixture.finish()
}

#[test]
fn incoming_checked_style_quota_does_not_retire_the_current_receiver() -> Result<()> {
    isolated(
        "app::incoming_article_prepublication_tests::incoming_checked_style_quota_does_not_retire_the_current_receiver",
        rejected_style,
    )
}

fn cache_sql_failure() -> Result<()> {
    let fixture = Fixture::new("Unrelated preserved Reader body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state)?;
        let database = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        database.execute_batch(&format!(
            "CREATE TRIGGER refuse_incoming_html BEFORE INSERT ON settings
             WHEN NEW.key='article.html:{ID}'
             BEGIN SELECT RAISE(ABORT,'synthetic incoming HTML cache failure'); END;"
        ))?;
        let incoming = event(&mut state)?;
        let retained = Retained::capture(&state);
        let error = state.handle_job(incoming).unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        retained.assert_same(&state);
        // The raw Chapter SQL write is known to have completed. Do not undo it
        // or install a partially persisted target into the old receiver.
        assert_eq!(
            state
                .store
                .chapter_body(&format!("article:{ID}"), 0)?
                .as_deref(),
            Some(NEW)
        );
        assert_eq!(
            state
                .store
                .setting::<String>(&format!("article.html:{ID}"))?
                .as_deref(),
            Some(OLD_HTML)
        );
        assert_eq!(
            state.store.book(&format!("article:{ID}"))?.unwrap().intro,
            "Preserved incoming introduction"
        );
        database.execute_batch("DROP TRIGGER refuse_incoming_html")?;
    }
    fixture.finish()
}

#[test]
fn incoming_sql_cache_failure_keeps_its_cause_and_old_receiver_without_undoing_commit() -> Result<()>
{
    isolated(
        "app::incoming_article_prepublication_tests::incoming_sql_cache_failure_keeps_its_cause_and_old_receiver_without_undoing_commit",
        cache_sql_failure,
    )
}

fn admitted_success() -> Result<()> {
    let fixture = Fixture::new("Unrelated preserved Reader body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state)?;
        let draft = state.reader.draft.query.clone();
        let incoming = event(&mut state)?;
        let account = state.jobs.session();
        assert!(state.handle_job(incoming)?);
        assert_eq!(state.article.body, NEW);
        assert_eq!(state.article.html, NEW_HTML);
        assert_eq!(
            state
                .store
                .chapter_body(&format!("article:{ID}"), 0)?
                .as_deref(),
            Some(NEW)
        );
        assert!(!state.pending_articles.contains(ID));
        assert!(state.pending_articles.contains(OTHER));
        assert_eq!(state.reader.draft.query, draft);
        assert_eq!(state.jobs.session(), account);
        assert!(state.article.pages.len() > 0);
    }
    fixture.finish()
}

#[test]
fn admitted_incoming_article_publishes_once_and_settles_only_its_own_request() -> Result<()> {
    isolated(
        "app::incoming_article_prepublication_tests::admitted_incoming_article_publishes_once_and_settles_only_its_own_request",
        admitted_success,
    )
}
