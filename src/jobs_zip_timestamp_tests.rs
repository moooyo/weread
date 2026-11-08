//! Genuine image-download workers reject a small malformed timestamp ZIP.

use super::review_output_budget_tests::{RecordedRequest, read_request};
use super::*;
use crate::{
    book_assets::BookAssetStore,
    cache::FileCache,
    domain::{Book, Chapter, ContentStatus, DownloadStatus, DownloadTask},
    jobs::{DownloadPlan, JobResult, Jobs},
    source::images::{DownloadedChapterImages, ImageAsset},
    store::Store,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

const BOOK: &str = "zip-timestamp-worker-book";
const UID: &str = "zip-timestamp-worker-chapter";
const VID: &str = "synthetic-zip-worker-vid";
const ARCHIVE_URL: &str = "https://zip-fixture.invalid/archive.zip";
const HTML: &str = "<html><body><p>Worker chapter.</p><img src='image.png'></body></html>";
const TEXT: &str = "Worker chapter.";
const TIMESTAMP_ERROR: &str = "flags and len don't match in extended timestamp field";

fn auth() -> AuthSession {
    AuthSession {
        api_key: "synthetic-zip-worker-key".into(),
        user_vid: VID.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), VID.into()),
            ("wr_skey".into(), "synthetic-zip-worker-session".into()),
        ]),
        ..Default::default()
    }
}

