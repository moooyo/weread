//! Borrowed Shelf MP admission and one normalization per unique owner.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn limits() -> MpAccountLimits {
    MpAccountLimits {
        owner_bytes: 64,
        records: 8,
        metadata_bytes: 4096,
    }
}

fn record(owner: &str, title: &str, cover: &str) -> Value {
    json!({
        "bookInfo": {"bookId": owner, "title": title, "cover": cover},
        "updateTime": 11,
        "unreadCount": 3,
    })
}

fn permanent_resource_error(error: &SourceError) {
    assert!(matches!(
        error,
        SourceError::ResponseTooLarge | SourceError::InvalidResponse("mp_owner")
    ));
    assert!(!error.requires_login());
    assert!(!error.retryable());
}

#[test]
fn shelf_mp_duplicates_normalize_only_the_first_record_per_owner() {
    let mut first = record("MP_WXS_B", "First B", "https://example.invalid/t3_b.png");
    first["bookInfo"]["lastChapterCreateTime"] = json!(42);
    let mut discarded = record(
        "MP_WXS_B",
        "Later B",
        "https://example.invalid/t9_later.png",
    );
    discarded["updateTime"] = json!(900);
    discarded["unreadCount"] = json!(99);
    let records = vec![
        first,
        record("MP_WXS_A", "First A", ""),
        discarded,
        record("MP_WXS_C", "First C", ""),
        record("MP_WXS_A", "Later A", ""),
    ];
    let mut copied = Vec::new();
    let accounts = normalize_shelf_mp_accounts_with_observer(&records, limits(), |owner| {
        copied.push(owner.to_owned());
    })
    .unwrap();
    assert_eq!(copied, ["MP_WXS_B", "MP_WXS_A", "MP_WXS_C"]);
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[0].name, "First B");
    assert_eq!(accounts[0].cover_url, "https://example.invalid/t9_b.png");
    assert_eq!(accounts[0].updated_at, 42);
    assert_eq!(accounts[0].unread_count, 3);
    assert_eq!(accounts[1].name, "First A");
    assert_eq!(accounts[2].name, "First C");
}

#[test]
fn shelf_mp_duplicate_raw_records_spend_count_before_normalization() {
    let records = vec![record("MP_WXS_same", "First", ""); 3];
    let mut copied = 0;
    let error = normalize_shelf_mp_accounts_with_observer(
        &records,
        MpAccountLimits {
            records: 2,
            ..limits()
        },
        |_| copied += 1,
    )
    .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(copied, 0);
    let accounts = normalize_shelf_mp_accounts(
        &records,
        MpAccountLimits {
            records: 3,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "MP_WXS_same");
}

#[test]
fn shelf_mp_discarded_duplicate_fields_spend_complete_metadata_admission() {
    let owner = "MP_WXS_same";
    let first_title = "First";
    let later_title = "Discarded title with more metadata";
    let later_cover = "https://example.invalid/t123456_discarded.png";
    let first_fee =
        2 * (owner.len() + first_title.len() + std::mem::size_of::<MpAccount>()) + owner.len();
    let later_fee = 2
        * (owner.len() + later_title.len() + later_cover.len() + std::mem::size_of::<MpAccount>())
        + owner.len();
    let records = vec![
        record(owner, first_title, ""),
        record(owner, later_title, later_cover),
    ];
    let mut copied = 0;
    let error = normalize_shelf_mp_accounts_with_observer(
        &records,
        MpAccountLimits {
            metadata_bytes: first_fee + later_fee - 1,
            ..limits()
        },
        |_| copied += 1,
    )
    .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(copied, 0);
    let accounts = normalize_shelf_mp_accounts_with_observer(
        &records,
        MpAccountLimits {
            metadata_bytes: first_fee + later_fee,
            ..limits()
        },
        |_| copied += 1,
    )
    .unwrap();
    assert_eq!(copied, 1);
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].name, first_title);
    assert_eq!(accounts[0].cover_url, "");
}

