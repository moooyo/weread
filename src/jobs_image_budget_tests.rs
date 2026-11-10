//! Real paid Full Jobs producers preserve state when article image admission fails.

use super::*;
use crate::article::{ImageTestLimits, image_budget_tests::ObservationScope as ImageScope};

#[test]
fn actual_full_jobs_image_url_quota_releases_credit_without_new_body_or_payload() {
    let html =
        "<html><body><p>A</p><img src='https://image-fixture.invalid/figure.png'></body></html>";
    let fixture = Fixture::new(html);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = calibrated_client(&runtime, &fixture, html);
    let before_session = client.session();
    let (_directory, store, assets, task, note) = state();
    let original_chapter = store.chapter(BOOK, 0).unwrap().unwrap();
    let original_book = store.book(BOOK).unwrap().unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    for (request, limits) in [
        (
            101,
            ImageTestLimits {
                raw_url_bytes: 8,
                ..ImageTestLimits::default()
            },
        ),
        (
            102,
            ImageTestLimits {
                metadata_bytes: 0,
                ..ImageTestLimits::default()
            },
        ),
    ] {
        let scope = ImageScope::start(limits);
        let plan =
            DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap()).unwrap();
        assert!(jobs.download(request, task.clone(), plan).is_none());
        let terminal = acknowledge_quota_failure(&runtime, &receiver, &jobs, &task, request);
        assert!(jobs.accepts(&terminal));
        assert_eq!(terminal.request, request);
        assert!(terminal.credits.is_some());
        assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
        assert_eq!(scope.stats().usage.url_parse_calls, 0);
        assert_eq!(scope.stats().usage.url_input_copies, 0);
        assert_eq!(scope.stats().usage.url_hashes, 0);
        assert_eq!(store.task(&task.id).unwrap().unwrap(), task);
        assert_eq!(store.chapter(BOOK, 0).unwrap().unwrap(), original_chapter);
        assert_eq!(store.book(BOOK).unwrap().unwrap(), original_book);
        assert_eq!(store.annotations(BOOK).unwrap(), vec![note.clone()]);
        assert!(assets.chapter(&store, BOOK, UID).unwrap().is_none());
        assert_eq!(assets.usage().unwrap(), 0);
        assert_eq!(client.session(), before_session);
        assert_eq!(jobs.session().unwrap(), before_session);
        drop(terminal);
        assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    }
    request_contract(&fixture.finish(), 2);
}
