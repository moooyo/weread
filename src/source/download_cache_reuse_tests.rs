//! Durable complete chapters must not acquire Reader, shard, or image responses again.

use super::*;
use crate::{
    book_assets::BookAssetStore,
    domain::{Book, Chapter, ContentStatus, DownloadChapterStatus, DownloadStatus, DownloadTask},
    jobs::{DownloadPlan, JobEvent, JobResult, Jobs},
    store::Store,
};
use std::{
    io::Cursor,
    sync::mpsc::{self, Receiver},
};

const BOOK: &str = "fixture-book";
const IMAGE_PATH: &str = "/assets/cache-reuse.png";
const TEXT: &str = "Complete cache fixture text.";

fn chapter_id(index: u32) -> String {
    format!("cache-reuse-chapter-{index}")
}

fn reader_path(index: u32) -> String {
    let url = crate::source::protocol::reader_url_checked(BOOK, Some(&chapter_id(index))).unwrap();
    reqwest::Url::parse(&url).unwrap().path().to_owned()
}

fn fixture(count: u32, with_image: bool) -> Fixture {
    let chapters = (0..count)
        .map(|index| {
            json!({
                "chapterUid": chapter_id(index),
                "chapterIdx": index + 1,
                "title": format!("Cache fixture chapter {index}"),
                "wordCount": 1,
                "paid": 1,
                "price": 0,
            })
        })
        .collect::<Vec<_>>();
    let pages = (0..count)
        .map(|index| {
            let state = json!({
                "user": { "vid": fixture_auth().user_vid },
                "reader": {
                    "bookId": BOOK,
                    "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
                    "chapterInfos": chapters,
                    "currentChapter": chapters[index as usize],
                    "psvts": format!("cache-reuse-clock-{index}"),
                    "pclts": 0,
                },
            });
            (
                reader_path(index),
                format!("<script>window.__INITIAL_STATE__={state};</script>"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let catalog = json!({ "data": [{ "bookId": BOOK, "updated": chapters }] }).to_string();
    let html = if with_image {
        format!(
            "<html><body><p>{TEXT}</p><img src=\"https://res.weread.qq.com{IMAGE_PATH}\"></body></html>"
        )
    } else {
        format!("<html><body><p>{TEXT}</p></body></html>")
    };
    let (first, second) = crate::source::content::fixture_text_shards(&html);
    let mut image = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 6, image::Rgb([53, 89, 127])))
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    let image = image.into_inner();
    Fixture::serve(move |request| {
        if let Some(page) = pages.get(&request.path) {
            return Reply::gzip(page);
        }
        match request.path.as_str() {
            "/web/book/chapterInfos" => Reply::gzip(&catalog),
            "/web/book/chapter/e_0" => Reply::text(&first),
            "/web/book/chapter/e_1" => Reply::text(&second),
            "/web/book/chapter/e_3" => Reply::text(""),
            IMAGE_PATH => Reply::text(&image),
            _ => panic!("Unexpected cache fixture operation: {}", request.path),
        }
    })
}

fn client(fixture: &Fixture) -> WeReadClient {
    let mut client = fixture.client();
    client.test_resource_origin = Some(fixture.origin.clone());
    client.resource_http = client.http.clone();
    client
}

fn seed(count: u32) -> anyhow::Result<(tempfile::TempDir, Store, BookAssetStore)> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("reader.sqlite3"))?;
    let assets = BookAssetStore::open(directory.path().join("images"), 8 * 1024 * 1024)?;
    let mut book = Book::new(BOOK, "Complete cache fixture");
    book.source = "weread".into();
    book.total_chapters = count;
    store.upsert_book(&book)?;
    for index in 0..count {
        store.upsert_chapter(&Chapter {
            id: chapter_id(index),
            book_id: BOOK.into(),
            index,
            title: format!("Cache fixture chapter {index}"),
            ..Default::default()
        })?;
    }
    Ok((directory, store, assets))
}

