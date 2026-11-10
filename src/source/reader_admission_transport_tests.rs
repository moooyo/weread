//! Adaptive response tiers through the actual chapter HTTP path.
//!
//! HTML comments pad these exact transport boundaries. The separate catalog
//! benchmark uses genuine catalog metadata to measure Reader parsing work.

use super::*;
use crate::source::reader_admission::{READER_BUDGET_BYTES, ReaderAdmission};

const TWO_MIB: usize = 2 * 1024 * 1024;
const FOUR_MIB: usize = 4 * 1024 * 1024;
const COMMON_READER_BYTES: usize = 1_415_578;

#[derive(Clone, Copy)]
enum ReaderEncoding {
    DeclaredLength,
    GzipDecoded,
}

impl ReaderEncoding {
    fn encode(self, html: String) -> Vec<u8> {
        match self {
            Self::DeclaredLength => html.into_bytes(),
            Self::GzipDecoded => {
                let bytes = Reply::gzip(html).body;
                assert!(bytes.len() < SMALL_RESPONSE_LIMIT);
                bytes
            }
        }
    }

    fn reply(self, bytes: &[u8]) -> Reply {
        match self {
            // bounded_fixture writes Content-Length from this complete body.
            Self::DeclaredLength => Reply::text(bytes),
            Self::GzipDecoded => compressed_reply(bytes),
        }
    }
}

fn transport_clock(index: usize) -> String {
    format!("transport-boundary-clock-{index}")
}

fn transport_reader(index: usize, bytes: usize) -> String {
    let chapter = json!({
        "chapterUid": chapter_id(index),
        "chapterIdx": index + 1,
        "title": format!("Synthetic transport chapter {index}"),
        "wordCount": 1,
        "paid": 1,
        "price": 0,
    });
    let state = json!({
        "user": { "vid": fixture_auth().user_vid },
        "reader": {
            "bookId": BOOK,
            "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
            "chapterInfos": [chapter.clone()],
            "currentChapter": chapter,
            "psvts": transport_clock(index),
            "pclts": 0,
            "token": format!("transport-boundary-token-{index}"),
        },
    });
    let mut html = format!("<script>window.__INITIAL_STATE__={state};</script><!--");
    assert!(html.len() + 3 <= bytes);
    html.extend(std::iter::repeat_n('x', bytes - html.len() - 3));
    html.push_str("-->");
    assert_eq!(html.len(), bytes);
    html
}

fn transport_shard_reply(request: &RecordedRequest) -> Reply {
    match request.path.as_str() {
        "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
        "/web/book/chapter/t_0" => Reply::text(TXT_SHARD),
        "/web/book/chapter/t_1" => Reply::status(404),
        _ => panic!("Unexpected synthetic transport request"),
    }
}

fn assert_released(admission: &ReaderAdmission) {
    let snapshot = admission.snapshot();
    assert_eq!(snapshot.available_bytes, READER_BUDGET_BYTES);
    assert_eq!(snapshot.reserved_bytes, 0);
    assert_eq!(snapshot.response_allowance_bytes, 0);
    assert_eq!(snapshot.pages_active, 0);
    assert_eq!(snapshot.parsers_active, 0);
}

