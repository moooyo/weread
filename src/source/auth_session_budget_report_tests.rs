//! Full time-report preflight and elapsed POSTs use only loopback source endpoints.

use super::*;
use crate::source::login::auth_budget::reset_work;

fn near_limit() -> AuthSession {
    let mut session = auth();
    let bytes = serde_json::to_vec(&session).unwrap().len();
    session
        .account_name
        .push_str(&"x".repeat(AuthSession::MAX_ENCODED_BYTES - bytes));
    assert_eq!(
        serde_json::to_vec(&session).unwrap().len(),
        AuthSession::MAX_ENCODED_BYTES
    );
    session.validate_resource_budget().unwrap();
    session
}

fn ticket(mut reply: Reply) -> Reply {
    reply.headers.push(("x-wr-ticket".into(), "x".into()));
    reply
}

#[tokio::test(flavor = "current_thread")]
async fn whole_account_rejection_after_elapsed_post_preserves_acknowledgement_or_uncertainty() {
    for acknowledged in [true, false] {
        let fixture = HttpFixture::new(move |request| match request.path() {
            "/gateway" => Reply::json(serde_json::json!({
                "bookId":"whole-budget-book","progress":1,"chapterUid":"1",
                "chapterIdx":0,"chapterOffset":1
            })),
            "/web/book/chapterInfos" => Reply::json(serde_json::json!({"data":[{
                "bookId":"whole-budget-book","updated":[{
                    "chapterUid":"1","chapterIdx":0,"title":"One","wordCount":100
                }]
            }]})),
            path if path.starts_with("/web/reader/") => Reply::text(
                r#"<script>window.__INITIAL_STATE__={"reader":{"psvts":"fixture-clock","pclts":"fixture-client-clock"}};</script>"#,
            ),
            "/web/book/read" => {
                if request.json().get("rt").is_some() {
                    ticket(if acknowledged {
                        Reply::json(serde_json::json!({"succ":1,"synckey":9}))
                    } else {
                        Reply::text("not-json")
                    })
                } else {
                    Reply::json(serde_json::json!({"succ":1}))
                }
            }
            path => panic!("Unexpected loopback whole-account report request: {path}"),
        });
        let original = near_limit();
        let client = fixture.client(original.clone());
        let now = crate::source::protocol::now_seconds();
        let request = crate::source::report::ReadReportRequest {
            session_id: "whole-account-report".into(),
            account_key: crate::source::report::account_key(&original).unwrap(),
            book_id: "whole-budget-book".into(),
            started_at: now - 120,
            seconds: 60,
            progress_write_allowed: true,
            active_reader_intent: true,
            position: Some(crate::source::ReadingPosition {
                chapter_uid: "1".into(),
                chapter_index: 0,
                chapter_offset: 1,
                progress: 1,
                updated_at: now,
                ..Default::default()
            }),
        };
        reset_work();
        let result = client.report_time(&request).await;
        if acknowledged {
            let receipt = result.unwrap();
            assert_eq!(receipt.seconds, 60);
            assert!(receipt.has_synckey);
        } else {
            assert!(matches!(result, Err(SourceError::DeliveryUncertain)));
        }
        assert_eq!(client.session(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path() == "/web/book/read")
                .count(),
            2
        );
    }
}
