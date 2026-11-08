//! Checked bodyless plans and genuine cached producers under packet admission.

use super::*;
use std::sync::mpsc;

fn task(count: u32) -> domain::DownloadTask {
    domain::DownloadTask {
        id: "bodyless-task".into(),
        book_id: "bodyless-book".into(),
        account_key: "bodyless-account".into(),
        chapters: (0..count).collect(),
        status: domain::DownloadStatus::Running,
        include_images: false,
        ..Default::default()
    }
}

fn headers(count: u32, cached: bool) -> Vec<domain::Chapter> {
    (0..count)
        .map(|index| domain::Chapter {
            id: format!("uid-{index}"),
            book_id: "bodyless-book".into(),
            index,
            title: format!("Title {index}"),
            cached,
            body: None,
            content_status: if cached {
                domain::ContentStatus::Full
            } else {
                domain::ContentStatus::Unknown
            },
            content_account_key: if cached {
                "bodyless-account".into()
            } else {
                String::new()
            },
        })
        .collect()
}

#[test]
fn sparse_plans_keep_only_remaining_task_metadata() {
    let mut task = task(0);
    task.chapters = vec![1, 3];
    task.next_index = 1;
    let mut headers = headers(5, true);
    headers[0].title = "Unrelated metadata ".repeat(1024);
    let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
    assert_eq!(plan.chapters.keys().copied().collect::<Vec<_>>(), [3]);
    assert_eq!(plan.chapters[&3].id, "uid-3");
    assert!(plan.chapters[&3].cached_full);
    assert_eq!(plan.indices, [1, 3]);
    assert_eq!(plan.next_index, 1);
    assert!(headers.iter().all(|header| header.body.is_none()));
}

#[test]
fn body_bearing_inputs_and_excess_entry_counts_never_create_a_plan() {
    let task = task(1);
    let mut headers = headers(1, true);
    headers[0].body = Some("A body must remain in SQL".into());
    let error = DownloadPlan::from_headers(&task, &headers).unwrap_err();
    assert!(matches!(
        &error,
        SourceError::InvalidResponse("download_plan_body")
    ));
    assert!(!error.requires_login() && !error.retryable());
    let too_many = vec![domain::Chapter::default(); MAX_DOWNLOAD_CHAPTERS + 1];
    assert!(matches!(
        DownloadPlan::from_headers(&task, &too_many),
        Err(SourceError::ResponseTooLarge)
    ));
}

#[test]
fn malformed_identities_duplicates_and_missing_task_indices_are_rejected() {
    let mut task = task(2);
    let original = headers(2, false);
    task.book_id = "b".repeat(MAX_DOWNLOAD_IDENTIFIER_BYTES + 1);
    assert!(DownloadPlan::from_headers(&task, &original).is_err());
    task.book_id = "bodyless-book".into();
    task.account_key = "a".repeat(MAX_DOWNLOAD_IDENTIFIER_BYTES + 1);
    assert!(DownloadPlan::from_headers(&task, &original).is_err());
    task.account_key = "bodyless-account".into();
    task.error = Some("e".repeat(MAX_DOWNLOAD_IDENTIFIER_BYTES + 1));
    assert!(matches!(
        DownloadPlan::from_headers(&task, &original),
        Err(SourceError::ResponseTooLarge)
    ));
    task.error = None;
    let mut invalid = original.clone();
    invalid[0].id = "u".repeat(MAX_DOWNLOAD_IDENTIFIER_BYTES + 1);
    assert!(DownloadPlan::from_headers(&task, &invalid).is_err());
    invalid = original.clone();
    invalid[1].index = 0;
    assert!(DownloadPlan::from_headers(&task, &invalid).is_err());
    invalid = original.clone();
    invalid[1].id = invalid[0].id.clone();
    assert!(DownloadPlan::from_headers(&task, &invalid).is_err());
    assert!(matches!(
        DownloadPlan::from_headers(&task, &original[..1]),
        Err(SourceError::InvalidResponse("missing_chapter"))
    ));
    task.chapters = vec![0, 0];
    assert!(matches!(
        DownloadPlan::from_headers(&task, &original),
        Err(SourceError::InvalidResponse("download_cursor"))
    ));
}

#[test]
fn a_small_metadata_allowance_rejects_before_owned_plan_entries() {
    let task = task(1);
    let mut headers = headers(3, true);
    headers[2].title = "long unrelated title ".repeat(64);
    assert!(matches!(
        DownloadPlan::from_headers_with_limit(&task, &headers, 128),
        Err(SourceError::ResponseTooLarge)
    ));
    let plan = DownloadPlan::from_headers_with_limit(&task, &headers, 16 * 1024).unwrap();
    assert_eq!(plan.chapters.len(), 1);
    assert_eq!(plan.chapters[&0].title, "Title 0");
}

#[test]
fn only_full_cached_headers_from_the_tasks_account_can_skip_transport() {
    let task = task(4);
    let mut headers = headers(4, true);
    headers[1].content_status = domain::ContentStatus::Preview;
    headers[2].cached = false;
    headers[3].content_account_key = "another-account".into();
    let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
    assert!(plan.chapters[&0].cached_full);
    assert!((1..4).all(|index| !plan.chapters[&index].cached_full));
}

#[test]
fn mismatched_plan_submission_does_not_cancel_a_current_task() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
    let first = task(1);
    let plan = DownloadPlan::from_headers(&first, &headers(1, false)).unwrap();
    assert!(jobs.download(1, first.clone(), plan).is_none());
    let plan = DownloadPlan::from_headers(&first, &headers(1, false)).unwrap();
    let mut replacement = first.clone();
    replacement.id = "replacement-task".into();
    replacement.account_key = "replaced-account".into();
    let rejection = jobs.download(2, replacement, plan).unwrap();
    assert!(matches!(
        rejection.result,
        JobResult::DownloadFailed {
            error: SourceError::InvalidResponse("download_plan"),
            ..
        }
    ));
    assert!(rejection.credits.is_none());
    assert_eq!(jobs.active_download().as_deref(), Some(first.id.as_str()));
    assert!(receiver.try_recv().is_err());
    assert!(jobs.cancel_download(&first.id).is_some());
    runtime.block_on(tokio::task::yield_now());
}
