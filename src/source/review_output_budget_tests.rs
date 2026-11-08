//! Borrowed review output admission and genuine single-range HTTP producers.

use super::*;
use std::{
    cell::Cell,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

std::thread_local! {
    static MATERIALIZATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn observe_materialization() {
    MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
}

fn reset_materializations() {
    MATERIALIZATIONS.with(|count| count.set(0));
}

fn materializations() -> usize {
    MATERIALIZATIONS.with(Cell::get)
}

pub(super) fn permanent_capacity_error(error: &SourceError) {
    assert!(matches!(error, SourceError::ResponseTooLarge));
    assert!(!error.retryable());
    assert!(!error.requires_login());
}

fn normalize(value: &Value, limits: ReviewOutputLimits) -> Result<Vec<RemoteReview>> {
    normalize_review_candidates(
        review_records(value).iter(),
        &mut ReviewOutputBudget::new(limits),
    )
}

fn tiny_review(id: &str, text: &str) -> Value {
    json!({"reviewId":id,"author":{"nick":"Fixture author"},"content":text})
}

#[test]
fn review_aliases_numeric_fields_and_three_wrappers_keep_their_selection() {
    let value = json!({"reviews":[
        {"review":{"review":{"review":{
            "reviewId":17,"author":{"nick":21,"name":"unused"},
            "content":"","htmlContent":"<p>A&amp;B</p>","text":"unused",
            "likesCount":4,"createTime":123,"star":40
        }}}},
        {"id":"second","author":"Direct author","content":0,"htmlContent":"unused"},
        {}
    ]});
    reset_materializations();
    let output = normalize(&value, ReviewOutputLimits::default()).unwrap();
    assert_eq!(output.len(), 3);
    assert_eq!(output[0].id, "17");
    assert_eq!(output[0].author, "21");
    assert_eq!(output[0].text, "A&B");
    assert_eq!(output[0].likes, 4);
    assert_eq!(output[0].published_at, 123);
    assert!(output[0].recommended);
    assert_eq!(output[1].text, "0");
    assert_eq!(output[1].author, "Direct author");
    assert!(output[2].id.is_empty() && output[2].text.is_empty());
    assert_eq!(materializations(), 3);
}

#[test]
fn review_first_empty_alias_and_empty_page_arrays_keep_their_original_meaning() {
    let value = json!({"reviews":[
        {"reviewId":"","id":"ignored","author":{"nick":"","name":"ignored"},"content":"kept"},
        {"pageReviews":[],"reviewId":"must-not-fall-back","content":"ignored"},
        {"pageReviews":[{"id":"page","text":"Page text"}]},
        {"pageReviews":null,"id":"outer","content":"Outer text"}
    ]});
    reset_materializations();
    let output = normalize_review_candidates(
        thought_review_records(&value),
        &mut ReviewOutputBudget::new(ReviewOutputLimits::default()),
    )
    .unwrap();
    assert_eq!(
        output
            .iter()
            .map(|review| review.id.as_str())
            .collect::<Vec<_>>(),
        ["", "page", "outer"]
    );
    assert!(output[0].author.is_empty());
    assert_eq!(output[0].text, "kept");
    assert_eq!(materializations(), 3);
}

#[test]
fn missing_non_array_and_empty_review_outputs_create_no_records() {
    reset_materializations();
    for value in [
        json!({}),
        json!({"reviews":null}),
        json!({"reviews":{}}),
        json!({"reviews":[]}),
    ] {
        assert!(
            normalize(&value, ReviewOutputLimits::default())
                .unwrap()
                .is_empty()
        );
        assert!(
            normalize_review_candidates(
                thought_review_records(&value),
                &mut ReviewOutputBudget::new(ReviewOutputLimits::default()),
            )
            .unwrap()
            .is_empty()
        );
    }
    assert_eq!(materializations(), 0);
}

#[test]
fn actual_review_id_author_and_text_byte_boundaries_precede_copy_or_html_conversion() {
    for (field, maximum) in [
        ("reviewId", MAX_REVIEW_ID_BYTES),
        ("author", MAX_REVIEW_AUTHOR_BYTES),
        ("content", MAX_REVIEW_TEXT_BYTES),
    ] {
        let mut record = tiny_review("id", "Small text");
        record[field] = json!("x".repeat(maximum));
        let value = json!({"reviews":[record.clone()]});
        reset_materializations();
        assert_eq!(
            normalize(&value, ReviewOutputLimits::default())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(materializations(), 1);
        record[field] = json!("x".repeat(maximum + 1));
        let value = json!({"reviews":[tiny_review("first", "Keep first"), record]});
        reset_materializations();
        permanent_capacity_error(&normalize(&value, ReviewOutputLimits::default()).unwrap_err());
        assert_eq!(
            materializations(),
            0,
            "A later field failure published earlier output"
        );
    }
}

#[test]
fn selected_text_is_borrowed_and_unused_fallback_text_does_not_consume_its_limit() {
    let value = json!({"reviews":[{
        "id":"id","content":"Selected","htmlContent":"Unused long fallback",
        "text":"Another unused fallback"
    }]});
    let selected = review_fields(&value["reviews"][0]);
    assert!(matches!(selected.text, Cow::Borrowed("Selected")));
    reset_materializations();
    let output = normalize(
        &value,
        ReviewOutputLimits {
            text_bytes: 8,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output[0].text, "Selected");
    assert_eq!(materializations(), 1);
}

#[test]
fn review_count_admission_includes_empty_items_before_any_vector_materialization() {
    for count in [MAX_REVIEW_OUTPUT_RECORDS, MAX_REVIEW_OUTPUT_RECORDS + 1] {
        let value = json!({"reviews":vec![Value::Null; count]});
        reset_materializations();
        let result = normalize(&value, ReviewOutputLimits::default());
        if count == MAX_REVIEW_OUTPUT_RECORDS {
            assert_eq!(result.unwrap().len(), count);
            assert_eq!(materializations(), count);
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(materializations(), 0);
        }
    }
}

#[test]
fn complete_review_work_fee_accepts_exact_and_rejects_one_less_without_partial_output() {
    let value =
        json!({"reviews":[tiny_review("one", "<p>One</p>"), tiny_review("two", "Two\nlines")]});
    let mut calibration = ReviewOutputBudget::new(ReviewOutputLimits::default());
    for record in review_records(&value) {
        calibration.admit(record).unwrap();
    }
    let fee = calibration.work_bytes;
    for allowance in [fee, fee - 1] {
        reset_materializations();
        let result = normalize(
            &value,
            ReviewOutputLimits {
                work_bytes: allowance,
                ..Default::default()
            },
        );
        if allowance == fee {
            let output = result.unwrap();
            assert_eq!(output[0].text, "One");
            assert_eq!(output[1].text, "Two\nlines");
            assert_eq!(materializations(), 2);
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(materializations(), 0);
        }
    }
}

#[test]
fn numeric_review_strings_pay_scan_work_before_scalar_conversion_or_output() {
    let record = tiny_review("id", "Text");
    let mut baseline = ReviewOutputBudget::new(ReviewOutputLimits::default());
    baseline.admit(&record).unwrap();
    let mut with_numbers = record.clone();
    let likes = format!("{}1", "0".repeat(40));
    let time = format!("{}123", "0".repeat(40));
    with_numbers["likesCount"] = json!(likes);
    with_numbers["createTime"] = json!(time);
    let mut calibration = ReviewOutputBudget::new(ReviewOutputLimits::default());
    calibration.admit(&with_numbers).unwrap();
    assert_eq!(calibration.work_bytes - baseline.work_bytes, 2 * (41 + 43));
    let value = json!({"reviews":[with_numbers]});
    reset_materializations();
    permanent_capacity_error(
        &normalize(
            &value,
            ReviewOutputLimits {
                work_bytes: calibration.work_bytes - 1,
                ..Default::default()
            },
        )
        .unwrap_err(),
    );
    assert_eq!(materializations(), 0);
    let output = normalize(&value, ReviewOutputLimits::default()).unwrap();
    assert_eq!(output[0].likes, 1);
    assert_eq!(output[0].published_at, 123);
}

#[test]
fn one_review_operation_keeps_count_and_work_spent_across_borrowed_responses() {
    // This is a pure operation-budget case, not a multi-range HTTP experiment.
    let first = json!({"reviews":[tiny_review("one", "One"), {}]});
    let second = json!({"reviews":[tiny_review("two", "Two"), {}]});
    let mut budget = ReviewOutputBudget::new(ReviewOutputLimits {
        records: 3,
        ..Default::default()
    });
    normalize_review_candidates(review_records(&first).iter(), &mut budget).unwrap();
    let spent = budget.work_bytes;
    reset_materializations();
    permanent_capacity_error(
        &normalize_review_candidates(review_records(&second).iter(), &mut budget).unwrap_err(),
    );
    assert!(budget.work_bytes > spent);
    assert_eq!(budget.records, 3);
    assert_eq!(materializations(), 0);
    assert_eq!(
        ReviewOutputBudget::new(ReviewOutputLimits {
            records: usize::MAX,
            text_bytes: usize::MAX,
            work_bytes: usize::MAX,
            ..Default::default()
        })
        .limits
        .records,
        MAX_REVIEW_OUTPUT_RECORDS
    );
}

#[derive(Clone, Debug)]
pub(super) struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(super) fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut fragment = [0; 2048];
    let header_end = loop {
        let read = stream.read(&mut fragment).unwrap();
        assert!(read > 0, "Incomplete fixture headers");
        assert!(
            bytes.len() + read <= 65536,
            "Fixture request exceeds its allowance"
        );
        bytes.extend_from_slice(&fragment[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let path = first.next().unwrap().to_owned();
    let headers: BTreeMap<_, _> = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    assert!(header_end + length <= 65536);
    while bytes.len() < header_end + length {
        let read = stream.read(&mut fragment).unwrap();
        assert!(read > 0);
        assert!(bytes.len() + read <= 65536);
        bytes.extend_from_slice(&fragment[..read]);
    }
    RecordedRequest {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    }
}

pub(super) struct GatewayFixture {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl GatewayFixture {
    pub fn new(replies: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            for reply in replies {
                let mut stream = loop {
                    if stopped.load(Ordering::Acquire) {
                        return;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Missing fixture gateway request");
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("Fixture listener failed: {error}"),
                    }
                };
                captured.lock().unwrap().push(read_request(&mut stream));
                let body = serde_json::to_vec(&reply).unwrap();
                write!(stream, "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            address,
            stop,
            records,
            worker: Some(worker),
        }
    }

    pub fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(AuthSession {
            api_key: "synthetic-output-budget-key".into(),
            user_vid: "synthetic-output-budget-owner".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "synthetic-output-budget-owner".into()),
                ("wr_skey".into(), "synthetic-output-budget-session".into()),
            ]),
            ..Default::default()
        })
        .unwrap();
        client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
            .no_proxy()
            .resolve("weread.qq.com", self.address)
            .resolve("i.weread.qq.com", self.address)
            .build()
            .unwrap();
        client.test_gateway_url = Some(format!("http://{}/gateway", self.address));
        client
    }

    pub fn finish(mut self) -> Vec<RecordedRequest> {
        self.worker.take().unwrap().join().unwrap();
        self.records.lock().unwrap().clone()
    }
}

impl Drop for GatewayFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.unwrap();
            }
        }
    }
}

