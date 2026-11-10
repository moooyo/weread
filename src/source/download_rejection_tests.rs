use super::*;

#[test]
fn txt_rejection_returns_once_without_fast_retries_renewal_or_format_reprobe() {
    for confirmed_text in [false, true] {
        let fixture = Fixture::serve(|request| {
            if request.path.starts_with("/web/reader/") {
                return Reply::text(reader("fixture-rejected-read-clock"));
            }
            match request.path.as_str() {
                "/web/book/chapter/t_0" => Reply::text(r#"{"errCode":-10102}"#),
                "/web/login/renewal" => Reply::text(r#"{"succ":1}"#),
                _ => txt_reply(request),
            }
        });
        let client = fixture.client();
        let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
        hints.confirmed_text = confirmed_text;
        let result = runtime().block_on(client.chapter_with_hints(
            "fixture-book",
            "fixture-chapter",
            &mut hints,
        ));
        assert!(
            matches!(result, Err(SourceError::Api(code)) if code == "-10102"),
            "The upstream rejection must remain intact for confirmed_text={confirmed_text}"
        );

        let requests = fixture.finish();
        let probes = usize::from(!confirmed_text);
        assert_eq!(requests.len(), 2 + probes);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1
        );
        assert_eq!(count(&requests, "/web/book/chapter/t_0"), 1);
        assert_eq!(count(&requests, "/web/book/chapter/t_1"), 0);
        assert_eq!(count(&requests, "/web/book/chapter/e_0"), probes);
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}
