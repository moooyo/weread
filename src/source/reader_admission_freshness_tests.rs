//! Parser queueing must not make request-bound access evidence reusable.

use super::*;
use crate::source::reader_admission::{READER_BUDGET_BYTES, ReaderTier};

#[derive(Default)]
struct ParserHolds {
    released: Mutex<bool>,
    changed: Condvar,
}

impl ParserHolds {
    fn wait(&self) {
        let deadline = Instant::now() + SHARD_HOLD_WAIT;
        let mut released = self.released.lock().unwrap();
        while !*released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "The parser fixture was not released");
            released = self.changed.wait_timeout(released, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.changed.notify_all();
    }
}

struct ReleaseParsers(Arc<ParserHolds>);

impl Drop for ReleaseParsers {
    fn drop(&mut self) {
        self.0.release();
    }
}

async fn wait_until(condition: impl Fn() -> bool) {
    tokio::time::timeout(CASE_WAIT, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("The bounded parser state was not reached");
}

fn parser_queue_session_transition(replace_account: bool) {
    let chapters = catalog();
    let pages = (1..=3)
        .map(|attempt| {
            Reply::gzip(large_reader(0, &chapters).replace(
                "large-catalog-clock-0",
                &format!("fresh-reader-clock-{attempt}"),
            ))
            .body
        })
        .collect::<Vec<_>>();
    let readers = Arc::new(AtomicUsize::new(0));
    let observed_readers = readers.clone();
    let shards = Arc::new(AtomicUsize::new(0));
    let observed_shards = shards.clone();
    let fixture = bounded_fixture(move |request| {
        if request.path == reader_path(0) {
            let attempt = observed_readers.fetch_add(1, Ordering::SeqCst);
            return compressed_reply(&pages[attempt]);
        }
        observed_shards.fetch_add(1, Ordering::SeqCst);
        assert!(!replace_account, "An obsolete account dispatched a shard");
        assert_eq!(clock(request), "fresh-reader-clock-3");
        match request.path.as_str() {
            "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
            "/web/book/chapter/t_0" => Reply::text(TXT_SHARD),
            "/web/book/chapter/t_1" => Reply::status(404),
            _ => panic!("Unexpected synthetic operation"),
        }
    });
    let client = fixture_client(&fixture);
    let admission = Arc::new(ReaderAdmission::new());
    let holds = Arc::new(ParserHolds::default());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let cleanup = ReleaseParsers(holds.clone());
    runtime.block_on(async {
        let mut blockers = Vec::new();
        for _ in 0..2 {
            let admission = admission.clone();
            let holds = holds.clone();
            blockers.push(tokio::spawn(async move {
                admission
                    .acquire(ReaderTier::INITIAL)
                    .await
                    .unwrap()
                    .page("Synthetic parser hold".into())
                    .parse(move |_| {
                        holds.wait();
                        Ok(())
                    })
                    .await
            }));
        }
        wait_until(|| admission.snapshot().parsers_active == 2).await;
        let mut hints = ChapterReadHints::for_book(&client, BOOK);
        hints.set_download_reader_gate(admission.clone());
        let reading = client.clone();
        let chapter = tokio::spawn(async move {
            let result = reading
                .chapter_prefetch_with_hints(BOOK, &chapter_id(0), &mut hints)
                .await;
            (result, hints)
        });
        wait_until(|| admission.snapshot().parsers_waiting == 1).await;
        assert_eq!(readers.load(Ordering::SeqCst), 2);
        assert_eq!(shards.load(Ordering::SeqCst), 0);
        if replace_account {
            let mut session = fixture_auth();
            session.api_key = "synthetic-replacement-key".into();
            session.user_vid = "synthetic-replacement-vid".into();
            client.replace_session(session).unwrap();
        } else {
            let mut generation = client.renewal.lock().await;
            *generation = generation.wrapping_add(1);
        }
        holds.release();
        for blocker in blockers {
            tokio::time::timeout(CASE_WAIT, blocker)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        let (result, hints) = tokio::time::timeout(CASE_WAIT, chapter)
            .await
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(
            &admission,
            &hints.download_reader.as_ref().unwrap().admission,
        ));
        if replace_account {
            assert!(matches!(
                result,
                Err(SourceError::Unsupported("chapter_account_changed"))
            ));
            assert_eq!(
                hints.download_reader.as_ref().unwrap().tier(),
                ReaderTier::INITIAL
            );
            assert_eq!(readers.load(Ordering::SeqCst), 2);
            assert_eq!(shards.load(Ordering::SeqCst), 0);
        } else {
            let content = result.unwrap();
            assert_eq!(content.plain_text, "Hello, reader!");
            assert_eq!(content.content_status, ContentStatus::Full);
            assert_eq!(readers.load(Ordering::SeqCst), 3);
            assert_eq!(shards.load(Ordering::SeqCst), 3);
        }
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.available_bytes, READER_BUDGET_BYTES);
        assert_eq!(snapshot.reserved_bytes, 0);
        assert_eq!(snapshot.response_allowance_bytes, 0);
        assert_eq!(snapshot.pages_active, 0);
        assert_eq!(snapshot.parsers_active, 0);
        assert_eq!(snapshot.parsers_waiting, 0);
        assert_eq!(snapshot.peak_parsers_active, 2);
    });
    drop(cleanup);
    fixture.finish();
}

#[test]
fn queued_reader_rechecks_access_after_renewal_and_account_replacement() {
    parser_queue_session_transition(false);
    parser_queue_session_transition(true);
}

#[test]
fn task_and_book_scope_reset_only_the_learned_reader_tier() {
    let client = WeReadClient::new(fixture_auth()).unwrap();
    let admission = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(admission.clone());
    let old_context = hints.download_reader.as_ref().unwrap().clone();
    old_context.advance(ReaderTier::INITIAL).unwrap();
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier().bytes(),
        2 * 1024 * 1024
    );
    hints.bind(&client, "another-synthetic-book");
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier(),
        ReaderTier::INITIAL
    );
    assert_eq!(old_context.tier().bytes(), 2 * 1024 * 1024);
    assert!(Arc::ptr_eq(
        &admission,
        &hints.download_reader.as_ref().unwrap().admission
    ));
    let mut another_task = ChapterReadHints::for_book(&client, "another-synthetic-book");
    another_task.set_download_reader_gate(admission.clone());
    assert_eq!(
        another_task.download_reader.as_ref().unwrap().tier(),
        ReaderTier::INITIAL
    );
    assert!(Arc::ptr_eq(
        &old_context.admission,
        &another_task.download_reader.as_ref().unwrap().admission,
    ));
}
