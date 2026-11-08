//! Borrowed metadata admission and genuine loopback publication responses.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn limits() -> MpArticleLimits {
    MpArticleLimits {
        owner_bytes: 32,
        id_bytes: 32,
        records: 8,
        metadata_bytes: 8192,
    }
}

fn record(id: &str, title: &str) -> Value {
    json!({"reviewId": id, "mpInfo": {"title": title}})
}

fn permanent_resource_error(error: &SourceError) {
    assert!(matches!(
        error,
        SourceError::ResponseTooLarge | SourceError::InvalidResponse("mp_owner" | "mp_article_id")
    ));
    assert!(!error.requires_login());
    assert!(!error.retryable());
}

#[test]
fn mp_owner_and_identifier_limits_precede_owned_mapping() {
    let limits = MpArticleLimits {
        owner_bytes: 4,
        id_bytes: 4,
        ..limits()
    };
    let cases = [
        ("owner", record("a", "small")),
        ("", record("a", "small")),
        ("a\n", record("a", "small")),
        ("acct", record("abcde", "small")),
        ("acct", record("a\n", "small")),
    ];
    for (owner, value) in cases {
        let mut copied = 0;
        let error =
            normalize_mp_articles_with_observer(&value, owner, limits, || copied += 1).unwrap_err();
        permanent_resource_error(&error);
        assert_eq!(copied, 0);
    }
    let mapped =
        normalize_mp_articles(&record("\u{732b}r", "Unicode"), "\u{4e2d}a", limits).unwrap();
    assert_eq!((mapped[0].id.len(), mapped[0].account_id.len()), (4, 4));
}

#[test]
fn mp_candidate_count_covers_nested_empty_and_duplicate_records() {
    let value = json!({"reviews": {"pages": [
        record("same", "First"),
        record("same", "Discarded duplicate"),
        {"reviewId": "", "mpInfo": {}},
    ]}});
    let mut copied = 0;
    let error = normalize_mp_articles_with_observer(
        &value,
        "owner",
        MpArticleLimits {
            records: 2,
            ..limits()
        },
        || copied += 1,
    )
    .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(copied, 0);

    let mut copied = 0;
    let mapped = normalize_mp_articles_with_observer(
        &value,
        "owner",
        MpArticleLimits {
            records: 3,
            ..limits()
        },
        || copied += 1,
    )
    .unwrap();
    assert_eq!(copied, 1);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].title, "First");
}

#[test]
fn mp_metadata_forecast_exact_limit_precedes_all_output_copying() {
    let value = json!([
        {"reviewId": "firstid", "mpInfo": {
            "title": "\u{732b}", "author": "Writer",
            "originalUrl": "https://example.invalid/a",
            "pic_url": "https://example.invalid/i",
        }},
        {"reviewId": "second!", "mpInfo": {
            "title": "\u{732b}", "author": "Writer",
            "originalUrl": "https://example.invalid/a",
            "pic_url": "https://example.invalid/i",
        }},
    ]);
    // Each candidate has 71 selected text bytes including the owner, and a
    // seven-byte membership ID, in addition to two fixed record forecasts.
    let fee = 2 * (71 + std::mem::size_of::<MpArticle>()) + 7;
    let mut copied = 0;
    let mapped = normalize_mp_articles_with_observer(
        &value,
        "owner",
        MpArticleLimits {
            metadata_bytes: 2 * fee,
            ..limits()
        },
        || copied += 1,
    )
    .unwrap();
    assert_eq!((mapped.len(), copied), (2, 2));
    assert_eq!(mapped[0].title, "\u{732b}");

    let mut copied = 0;
    let error = normalize_mp_articles_with_observer(
        &value,
        "owner",
        MpArticleLimits {
            metadata_bytes: 2 * fee - 1,
            ..limits()
        },
        || copied += 1,
    )
    .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(copied, 0);
}

