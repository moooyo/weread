//! Bounded speculative chapter reads retain the ordinary source contracts.

use super::*;

const PREFETCH_RESPONSE_BYTES: usize = 512 * 1024;
const PREFETCH_CONTENT_BYTES: usize = 4 * 1024 * 1024;

#[test]
fn cloned_task_hints_share_rotation_fallback_before_ordered_merge() {
    let fixture = Fixture::serve(|request| {
        if request.path.starts_with("/web/reader/") {
            return Reply::gzip(reader("fixture-rotation-clock"))
                .cookie("wr_skey", "fixture-rotated-key");
        }
        epub_reply(request, "fixture-rotation-clock")
    });
    let client = fixture.client();
    let owner = ChapterReadHints::for_book(&client, "fixture-book");
    let mut first = owner.clone();
    let sibling = owner.clone();
    runtime()
        .block_on(client.chapter_prefetch_with_hints("fixture-book", "1", &mut first))
        .unwrap();
    assert!(first.serial_epub.load(Ordering::SeqCst));
    assert!(sibling.serial_epub.load(Ordering::SeqCst));
    assert!(owner.serial_epub.load(Ordering::SeqCst));

    let resumed = ChapterReadHints::for_book(&client, "fixture-book");
    assert!(!resumed.serial_epub.load(Ordering::SeqCst));
    assert!(!Arc::ptr_eq(&owner.serial_epub, &resumed.serial_epub));
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn merging_task_hints_preserves_scope_and_accepts_later_format_invalidation() {
    runtime().block_on(async {
        let client = WeReadClient::new(fixture_auth()).unwrap();
        let mut owner = ChapterReadHints::for_book(&client, "fixture-book");
        let mut completed = owner.clone();
        completed.confirmed_text = true;
        owner.merge(completed);
        assert!(owner.confirmed_text);

        let mut reprobed = owner.clone();
        reprobed.confirmed_text = false;
        owner.merge(reprobed);
        assert!(!owner.confirmed_text);

        let other_client = WeReadClient::new(fixture_auth()).unwrap();
        for mut foreign in [
            ChapterReadHints::for_book(&client, "fixture-book"),
            ChapterReadHints::for_book(&client, "fixture-other-book"),
            ChapterReadHints::for_book(&other_client, "fixture-book"),
            ChapterReadHints::default(),
        ] {
            foreign.confirmed_text = true;
            foreign.serial_epub.store(true, Ordering::SeqCst);
            owner.merge(foreign);
            assert!(!owner.confirmed_text);
            assert!(!owner.serial_epub.load(Ordering::SeqCst));
        }

        let mut old_account = owner.clone();
        old_account.confirmed_text = true;
        let mut replacement = fixture_auth();
        replacement.user_vid = "fixture-replacement-account".into();
        replacement
            .cookies
            .insert("wr_vid".into(), "fixture-replacement-account".into());
        client.replace_session(replacement).unwrap();
        owner.bind(&client, "fixture-book");
        owner.merge(old_account);
        assert!(!owner.confirmed_text);
        assert!(!owner.serial_epub.load(Ordering::SeqCst));
    });
}

fn padded_reader(bytes: usize) -> String {
    let mut html = reader("fixture-txt-clock");
    assert!(html.len() <= bytes);
    html.extend(std::iter::repeat_n(' ', bytes - html.len()));
    html
}

#[test]
fn prefetch_accepts_the_exact_decompressed_reader_limit() {
    let fixture = Fixture::serve(|request| {
        if request.path.starts_with("/web/reader/") {
            return Reply::gzip(padded_reader(PREFETCH_RESPONSE_BYTES));
        }
        txt_reply(request)
    });
    let client = fixture.client();
    let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
    let content = runtime()
        .block_on(client.chapter_prefetch_with_hints("fixture-book", "1", &mut hints))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

#[test]
fn oversized_compressed_reader_is_not_retried_and_normal_read_keeps_its_limit() {
    for prefetch in [true, false] {
        let fixture = Fixture::serve(|request| {
            if request.path.starts_with("/web/reader/") {
                return Reply::gzip(padded_reader(PREFETCH_RESPONSE_BYTES + 1));
            }
            txt_reply(request)
        });
        let client = fixture.client();
        let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
        let content = runtime().block_on(async {
            if prefetch {
                client
                    .chapter_prefetch_with_hints("fixture-book", "1", &mut hints)
                    .await
            } else {
                client
                    .chapter_with_hints("fixture-book", "1", &mut hints)
                    .await
            }
        });
        if prefetch {
            assert!(matches!(content, Err(SourceError::ResponseTooLarge)));
        } else {
            assert_eq!(content.unwrap().plain_text, "Hello, reader!");
        }
        let requests = fixture.finish();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1
        );
        assert_eq!(requests.len(), if prefetch { 1 } else { 4 });
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}

#[test]
fn prefetch_limits_every_text_and_parallel_epub_shard_without_retries() {
    for endpoint in [
        "/web/book/chapter/e_0",
        "/web/book/chapter/t_0",
        "/web/book/chapter/e_1",
        "/web/book/chapter/e_3",
    ] {
        let fixture = Fixture::serve(move |request| {
            if request.path == endpoint {
                return Reply::gzip("x".repeat(PREFETCH_RESPONSE_BYTES + 1));
            }
            if endpoint.ends_with("t_0") {
                txt_reply(request)
            } else {
                epub_reply(request, "fixture-epub-clock")
            }
        });
        let client = fixture.client();
        let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
        let result =
            runtime().block_on(client.chapter_prefetch_with_hints("fixture-book", "1", &mut hints));
        assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
        let requests = fixture.finish();
        assert_eq!(count(&requests, endpoint), 1);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1
        );
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}

#[test]
fn prefetch_limits_retained_content_even_when_every_shard_fits() {
    let text = "\"".repeat(640 * 1024);
    let (first, second) = crate::source::content::fixture_text_shards(&text);
    assert!(first.len() < PREFETCH_RESPONSE_BYTES);
    assert!(second.len() < PREFETCH_RESPONSE_BYTES);
    for prefetch in [true, false] {
        let first = first.clone();
        let second = second.clone();
        let fixture = Fixture::serve(move |request| match request.path.as_str() {
            "/web/book/chapter/t_0" => Reply::gzip(&first),
            "/web/book/chapter/t_1" => Reply::gzip(&second),
            _ => txt_reply(request),
        });
        let client = fixture.client();
        let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
        let content = runtime().block_on(async {
            if prefetch {
                client
                    .chapter_prefetch_with_hints("fixture-book", "1", &mut hints)
                    .await
            } else {
                client
                    .chapter_with_hints("fixture-book", "1", &mut hints)
                    .await
            }
        });
        if prefetch {
            assert!(matches!(content, Err(SourceError::ResponseTooLarge)));
        } else {
            let content = content.unwrap();
            assert_eq!(content.plain_text, text);
            assert!(content.html.len() + content.plain_text.len() > PREFETCH_CONTENT_BYTES);
        }
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}

#[test]
fn response_diagnostics_do_not_report_declared_bytes_as_observed_bytes() {
    let fixture = Fixture::serve(|_| Reply::text("x".repeat(128)));
    let client = fixture.client();
    let mut observations = Vec::new();
    let result = runtime().block_on(async {
        let response = client
            .http
            .get(format!("{}/fixture", fixture.origin))
            .send()
            .await
            .unwrap();
        response_bytes_observed(response, 64, |observed, declared| {
            observations.push((observed, declared));
        })
        .await
    });
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(observations, vec![(0, Some(128))]);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn response_diagnostics_count_decoded_chunks_and_preserve_the_exact_boundary() {
    for length in [64, 128] {
        let fixture = Fixture::serve(move |_| Reply::gzip("x".repeat(length)));
        let client = fixture.client();
        let mut observations = Vec::new();
        let result = runtime().block_on(async {
            let response = client
                .http
                .get(format!("{}/fixture", fixture.origin))
                .send()
                .await
                .unwrap();
            response_bytes_observed(response, 64, |observed, declared| {
                observations.push((observed, declared));
            })
            .await
        });
        if length == 64 {
            assert_eq!(result.unwrap(), vec![b'x'; 64]);
            assert!(observations.is_empty());
        } else {
            assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
            assert_eq!(observations.len(), 1);
            assert!(observations[0].0 > 64 && observations[0].0 <= length);
            assert_eq!(observations[0].1, None);
        }
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn chapter_http_diagnostics_survive_hint_binding_without_retaining_active_reads() {
    let fixture = Fixture::serve(txt_reply);
    let client = fixture.client();
    let diagnostics = DownloadDiagnostics::new(8, 2, false);
    let mut hints = ChapterReadHints::default();
    hints.set_download_diagnostics(diagnostics.clone());
    let content = runtime()
        .block_on(client.chapter_prefetch_with_hints("fixture-book", "1", &mut hints))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    assert_eq!(diagnostics.activity_snapshot(), (0, 0, 0, 1));
    assert!(Arc::ptr_eq(
        hints.download_diagnostics.as_ref().unwrap(),
        &diagnostics,
    ));
    let mut sibling = hints.clone();
    sibling.bind(&client, "fixture-other-book");
    assert!(Arc::ptr_eq(
        sibling.download_diagnostics.as_ref().unwrap(),
        &diagnostics,
    ));
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn cancelling_a_chapter_releases_both_actual_http_activity_guards() {
    let arrived = Arc::new(AtomicUsize::new(0));
    let arrivals = arrived.clone();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let releasing = release.clone();
    let fixture = Fixture::serve(move |request| {
        if matches!(
            request.path.as_str(),
            "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
        ) {
            arrivals.fetch_add(1, Ordering::SeqCst);
            let (lock, ready) = &*releasing;
            let mut released = lock.lock().unwrap();
            while !*released {
                let (next, timeout) = ready
                    .wait_timeout(released, Duration::from_secs(3))
                    .unwrap();
                assert!(
                    !timeout.timed_out(),
                    "Cancellation must release the held HTTP replies"
                );
                released = next;
            }
        }
        epub_reply(request, "fixture-http-cancellation-clock")
    });
    let client = fixture.client();
    let diagnostics = DownloadDiagnostics::new(8, 1, false);
    let mut hints = ChapterReadHints::for_book(&client, "fixture-book");
    hints.set_download_diagnostics(diagnostics.clone());
    runtime().block_on(async {
        let mut chapter =
            Box::pin(client.chapter_prefetch_with_hints("fixture-book", "1", &mut hints));
        tokio::select! {
            _ = &mut chapter => panic!("The held replies must keep the chapter pending"),
            result = tokio::time::timeout(Duration::from_secs(2), async {
                while arrived.load(Ordering::SeqCst) < 2 {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }) => result.unwrap(),
        }
        assert_eq!(diagnostics.activity_snapshot(), (0, 0, 2, 2));
        drop(chapter);
        assert_eq!(diagnostics.activity_snapshot(), (0, 0, 0, 2));
        let (lock, ready) = &*release;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    });
    assert_eq!(fixture.finish().len(), 4);
}