#[test]
fn reader_transport_boundaries_expand_once_and_retain_the_learned_tier() {
    for encoding in [ReaderEncoding::DeclaredLength, ReaderEncoding::GzipDecoded] {
        let sizes = [
            SMALL_RESPONSE_LIMIT,
            SMALL_RESPONSE_LIMIT + 1,
            COMMON_READER_BYTES,
            TWO_MIB,
            TWO_MIB + 1,
            3 * 1024 * 1024,
            SMALL_RESPONSE_LIMIT - 1,
        ];
        let pages = sizes
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| {
                (
                    reader_path(index),
                    (index, encoding.encode(transport_reader(index, bytes))),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let admission = Arc::new(ReaderAdmission::new());
        let observed_admission = admission.clone();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let recorded = observations.clone();
        let fixture = bounded_fixture(move |request| {
            if let Some((index, bytes)) = pages.get(&request.path) {
                let snapshot = observed_admission.snapshot();
                assert_eq!(snapshot.pages_active, 1);
                assert_eq!(snapshot.reserved_bytes, snapshot.response_allowance_bytes);
                recorded
                    .lock()
                    .unwrap()
                    .push((*index, snapshot.response_allowance_bytes));
                return encoding.reply(bytes);
            }
            transport_shard_reply(request)
        });
        let client = fixture_client(&fixture);
        let mut hints = ChapterReadHints::for_book(&client, BOOK);
        hints.set_download_reader_gate(admission.clone());
        let expected_tiers = [
            SMALL_RESPONSE_LIMIT,
            TWO_MIB,
            TWO_MIB,
            TWO_MIB,
            FOUR_MIB,
            FOUR_MIB,
            FOUR_MIB,
        ];
        runtime().block_on(async {
            tokio::time::timeout(HTTP_TIMEOUT, async {
                for (index, tier_bytes) in expected_tiers.into_iter().enumerate() {
                    let content = client
                        .chapter_prefetch_with_hints(BOOK, &chapter_id(index), &mut hints)
                        .await
                        .unwrap();
                    assert_eq!(content.plain_text, "Hello, reader!");
                    assert_eq!(content.content_status, ContentStatus::Full);
                    assert_eq!(
                        hints.download_reader.as_ref().unwrap().tier().bytes(),
                        tier_bytes
                    );
                    assert_released(&admission);
                }
            })
            .await
            .unwrap();
        });
        assert_eq!(
            *observations.lock().unwrap(),
            vec![
                (0, SMALL_RESPONSE_LIMIT),
                (1, SMALL_RESPONSE_LIMIT),
                (1, TWO_MIB),
                (2, TWO_MIB),
                (3, TWO_MIB),
                (4, TWO_MIB),
                (4, FOUR_MIB),
                (5, FOUR_MIB),
                (6, FOUR_MIB),
            ],
            "Only a transport overflow may advance this task's Reader allowance",
        );
        assert_eq!(admission.snapshot().peak_response_allowance_bytes, FOUR_MIB);
        assert_eq!(admission.snapshot().peak_reserved_bytes, FOUR_MIB);
        let requests = fixture.finish();
        for (index, reader_reads) in [1, 2, 1, 1, 2, 1, 1].into_iter().enumerate() {
            assert_eq!(count(&requests, &reader_path(index)), reader_reads);
            for endpoint in ["/web/book/chapter/t_0", "/web/book/chapter/t_1"] {
                assert_eq!(
                    requests
                        .iter()
                        .filter(|request| {
                            request.path == endpoint && clock(request) == transport_clock(index)
                        })
                        .count(),
                    1,
                    "Each chapter must consume its own fresh Reader clock",
                );
            }
        }
        assert_eq!(count(&requests, "/web/book/chapter/e_0"), 1);
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}

#[test]
fn an_expanded_reader_does_not_expand_the_decoded_shard_limit() {
    let page = ReaderEncoding::GzipDecoded.encode(transport_reader(0, COMMON_READER_BYTES));
    let shard = Reply::gzip("x".repeat(SMALL_RESPONSE_LIMIT + 1)).body;
    assert!(shard.len() < SMALL_RESPONSE_LIMIT);
    let fixture = bounded_fixture(move |request| {
        if request.path == reader_path(0) {
            return compressed_reply(&page);
        }
        if request.path == "/web/book/chapter/t_0" {
            return compressed_reply(&shard);
        }
        transport_shard_reply(request)
    });
    let client = fixture_client(&fixture);
    let admission = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(admission.clone());
    let result = runtime().block_on(async {
        tokio::time::timeout(
            HTTP_TIMEOUT,
            client.chapter_prefetch_with_hints(BOOK, &chapter_id(0), &mut hints),
        )
        .await
        .unwrap()
    });
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier().bytes(),
        TWO_MIB
    );
    assert_eq!(admission.snapshot().peak_response_allowance_bytes, TWO_MIB);
    assert_released(&admission);
    let requests = fixture.finish();
    assert_eq!(count(&requests, &reader_path(0)), 2);
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_1"), 0);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

#[test]
fn an_expanded_reader_keeps_the_completed_content_limit_and_ordinary_read_contract() {
    let text = "\"".repeat(640 * 1024);
    let (first, second) = crate::source::content::fixture_text_shards(&text);
    assert!(first.len() < SMALL_RESPONSE_LIMIT);
    assert!(second.len() < SMALL_RESPONSE_LIMIT);
    let first = Reply::gzip(first).body;
    let second = Reply::gzip(second).body;
    let page = ReaderEncoding::GzipDecoded.encode(transport_reader(0, COMMON_READER_BYTES));
    let fixture = bounded_fixture(move |request| {
        if request.path == reader_path(0) {
            return compressed_reply(&page);
        }
        match request.path.as_str() {
            "/web/book/chapter/t_0" => compressed_reply(&first),
            "/web/book/chapter/t_1" => compressed_reply(&second),
            _ => transport_shard_reply(request),
        }
    });
    let client = fixture_client(&fixture);
    let admission = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(admission.clone());
    runtime().block_on(async {
        tokio::time::timeout(HTTP_TIMEOUT, async {
            let result = client
                .chapter_prefetch_with_hints(BOOK, &chapter_id(0), &mut hints)
                .await;
            assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
            assert_eq!(
                hints.download_reader.as_ref().unwrap().tier().bytes(),
                TWO_MIB
            );
            assert_eq!(admission.snapshot().peak_response_allowance_bytes, TWO_MIB);
            assert_released(&admission);
            let content = client
                .chapter_with_hints(BOOK, &chapter_id(0), &mut hints)
                .await
                .unwrap();
            assert_eq!(content.plain_text, text);
            assert_eq!(content.content_status, ContentStatus::Full);
            assert!(content.html.len() + content.plain_text.len() > FOUR_MIB);
            assert_eq!(
                hints.download_reader.as_ref().unwrap().tier().bytes(),
                TWO_MIB
            );
            assert_eq!(
                admission.snapshot().peak_response_allowance_bytes,
                24 * 1024 * 1024
            );
            assert_released(&admission);
        })
        .await
        .unwrap();
    });
    let requests = fixture.finish();
    assert_eq!(count(&requests, &reader_path(0)), 3);
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 2);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 2);
    assert_eq!(count(&requests, "/web/book/chapter/t_1"), 2);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}
