//! Small merge-entry regressions for duplicate metadata work and shelf semantics.

use super::*;

fn old_book(id: &str, source: &str, on_shelf: bool) -> Book {
    Book {
        id: id.into(),
        title: format!("Old {id}"),
        author: "Old author".into(),
        category: "Old category".into(),
        intro: "Stored introduction ".repeat(8),
        cover_url: "https://example.invalid/old-cover".into(),
        source: source.into(),
        progress: 0.75,
        total_chapters: 9,
        cached_chapters: 5,
        updated: true,
        on_shelf,
        ..Book::default()
    }
}

fn remote_book(id: &str, title: &str) -> source::RemoteBook {
    source::RemoteBook {
        id: id.into(),
        title: title.into(),
        ..source::RemoteBook::default()
    }
}

#[test]
fn shelf_duplicate_records_normalize_old_metadata_once() {
    let existing = vec![old_book("same", "weread", true)];
    let remote = vec![remote_book("same", ""); 24];
    let mut normalized = 0;
    let mut old_copies = 0;
    let mut copied_intro_bytes = 0;
    let merged = merge_shelf_with_observer(&remote, &existing, &BTreeMap::new(), |_, old| {
        normalized += 1;
        if let Some(old) = old {
            old_copies += 1;
            copied_intro_bytes += old.intro.len();
        }
    });
    assert_eq!((normalized, old_copies), (1, 1));
    assert_eq!(copied_intro_bytes, existing[0].intro.len());
    assert_eq!(merged, existing);
    assert_eq!(merge_shelf(&remote, &existing, &BTreeMap::new()), merged);
}

#[test]
fn shelf_merge_keeps_last_record_and_first_seen_order() {
    let remote = vec![
        remote_book("second", "First second"),
        remote_book("first", "First first"),
        remote_book("second", "Last second"),
        remote_book("third", "Third"),
        remote_book("first", "Last first"),
    ];
    let merged = merge_shelf(&remote, &[], &BTreeMap::new());
    assert_eq!(
        merged
            .iter()
            .map(|book| (book.id.as_str(), book.title.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("second", "Last second"),
            ("first", "Last first"),
            ("third", "Third"),
        ]
    );
}

#[test]
fn shelf_merge_duplicates_preserve_overrides_and_local_books() {
    let existing = vec![
        old_book("local", "local", true),
        old_book("pinned", "weread", true),
        old_book("hidden", "weread", true),
        old_book("ordinary", "weread", true),
    ];
    let remote = vec![
        remote_book("ordinary", "First ordinary"),
        remote_book("hidden", "Remote hidden"),
        remote_book("local", "Remote local"),
        remote_book("pinned", "First pinned"),
        remote_book("ordinary", "Last ordinary"),
        remote_book("pinned", "Last pinned"),
        remote_book("local", "Later remote local"),
    ];
    let overrides = BTreeMap::from([("pinned".into(), true), ("hidden".into(), false)]);
    let mut normalized_ids = Vec::new();
    let merged = merge_shelf_with_observer(&remote, &existing, &overrides, |book, _| {
        normalized_ids.push(book.id.clone());
    });
    assert_eq!(normalized_ids, vec!["ordinary", "pinned"]);
    assert_eq!(
        merged
            .iter()
            .map(|book| book.id.as_str())
            .collect::<Vec<_>>(),
        vec!["local", "pinned", "ordinary"]
    );
    assert_eq!(merged[0], existing[0]);
    assert_eq!(merged[1].title, "Last pinned");
    assert_eq!(merged[2].title, "Last ordinary");
    assert!(merged.iter().all(|book| book.on_shelf));
}

#[test]
fn shelf_merge_duplicate_metadata_defaults_keep_progress_and_cache_counts() {
    let mut old = old_book("same", "weread", false);
    old.finished = true;
    let earlier = source::RemoteBook {
        id: "same".into(),
        title: "Earlier transient title".into(),
        author: "Earlier transient author".into(),
        intro: "Earlier transient introduction".into(),
        progress: 90,
        chapter_count: 20,
        ..source::RemoteBook::default()
    };
    let final_record = source::RemoteBook {
        id: "same".into(),
        progress: 30,
        chapter_count: 4,
        ..source::RemoteBook::default()
    };
    let merged = merge_shelf(&[earlier, final_record], &[old.clone()], &BTreeMap::new());
    let mut expected = old;
    expected.on_shelf = true;
    assert_eq!(merged, vec![expected]);
}
