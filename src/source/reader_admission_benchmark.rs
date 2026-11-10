//! Controlled Reader admission comparison using synthetic loopback responses.
//!
//! The one-page case retains the previous 24 MiB exclusive admission shape.
//! All cases use the current parser and fresh chapter-specific Reader requests.
//! Timings describe this fixture only; reservations are not process RSS.

use super::*;
use crate::download_diagnostics::DownloadDiagnostics;
use crate::source::reader_admission::{ReaderAdmission, ReaderTier};

const MEASURED_CHAPTERS: usize = 16;
const WARMUP_CHAPTER: usize = MEASURED_CHAPTERS;
const RESPONSE_TIER_BYTES: usize = 2 * 1024 * 1024;
const TARGET_READER_BYTES: usize = 1_415_578;
const READER_RESPONSE_DELAY: Duration = Duration::from_millis(90);
const COMPLETE_WAIT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct ReaderHttpState {
    active: usize,
    peak: usize,
    arrivals: BTreeSet<usize>,
    released: bool,
}

#[derive(Default)]
struct ReaderHttpGate {
    state: Mutex<ReaderHttpState>,
    changed: Condvar,
}

impl ReaderHttpGate {
    fn enter(self: &Arc<Self>, index: usize) -> HeldReaderHttp {
        let mut state = self.state.lock().unwrap();
        assert!(
            state.arrivals.insert(index),
            "The warmed response tier must acquire each measured Reader once"
        );
        state.active += 1;
        state.peak = state.peak.max(state.active);
        self.changed.notify_all();
        let deadline = Instant::now() + SHARD_HOLD_WAIT;
        while !state.released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "The Reader response gate was not released"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        HeldReaderHttp(self.clone())
    }

    fn wait_for_first_wave(&self, eligible: usize) {
        let deadline = Instant::now() + CASE_WAIT;
        let mut state = self.state.lock().unwrap();
        while state.active < eligible {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "The eligible Reader HTTP wave did not start"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        assert_eq!(state.active, eligible);
        assert_eq!(state.peak, eligible);
    }

    fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .released = true;
        self.changed.notify_all();
    }
}

/// This interval covers an actual received HTTP request before its response
/// headers are written. The first-wave barrier proves the reported peak is
/// attained, while the production byte quota bounds all remaining intervals.
struct HeldReaderHttp(Arc<ReaderHttpGate>);