#[test]
fn mp_nested_aliases_keep_first_seen_records_and_depth_contract() {
    let value = json!({"reviews": [
        {"reviewId": "outer", "review": {"reviewId": "inner", "mpInfo": {
            "originalId": "original", "title": "First",
            "authorName": "Alias author", "original_url": "https://example.invalid/a",
            "picUrl": "https://example.invalid/i",
        }}},
        {"reviewId": 17, "mpInfo": {"title": 34, "author": "", "authorName": "Unused"}},
        {"mpInfo": {"originalId": "fallback", "title": "Fallback"}},
        record("outer", "Discarded duplicate"),
    ]});
    let mapped = normalize_mp_articles(&value, "opaque owner: \u{4e2d}", limits()).unwrap();
    assert_eq!(
        mapped
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["outer", "17", "fallback"]
    );
    assert_eq!(mapped[0].title, "First");
    assert_eq!(mapped[0].author, "Alias author");
    assert_eq!(mapped[0].original_url, "https://example.invalid/a");
    assert_eq!(mapped[0].cover_url, "https://example.invalid/i");
    assert_eq!(mapped[1].title, "34");
    assert!(mapped[1].author.is_empty());

    let mut deep = record("deep", "Depth ten");
    for _ in 0..10 {
        deep = json!({"nested": deep});
    }
    assert_eq!(
        normalize_mp_articles(&deep, "owner", limits())
            .unwrap()
            .len(),
        1
    );
    deep = json!({"nested": deep});
    assert!(
        normalize_mp_articles(&deep, "owner", limits())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mp_unselected_aliases_are_borrowed_and_empty_ids_keep_their_skip_behavior() {
    let value = json!([
        {"reviewId": "a", "mpInfo": {
            "originalId": "x".repeat(64), "title": "x",
            "originalUrl": "short", "content_url": "x".repeat(64),
        }},
        {"reviewId": "", "mpInfo": {"title": "x".repeat(64)}},
    ]);
    let fee = 2 * (11 + std::mem::size_of::<MpArticle>()) + 1;
    let mut copied = 0;
    let mapped = normalize_mp_articles_with_observer(
        &value,
        "acct",
        MpArticleLimits {
            id_bytes: 4,
            records: 2,
            metadata_bytes: fee,
            ..limits()
        },
        || copied += 1,
    )
    .unwrap();
    assert_eq!((mapped.len(), copied), (1, 1));
    assert_eq!(mapped[0].original_url, "short");
}

async fn fixture(value: Value) -> (WeReadClient, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let body = value.to_string();
    let response = format!(
        "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(3), async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                assert!(request.len() + read <= 32 * 1024);
                request.extend_from_slice(&buffer[..read]);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        })
        .await
        .expect("The fixture exchange has a finite deadline")
    });
    let mut client = WeReadClient::new(AuthSession {
        api_key: "synthetic-publication-key".into(),
        cookies: BTreeMap::from([("wr_gid".into(), "synthetic-publication-gid".into())]),
        user_vid: "synthetic-publication-owner".into(),
        ..AuthSession::default()
    })
    .unwrap();
    // A hard-coded-origin regression still cannot send synthetic account
    // material to a live service. Both production hosts resolve to loopback.
    client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
        .no_proxy()
        .resolve("weread.qq.com", address)
        .resolve("i.weread.qq.com", address)
        .build()
        .unwrap();
    client.test_web_origin = Some(origin.clone());
    client.test_gateway_url = Some(format!("{origin}/gateway"));
    (client, server)
}

fn query(record: &str) -> BTreeMap<String, String> {
    let target = record.split_whitespace().nth(1).unwrap();
    reqwest::Url::parse(&format!("http://fixture.invalid{target}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn mp_owner_rejection_precedes_transport_and_preserves_authentication() {
    let mut client = WeReadClient::new(AuthSession::default()).unwrap();
    client.test_web_origin = Some("invalid test origin".into());
    let before = client.session();
    let error = client
        .mp_articles_with_limits(
            "owner",
            0,
            100,
            MpArticleLimits {
                owner_bytes: 4,
                ..limits()
            },
        )
        .await
        .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(client.session(), before);
}

#[tokio::test(flavor = "current_thread")]
async fn mp_loopback_response_checks_decoded_owner_and_count_before_mapping() {
    let owner = "\u{4e2d}/? &+";
    let (client, server) = fixture(json!({"reviews": [record("a", "A"), record("b", "B")]})).await;
    let mapped = client.mp_articles(owner, 42, u32::MAX).await.unwrap();
    assert_eq!(mapped.len(), 2);
    assert!(mapped.iter().all(|item| item.account_id == owner));
    let request_record = server.await.unwrap();
    assert!(request_record.starts_with("GET /web/mp/articles?"));
    let params = query(&request_record);
    assert_eq!(params.get("bookId").map(String::as_str), Some(owner));
    assert_eq!(params.get("maxIdx").map(String::as_str), Some("42"));
    assert_eq!(params.get("count").map(String::as_str), Some("100"));

    let value = json!({"reviews": [record("a", "A"), record("b", "B"), record("c", "C")]});
    let (client, server) = fixture(value).await;
    let before = client.session();
    let error = client
        .mp_articles_with_limits(
            owner,
            42,
            u32::MAX,
            MpArticleLimits {
                records: 2,
                ..limits()
            },
        )
        .await
        .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(client.session(), before);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn mp_loopback_exact_metadata_allowance_and_one_byte_overshoot() {
    let value = record("A", "x");
    let fee = 2 * (7 + std::mem::size_of::<MpArticle>()) + 1;
    let (client, server) = fixture(value.clone()).await;
    let mapped = client
        .mp_articles_with_limits(
            "owner",
            0,
            1,
            MpArticleLimits {
                metadata_bytes: fee,
                ..limits()
            },
        )
        .await
        .unwrap();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].account_id, "owner");
    server.await.unwrap();

    let (client, server) = fixture(value).await;
    let error = client
        .mp_articles_with_limits(
            "owner",
            0,
            1,
            MpArticleLimits {
                metadata_bytes: fee - 1,
                ..limits()
            },
        )
        .await
        .unwrap_err();
    permanent_resource_error(&error);
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn mp_shelf_owner_admission_precedes_normalized_account_copy() {
    let prefix = "MP_WXS_";
    let owner = format!(
        "{prefix}{}",
        "x".repeat(MAX_MP_ACCOUNT_ID_BYTES - prefix.len())
    );
    let (client, server) = fixture(json!({"books": [{
        "bookInfo": {"book_id": owner.clone(), "title": "Publication"},
    }]}))
    .await;
    let mapped = client.shelf_snapshot().await.unwrap();
    assert_eq!(mapped.mp_accounts.len(), 1);
    assert_eq!(mapped.mp_accounts[0].id.len(), MAX_MP_ACCOUNT_ID_BYTES);
    assert!(mapped.books.is_empty());
    server.await.unwrap();

    let oversized = format!("{owner}x");
    let (client, server) = fixture(json!({"books": [{
        "bookId": oversized, "title": "Publication",
    }]}))
    .await;
    let before = client.session();
    let error = client.shelf_snapshot().await.unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(client.session(), before);
    server.await.unwrap();
}