fn retained_timestamp_worker_zip() -> (Vec<u8>, Vec<u8>) {
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["image.png", "other.png"] {
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(&png).unwrap();
    }
    let original = writer.finish().unwrap().into_inner();
    let footer = original.len().checked_sub(22).unwrap();
    assert_eq!(&original[footer..footer + 4], b"PK\x05\x06");
    assert_eq!(&original[footer + 20..], &[0, 0]);
    assert_eq!(
        u16::from_le_bytes(original[footer + 8..footer + 10].try_into().unwrap()),
        2
    );
    assert_eq!(
        u16::from_le_bytes(original[footer + 10..footer + 12].try_into().unwrap()),
        2
    );
    let central =
        u32::from_le_bytes(original[footer + 16..footer + 20].try_into().unwrap()) as usize;
    assert_eq!(
        u32::from_le_bytes(original[footer + 12..footer + 16].try_into().unwrap()) as usize,
        footer - central
    );
    let extra = [0x55, 0x54, 1, 0, 0].repeat(3);
    let mut bytes = original[..central].to_vec();
    let mut offset = central;
    for _ in 0..2 {
        assert_eq!(&original[offset..offset + 4], b"PK\x01\x02");
        let name = usize::from(u16::from_le_bytes(
            original[offset + 28..offset + 30].try_into().unwrap(),
        ));
        let old_extra = u16::from_le_bytes(original[offset + 30..offset + 32].try_into().unwrap());
        let comment = usize::from(u16::from_le_bytes(
            original[offset + 32..offset + 34].try_into().unwrap(),
        ));
        assert_eq!(old_extra, 0);
        let mut header = original[offset..offset + 46].to_vec();
        header[30..32].copy_from_slice(&u16::try_from(extra.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&original[offset + 46..offset + 46 + name]);
        bytes.extend_from_slice(&extra);
        bytes.extend_from_slice(&original[offset + 46 + name..offset + 46 + name + comment]);
        offset += 46 + name + comment;
    }
    assert_eq!(offset, footer);
    let mut footer_bytes = original[footer..].to_vec();
    footer_bytes[12..16]
        .copy_from_slice(&u32::try_from(bytes.len() - central).unwrap().to_le_bytes());
    bytes.extend_from_slice(&footer_bytes);
    assert!(bytes.len() < 2048);
    (bytes, png)
}

#[test]
fn actual_image_download_worker_preserves_state_when_retained_zip_records_exceed_quota() {
    let (bytes, png) = retained_timestamp_worker_zip();
    let encoded = shard(HTML);
    assert_eq!(
        super::super::content::decode_shards(&[&encoded]).unwrap(),
        HTML
    );
    assert_eq!(super::super::content::html_to_text(HTML), TEXT);
    let mut calibrated =
        zip::ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 2).unwrap();
    assert_eq!(calibrated.len(), 2);
    for ordinal in 0..2 {
        let mut entry = calibrated.by_index(ordinal).unwrap();
        assert_eq!(entry.extra_data_fields().count(), 3);
        let mut actual = Vec::new();
        entry.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, png);
    }
    let decoded = super::super::images::decode_archive(&bytes).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].bytes, png);
    assert_eq!(decoded[0].source_names, ["image.png", "other.png"]);
    let fixture = WorkerHttpFixture::new(bytes, encoded);
    // The actual asynchronous download executes on this thread until image
    // acquisition fails, before the later blocking preparation is admitted.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = {
        let _context = runtime.enter();
        fixture.client()
    };
    let before = client.session();
    let book = runtime.block_on(client.book_info(BOOK)).unwrap();
    assert_eq!(
        (book.id.as_str(), book.title.as_str()),
        (BOOK, "Worker book")
    );
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    store
        .upsert_book(&Book {
            id: BOOK.into(),
            title: "Original retained-quota book".into(),
            source: "weread".into(),
            total_chapters: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: UID.into(),
            book_id: BOOK.into(),
            index: 0,
            title: "Original retained-quota chapter".into(),
            cached: true,
            body: Some(TEXT.into()),
            content_status: ContentStatus::Full,
            content_account_key: owner(),
        })
        .unwrap();
    let task = DownloadTask {
        id: "retained-zip-worker-task".into(),
        book_id: BOOK.into(),
        account_key: owner(),
        chapters: vec![0],
        status: DownloadStatus::Running,
        include_images: true,
        next_index: 0,
        updated_at: 17,
        ..Default::default()
    };
    store.save_task(&task).unwrap();
    let original_chapter = store.chapter(BOOK, 0).unwrap().unwrap();
    let original_book = store.book(BOOK).unwrap().unwrap();
    let covers = FileCache::open(directory.path().join("covers"), 4096).unwrap();
    let cover_path = covers
        .put("old-cover", b"Preserve cover through retained quota")
        .unwrap();
    let cover_bytes = std::fs::read(&cover_path).unwrap();
    let cover_usage = covers.usage().unwrap();
    let image_id = hex::encode(Sha256::digest(&png));
    let downloaded = DownloadedChapterImages {
        assets: vec![ImageAsset {
            id: image_id.clone(),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["image.png".into()],
            bytes: png.clone(),
        }],
        source_to_asset: BTreeMap::from([("image.png".into(), image_id)]),
    };
    let assets = BookAssetStore::open(directory.path().join("images"), 128 * 1024).unwrap();
    let manifest = assets
        .commit_chapter(&store, BOOK, UID, HTML, TEXT, Some(&downloaded))
        .unwrap();
    assert!(manifest.complete);
    let image_path = assets.image_path(&manifest.images[0]).unwrap().unwrap();
    let image_bytes = std::fs::read(&image_path).unwrap();
    let image_usage = assets.usage().unwrap();
    assert!(cover_usage > 0 && image_usage > 0);
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    for request in [91, 92] {
        super::super::images::with_zip_retention_test_limits(
            zip::read::MetadataRetentionLimits {
                extra_records: 5,
                ..Default::default()
            },
            png.len(),
            || {
                let plan = DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap())
                    .unwrap();
                assert!(jobs.download(request, task.clone(), plan).is_none());
                let terminal = runtime.block_on(async {
                    let deadline = Instant::now() + Duration::from_secs(8);
                    loop {
                        if let Ok(event) = receiver.try_recv() {
                            break event;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "The actual retained-quota worker did not settle"
                        );
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                });
                assert!(jobs.accepts(&terminal));
                assert_eq!(
                    (terminal.request, terminal.account_epoch),
                    (request, jobs.account_epoch())
                );
                assert!(matches!(&terminal.result, JobResult::DownloadFailed {
                task_id, error: SourceError::ResponseTooLarge } if task_id == &task.id));
                assert!(terminal.credits.is_some());
                assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
                assert!(jobs.active_download().is_none());
                assert!(
                    receiver.try_recv().is_err(),
                    "A quota refusal must not publish chapter/progress packets"
                );
                let usage = super::super::images::zip_retention_usage_for_test();
                assert_eq!(
                    (
                        usage.extra_preflights,
                        usage.extra_records,
                        usage.extra_pushes
                    ),
                    (6, 5, 5)
                );
                assert!(usage.denied);
                assert_eq!(usage.operations_after_denial, 0);
                assert_eq!(
                    super::super::images::zip_retention_payload_copies_for_test(),
                    0
                );
                assert_eq!(store.task(&task.id).unwrap().unwrap(), task);
                assert_eq!(store.chapter(BOOK, 0).unwrap().unwrap(), original_chapter);
                assert_eq!(store.book(BOOK).unwrap().unwrap(), original_book);
                assert_eq!(client.session(), before);
                assert_eq!(jobs.session().unwrap(), before);
                assert_eq!(covers.usage().unwrap(), cover_usage);
                assert_eq!(std::fs::read(&cover_path).unwrap(), cover_bytes);
                assert_eq!(assets.usage().unwrap(), image_usage);
                assert_eq!(
                    assets.chapter(&store, BOOK, UID).unwrap().unwrap(),
                    manifest
                );
                assert_eq!(std::fs::read(&image_path).unwrap(), image_bytes);
                drop(terminal);
                assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
            },
        );
    }
    let requests = fixture.finish();
    assert_eq!(requests.len(), 13);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/archive.zip")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/gateway")
            .count(),
        1
    );
    for request in requests
        .iter()
        .filter(|request| request.path == "/archive.zip")
    {
        assert_eq!(request.method, "GET");
        assert!(!request.headers.contains_key("cookie"));
    }
}