impl Drop for HeldReaderHttp {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

struct CancelAndReleaseReaders {
    worker: tokio::task::AbortHandle,
    gate: Arc<ReaderHttpGate>,
}

impl Drop for CancelAndReleaseReaders {
    fn drop(&mut self) {
        self.worker.abort();
        self.gate.release();
    }
}

fn benchmark_catalog() -> Vec<Value> {
    let mut chapters = catalog();
    let base_bytes = large_reader(0, &chapters).len();
    assert!(base_bytes < TARGET_READER_BYTES);
    // Increase genuine per-chapter metadata, including currentChapter, rather
    // than giving the Reader parser a large irrelevant whitespace suffix.
    let extra_title_bytes = (TARGET_READER_BYTES - base_bytes) / (chapters.len() + 1);
    let extra_title =
        "synthetic metadata ".repeat(extra_title_bytes.div_ceil("synthetic metadata ".len()));
    for chapter in &mut chapters {
        chapter["title"] = Value::String(format!(
            "{}{}",
            chapter["title"].as_str().unwrap(),
            &extra_title[..extra_title_bytes],
        ));
    }
    chapters
}

struct ReaderComparison {
    eligible: usize,
    legacy_one_page: bool,
    elapsed_ms: u128,
    decoded_page_min_bytes: usize,
    decoded_page_max_bytes: usize,
    reader_http_peak: usize,
    peak_reserved_bytes: usize,
    peak_response_allowance_bytes: usize,
    peak_parser_workers: usize,
}

impl ReaderComparison {
    fn report(&self) {
        println!(
            "reader_fixture legacy_one_page={} eligible_reader_http={} chapter_workers={} completed_chapters={} decoded_page_min_bytes={} decoded_page_max_bytes={} response_delay_ms={} elapsed_ms={} reader_http_peak={} peak_reserved_bytes={} peak_response_allowance_bytes={} peak_parser_workers={}",
            u8::from(self.legacy_one_page),
            self.eligible,
            WINDOW,
            MEASURED_CHAPTERS,
            self.decoded_page_min_bytes,
            self.decoded_page_max_bytes,
            READER_RESPONSE_DELAY.as_millis(),
            self.elapsed_ms,
            self.reader_http_peak,
            self.peak_reserved_bytes,
            self.peak_response_allowance_bytes,
            self.peak_parser_workers,
        );
    }
}

fn compare_case(eligible: usize, legacy_one_page: bool) -> ReaderComparison {
    assert!(matches!(eligible, 1 | 2 | 4 | 8));
    assert_eq!(legacy_one_page, eligible == 1);
    let chapters = benchmark_catalog();
    let mut decoded_page_min_bytes = usize::MAX;
    let mut decoded_page_max_bytes = 0;
    let pages = (0..=WARMUP_CHAPTER)
        .map(|index| {
            let html = large_reader(index, &chapters);
            assert!(html.len().abs_diff(TARGET_READER_BYTES) <= CATALOG_LEN + 64);
            decoded_page_min_bytes = decoded_page_min_bytes.min(html.len());
            decoded_page_max_bytes = decoded_page_max_bytes.max(html.len());
            let reply = Reply::gzip(html);
            assert!(reply.body.len() < SMALL_RESPONSE_LIMIT);
            (reader_path(index), (index, reply.body))
        })
        .collect::<BTreeMap<_, _>>();
    let gate = Arc::new(ReaderHttpGate::default());
    let observed = gate.clone();
    let fixture = bounded_fixture(move |request| {
        if let Some((index, bytes)) = pages.get(&request.path) {
            if *index == WARMUP_CHAPTER {
                return compressed_reply(bytes);
            }
            let _http = observed.enter(*index);
            thread::sleep(READER_RESPONSE_DELAY);
            return compressed_reply(bytes);
        }
        let index = clock(request)
            .strip_prefix("large-catalog-clock-")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(index <= WARMUP_CHAPTER);
        match request.path.as_str() {
            "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
            "/web/book/chapter/t_0" => Reply::text(TXT_SHARD),
            "/web/book/chapter/t_1" => Reply::status(404),
            _ => panic!("Unexpected synthetic comparison request"),
        }
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let client = fixture_client(&fixture);
    let budget_bytes = if legacy_one_page {
        16 * 1024 * 1024
    } else {
        eligible * RESPONSE_TIER_BYTES
    };
    let admission = Arc::new(ReaderAdmission::for_tests(budget_bytes, 2));
    let diagnostics = DownloadDiagnostics::new(WINDOW as u32, MEASURED_CHAPTERS, false);
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(admission.clone());
    hints.set_download_diagnostics(diagnostics.clone());
    // A real warmup learns only response size and format. The timed requests
    // still fetch and parse distinct Reader sessions for every chapter.
    let warmup = runtime
        .block_on(async {
            tokio::time::timeout(COMPLETE_WAIT, async {
                client
                    .chapter_prefetch_with_hints(BOOK, &chapter_id(WARMUP_CHAPTER), &mut hints)
                    .await
            })
            .await
        })
        .unwrap()
        .unwrap();
    assert_eq!(warmup.content_status, ContentStatus::Full);
    assert_eq!(warmup.plain_text, "Hello, reader!");
    drop(warmup);
    let reader = hints.download_reader.as_ref().unwrap();
    assert_eq!(reader.tier().bytes(), RESPONSE_TIER_BYTES);
    if legacy_one_page {
        // Model the previous one-page admission policy using the same source
        // implementation; this is not a measurement of an older binary.
        while reader.tier() != ReaderTier::FULL {
            assert!(reader.advance(reader.tier()).is_some());
        }
    }
    let began = Instant::now();
    let worker = runtime.spawn(async move {
        let mut workers = tokio::task::JoinSet::new();
        for lane in 0..WINDOW {
            let client = client.clone();
            let mut hints = hints.clone();
            workers.spawn(async move {
                let mut completed = Vec::new();
                for index in (lane..MEASURED_CHAPTERS).step_by(WINDOW) {
                    let content = client
                        .chapter_prefetch_with_hints(BOOK, &chapter_id(index), &mut hints)
                        .await
                        .unwrap();
                    assert_eq!(content.content_status, ContentStatus::Full);
                    assert_eq!(content.plain_text, "Hello, reader!");
                    completed.push(index);
                }
                completed
            });
        }
        let mut completed = Vec::new();
        while let Some(result) = workers.join_next().await {
            completed.extend(result.unwrap());
        }
        completed.sort_unstable();
        completed
    });
    let cleanup = CancelAndReleaseReaders {
        worker: worker.abort_handle(),
        gate: gate.clone(),
    };
    gate.wait_for_first_wave(eligible);
    let held = admission.snapshot();
    assert_eq!(held.pages_active, eligible);
    assert_eq!(held.reserved_bytes, budget_bytes);
    assert_eq!(held.available_bytes, 0);
    assert_eq!(held.parsers_active, 0);
    // These are actual client HTTP futures at the server's pre-response barrier,
    // not configured workers, completed chapter bodies, or queued results.
    assert_eq!(diagnostics.activity_snapshot().2, eligible);
    gate.release();
    let completed = runtime
        .block_on(async { tokio::time::timeout(COMPLETE_WAIT, worker).await })
        .unwrap()
        .unwrap();
    let elapsed_ms = began.elapsed().as_millis();
    assert_eq!(completed, (0..MEASURED_CHAPTERS).collect::<Vec<_>>());
    drop(cleanup);
    let snapshot = admission.snapshot();
    assert_eq!(snapshot.budget_bytes, budget_bytes);
    assert_eq!(snapshot.available_bytes, budget_bytes);
    assert_eq!(snapshot.reserved_bytes, 0);
    assert_eq!(snapshot.response_allowance_bytes, 0);
    assert_eq!(snapshot.pages_active, 0);
    assert_eq!(snapshot.parsers_active, 0);
    assert_eq!(snapshot.peak_reserved_bytes, budget_bytes);
    assert_eq!(snapshot.peak_pages_active, eligible);
    assert!((1..=2).contains(&snapshot.peak_parsers_active));
    assert_eq!(
        snapshot.peak_response_allowance_bytes,
        if legacy_one_page {
            24 * 1024 * 1024
        } else {
            budget_bytes
        },
    );
    assert_eq!(diagnostics.activity_snapshot().2, 0);
    let reader_http_peak = {
        let state = gate.state.lock().unwrap();
        assert_eq!(state.active, 0);
        assert_eq!(state.peak, eligible);
        assert_eq!(state.arrivals.len(), MEASURED_CHAPTERS);
        state.peak
    };
    let requests = fixture.finish();
    assert_eq!(count(&requests, &reader_path(WARMUP_CHAPTER)), 2);
    for index in 0..MEASURED_CHAPTERS {
        assert_eq!(count(&requests, &reader_path(index)), 1);
        for endpoint in ["/web/book/chapter/t_0", "/web/book/chapter/t_1"] {
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| {
                        request.path == endpoint
                            && clock(request) == format!("large-catalog-clock-{index}")
                    })
                    .count(),
                1,
                "A chapter must use the clock from its own fresh Reader response",
            );
        }
    }
    assert_eq!(
        count(&requests, "/web/book/chapter/t_0"),
        MEASURED_CHAPTERS + 1
    );
    assert_eq!(
        count(&requests, "/web/book/chapter/t_1"),
        MEASURED_CHAPTERS + 1
    );
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 1);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
    ReaderComparison {
        eligible,
        legacy_one_page,
        elapsed_ms,
        decoded_page_min_bytes,
        decoded_page_max_bytes,
        reader_http_peak,
        peak_reserved_bytes: snapshot.peak_reserved_bytes,
        peak_response_allowance_bytes: snapshot.peak_response_allowance_bytes,
        peak_parser_workers: snapshot.peak_parsers_active,
    }
}

#[test]
fn adaptive_reader_budget_reaches_eight_real_http_requests() {
    compare_case(8, false).report();
}

#[test]
#[ignore = "Run explicitly in the designated remote verification environment"]
fn compare_one_page_and_two_four_eight_reader_admission() {
    for eligible in [1, 2, 4, 8] {
        compare_case(eligible, eligible == 1).report();
    }
}
