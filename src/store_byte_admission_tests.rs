//! Real bundled SQLite byte gates, encoding bounds, and read snapshots.

use super::*;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    encoding_reads: usize,
    owned_bytes: usize,
    body_copies: usize,
    uid_copies: usize,
    header_copies: usize,
    setting_reads: usize,
    array_reads: usize,
    manifest_reads: usize,
}

thread_local! {
    static STATS: Cell<Option<Stats>> = const { Cell::new(None) };
    static HEADER_HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}

struct Scope;

impl Scope {
    fn start() -> Self {
        STATS.with(|state| {
            assert!(state.get().is_none());
            state.set(Some(Stats::default()));
        });
        Self
    }
    fn stats(&self) -> Stats {
        STATS.with(|state| state.get().unwrap())
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        STATS.with(|state| state.set(None));
        HEADER_HOOK.with(|hook| hook.borrow_mut().take());
    }
}

fn observe(operation: impl FnOnce(&mut Stats)) {
    STATS.with(|state| {
        if let Some(mut stats) = state.get() {
            operation(&mut stats);
            state.set(Some(stats));
        }
    });
}

pub(super) fn observe_encoding_read() {
    observe(|stats| stats.encoding_reads += 1);
}

pub(super) fn observe_text_copy(kind: &str, bytes: usize) {
    observe(|stats| {
        stats.owned_bytes += bytes;
        match kind {
            "body" => stats.body_copies += 1,
            "uid" => stats.uid_copies += 1,
            "header" => stats.header_copies += 1,
            "setting" => stats.setting_reads += 1,
            "array" => stats.array_reads += 1,
            "manifest" => stats.manifest_reads += 1,
            _ => panic!("Unknown SQL text observation"),
        }
    });
}

pub(super) fn after_header_preflight() -> Result<()> {
    let hook = HEADER_HOOK.with(|hook| hook.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

fn fixture(encoding: &str) -> Result<(tempfile::TempDir, Store)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("metadata.sqlite3");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        initialize_schema(&mut connection)?;
    }
    let store = Store::open(&path)?;
    let mut book = Book::new("book", "Byte fixture");
    book.source = "local".into();
    store.upsert_book(&book)?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some("body".into()),
        ..Chapter::default()
    })?;
    Ok((directory, store))
}

#[derive(Debug)]
struct Opcode {
    name: String,
    column: i64,
    detail: String,
    flags: i64,
}

fn explain(connection: &Connection, sql: &str) -> Result<Vec<Opcode>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| {
        Ok(Opcode {
            name: row.get(1)?,
            column: row.get(3)?,
            detail: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            flags: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[test]
fn actual_bundled_sqlite_version_supports_direct_octet_length() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let version: String = store
        .connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    assert_eq!(version, "3.50.2");
    assert_eq!(
        store
            .connection
            .query_row("SELECT octet_length('abc')", [], |row| row.get::<_, i64>(0))?,
        3
    );
    Ok(())
}

#[test]
fn actual_vm_direct_byte_column_has_metadata_flags_that_cast_blocks() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store.set_setting("fixture", &"value")?;
    let direct = explain(
        &store.connection,
        "EXPLAIN SELECT COALESCE(octet_length(value),0) FROM settings WHERE key='fixture'",
    )?;
    let cast = explain(
        &store.connection,
        "EXPLAIN SELECT length(CAST(value AS BLOB)) FROM settings WHERE key='fixture'",
    )?;
    assert!(
        direct
            .iter()
            .any(|op| op.name == "Function" && op.detail.contains("octet_length"))
    );
    assert!(
        direct
            .iter()
            .any(|op| op.name == "Column" && op.column == 1 && op.flags != 0)
    );
    assert!(
        cast.iter()
            .any(|op| op.name == "Column" && op.column == 1 && op.flags == 0)
    );
    Ok(())
}

#[test]
fn actual_vm_byte_preflight_precedes_the_materializing_case_branch() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let ops = explain(
        &store.connection,
        "EXPLAIN SELECT CASE WHEN typeof(value)='text' AND COALESCE(octet_length(value),0)<=8 THEN value ELSE NULL END FROM settings WHERE key='fixture'",
    )?;
    let function = ops
        .iter()
        .position(|op| op.name == "Function" && op.detail.contains("octet_length"))
        .unwrap();
    assert!(
        ops[..function]
            .iter()
            .any(|op| op.name == "Column" && op.column == 1 && op.flags != 0)
    );
    assert!(
        ops[function..]
            .iter()
            .any(|op| op.name == "Column" && op.column == 1 && op.flags == 0)
    );
    Ok(())
}