pub(super) fn assert_gateway(request: &RecordedRequest, name: &str) -> Value {
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/gateway");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer synthetic-output-budget-key")
    );
    assert!(!request.headers.contains_key("cookie"));
    let payload: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(payload["api_name"], name);
    payload
}

#[tokio::test(flavor = "current_thread")]
async fn actual_review_gateway_rejection_preserves_auth_and_stops_before_materialization() {
    let fixture = GatewayFixture::new(vec![
        json!({"reviews":[tiny_review("ok", "Small"), tiny_review("late", "Oversized")]}),
    ]);
    let client = fixture.client();
    let before = client.session();
    reset_materializations();
    permanent_capacity_error(
        &client
            .reviews_with_output_limits(
                "book",
                true,
                ReviewOutputLimits {
                    text_bytes: 5,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err(),
    );
    assert_eq!(client.session(), before);
    assert_eq!(materializations(), 0);
    let records = fixture.finish();
    assert_eq!(records.len(), 1);
    let payload = assert_gateway(&records[0], "/review/list");
    assert_eq!(payload["reviewListType"], 3);
    assert_eq!(payload["count"], 30);
}

#[tokio::test(flavor = "current_thread")]
async fn actual_single_range_thought_response_counts_all_page_reviews_before_copying() {
    let fixture = GatewayFixture::new(vec![
        json!({"reviews":[{"pageReviews":vec![Value::Null; MAX_REVIEW_OUTPUT_RECORDS + 1]}]}),
    ]);
    let client = fixture.client();
    let before = client.session();
    reset_materializations();
    permanent_capacity_error(
        &client
            .thoughts("book", "17", &["one-real-range".into()])
            .await
            .unwrap_err(),
    );
    assert_eq!(client.session(), before);
    assert_eq!(materializations(), 0);
    let records = fixture.finish();
    assert_eq!(records.len(), 1);
    let payload = assert_gateway(&records[0], "/book/readreviews");
    assert_eq!(payload["chapterUid"], 17);
    assert_eq!(payload["reviews"].as_array().unwrap().len(), 1);
    assert_eq!(payload["reviews"][0]["range"], "one-real-range");
    assert_eq!(payload["reviews"][0]["count"], 30);
}

#[test]
fn actual_queued_review_capacity_error_keeps_metadata_credit_until_consumed() {
    use crate::jobs::{JobResult, Jobs};
    use std::sync::mpsc;
    let fixture = GatewayFixture::new(vec![
        json!({"reviews":vec![Value::Null; MAX_REVIEW_OUTPUT_RECORDS + 1]}),
    ]);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let client = {
        let _context = runtime.enter();
        fixture.client()
    };
    let before = client.session();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    let available = jobs.metadata_capacity();
    assert_eq!(available, 8);
    assert!(jobs.reviews(71, "book".into(), false).is_none());
    let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(event.request, 71);
    assert!(event.credits.is_some());
    match &event.result {
        JobResult::Reviews {
            result: Err(error), ..
        } => permanent_capacity_error(error),
        other => panic!("The genuine review producer did not reject capacity: {other:?}"),
    }
    assert_eq!(jobs.metadata_capacity(), available - 1);
    assert_eq!(client.session(), before);
    drop(event);
    assert_eq!(jobs.metadata_capacity(), available);
    assert!(receiver.try_recv().is_err());
    let records = fixture.finish();
    assert_eq!(records.len(), 1);
    assert_gateway(&records[0], "/review/list");
}
