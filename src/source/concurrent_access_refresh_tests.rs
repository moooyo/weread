//! Access proofs must survive a different chapter renewing the shared session.

use super::*;
use crate::domain::ContentStatus;

const BOOK: &str = "fixture-book";
const FIRST: &str = "17";
const SECOND: &str = "18";

fn reader_path(chapter: &str) -> String {
    reqwest::Url::parse(&crate::source::protocol::reader_url_checked(BOOK, Some(chapter)).unwrap())
        .unwrap()
        .path()
        .to_owned()
}

fn granted_reader(chapter: &str, clock: &str) -> String {
    let chapter = json!({
        "chapterUid": chapter,
        "chapterIdx": 0,
        "wordCount": 1,
        "paid": 1,
        "price": 0,
    });
    let state = json!({
        "user": { "vid": "fixture-account" },
        "reader": {
            "bookId": BOOK,
            "psvts": clock,
            "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
            "chapterInfos": [chapter.clone()],
            "currentChapter": chapter,
        },
    });
    format!("<script>window.__INITIAL_STATE__={state};</script>")
}

fn request_chapter(request: &RecordedRequest) -> String {
    serde_json::from_slice::<Value>(&request.body).unwrap()["c"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_sibling_renewal_refreshes_the_inflight_chapter_access_proof() {
    for prefetch in [false, true] {
        let first_path = reader_path(FIRST);
        let second_path = reader_path(SECOND);
        let first_uid = crate::source::protocol::encoded_id(FIRST);
        let first_arrived = Arc::new(AtomicBool::new(false));
        let arrival = first_arrived.clone();
        let release_first = Arc::new((Mutex::new(false), Condvar::new()));
        let release = release_first.clone();
        let fixture_first_path = first_path.clone();
        let fixture_second_path = second_path.clone();
        let fixture_first_uid = first_uid.clone();
        let fixture = Fixture::serve(move |request| {
            let renewed = request.headers["cookie"].contains("fixture-renewed-key");
            if request.path == "/web/login/renewal" {
                assert!(!renewed);
                return Reply::gzip(r#"{"succ":1}"#)
                    .cookie("wr_skey", "fixture-renewed-key")
                    .cookie("wr_rt", "fixture-renewed-refresh");
            }
            if request.path == fixture_second_path {
                if !renewed {
                    return Reply::gzip(r#"{"errCode":-2012}"#);
                }
                return Reply::gzip(granted_reader(SECOND, "fixture-second-fresh"));
            }
            if request.path == fixture_first_path {
                return Reply::gzip(granted_reader(
                    FIRST,
                    if renewed {
                        "fixture-first-fresh"
                    } else {
                        "fixture-first-original"
                    },
                ));
            }
            let first = request_chapter(request) == fixture_first_uid;
            if first && !renewed && request.path == "/web/book/chapter/t_1" {
                arrival.store(true, Ordering::SeqCst);
                let (lock, ready) = &*release;
                let mut released = lock.lock().unwrap();
                while !*released {
                    let (next, timeout) = ready
                        .wait_timeout(released, Duration::from_secs(3))
                        .unwrap();
                    assert!(
                        !timeout.timed_out(),
                        "The sibling renewal must complete before releasing the stale chapter"
                    );
                    released = next;
                }
            }
            match request.path.as_str() {
                "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
                "/web/book/chapter/t_0" => Reply::text(TXT_SHARD),
                "/web/book/chapter/t_1" => Reply::status(404),
                _ => panic!("Unexpected synthetic operation"),
            }
        });
        let client = fixture.client();
        runtime().block_on(async {
            let mut first_hints = ChapterReadHints::default();
            let mut first = Box::pin(async {
                if prefetch {
                    client
                        .chapter_prefetch_with_hints(BOOK, FIRST, &mut first_hints)
                        .await
                } else {
                    client
                        .chapter_with_hints(BOOK, FIRST, &mut first_hints)
                        .await
                }
            });
            tokio::select! {
                result = &mut first => panic!("The first chapter escaped its response gate: {result:?}"),
                result = tokio::time::timeout(Duration::from_secs(2), async {
                    while !first_arrived.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                }) => result.unwrap(),
            }
            let second = client.chapter(BOOK, SECOND).await;
            let (lock, ready) = &*release_first;
            *lock.lock().unwrap() = true;
            ready.notify_all();
            assert_eq!(second.unwrap().content_status, ContentStatus::Full);
            let content = first.await.unwrap();
            assert_eq!(content.plain_text, "Hello, reader!");
            assert_eq!(content.content_status, ContentStatus::Full);
        });
        let requests = fixture.finish();
        assert_eq!(count(&requests, "/web/login/renewal"), 1);
        assert_eq!(count(&requests, &first_path), 2);
        // A renewal inside Reader acquisition invalidates that attempt's
        // initial generation; the existing bounded access refresh reads again.
        assert_eq!(count(&requests, &second_path), 3);
        let first_bodies = requests
            .iter()
            .filter(|request| {
                request.path == "/web/book/chapter/t_0" && request_chapter(request) == first_uid
            })
            .collect::<Vec<_>>();
        assert_eq!(first_bodies.len(), 2);
        assert_eq!(clock(first_bodies[0]), "fixture-first-original");
        assert_eq!(clock(first_bodies[1]), "fixture-first-fresh");
    }
}

#[test]
fn stable_unknown_access_is_not_promoted_or_retried() {
    for prefetch in [false, true] {
        let fixture = Fixture::serve(txt_reply);
        let client = fixture.client();
        let mut hints = ChapterReadHints::default();
        let content = runtime().block_on(async {
            if prefetch {
                client
                    .chapter_prefetch_with_hints(BOOK, FIRST, &mut hints)
                    .await
            } else {
                client.chapter_with_hints(BOOK, FIRST, &mut hints).await
            }
        });
        let content = content.unwrap();
        assert_eq!(content.plain_text, "Hello, reader!");
        assert_eq!(content.content_status, ContentStatus::Unknown);
        let requests = fixture.finish();
        assert_eq!(count(&requests, &reader_path(FIRST)), 1);
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}