fn owner() -> String {
    hex::encode(Sha256::digest(
        format!("weread-download-vid:{VID}").as_bytes(),
    ))
}

struct TimestampZip {
    baseline: Vec<u8>,
    valid: Vec<u8>,
    malformed: Vec<u8>,
    central: usize,
    extra_start: usize,
}

fn timestamp_zip() -> TimestampZip {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "image.png",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"small archive member").unwrap();
    let baseline = writer.finish().unwrap().into_inner();
    let eocd = baseline.len().checked_sub(22).unwrap();
    assert_eq!(&baseline[eocd..eocd + 4], b"PK\x05\x06");
    assert_eq!(&baseline[eocd + 20..], &[0, 0]);
    assert_eq!(
        baseline
            .windows(4)
            .filter(|part| *part == b"PK\x05\x06")
            .count(),
        1
    );
    assert_eq!(
        u16::from_le_bytes(baseline[eocd + 10..eocd + 12].try_into().unwrap()),
        1
    );
    // Follow the EOCD's declared directory offset rather than finding a magic
    // sequence which could also occur in an entry body or another fixed field.
    let central = u32::from_le_bytes(baseline[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    assert_eq!(&baseline[central..central + 4], b"PK\x01\x02");
    let name_bytes =
        u16::from_le_bytes(baseline[central + 28..central + 30].try_into().unwrap()) as usize;
    let extra_bytes = u16::from_le_bytes(baseline[central + 30..central + 32].try_into().unwrap());
    let comment_bytes =
        u16::from_le_bytes(baseline[central + 32..central + 34].try_into().unwrap()) as usize;
    assert_eq!(extra_bytes, 0);
    let extra_start = central + 46 + name_bytes;
    assert_eq!(extra_start + comment_bytes, eocd);
    assert_eq!(
        u32::from_le_bytes(baseline[eocd + 12..eocd + 16].try_into().unwrap()) as usize,
        eocd - central,
    );
    let inject = |field: [u8; 5]| {
        let mut bytes = baseline[..extra_start].to_vec();
        bytes[central + 30..central + 32].copy_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&field);
        bytes.extend_from_slice(&baseline[extra_start..eocd]);
        let mut footer = baseline[eocd..].to_vec();
        footer[12..16]
            .copy_from_slice(&u32::try_from(bytes.len() - central).unwrap().to_le_bytes());
        bytes.extend_from_slice(&footer);
        assert!(bytes.len() < 1024);
        let footer_start = bytes.len() - 22;
        assert_eq!(
            u32::from_le_bytes(
                bytes[footer_start + 16..footer_start + 20]
                    .try_into()
                    .unwrap()
            ) as usize,
            central,
        );
        assert_eq!(
            u32::from_le_bytes(
                bytes[footer_start + 12..footer_start + 16]
                    .try_into()
                    .unwrap()
            ) as usize,
            footer_start - central,
        );
        bytes
    };
    // The zero-sized 0x5455 field has one following byte. The old debug path
    // could consume it as flags and subtract from zero. Release already returned
    // an error; this test makes no claim of a historical release failure.
    let valid = inject([0x55, 0x54, 0x01, 0x00, 0x00]);
    let malformed = inject([0x55, 0x54, 0x00, 0x00, 0x00]);
    assert_eq!(valid.len(), malformed.len());
    assert_eq!(
        valid
            .iter()
            .zip(&malformed)
            .enumerate()
            .filter_map(|(index, (a, b))| (a != b).then_some(index))
            .collect::<Vec<_>>(),
        [extra_start + 2],
    );
    TimestampZip {
        baseline,
        valid,
        malformed,
        central,
        extra_start,
    }
}