#[test]
fn a_small_overflow_row_is_rejected_before_owned_json_or_deserialization() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let value = "v".repeat(16 * 1024);
    store.set_setting("fixture", &value)?;
    let scope = Scope::start();
    assert!(
        store
            .bounded_setting::<String>("fixture", 16)
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(scope.stats().owned_bytes, 0);
    assert_eq!(scope.stats().setting_reads, 0);
    assert_eq!(store.setting::<String>("fixture")?, Some(value));
    Ok(())
}

#[test]
fn array_encoded_and_record_limits_precede_serde_and_keep_the_cache() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let value = vec!["a", "b", "c"];
    store.set_setting("fixture", &value)?;
    let encoded = serde_json::to_string(&value)?.len();
    let scope = Scope::start();
    for (bytes, records) in [(encoded - 1, 3), (encoded, 2)] {
        assert!(
            store
                .bounded_array_setting::<String>("fixture", bytes, records)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
    }
    assert_eq!(scope.stats().array_reads, 0);
    assert_eq!(
        store.bounded_array_setting::<String>("fixture", encoded, 3)?,
        Some(value.iter().map(|value| value.to_string()).collect())
    );
    assert_eq!(scope.stats().owned_bytes, 0);
    Ok(())
}

#[test]
fn blob_invalid_utf8_and_missing_settings_have_distinct_safe_results() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    assert_eq!(store.bounded_setting::<String>("missing", 16)?, None);
    let scope = Scope::start();
    for value in ["x'80'", "CAST(x'80' AS TEXT)"] {
        store.connection.execute_batch(&format!("INSERT INTO settings VALUES ('fixture',{value}) ON CONFLICT(key) DO UPDATE SET value=excluded.value"))?;
        assert!(
            store
                .bounded_setting::<String>("fixture", 16)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
    }
    assert_eq!(scope.stats().setting_reads, 0);
    assert_eq!(scope.stats().owned_bytes, 0);
    Ok(())
}

#[test]
fn utf16_setting_gates_use_raw_octets_and_the_conservative_utf8_boundary() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding)?;
        store.set_setting("fixture", &vec!["\u{4e2d}"])?;
        let snapshot = store.resource_verification_snapshot()?;
        let encoded = snapshot.setting_encoded_bytes("fixture")?.unwrap();
        let upper = snapshot.utf8_upper_bytes(encoded)?;
        assert_eq!(encoded, 10);
        assert_eq!(upper, 15);
        drop(snapshot);
        let scope = Scope::start();
        assert!(
            store
                .bounded_array_setting::<String>("fixture", upper - 1, 1)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(scope.stats().array_reads, 0);
        assert_eq!(
            store.bounded_array_setting::<String>("fixture", upper, 1)?,
            Some(vec!["\u{4e2d}".into()])
        );
        assert_eq!(scope.stats().owned_bytes, 0);
    }
    Ok(())
}

#[test]
fn unique_body_byte_metadata_preserves_null_empty_nul_and_text_type() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    for (sql, bytes, body) in [
        ("UPDATE chapters SET body=NULL", None, None),
        ("UPDATE chapters SET body=''", Some(0), Some("")),
        (
            "UPDATE chapters SET body='a'||char(0)||'b'",
            Some(3),
            Some("a\0b"),
        ),
    ] {
        store.connection.execute(sql, [])?;
        assert_eq!(store.chapter_body_bytes_by_uid("book", "uid")?, bytes);
        assert_eq!(
            store.bounded_chapter_body_by_uid("book", "uid", 3)?,
            body.map(str::to_owned)
        );
    }
    assert_eq!(store.chapter_scalar_count("book", 0)?, 1);
    store
        .connection
        .execute("UPDATE chapters SET body=x'80'", [])?;
    assert!(
        store
            .chapter_body_bytes_by_uid("book", "uid")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    Ok(())
}

