//! Small legal imported books verify intro-free metadata and intact detail data.

use super::*;
use std::{
    cell::RefCell,
    io::{Cursor, Write},
};

thread_local! {
    static HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}

pub(super) fn after_preflight() -> Result<()> {
    if let Some(hook) = HOOK.with(|hook| hook.borrow_mut().take()) {
        hook()?;
    }
    Ok(())
}

fn epub(path: &Path, ordinal: usize, intro: &str) -> Result<crate::importer::ImportedBook> {
    let package = format!(
        "<package version='2.0'><metadata><title>Metadata book {ordinal}</title><creator>Fixture author</creator><subject>Fixture category</subject><description>{intro}</description></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine><itemref idref='chapter'/></spine></package>"
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in [
        ("mimetype", b"application/epub+zip".as_slice()),
        (
            "META-INF/container.xml",
            b"<container><rootfiles><rootfile full-path='book.opf'/></rootfiles></container>"
                .as_slice(),
        ),
        ("book.opf", package.as_bytes()),
        (
            "chapter.xhtml",
            b"<html><body><h1>Fixture chapter</h1><p>Short retained body.</p></body></html>"
                .as_slice(),
        ),
    ] {
        writer.start_file(name, options)?;
        writer.write_all(bytes)?;
    }
    let bytes = writer.finish()?.into_inner();
    assert!(bytes.len() < 8192);
    std::fs::write(path, bytes)?;
    crate::importer::import_file(path)
}

fn imported_fixture() -> Result<(tempfile::TempDir, Store, Vec<Book>)> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("library.sqlite"))?;
    let mut books = Vec::new();
    for ordinal in 0..3 {
        let intro = format!("Retained introduction {ordinal}:{}", "x".repeat(4096));
        let imported = epub(
            &directory.path().join(format!("fixture-{ordinal}.epub")),
            ordinal,
            &intro,
        )?;
        assert_eq!(imported.book.intro, intro);
        store.upsert_book(&imported.book)?;
        store.upsert_chapters(&imported.chapters)?;
        books.push(imported.book);
    }
    Ok((directory, store, books))
}

#[test]
fn legal_epub_descriptions_remain_intact_while_list_and_stat_reads_skip_intro() -> Result<()> {
    let (_directory, store, books) = imported_fixture()?;
    store.reset_book_read_stats();
    let metadata = store.book_metadata()?;
    let statistics = store.book_statistics_metadata()?;
    assert_eq!(metadata.len(), 3);
    assert_eq!(statistics.len(), 3);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book_read_stats().metadata_rows, 6);
    for book in books {
        let listed = metadata
            .iter()
            .find(|metadata| metadata.id == book.id)
            .unwrap();
        assert_eq!(
            (
                listed.title.as_str(),
                listed.author.as_str(),
                listed.category.as_str()
            ),
            (
                book.title.as_str(),
                book.author.as_str(),
                book.category.as_str()
            )
        );
        assert_eq!(store.book(&book.id)?.unwrap().intro, book.intro);
    }
    assert_eq!(store.book_read_stats().intro_copies, 3);
    Ok(())
}

#[test]
fn metadata_rows_cells_and_aggregate_are_admitted_before_any_string_copy() -> Result<()> {
    let (_directory, store, _books) = imported_fixture()?;
    for limits in [
        BookMetadataLimits {
            rows: 2,
            ..Default::default()
        },
        BookMetadataLimits {
            sql_cells: 4,
            ..Default::default()
        },
        BookMetadataLimits {
            bytes: 1,
            ..Default::default()
        },
    ] {
        store.reset_book_read_stats();
        assert!(
            store
                .book_metadata_with_limits(limits)
                .unwrap_err()
                .downcast_ref::<BookMetadataLimit>()
                .is_some()
        );
        assert_eq!(store.book_read_stats(), BookReadStats::default());
    }
    assert_eq!(store.book_metadata()?.len(), 3);
    Ok(())
}

#[test]
fn exact_needed_field_budget_and_one_less_keep_full_book_history() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("library.sqlite"))?;
    let book = Book {
        id: "id".into(),
        title: "Title".into(),
        author: "Author".into(),
        category: "Category".into(),
        intro: "Unrelated preserved introduction".into(),
        source: "local".into(),
        on_shelf: true,
        ..Default::default()
    };
    store.upsert_book(&book)?;
    let string_bytes = book.id.len()
        + book.title.len()
        + book.author.len()
        + book.category.len()
        + book.cover_url.len()
        + book.source.len();
    let required = 2
        * (std::mem::size_of::<BookMetadata>() + std::mem::size_of::<BookScalarMetadata>())
        + string_bytes;
    for maximum in [required, required - 1] {
        store.reset_book_read_stats();
        let result = store.book_metadata_with_limits(BookMetadataLimits {
            bytes: maximum,
            ..Default::default()
        });
        if maximum == required {
            assert_eq!(result?[0].id, "id");
            assert_eq!(store.book_read_stats().metadata_rows, 1);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert_eq!(store.book_read_stats().metadata_rows, 0);
        }
        assert_eq!(store.book_read_stats().intro_copies, 0);
    }
    assert_eq!(store.book("id")?, Some(book));
    Ok(())
}