fn shard(html: &str) -> String {
    // Encode the inverse of the recorded content permutation. The mandatory
    // decoder calibration below prevents a broken shard from faking a ZIP error.
    let mut encoded = base64::engine::general_purpose::STANDARD
        .encode(html)
        .into_bytes();
    let length = encoded.len();
    let n = 4.min(length.div_ceil(10));
    let tail = encoded[length - n..]
        .iter()
        .rev()
        .map(|byte| {
            let mut radix_four = 0u64;
            let mut begun = false;
            for bit in (0..8).rev() {
                let digit = (byte >> bit) & 1;
                if digit != 0 {
                    begun = true;
                }
                if begun {
                    radix_four = radix_four * 4 + u64::from(digit);
                }
            }
            radix_four.to_string()
        })
        .collect::<String>();
    let modulus = length - n - 2;
    let step = modulus.to_string().len();
    let mut positions = Vec::new();
    let mut index = 0;
    while positions.len() < 10 && index + step < tail.len() {
        positions.push(tail[index..index + step].parse::<usize>().unwrap() % modulus);
        positions.push(
            tail[index + 1..(index + step + 1).min(tail.len())]
                .parse::<usize>()
                .unwrap()
                % modulus,
        );
        index += step;
    }
    for pair in positions.chunks_exact(2) {
        for offset in 0..=1 {
            encoded.swap(pair[0] + offset, pair[1] + offset);
        }
    }
    let body = format!("a{}", String::from_utf8(encoded).unwrap());
    format!("{:X}{body}", md5::compute(body.as_bytes()))
}

fn reader_state() -> String {
    format!(
        "<script>window.__INITIAL_STATE__ = {};</script>",
        json!({
            "user":{"vid":VID},
            "reader":{
                "bookId":BOOK,"psvts":"zip-worker-clock","pclts":0,
                "isBookInfoError":false,"isBookForbidden":false,
                "bookInfo":{"bookId":BOOK,"payType":32,"paid":0},
                "chapterInfos":[{"chapterUid":UID,"chapterIdx":0,"wordCount":3,"paid":0,"price":0}],
                "currentChapter":{"chapterUid":UID,"chapterIdx":0,"wordCount":3,"paid":0,"price":0}
            }
        })
    )
}