#[test]
fn ambiguous_uid_body_metadata_rejects_before_any_body_copy() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 1,
        body: Some("other".into()),
        ..Chapter::default()
    })?;
    let scope = Scope::start();
    assert!(
        store
            .chapter_body_bytes_by_uid("book", "uid")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert!(
        store
            .bounded_chapter_body_by_uid("book", "uid", 64)
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(scope.stats().body_copies, 0);
    Ok(())
}

#[test]
fn bounded_body_exact_and_utf16_conversion_limits_precede_ownership() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding)?;
        store
            .connection
            .execute("UPDATE chapters SET body='\u{4e2d}'", [])?;
        let scope = Scope::start();
        assert!(
            store
                .bounded_chapter_body_by_uid("book", "uid", 2)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(scope.stats().body_copies, 0);
        assert_eq!(
            store.bounded_chapter_body_by_uid("book", "uid", 3)?,
            Some("\u{4e2d}".into())
        );
        assert_eq!(scope.stats().owned_bytes, 3);
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct Manifest {
    images: Vec<String>,
}

#[test]
fn manifest_byte_and_image_count_gates_precede_generic_materialization() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store.set_setting("fixture", &serde_json::json!({"images":["a","b"]}))?;
    let encoded = store.setting_encoded_bytes("fixture")?.unwrap();
    let scope = Scope::start();
    for (bytes, count) in [(encoded - 1, 2), (encoded, 1)] {
        assert!(
            store
                .image_manifest_metadata("fixture", bytes, count)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert!(
            store
                .bounded_image_manifest_setting::<Manifest>("fixture", bytes, count)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
    }
    assert_eq!(scope.stats().manifest_reads, 0);
    assert_eq!(
        store.image_manifest_metadata("fixture", encoded, 2)?,
        Some((encoded, 2))
    );
    assert_eq!(
        store.bounded_image_manifest_setting::<Manifest>("fixture", encoded, 2)?,
        Some(Manifest {
            images: vec!["a".into(), "b".into()]
        })
    );
    Ok(())
}

#[test]
fn duplicate_or_escaped_images_keys_cannot_bypass_the_read_count_recheck() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let scope = Scope::start();
    for value in [
        r#"{"images":["a"],"images":["a","b"]}"#,
        r#"{"images":["a"],"im\u0061ges":["a","b"]}"#,
    ] {
        store.connection.execute("INSERT INTO settings VALUES ('fixture',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value])?;
        assert!(
            store
                .bounded_image_manifest_setting::<serde_json::Value>("fixture", value.len(), 1)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
    }
    assert_eq!(scope.stats().manifest_reads, 0);
    Ok(())
}

#[test]
fn resource_snapshot_keeps_scalar_and_manifest_body_reads_on_one_version() -> Result<()> {
    let (directory, store) = fixture("UTF-8")?;
    store.set_setting("fixture", &serde_json::json!({"images":["old"]}))?;
    let snapshot = store.resource_verification_snapshot()?;
    let bytes = snapshot.setting_encoded_bytes("fixture")?.unwrap();
    assert_eq!(snapshot.chapter_body_bytes_by_uid("book", "uid")?, Some(4));
    let other = Connection::open(directory.path().join("metadata.sqlite3"))?;
    other.execute(
        "UPDATE settings SET value=?1 WHERE key='fixture'",
        [r#"{"images":["new","extra"]}"#],
    )?;
    other.execute("UPDATE chapters SET body='new larger body'", [])?;
    assert_eq!(
        snapshot.image_manifest_metadata("fixture", bytes, 1)?,
        Some((bytes, 1))
    );
    assert_eq!(
        snapshot.bounded_image_manifest_setting::<Manifest>("fixture", bytes, 1)?,
        Some(Manifest {
            images: vec!["old".into()]
        })
    );
    assert_eq!(
        snapshot.bounded_chapter_body_by_uid("book", "uid", 4)?,
        Some("body".into())
    );
    drop(snapshot);
    assert!(
        store
            .bounded_image_manifest_setting::<Manifest>("fixture", bytes, 1)
            .is_err()
    );
    assert!(store.bounded_chapter_body_by_uid("book", "uid", 4).is_err());
    Ok(())
}

#[test]
fn separate_convenience_reads_recheck_growth_instead_of_trusting_old_lengths() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store.set_setting("fixture", &serde_json::json!({"images":["a"]}))?;
    let old = store.setting_encoded_bytes("fixture")?.unwrap();
    store.set_setting("fixture", &serde_json::json!({"images":["a","longer"]}))?;
    let scope = Scope::start();
    assert!(
        store
            .bounded_image_manifest_setting::<Manifest>("fixture", old, 1)
            .is_err()
    );
    assert_eq!(scope.stats().manifest_reads, 0);
    assert_eq!(store.chapter_body_bytes_by_uid("book", "uid")?, Some(4));
    store
        .connection
        .execute("UPDATE chapters SET body='larger'", [])?;
    assert!(store.bounded_chapter_body_by_uid("book", "uid", 4).is_err());
    assert_eq!(scope.stats().body_copies, 0);
    Ok(())
}

#[test]
fn uid_header_owner_title_and_summary_byte_queries_do_not_load_bodies() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store
        .connection
        .execute("UPDATE chapters SET body=?1", [&"b".repeat(16 * 1024)])?;
    let scope = Scope::start();
    assert_eq!(store.chapter_uid("book", 0)?, Some("uid".into()));
    assert_eq!(store.chapter_metadata("book")?[0].body, None);
    assert_eq!(store.summary()?.body_bytes, 16 * 1024);
    assert_eq!(scope.stats().body_copies, 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    Ok(())
}

#[test]
fn oversized_or_blob_header_fields_reject_before_owned_headers() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let scope = Scope::start();
    store
        .connection
        .execute("UPDATE chapters SET source_id=?1", [&"u".repeat(4097)])?;
    assert!(
        store
            .chapter_uid("book", 0)
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert!(
        store
            .chapter_headers("book")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    store.connection.execute(
        "UPDATE chapters SET source_id='uid',content_account_key=x'80'",
        [],
    )?;
    assert!(
        store
            .chapter_headers("book")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(scope.stats().header_copies, 0);
    assert_eq!(scope.stats().uid_copies, 0);
    Ok(())
}

#[test]
fn header_byte_preflight_and_materialization_share_the_same_snapshot() -> Result<()> {
    let (directory, store) = fixture("UTF-8")?;
    let other = Connection::open(directory.path().join("metadata.sqlite3"))?;
    HEADER_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            other.execute(
                "UPDATE chapters SET source_id='changed',title='changed'",
                [],
            )?;
            Ok(())
        }))
    });
    let scope = Scope::start();
    let header = store.chapter_metadata("book")?.remove(0);
    assert_eq!(header.id, "uid");
    assert_eq!(header.title, "Fixture chapter");
    assert_eq!(store.chapter_uid("book", 0)?, Some("changed".into()));
    assert_eq!(scope.stats().body_copies, 0);
    Ok(())
}

