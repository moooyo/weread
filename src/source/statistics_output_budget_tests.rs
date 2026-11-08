//! Borrowed statistics output admission and genuine HTTP/queued producers.

use super::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use crate::jobs::{JobEvent, JobResult, Jobs};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Probe {
    borrowed_records: usize,
    map_admissions: usize,
    proof_records: usize,
    metadata_bytes: usize,
    work_bytes: usize,
    copied_keys: usize,
    copied_key_bytes: usize,
    minimum_copy_work: usize,
    summaries: usize,
}

thread_local! {
    static PROBE: Cell<Probe> = Cell::new(Probe::default());
}

pub(super) fn observe_borrowed_record() {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.borrowed_records += 1;
        slot.set(probe);
    });
}

pub(super) fn observe_map_admission(records: usize, metadata: usize, work: usize) {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.map_admissions += 1;
        probe.proof_records = records;
        probe.metadata_bytes = metadata;
        probe.work_bytes = work;
        slot.set(probe);
    });
}

pub(super) fn observe_key_copy(bytes: usize, records: usize, metadata: usize, work: usize) {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        assert_eq!(probe.proof_records, records);
        assert_eq!(probe.metadata_bytes, metadata);
        assert_eq!(probe.work_bytes, work);
        probe.minimum_copy_work = if probe.copied_keys == 0 {
            work
        } else {
            probe.minimum_copy_work.min(work)
        };
        probe.copied_keys += 1;
        probe.copied_key_bytes += bytes;
        slot.set(probe);
    });
}

pub(super) fn observe_summary_materialization() {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.summaries += 1;
        slot.set(probe);
    });
}

fn reset() {
    PROBE.with(|slot| slot.set(Probe::default()));
}

fn probe() -> Probe {
    PROBE.with(Cell::get)
}

fn normalize(value: &Value, limits: StatisticsOutputLimits) -> Result<ReadStatistics> {
    normalize_statistics(value.clone(), limits)
}

fn assert_capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert!(!SourceError::ResponseTooLarge.requires_login());
    assert!(!SourceError::ResponseTooLarge.retryable());
    assert_eq!(probe().map_admissions, 0);
    assert_eq!(probe().copied_keys, 0);
    assert_eq!(probe().copied_key_bytes, 0);
    assert_eq!(probe().summaries, 0);
}

#[test]
fn statistics_aliases_all_map_keys_ordering_and_raw_survive_the_complete_output_gate() {
    let value = json!({
        "totalReadTime":"4294967296","readDays":1.5,"dayAverageReadTime":"3.5","baseTime":"-123",
        "readTimes":{"z":"0007","1":u64::MAX,"":3,"10":7.0,"false":false,"bad":"7.5","negative":-1,"object":{"unused":9}},
        "unselected":{"history":[1,2,3],"label":"Preserved raw fixture"}
    });
    reset();
    let output = normalize(&value, StatisticsOutputLimits::default()).unwrap();
    assert_eq!(output.total_seconds, u64::from(u32::MAX));
    assert_eq!(output.read_days, 1);
    assert_eq!(output.average_seconds, 3);
    assert_eq!(output.base_time, -123);
    assert_eq!(output.daily_seconds["z"], 7);
    assert_eq!(output.daily_seconds["1"], u64::MAX);
    assert_eq!(output.daily_seconds[""], 3);
    for key in ["10", "false", "bad", "negative", "object"] {
        assert_eq!(output.daily_seconds[key], 0);
    }
    assert_eq!(
        output
            .daily_seconds
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["", "1", "10", "bad", "false", "negative", "object", "z"]
    );
    assert_eq!(output.raw, value);
    assert_eq!(probe().borrowed_records, 8);
    assert_eq!(probe().copied_keys, 8);
    assert_eq!(probe().summaries, 1);
    assert!(probe().metadata_bytes > probe().copied_key_bytes);
    assert!(probe().minimum_copy_work > probe().metadata_bytes);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("statistics.json");
    std::fs::write(&path, serde_json::to_vec(&output).unwrap()).unwrap();
    let restored: ReadStatistics = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(restored.daily_seconds, output.daily_seconds);
    assert_eq!(restored.raw, value);
}