#[test]
fn invalid_needed_text_and_scalar_types_fail_before_owning_metadata() -> Result<()> {
    let (_directory, store, books) = imported_fixture()?;
    let id = &books[0].id;
    store
        .connection
        .execute("UPDATE books SET author=x'80' WHERE id=?1", [id])?;
    store.reset_book_read_stats();
    assert!(
        store
            .book_metadata()
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(store.book_read_stats(), BookReadStats::default());
    // Statistics does not need author and remains independent of that field.
    assert_eq!(store.book_statistics_metadata()?.len(), 3);
    store
        .connection
        .execute("UPDATE books SET cached_chapters=-1 WHERE id=?1", [id])?;
    store.reset_book_read_stats();
    assert!(
        store
            .book_metadata()
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(store.book_read_stats().metadata_rows, 0);
    Ok(())
}

#[test]
fn same_snapshot_needed_fields_do_not_adopt_late_changes_or_read_intro() -> Result<()> {
    let (directory, store, books) = imported_fixture()?;
    let id = books[0].id.clone();
    let original = books[0].title.clone();
    let other = Connection::open(directory.path().join("library.sqlite"))?;
    HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            other.execute(
                "UPDATE books SET title='Changed',intro='Changed intro' WHERE id=?1",
                [&id],
            )?;
            Ok(())
        }))
    });
    store.reset_book_read_stats();
    let metadata = store.book_metadata()?;
    assert_eq!(
        metadata
            .iter()
            .find(|book| book.id == books[0].id)
            .unwrap()
            .title,
        original
    );
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book(&books[0].id)?.unwrap().intro, "Changed intro");
    Ok(())
}

#[test]
fn metadata_search_and_shelf_flag_do_not_read_or_overwrite_introductions() -> Result<()> {
    let (_directory, store, books) = imported_fixture()?;
    store.reset_book_read_stats();
    let results = store.search_book_metadata("METADATA BOOK 1", true)?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, books[1].id);
    assert_eq!(store.book_on_shelf(&books[1].id)?, Some(true));
    store.set_on_shelf(&books[1].id, false)?;
    assert_eq!(store.book_on_shelf(&books[1].id)?, Some(false));
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book(&books[1].id)?.unwrap().intro, books[1].intro);
    Ok(())
}

#[test]
fn utf16_metadata_fields_use_the_same_cached_conversion_upper_bound() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("library.sqlite");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            migrate(&mut connection)?;
        }
        let store = Store::open(&path)?;
        let book = Book {
            intro: "Preserved UTF16 introduction".into(),
            source: "local".into(),
            ..Book::new("id", "\u{4e2d}")
        };
        store.upsert_book(&book)?;
        store.reset_book_read_stats();
        assert_eq!(store.book_metadata()?[0].title, "\u{4e2d}");
        assert_eq!(store.book_statistics_metadata()?[0].title, "\u{4e2d}");
        assert_eq!(store.book_read_stats().intro_copies, 0);
        assert_eq!(store.book("id")?.unwrap().intro, book.intro);
    }
    Ok(())
}

#[test]
fn editing_a_small_note_preserves_a_legal_oversized_quote_in_sql() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("library.sqlite"))?;
    let book = Book {
        source: "local".into(),
        ..Book::new("book", "Fixture")
    };
    store.upsert_book(&book)?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        body: Some("Body".into()),
        ..Default::default()
    })?;
    let quote = "q".repeat(16 * 1024 + 1);
    store.save_annotation(&Annotation {
        id: "note".into(),
        book_id: "book".into(),
        start: 0,
        end: 1,
        quote: quote.clone(),
        note: "old note".into(),
        created_at: 1,
        ..Default::default()
    })?;
    let header = store.annotation_header_by_id("book", "note")?.unwrap();
    store.reset_annotation_read_stats();
    assert!(store.update_annotation_note_preserving_quote("book", &header, "new note", 2)?);
    assert_eq!(store.annotation_read_stats().quote_copies, 0);
    let (stored_quote, stored_note): (String, String) = store.connection.query_row(
        "SELECT quote,note FROM annotations WHERE id='note'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(stored_quote, quote);
    assert_eq!(stored_note, "new note");
    assert!(!store.update_annotation_note_preserving_quote("book", &header, "stale note", 3)?);
    Ok(())
}