fn task(id: &str, indices: Vec<u32>, include_images: bool) -> DownloadTask {
    DownloadTask {
        id: id.into(),
        book_id: BOOK.into(),
        account_key: crate::source::report::account_key(&fixture_auth()).unwrap(),
        chapters: indices,
        status: DownloadStatus::Running,
        include_images,
        updated_at: 100,
        ..Default::default()
    }
}

fn checked_plan(
    store: &Store,
    assets: &BookAssetStore,
    task: &mut DownloadTask,
) -> anyhow::Result<DownloadPlan> {
    task.status = DownloadStatus::Running;
    store.save_task(task)?;
    let headers = store.chapter_metadata(&task.book_id)?;
    let states = if task.include_images {
        let selected = task.chapters.iter().copied().collect::<BTreeSet<_>>();
        let uids = headers
            .iter()
            .filter(|chapter| {
                selected.contains(&chapter.index)
                    && chapter.cached
                    && chapter.content_status == ContentStatus::Full
                    && chapter.content_account_key == task.account_key
            })
            .map(|chapter| chapter.id.as_str())
            .collect::<Vec<_>>();
        let proof = assets.verify_complete_book(store, &task.book_id, &uids)?;
        store.prepare_download_chapter_states_with_verified_images(
            task,
            proof,
            task.updated_at + 1,
        )?
    } else {
        store.prepare_download_chapter_states(task, task.updated_at + 1)?
    };
    *task = store.task(&task.id)?.unwrap();
    let mut plan = DownloadPlan::from_headers_with_states(task, &headers, &states)?;
    if task.include_images {
        let candidates = headers
            .iter()
            .filter(|chapter| {
                states.iter().any(|state| {
                    state.chapter_index == chapter.index
                        && state.chapter_uid == chapter.id
                        && state.status != DownloadChapterStatus::Complete
                }) && chapter.cached
                    && chapter.content_status == ContentStatus::Full
                    && chapter.content_account_key == task.account_key
            })
            .collect::<Vec<_>>();
        let uids = candidates
            .iter()
            .map(|chapter| chapter.id.as_str())
            .collect::<Vec<_>>();
        let complete = assets.complete_many(store, &task.book_id, &uids)?;
        assert_eq!(complete.len(), candidates.len());
        for (chapter, complete) in candidates.into_iter().zip(complete) {
            if complete {
                plan.mark_complete_cached_images(chapter.index, &chapter.id)?;
            }
        }
    }
    Ok(plan)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Checkpoints {
    downloaded: Vec<u32>,
    cached: Vec<u32>,
}

fn download(
    jobs: &Jobs,
    receiver: &Receiver<JobEvent>,
    store: &Store,
    assets: &BookAssetStore,
    mut task: DownloadTask,
    plan: DownloadPlan,
) -> anyhow::Result<Checkpoints> {
    assert_eq!(task.status, DownloadStatus::Running);
    assert_eq!(store.task(&task.id)?.as_ref(), Some(&task));
    assert!(
        jobs.download_with_concurrency(77, task.clone(), plan, 8)
            .is_none()
    );
    let mut observed = Checkpoints::default();
    loop {
        let JobEvent {
            result, credits, ..
        } = receiver.recv_timeout(Duration::from_secs(8))?;
        match result {
            JobResult::Downloaded {
                task_id,
                chapter,
                next_index,
                html,
                metadata,
                images,
                checkpoint,
            } => {
                assert_eq!(task_id, task.id);
                assert!(credits.is_some());
                let ordinal = next_index.checked_sub(1).expect("Missing chapter ordinal");
                assert_eq!(task.chapters.get(ordinal), Some(&chapter.index));
                assert_eq!(chapter.content_status, ContentStatus::Full);
                assert_eq!(chapter.body.as_deref(), Some(TEXT));
                let html = html.expect("The real worker must prepare the chapter HTML");
                let metadata = metadata.expect("The real worker must prepare image metadata");
                store.validate_independent_download_chapter(&task, &chapter, ordinal)?;
                let publication = assets.commit_prepared_download_chapter_metadata(
                    store,
                    &task.book_id,
                    &chapter.id,
                    &html,
                    chapter.body.as_deref().unwrap(),
                    &metadata,
                    images.as_ref(),
                )?;
                let updated_at = task.updated_at + 1;
                if task.include_images {
                    let proof = publication
                        .expect("A complete image worker must publish verified resources");
                    store.commit_verified_image_download_chapter(
                        &task, &chapter, ordinal, proof, updated_at,
                    )?;
                } else {
                    store.commit_independent_download_chapter(
                        &task, &chapter, ordinal, updated_at,
                    )?;
                }
                assert!(!observed.downloaded.contains(&chapter.index));
                observed.downloaded.push(chapter.index);
                task = store.task(&task.id)?.unwrap();
                checkpoint.send(Ok(())).unwrap();
            }
            JobResult::CachedDownloaded {
                task_id,
                chapter_id,
                chapter_index,
                next_index,
                checkpoint,
            } => {
                assert_eq!(task_id, task.id);
                assert!(credits.is_some());
                let ordinal = next_index
                    .checked_sub(1)
                    .expect("Missing cached chapter ordinal");
                assert_eq!(task.chapters.get(ordinal), Some(&chapter_index));
                let updated_at = task.updated_at + 1;
                if task.include_images {
                    let proof = assets
                        .verify_complete_checkpoint(store, &task.book_id, &chapter_id)?
                        .expect("A cached image checkpoint needs current durable completeness");
                    store.commit_verified_cached_image_download_chapter(
                        &task,
                        proof,
                        chapter_index,
                        ordinal,
                        &task.account_key,
                        updated_at,
                    )?;
                } else {
                    store.commit_cached_download_chapter(
                        &task,
                        &chapter_id,
                        chapter_index,
                        ordinal,
                        &task.account_key,
                        updated_at,
                    )?;
                }
                assert!(!observed.cached.contains(&chapter_index));
                observed.cached.push(chapter_index);
                task = store.task(&task.id)?.unwrap();
                checkpoint.send(Ok(())).unwrap();
            }
            JobResult::DownloadProgress { .. } => {}
            JobResult::DownloadFinished { task_id } => {
                assert_eq!(task_id, task.id);
                assert_eq!(task.next_index, task.chapters.len());
                assert_eq!(task.completed_count, task.chapters.len());
                task.status = DownloadStatus::Complete;
                store.save_task(&task)?;
                observed.downloaded.sort_unstable();
                observed.cached.sort_unstable();
                return Ok(observed);
            }
            other => panic!("Unexpected cache fixture result: {other:?}"),
        }
        drop(credits);
    }
}

fn worker_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_second_whole_book_download_uses_durable_cache_without_any_http() -> anyhow::Result<()> {
    for (include_images, with_image) in [(false, false), (true, false), (true, true)] {
        let fixture = fixture(3, with_image);
        let runtime = worker_runtime();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client(&fixture)))?;
        let (_directory, store, assets) = seed(3)?;
        let mut first = task("first-whole-book", vec![0, 1, 2], include_images);
        let plan = checked_plan(&store, &assets, &mut first)?;
        assert_eq!(
            download(&jobs, &receiver, &store, &assets, first, plan)?,
            Checkpoints {
                downloaded: vec![0, 1, 2],
                cached: Vec::new(),
            }
        );
        let uids = (0..3).map(chapter_id).collect::<Vec<_>>();
        assert_eq!(
            assets.complete_many(
                &store,
                BOOK,
                &uids.iter().map(String::as_str).collect::<Vec<_>>()
            )?,
            vec![true; 3]
        );
        let before = fixture.records.lock().unwrap().len();
        assert!(before > 0);
        let mut second = task("second-whole-book", vec![0, 1, 2], include_images);
        let plan = checked_plan(&store, &assets, &mut second)?;
        assert_eq!((second.next_index, second.completed_count), (3, 3));
        assert!(
            store
                .download_chapter_states(&second.id)?
                .iter()
                .all(|state| state.status == DownloadChapterStatus::Complete)
        );
        assert_eq!(
            download(&jobs, &receiver, &store, &assets, second, plan)?,
            Checkpoints {
                downloaded: Vec::new(),
                cached: Vec::new(),
            }
        );
        assert_eq!(fixture.records.lock().unwrap().len(), before);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 3);
        drop(jobs);
        drop(receiver);
        runtime.shutdown_timeout(Duration::from_secs(1));
        let requests = fixture.finish();
        assert_eq!(requests.len(), before);
        for index in 0..3 {
            assert_eq!(count(&requests, &reader_path(index)), 1);
        }
        for endpoint in [
            "/web/book/chapter/e_0",
            "/web/book/chapter/e_1",
            "/web/book/chapter/e_3",
        ] {
            assert_eq!(count(&requests, endpoint), 3);
        }
        assert_eq!(
            count(&requests, "/web/book/chapterInfos"),
            usize::from(include_images)
        );
        assert_eq!(count(&requests, IMAGE_PATH), if with_image { 3 } else { 0 });
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
    Ok(())
}

