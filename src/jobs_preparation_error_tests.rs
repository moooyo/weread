//! Exact preparation quota classification with contextual and real nonquota causes.

use super::*;
use crate::article::{
    AnchorLimitError, AnchorLimitKind, AnchorUsage, StyleLimitError, StyleLimitKind,
};

fn contextual(error: impl std::error::Error + Send + Sync + 'static) -> anyhow::Error {
    anyhow::Error::new(error)
        .context("Prepare checked chapter metadata")
        .context("Complete download preparation")
}

#[test]
fn style_capacity_keeps_its_exact_type_through_preparation_context() {
    for kind in [StyleLimitKind::Work, StyleLimitKind::Metadata] {
        let error = contextual(StyleLimitError {
            kind,
            used: 8,
            requested: 1,
            limit: 8,
        });
        assert_eq!(error.downcast_ref::<StyleLimitError>().unwrap().kind, kind);
        let classified = classify_download_preparation_error(error);
        assert!(matches!(&classified, SourceError::ResponseTooLarge));
        assert!(!classified.requires_login());
        assert!(!classified.retryable());
    }
}

#[test]
fn anchor_capacity_keeps_its_exact_type_through_preparation_context() {
    for kind in [
        AnchorLimitKind::Occurrences,
        AnchorLimitKind::Events,
        AnchorLimitKind::Edges,
        AnchorLimitKind::MatchingWork,
    ] {
        let error = contextual(AnchorLimitError {
            kind,
            usage: AnchorUsage::default(),
            requested: 1,
            limit: 0,
        });
        assert_eq!(error.downcast_ref::<AnchorLimitError>().unwrap().kind, kind);
        let classified = classify_download_preparation_error(error);
        assert!(matches!(&classified, SourceError::ResponseTooLarge));
        assert!(!classified.requires_login());
        assert!(!classified.retryable());
    }
}

#[test]
fn actual_sql_io_and_digest_failures_keep_the_checkpoint_category() {
    let database = rusqlite::Connection::open_in_memory().unwrap();
    database
        .execute_batch(
            "CREATE TABLE evidence(id INTEGER PRIMARY KEY); INSERT INTO evidence VALUES(1);",
        )
        .unwrap();
    let sql = database
        .execute("INSERT INTO evidence VALUES(1)", [])
        .err()
        .expect("The duplicate evidence ID must fail its real SQL constraint");
    let sql = contextual(sql);
    assert!(sql.downcast_ref::<rusqlite::Error>().is_some());
    assert!(matches!(
        classify_download_preparation_error(sql),
        SourceError::CheckpointFailed,
    ));

    let directory = tempfile::tempdir().unwrap();
    let io = std::fs::File::open(directory.path().join("absent-evidence.bin"))
        .err()
        .expect("Opening absent preparation evidence must fail");
    let io = contextual(io);
    assert!(io.downcast_ref::<std::io::Error>().is_some());
    assert!(matches!(
        classify_download_preparation_error(io),
        SourceError::CheckpointFailed,
    ));

    let packet = source::images::DownloadedChapterImages {
        assets: vec![source::images::ImageAsset {
            id: "0".repeat(64),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["images/corrupt.png".into()],
            bytes: vec![1],
        }],
        ..Default::default()
    };
    let integrity = crate::book_assets::prepare_downloaded(packet)
        .err()
        .expect("The actual preparation must reject the mismatched original digest");
    assert!(format!("{integrity:#}").contains("acquired chapter image digest mismatch"));
    assert!(integrity.downcast_ref::<StyleLimitError>().is_none());
    assert!(integrity.downcast_ref::<AnchorLimitError>().is_none());
    assert!(matches!(
        classify_download_preparation_error(integrity.context("Prepare original image bytes")),
        SourceError::CheckpointFailed,
    ));
}