#[test]
fn encoding_is_read_once_per_store_and_reused_by_all_byte_boundaries() -> Result<()> {
    // Legacy database preparation uses its own connection and encoding read.
    // Start observation immediately before the single measured Store opens.
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("metadata.sqlite3");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", "UTF-16be")?;
        initialize_schema(&mut connection)?;
    }
    let scope = Scope::start();
    let store = Store::open(&path)?;
    assert_eq!(scope.stats().encoding_reads, 1);
    let mut book = Book::new("book", "Byte fixture");
    book.source = "local".into();
    store.upsert_book(&book)?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some("body".into()),
        ..Default::default()
    })?;
    store.set_setting("fixture", &vec!["a"])?;
    let raw = store.setting_encoded_bytes("fixture")?.unwrap();
    let upper = store
        .resource_verification_snapshot()?
        .utf8_upper_bytes(raw)?;
    store.bounded_array_setting::<String>("fixture", upper, 1)?;
    store.chapter_uid("book", 0)?;
    store.chapter_metadata("book")?;
    store.summary()?;
    assert_eq!(scope.stats().encoding_reads, 1);
    assert_eq!(scope.stats().body_copies, 0);
    Ok(())
}

#[test]
fn utf16_manifest_json_and_final_generic_read_use_the_same_byte_bound() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding)?;
        store.set_setting("fixture", &serde_json::json!({"images":["\u{4e2d}"]}))?;
        let snapshot = store.resource_verification_snapshot()?;
        let raw = snapshot.setting_encoded_bytes("fixture")?.unwrap();
        let upper = snapshot.utf8_upper_bytes(raw)?;
        assert!(upper >= raw);
        let scope = Scope::start();
        assert!(
            snapshot
                .bounded_image_manifest_setting::<Manifest>("fixture", upper - 1, 1)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(scope.stats().manifest_reads, 0);
        assert_eq!(
            snapshot.image_manifest_metadata("fixture", upper, 1)?,
            Some((raw, 1))
        );
        assert_eq!(
            snapshot.bounded_image_manifest_setting::<Manifest>("fixture", upper, 1)?,
            Some(Manifest {
                images: vec!["\u{4e2d}".into()]
            })
        );
    }
    Ok(())
}

#[test]
fn odd_and_malformed_utf16_text_is_bounded_or_typed_unavailable_before_owning() -> Result<()> {
    for (encoding, values) in [
        ("UTF-16le", ["00D8", "610000D8", "610000"]),
        ("UTF-16be", ["D800", "0061D800", "006100"]),
    ] {
        let (_directory, store) = fixture(encoding)?;
        for value in values {
            store.connection.execute_batch(&format!(
                "UPDATE chapters SET body=CAST(x'{value}' AS TEXT)"
            ))?;
            let snapshot = store.resource_verification_snapshot()?;
            let raw = snapshot.chapter_body_bytes_by_uid("book", "uid")?.unwrap();
            let upper = snapshot.utf8_upper_bytes(raw)?;
            let scope = Scope::start();
            match snapshot.bounded_chapter_body_by_uid("book", "uid", upper) {
                Ok(Some(body)) => assert!(body.len() <= upper),
                Err(error) => {
                    assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
                    assert_eq!(scope.stats().body_copies, 0);
                }
                Ok(None) => panic!("The malformed TEXT row still exists"),
            }
        }
    }
    Ok(())
}