#[test]
fn missing_or_non_object_read_times_keep_the_original_empty_map_and_summary_contract() {
    for times in [
        None,
        Some(Value::Null),
        Some(json!([])),
        Some(json!("legacy")),
        Some(json!({})),
    ] {
        let mut value = json!({"totalReadTime":17,"readDays":"3","dayAverageReadTime":5,"baseTime":123,"unselected":"Preserve me"});
        if let Some(times) = times {
            value["readTimes"] = times;
        }
        reset();
        let output = normalize(&value, StatisticsOutputLimits::default()).unwrap();
        assert!(output.daily_seconds.is_empty());
        assert_eq!(output.total_seconds, 17);
        assert_eq!(output.read_days, 3);
        assert_eq!(output.raw, value);
        assert_eq!(probe().copied_keys, 0);
        assert_eq!(probe().summaries, 1);
    }
}

#[test]
fn complete_count_rejection_precedes_every_key_copy_or_summary_parse() {
    let value = json!({"readTimes":{"1":1,"2":"2","3":3},"totalReadTime":"00017"});
    reset();
    assert_capacity(normalize(
        &value,
        StatisticsOutputLimits {
            records: 2,
            ..Default::default()
        },
    ));
    assert_eq!(probe().borrowed_records, 0);
    reset();
    let output = normalize(
        &value,
        StatisticsOutputLimits {
            records: 3,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output.daily_seconds.len(), 3);
    assert_eq!(probe().copied_keys, 3);
}

#[test]
fn exact_utf8_key_limit_and_later_one_over_are_checked_before_any_output() {
    let key = format!("\u{4e2d}{}", "x".repeat(MAX_STATISTICS_KEY_BYTES - 3));
    let mut times = serde_json::Map::new();
    times.insert(key.clone(), json!(7));
    let value = json!({"readTimes":times});
    reset();
    let output = normalize(&value, StatisticsOutputLimits::default()).unwrap();
    assert_eq!(output.daily_seconds[&key], 7);
    assert_eq!(probe().copied_key_bytes, MAX_STATISTICS_KEY_BYTES);
    let mut times = serde_json::Map::new();
    times.insert("first".into(), json!(1));
    times.insert(format!("{key}x"), json!(2));
    let value = json!({"readTimes":times});
    reset();
    assert_capacity(normalize(&value, StatisticsOutputLimits::default()));
    reset();
    assert_capacity(normalize(
        &value,
        StatisticsOutputLimits {
            records: usize::MAX,
            key_bytes: usize::MAX,
            metadata_bytes: usize::MAX,
            work_bytes: usize::MAX,
        },
    ));
}

#[test]
fn exact_output_metadata_forecast_is_observed_at_copy_and_one_less_rejects_atomically() {
    let value = json!({"readTimes":{"1":1,"22":"2"}});
    reset();
    normalize(&value, StatisticsOutputLimits::default()).unwrap();
    let metadata = probe().metadata_bytes;
    let work = probe().work_bytes;
    reset();
    let output = normalize(
        &value,
        StatisticsOutputLimits {
            metadata_bytes: metadata,
            work_bytes: work,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output.daily_seconds.len(), 2);
    assert_eq!(probe().minimum_copy_work, work);
    reset();
    assert_capacity(normalize(
        &value,
        StatisticsOutputLimits {
            metadata_bytes: metadata - 1,
            ..Default::default()
        },
    ));
    reset();
    assert_capacity(normalize(
        &value,
        StatisticsOutputLimits {
            work_bytes: work - 1,
            ..Default::default()
        },
    ));
}

#[test]
fn summary_and_bucket_numeric_string_work_is_admitted_before_map_or_count_fields() {
    let plain = json!({"readTimes":{"1":0},"totalReadTime":0});
    reset();
    normalize(&plain, StatisticsOutputLimits::default()).unwrap();
    let base = probe().work_bytes;
    for field in [
        "totalReadTime",
        "readDays",
        "dayAverageReadTime",
        "baseTime",
        "bucket",
    ] {
        let mut value = plain.clone();
        if field == "bucket" {
            value["readTimes"]["1"] = json!("0".repeat(64));
        } else {
            value[field] = json!("0".repeat(64));
        }
        reset();
        assert_capacity(normalize(
            &value,
            StatisticsOutputLimits {
                work_bytes: base + 127,
                ..Default::default()
            },
        ));
        reset();
        let output = normalize(
            &value,
            StatisticsOutputLimits {
                work_bytes: base + 128,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.daily_seconds["1"], 0);
        assert_eq!(probe().minimum_copy_work, base + 128);
    }
}

#[test]
fn shared_prefix_tree_comparison_forecast_grows_with_complete_record_count() {
    let value = json!({"readTimes":{"shared-prefix-a":1,"shared-prefix-b":2,"shared-prefix-c":3,"shared-prefix-d":4}});
    reset();
    normalize(&value, StatisticsOutputLimits::default()).unwrap();
    let complete = probe();
    let key_bytes = complete.copied_key_bytes;
    assert!(complete.work_bytes >= complete.metadata_bytes + key_bytes * 24);
    reset();
    assert_capacity(normalize(
        &value,
        StatisticsOutputLimits {
            work_bytes: complete.work_bytes - 1,
            ..Default::default()
        },
    ));
    assert_eq!(probe().borrowed_records, 4);
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_statistics_http_uses_the_gate_before_copy_and_preserves_normal_parameters() {
    let value = json!({"readTimes":{"1":1,"2":"2","3":3},"totalReadTime":6,"readDays":3,"unselected":"Keep raw"});
    let reply = value.clone();
    let fixture = HttpFixture::new(move |_| Reply::json(reply.clone()));
    let original = auth();
    let client = fixture.client(original.clone());
    reset();
    assert_capacity(
        client
            .statistics_with_output_limits(
                StatisticsPeriod::Month,
                Some(123),
                StatisticsOutputLimits {
                    records: 2,
                    ..Default::default()
                },
            )
            .await,
    );
    assert_eq!(client.session(), original);
    reset();
    let output = client
        .statistics(StatisticsPeriod::Total, Some(-1))
        .await
        .unwrap();
    assert_eq!(output.daily_seconds.len(), 3);
    assert_eq!(output.raw, value);
    assert_eq!(probe().copied_keys, 3);
    assert!(probe().minimum_copy_work > 0);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let first = requests[0].json();
    let second = requests[1].json();
    assert_eq!(first["api_name"], "/readdata/detail");
    assert_eq!(first["mode"], "monthly");
    assert_eq!(first["baseTime"], 123);
    assert_eq!(second["mode"], "overall");
    assert!(second.get("baseTime").is_none());
    assert!(
        requests
            .iter()
            .all(|request| request.method == "POST" && request.path() == "/gateway")
    );
}

async fn next_event(receiver: &std::sync::mpsc::Receiver<JobEvent>) -> JobEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(event) = receiver.try_recv() {
                break event;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn queued_statistics_capacity_error_keeps_credit_then_recovers_without_account_reset() {
    let calls = AtomicUsize::new(0);
    let valid = json!({"readTimes":{"1":"7"},"totalReadTime":7,"history":{"kept":true}});
    let reply = valid.clone();
    let fixture = HttpFixture::new(move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut times = serde_json::Map::new();
            times.insert("x".repeat(MAX_STATISTICS_KEY_BYTES + 1), json!(0));
            Reply::json(json!({"readTimes":times,"totalReadTime":9}))
        } else {
            Reply::json(reply.clone())
        }
    });
    let original = auth();
    let client = fixture.client(original.clone());
    let (sender, receiver) = std::sync::mpsc::channel();
    let jobs = Jobs::new(
        tokio::runtime::Handle::current(),
        sender,
        Some(client.clone()),
    )
    .unwrap();
    reset();
    assert!(jobs.statistics(91, StatisticsPeriod::Total, None).is_none());
    let event = next_event(&receiver).await;
    assert!(matches!(
        &event.result,
        JobResult::Statistics(Err(SourceError::ResponseTooLarge))
    ));
    assert_eq!(jobs.metadata_capacity(), 7);
    assert_eq!(probe().copied_keys, 0);
    assert_eq!(probe().summaries, 0);
    assert_eq!(client.session(), original);
    drop(event);
    assert_eq!(jobs.metadata_capacity(), 8);
    reset();
    assert!(jobs.statistics(92, StatisticsPeriod::Total, None).is_none());
    let event = next_event(&receiver).await;
    let JobResult::Statistics(Ok(output)) = &event.result else {
        panic!("The valid statistics result did not recover after capacity rejection");
    };
    assert_eq!(output.daily_seconds["1"], 7);
    assert_eq!(output.raw, valid);
    assert_eq!(probe().copied_keys, 1);
    assert_eq!(jobs.metadata_capacity(), 7);
    assert_eq!(client.session(), original);
    jobs.clear_session();
    assert!(!jobs.accepts(&event));
    assert_eq!(jobs.metadata_capacity(), 7);
    drop(event);
    assert_eq!(jobs.metadata_capacity(), 8);
    assert_eq!(fixture.finish().len(), 2);
}