struct WorkerHttpFixture {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl WorkerHttpFixture {
    fn new(archive: Vec<u8>, encoded: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let archive = Arc::new(archive);
        let encoded = Arc::new(encoded);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut children = Vec::new();
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        assert!(
                            children.len() < 20,
                            "The fixture made unexpected additional requests"
                        );
                        let records = captured.clone();
                        let archive = archive.clone();
                        let encoded = encoded.clone();
                        children.push(thread::spawn(move || {
                            let request = read_request(&mut stream);
                            let body = match request.path.as_str() {
                                "/gateway" => serde_json::to_vec(&json!({"bookId":BOOK,"title":"Worker book"})).unwrap(),
                                "/web/book/chapterInfos" => serde_json::to_vec(&json!({
                                    "data":[{"bookId":BOOK,"updated":[{
                                        "chapterUid":UID,"chapterIdx":0,"wordCount":3,
                                        "title":"Worker chapter","tar":ARCHIVE_URL
                                    }]}]
                                })).unwrap(),
                                "/web/book/chapter/e_0" => encoded.as_bytes().to_vec(),
                                "/web/book/chapter/e_1" | "/web/book/chapter/e_3" => Vec::new(),
                                "/archive.zip" => archive.as_ref().clone(),
                                path if path.starts_with("/web/reader/") => reader_state().into_bytes(),
                                path => panic!("Unexpected fixture path: {path}"),
                            };
                            records.lock().unwrap().push(request);
                            write!(stream, "HTTP/1.1 200 Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                            stream.write_all(&body).unwrap();
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "The worker HTTP fixture did not stop"
                        );
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("Fixture listener failed: {error}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            address,
            stopped,
            records,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(auth()).unwrap();
        let origin = format!("http://{}", self.address);
        client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
            .no_proxy()
            .resolve("weread.qq.com", self.address)
            .resolve("i.weread.qq.com", self.address)
            .build()
            .unwrap();
        client.resource_http = http_client_builder(0)
            .no_proxy()
            .resolve("zip-fixture.invalid", self.address)
            .build()
            .unwrap();
        client.test_web_origin = Some(origin.clone());
        client.test_gateway_url = Some(format!("{origin}/gateway"));
        client.test_resource_origin = Some(origin);
        client
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        self.stopped.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        self.records.lock().unwrap().clone()
    }
}

impl Drop for WorkerHttpFixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.unwrap();
            }
        }
    }
}

#[test]
fn actual_image_download_worker_rejects_zero_length_timestamp_zip_and_releases_ownership() {
    let encoded = shard(HTML);
    assert_eq!(
        super::super::content::decode_shards(&[&encoded]).unwrap(),
        HTML
    );
    assert_eq!(super::super::content::html_to_text(HTML), TEXT);
    let archive = timestamp_zip();
    // Baseline and a same-sized len=1/flags=0 control prove the directory,
    // offsets, timestamp field and payload CRC are all reachable and valid.
    for (label, bytes, has_timestamp) in [
        ("baseline", archive.baseline.as_slice(), false),
        ("valid timestamp", archive.valid.as_slice(), true),
    ] {
        let mut parsed = zip::ZipArchive::new_with_file_limit(
            Cursor::new(bytes),
            super::super::images::MAX_ARCHIVE_ENTRIES,
        )
        .unwrap_or_else(|error| {
            panic!(
                "{label} ZIP calibration failed: {error:?}; central={}",
                archive.central
            )
        });
        assert_eq!(parsed.len(), 1);
        let mut entry = parsed.by_index(0).unwrap();
        let timestamp = entry.extra_data_fields().find_map(|field| match field {
            zip::extra_fields::ExtraField::ExtendedTimestamp(timestamp) => Some((
                timestamp.mod_time(),
                timestamp.ac_time(),
                timestamp.cr_time(),
            )),
            _ => None,
        });
        let expected: Option<(Option<u32>, Option<u32>, Option<u32>)> =
            has_timestamp.then_some((None, None, None));
        assert_eq!(timestamp, expected);
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"small archive member");
    }
    assert_eq!(
        u16::from_le_bytes(
            archive.malformed[archive.extra_start..archive.extra_start + 2]
                .try_into()
                .unwrap()
        ),
        0x5455
    );
    let length = u16::from_le_bytes(
        archive.malformed[archive.extra_start + 2..archive.extra_start + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(length, 0);
    let mut timestamp =
        Cursor::new(&archive.malformed[archive.extra_start + 4..archive.extra_start + 5]);
    let field_error = zip::extra_fields::ExtendedTimestamp::try_from_reader(&mut timestamp, length);
    assert!(
        matches!(
            &field_error,
            Err(zip::result::ZipError::UnsupportedArchive(TIMESTAMP_ERROR))
        ),
        "The declared malformed timestamp returned another field error: {field_error:?}",
    );
    assert_eq!(timestamp.position(), 0);
    let malformed = archive.malformed;
    let error = zip::ZipArchive::new_with_file_limit(
        Cursor::new(malformed.as_slice()),
        super::super::images::MAX_ARCHIVE_ENTRIES,
    )
    .err()
    .expect("The malformed timestamp must reach the ZIP parser");
    // The pinned archive constructor discards a rejected CDF candidate and
    // searches before its EOCD. Its final error is distinct from the direct
    // field error above; accepting an arbitrary ZIP error would hide a bad fixture.
    assert!(
        matches!(&error, zip::result::ZipError::InvalidArchive(reason) if reason.as_ref() == "Could not find EOCD"),
        "The malformed timestamp had an unexpected constructor error: {error:?}; central={}, extra_start={}",
        archive.central,
        archive.extra_start,
    );
    let fixture = WorkerHttpFixture::new(malformed, encoded);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut client = {
        let _context = runtime.enter();
        fixture.client()
    };
    let before = client.session();
    let referer = super::super::protocol::reader_url(BOOK, Some(UID));
    let resource = resource_url(ARCHIVE_URL).unwrap();
    let request = client
        .resource_request(&resource, &referer, true)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.url().scheme(), "http");
    assert_eq!(request.url().host_str(), Some("127.0.0.1"));
    assert_eq!(request.url().path(), "/archive.zip");
    assert!(!request.headers().contains_key("Cookie"));
    assert!(matches!(
        client.resource_request(
            &reqwest::Url::parse("http://127.0.0.1/archive.zip").unwrap(),
            &referer,
            false
        ),
        Err(SourceError::InvalidResponse("resource_url"))
    ));
    let original_origin = client.test_resource_origin.clone();
    for origin in [
        "http://example.invalid",
        "https://127.0.0.1",
        "http://127.0.0.1/not-an-origin",
    ] {
        client.test_resource_origin = Some(origin.into());
        assert!(matches!(
            client.resource_request(&resource, &referer, false),
            Err(SourceError::InvalidResponse("resource_test_origin"))
        ));
    }
    client.test_resource_origin = original_origin;
    // This actual gateway call calibrates the real first-party HTTP route;
    // the later archive GET is separately required in the captured workload.
    let book = runtime.block_on(client.book_info(BOOK)).unwrap();
    assert_eq!(book.id, BOOK);
    assert_eq!(book.title, "Worker book");

    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    let account_key = owner();
    store
        .upsert_book(&Book {
            id: BOOK.into(),
            title: "Original book".into(),
            source: "weread".into(),
            total_chapters: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: UID.into(),
            book_id: BOOK.into(),
            index: 0,
            title: "Original chapter".into(),
            cached: true,
            body: Some(TEXT.into()),
            content_status: ContentStatus::Full,
            content_account_key: account_key.clone(),
        })
        .unwrap();
    let task = DownloadTask {
        id: "zip-timestamp-worker-task".into(),
        book_id: BOOK.into(),
        account_key,
        chapters: vec![0],
        status: DownloadStatus::Running,
        include_images: true,
        next_index: 0,
        updated_at: 17,
        ..Default::default()
    };
    store.save_task(&task).unwrap();
    let original_chapter = store.chapter(BOOK, 0).unwrap().unwrap();
    let original_book = store.book(BOOK).unwrap().unwrap();
    let covers = FileCache::open(directory.path().join("covers"), 4096).unwrap();
    let cover_path = covers
        .put("old-cover", b"Preserve existing cache bytes")
        .unwrap();
    let cover_bytes = std::fs::read(&cover_path).unwrap();
    let cover_usage = covers.usage().unwrap();

    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let image_id = hex::encode(Sha256::digest(&png));
    let downloaded = DownloadedChapterImages {
        assets: vec![ImageAsset {
            id: image_id.clone(),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["image.png".into()],
            bytes: png,
        }],
        source_to_asset: BTreeMap::from([("image.png".into(), image_id)]),
    };
    let assets = BookAssetStore::open(directory.path().join("images"), 128 * 1024).unwrap();
    let manifest = assets
        .commit_chapter(&store, BOOK, UID, HTML, TEXT, Some(&downloaded))
        .unwrap();
    assert!(manifest.complete);
    let image_path = assets.image_path(&manifest.images[0]).unwrap().unwrap();
    let image_bytes = std::fs::read(&image_path).unwrap();
    let image_usage = assets.usage().unwrap();
    assert!(cover_usage > 0 && image_usage > 0);

    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    for request in [77, 78] {
        let headers = store.chapter_headers(BOOK).unwrap();
        let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
        assert!(jobs.download(request, task.clone(), plan).is_none());
        let terminal = receiver
            .recv_timeout(Duration::from_secs(8))
            .expect("The genuine ZIP worker did not emit its terminal result");
        assert_eq!(terminal.request, request);
        assert_eq!(terminal.account_epoch, jobs.account_epoch());
        assert!(terminal.credits.is_some());
        match &terminal.result {
            JobResult::DownloadFailed {
                task_id,
                error: SourceError::InvalidResponse("chapter_zip"),
            } => {
                assert_eq!(task_id, &task.id);
            }
            other => panic!("The actual archive producer returned another path: {other:?}"),
        }
        assert!(jobs.active_download().is_none());
        assert!(
            receiver.try_recv().is_err(),
            "A rejected archive published a chapter or progress packet"
        );
        assert_eq!(store.task(&task.id).unwrap().unwrap(), task);
        assert_eq!(store.chapter(BOOK, 0).unwrap().unwrap(), original_chapter);
        assert_eq!(store.book(BOOK).unwrap().unwrap(), original_book);
        assert_eq!(client.session(), before);
        assert_eq!(jobs.session().unwrap(), before);
        assert_eq!(covers.usage().unwrap(), cover_usage);
        assert_eq!(std::fs::read(&cover_path).unwrap(), cover_bytes);
        assert_eq!(assets.usage().unwrap(), image_usage);
        assert_eq!(
            assets.chapter(&store, BOOK, UID).unwrap().unwrap(),
            manifest
        );
        assert_eq!(std::fs::read(&image_path).unwrap(), image_bytes);
        drop(terminal);
        // The second actual task above needs the first terminal's packet credit
        // to be released. A leaked credit fails its finite terminal deadline.
    }
    let requests = fixture.finish();
    assert_eq!(requests.len(), 13);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/gateway")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/archive.zip")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/book/chapterInfos")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path.starts_with("/web/reader/"))
            .count(),
        2
    );
    for path in [
        "/web/book/chapter/e_0",
        "/web/book/chapter/e_1",
        "/web/book/chapter/e_3",
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == path)
                .count(),
            2
        );
    }
    for request in &requests {
        if request.path == "/gateway" {
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(payload["api_name"], "/book/info");
            assert_eq!(payload["bookId"], BOOK);
            assert!(!request.headers.contains_key("cookie"));
        } else if request.path == "/archive.zip" {
            assert_eq!(request.method, "GET");
            assert_eq!(
                request.headers.get("referer").map(String::as_str),
                Some(referer.as_str())
            );
            assert!(!request.headers.contains_key("cookie"));
        } else {
            assert!(
                request
                    .headers
                    .get("cookie")
                    .unwrap()
                    .contains("synthetic-zip-worker-session")
            );
        }
    }
}