#[test]
fn queued_and_resumed_mixed_plans_do_not_prefetch_complete_chapters() -> anyhow::Result<()> {
    for include_images in [false, true] {
        for resumed in [false, true] {
            let fixture = fixture(6, false);
            let runtime = worker_runtime();
            let (sender, receiver) = mpsc::channel();
            let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client(&fixture)))?;
            let (_directory, store, assets) = seed(6)?;
            let mut queued = task("queued-whole-book", (0..6).collect(), include_images);
            queued.status = DownloadStatus::Pending;
            store.save_task(&queued)?;
            let mut first = task("prior-selected-chapters", vec![0, 2, 4], include_images);
            let plan = checked_plan(&store, &assets, &mut first)?;
            assert_eq!(
                download(&jobs, &receiver, &store, &assets, first, plan)?.downloaded,
                [0, 2, 4]
            );
            let mut queued = store.task(&queued.id)?.unwrap();
            queued.status = DownloadStatus::Running;
            queued.next_index = usize::from(resumed);
            let plan = checked_plan(&store, &assets, &mut queued)?;
            assert_eq!((queued.next_index, queued.completed_count), (1, 3));
            assert_eq!(
                store
                    .download_chapter_states(&queued.id)?
                    .iter()
                    .filter(|state| state.status == DownloadChapterStatus::Complete)
                    .map(|state| state.chapter_index)
                    .collect::<Vec<_>>(),
                [0, 2, 4]
            );
            assert_eq!(
                download(&jobs, &receiver, &store, &assets, queued, plan)?,
                Checkpoints {
                    downloaded: vec![1, 3, 5],
                    cached: Vec::new(),
                }
            );
            assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 6);
            drop(jobs);
            drop(receiver);
            runtime.shutdown_timeout(Duration::from_secs(1));
            let requests = fixture.finish();
            for index in 0..6 {
                assert_eq!(
                    count(&requests, &reader_path(index)),
                    1,
                    "The prefetch window fetched a complete chapter again"
                );
            }
            for endpoint in [
                "/web/book/chapter/e_0",
                "/web/book/chapter/e_1",
                "/web/book/chapter/e_3",
            ] {
                assert_eq!(count(&requests, endpoint), 6);
            }
            assert_eq!(
                count(&requests, "/web/book/chapterInfos"),
                if include_images { 2 } else { 0 }
            );
            assert_eq!(count(&requests, IMAGE_PATH), 0);
        }
    }
    Ok(())
}