#[test]
fn shelf_mp_owner_admission_is_atomic_and_preserves_opaque_identifiers() {
    let owner = "MP_WXS_\u{4e2d}/? &+";
    let records = vec![record(owner, "Opaque", ""), record("MP_WXS_", "Prefix", "")];
    let accounts = normalize_shelf_mp_accounts(
        &records,
        MpAccountLimits {
            owner_bytes: owner.len(),
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts[0].id, owner);
    assert_eq!(accounts[1].id, "MP_WXS_");
    for invalid in [format!("{owner}x"), "MP_WXS_bad\n".into()] {
        let rejected = vec![
            record("MP_WXS_ok", "First", ""),
            record(&invalid, "Bad", ""),
        ];
        let mut copied = 0;
        let error = normalize_shelf_mp_accounts_with_observer(
            &rejected,
            MpAccountLimits {
                owner_bytes: owner.len(),
                ..limits()
            },
            |_| copied += 1,
        )
        .unwrap_err();
        permanent_resource_error(&error);
        assert_eq!(copied, 0);
    }
}

#[test]
fn shelf_mp_aliases_keep_selected_fields_and_ignore_unused_raw_metadata() {
    let cover = "https://example.invalid/t2_numeric.png";
    let records = vec![json!({
        "bookInfo": {
            "bookId": false,
            "book_id": "MP_WXS_alias",
            "title": 123,
            "cover": false,
            "coverUrl": cover,
            "intro": "unused".repeat(512),
            "description": "unused".repeat(512),
            "updateTime": "23",
        },
        "book": {"bookId": "MP_WXS_hidden", "title": "Unused node"},
        "readUpdateTime": "29",
        "unread": "7",
    })];
    let fee = 2 * ("MP_WXS_alias".len() + 3 + cover.len() + std::mem::size_of::<MpAccount>())
        + "MP_WXS_alias".len();
    let accounts = normalize_shelf_mp_accounts(
        &records,
        MpAccountLimits {
            metadata_bytes: fee,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "MP_WXS_alias");
    assert_eq!(accounts[0].name, "123");
    assert_eq!(
        accounts[0].cover_url,
        "https://example.invalid/t9_numeric.png"
    );
    assert_eq!(accounts[0].updated_at, 29);
    assert_eq!(accounts[0].unread_count, 7);

    let empty_selected_cover = vec![json!({
        "book": {"bookId": "MP_WXS_empty", "title": "", "cover": "", "coverUrl": "unused".repeat(512)}
    })];
    let fee = 2 * ("MP_WXS_empty".len() + std::mem::size_of::<MpAccount>()) + "MP_WXS_empty".len();
    let accounts = normalize_shelf_mp_accounts(
        &empty_selected_cover,
        MpAccountLimits {
            metadata_bytes: fee,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts[0].name, "");
    assert_eq!(accounts[0].cover_url, "");
}

#[test]
fn shelf_mp_empty_numeric_and_other_prefix_ids_keep_original_classification() {
    let records = vec![
        json!({"bookId": "", "book_id": "MP_WXS_secondary", "title": "Empty first ID"}),
        json!({"bookId": 9, "book_id": "MP_WXS_secondary", "title": "Numeric first ID"}),
        json!({"bookId": "MP_OTHER", "title": "Another prefix"}),
        json!({"bookInfo": null, "book": {"bookId": "MP_WXS_hidden"}}),
        json!({"title": "Missing ID"}),
        record("MP_WXS_", "Existing prefix classification", ""),
    ];
    let accounts = normalize_shelf_mp_accounts(
        &records,
        MpAccountLimits {
            records: 1,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "MP_WXS_");
}

#[test]
fn shelf_mp_metadata_exact_limit_prices_utf8_and_original_cover_bytes() {
    let owner = "MP_WXS_\u{4e2d}";
    let title = "\u{732b}\u{72d7}";
    let cover = "https://example.invalid/t123456_\u{4e2d}.png";
    let records = vec![record(owner, title, cover)];
    let fee = 2 * (owner.len() + title.len() + cover.len() + std::mem::size_of::<MpAccount>())
        + owner.len();
    let accounts = normalize_shelf_mp_accounts(
        &records,
        MpAccountLimits {
            metadata_bytes: fee,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(accounts[0].name, title);
    assert_eq!(
        accounts[0].cover_url,
        "https://example.invalid/t9_\u{4e2d}.png"
    );
    let mut copied = 0;
    let error = normalize_shelf_mp_accounts_with_observer(
        &records,
        MpAccountLimits {
            metadata_bytes: fee - 1,
            ..limits()
        },
        |_| copied += 1,
    )
    .unwrap_err();
    permanent_resource_error(&error);
    assert_eq!(copied, 0);
}

async fn fixture(value: Value) -> (WeReadClient, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
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
            let header_end = loop {
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break end + 4;
                }
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                assert!(request.len() + read <= 32 * 1024);
                request.extend_from_slice(&buffer[..read]);
            };
            let content_length = std::str::from_utf8(&request[..header_end])
                .unwrap()
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find_map(|(name, value)| {
                    name.eq_ignore_ascii_case("Content-Length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(header_end + content_length <= 32 * 1024);
            while request.len() < header_end + content_length {
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
        api_key: "synthetic-mp-account-key".into(),
        cookies: BTreeMap::from([
            ("wr_gid".into(), "synthetic-mp-account-gid".into()),
            ("wr_vid".into(), "synthetic-mp-account-owner".into()),
        ]),
        user_vid: "synthetic-mp-account-owner".into(),
        ..AuthSession::default()
    })
    .unwrap();
    // A hard-coded-origin regression still resolves both production hosts to
    // this finite loopback exchange; synthetic credentials cannot reach them.
    client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
        .no_proxy()
        .resolve("weread.qq.com", address)
        .resolve("i.weread.qq.com", address)
        .build()
        .unwrap();
    client.test_gateway_url = Some(format!("http://{address}/gateway"));
    (client, server)
}

fn assert_shelf_gateway_request(request: &str) {
    assert!(request.starts_with("POST /gateway "));
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    let headers = headers.to_ascii_lowercase();
    assert!(headers.contains("authorization: bearer synthetic-mp-account-key"));
    assert!(!headers.contains("cookie:"));
    let payload: Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["api_name"], json!("/shelf/sync"));
}

#[tokio::test(flavor = "current_thread")]
async fn shelf_mp_loopback_keeps_first_owner_and_ordinary_book_records() {
    let records = vec![
        record("MP_WXS_B", "First B", "https://example.invalid/t2_b.png"),
        json!({"bookInfo": {"bookId": "ordinary", "title": "First ordinary"}}),
        record("MP_WXS_B", "Later B", ""),
        json!({"book": {"book_id": "MP_WXS_A", "title": "First A"}, "unread": 8}),
        json!({"bookId": "ordinary", "title": "Later ordinary"}),
        json!({"bookId": "", "title": "Empty"}),
        json!({"bookId": "MP_OTHER", "title": "Ordinary prefix"}),
    ];
    let (client, server) = fixture(json!({"books": records})).await;
    let before = client.session();
    let snapshot = client.shelf_snapshot().await.unwrap();
    assert_eq!(snapshot.mp_accounts.len(), 2);
    assert_eq!(snapshot.mp_accounts[0].id, "MP_WXS_B");
    assert_eq!(snapshot.mp_accounts[0].name, "First B");
    assert_eq!(
        snapshot.mp_accounts[0].cover_url,
        "https://example.invalid/t9_b.png"
    );
    assert_eq!(snapshot.mp_accounts[1].id, "MP_WXS_A");
    assert_eq!(snapshot.mp_accounts[1].unread_count, 8);
    assert_eq!(
        snapshot
            .books
            .iter()
            .map(|book| (book.id.as_str(), book.title.as_str()))
            .collect::<Vec<_>>(),
        [
            ("ordinary", "First ordinary"),
            ("ordinary", "Later ordinary"),
            ("MP_OTHER", "Ordinary prefix")
        ]
    );
    assert_eq!(client.session(), before);
    assert_shelf_gateway_request(&server.await.unwrap());
}

#[tokio::test(flavor = "current_thread")]
async fn shelf_mp_loopback_owner_and_duplicate_count_rejections_preserve_auth() {
    for (records, policy) in [
        (
            vec![
                record("MP_WXS_same", "First", ""),
                record("MP_WXS_same", "Later", ""),
            ],
            MpAccountLimits {
                records: 1,
                ..limits()
            },
        ),
        (
            vec![
                record("MP_WXS_ok", "First", ""),
                record("MP_WXS_owner_long", "Later", ""),
            ],
            MpAccountLimits {
                owner_bytes: "MP_WXS_ok".len(),
                ..limits()
            },
        ),
    ] {
        let (client, server) = fixture(json!({"books": records})).await;
        let before = client.session();
        let error = client
            .shelf_snapshot_with_mp_limits(policy)
            .await
            .unwrap_err();
        permanent_resource_error(&error);
        assert_eq!(client.session(), before);
        assert!(client.capabilities().mp_articles);
        assert!(client.capabilities().chapter_content);
        assert_shelf_gateway_request(&server.await.unwrap());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shelf_mp_loopback_exact_metadata_admission_and_one_byte_overshoot() {
    let owner = "MP_WXS_\u{4e2d}";
    let title = "Publication";
    let cover = "https://example.invalid/t21_cover.png";
    let fee = 2 * (owner.len() + title.len() + cover.len() + std::mem::size_of::<MpAccount>())
        + owner.len();
    for metadata_bytes in [fee, fee - 1] {
        let (client, server) = fixture(json!({"books": [record(owner, title, cover)]})).await;
        let before = client.session();
        let result = client
            .shelf_snapshot_with_mp_limits(MpAccountLimits {
                metadata_bytes,
                ..limits()
            })
            .await;
        if metadata_bytes == fee {
            let accounts = result.unwrap().mp_accounts;
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].id, owner);
            assert_eq!(accounts[0].name, title);
            assert_eq!(
                accounts[0].cover_url,
                "https://example.invalid/t9_cover.png"
            );
        } else {
            permanent_resource_error(&result.unwrap_err());
        }
        assert_eq!(client.session(), before);
        assert_shelf_gateway_request(&server.await.unwrap());
    }
}