#[test]
fn incomplete_or_corrupt_images_cannot_suppress_required_acquisition() -> anyhow::Result<()> {
    for corrupt in [false, true] {
        let fixture = fixture(1, true);
        let runtime = worker_runtime();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client(&fixture)))?;
        let (_directory, store, assets) = seed(1)?;
        let mut first = task("initial-image-content", vec![0], corrupt);
        let plan = checked_plan(&store, &assets, &mut first)?;
        download(&jobs, &receiver, &store, &assets, first, plan)?;
        if corrupt {
            let manifest = assets.chapter(&store, BOOK, &chapter_id(0))?.unwrap();
            let path = assets.image_path(&manifest.images[0])?.unwrap();
            std::fs::write(path, b"corrupt synthetic cached image")?;
        }
        assert_eq!(
            assets.complete_many(&store, BOOK, &[&chapter_id(0)])?,
            [false]
        );
        let before = fixture.records.lock().unwrap().len();
        let mut second = task("repair-image-content", vec![0], true);
        let plan = checked_plan(&store, &assets, &mut second)?;
        assert_eq!((second.next_index, second.completed_count), (0, 0));
        assert_eq!(
            store.download_chapter_states(&second.id)?[0].status,
            DownloadChapterStatus::Pending
        );
        assert_eq!(
            download(&jobs, &receiver, &store, &assets, second, plan)?,
            Checkpoints {
                downloaded: vec![0],
                cached: Vec::new(),
            }
        );
        assert!(fixture.records.lock().unwrap().len() > before);
        assert_eq!(
            assets.complete_many(&store, BOOK, &[&chapter_id(0)])?,
            [true]
        );
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
        drop(jobs);
        drop(receiver);
        runtime.shutdown_timeout(Duration::from_secs(1));
        let requests = fixture.finish();
        assert_eq!(count(&requests, &reader_path(0)), 2);
        assert_eq!(
            count(&requests, "/web/book/chapterInfos"),
            if corrupt { 2 } else { 1 }
        );
        assert_eq!(count(&requests, IMAGE_PATH), if corrupt { 2 } else { 1 });
    }
    Ok(())
}

#[test]
fn image_cache_markers_require_a_matching_full_owner_uid_and_image_task() {
    let task = task("marker-fences", vec![0], true);
    let valid = Chapter {
        id: chapter_id(0),
        book_id: BOOK.into(),
        index: 0,
        cached: true,
        content_status: ContentStatus::Full,
        content_account_key: task.account_key.clone(),
        ..Default::default()
    };
    for status in [
        ContentStatus::Preview,
        ContentStatus::Unknown,
        ContentStatus::Locked,
    ] {
        let mut header = valid.clone();
        header.content_status = status;
        let mut plan = DownloadPlan::from_headers(&task, &[header]).unwrap();
        assert!(plan.mark_complete_cached_images(0, &valid.id).is_err());
    }
    for header in [
        Chapter {
            cached: false,
            ..valid.clone()
        },
        Chapter {
            content_account_key: "another-fixture-owner".into(),
            ..valid.clone()
        },
        Chapter {
            content_account_key: String::new(),
            ..valid.clone()
        },
    ] {
        let mut plan = DownloadPlan::from_headers(&task, &[header]).unwrap();
        assert!(plan.mark_complete_cached_images(0, &valid.id).is_err());
    }
    let mut plan = DownloadPlan::from_headers(&task, std::slice::from_ref(&valid)).unwrap();
    assert!(
        plan.mark_complete_cached_images(0, "wrong-fixture-uid")
            .is_err()
    );
    assert!(plan.mark_complete_cached_images(1, &valid.id).is_err());
    assert!(plan.mark_complete_cached_images(0, &valid.id).is_ok());
    let text_task = DownloadTask {
        include_images: false,
        ..task
    };
    let mut plan = DownloadPlan::from_headers(&text_task, &[valid]).unwrap();
    assert!(plan.mark_complete_cached_images(0, &chapter_id(0)).is_err());
}
