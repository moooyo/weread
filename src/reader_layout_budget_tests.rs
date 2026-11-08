//! Small real layout constructors, storage overlap, and monotone work limits.

use super::*;
use layout_budget::{LayoutVec, with_layout_test_limits};

fn layout() -> Layout {
    Layout {
        width: 84.0,
        height: 72.0,
        font_size: 12.0,
        line_height: 1.5,
        indent: 0,
        paragraph_gap: 0.5,
        justify: false,
    }
}

#[test]
fn reader_layout_exact_peak_and_work_allowances_preserve_unicode_geometry() {
    let text = "甲e\u{301}\n\nbeta\r\n尾";
    let baseline = LayoutBudget::default();
    let expected =
        Paginator::try_new_in(text, &layout(), &TextMetrics::default(), &baseline).unwrap();
    let usage = baseline.usage();
    assert_eq!(expected.char_count as usize, text.chars().count());
    for line in expected.pages.iter().flat_map(|page| &page.lines) {
        assert_eq!(line.text, text_range(text, line.start, line.end));
        assert!(line.y.is_finite() && line.height > 0.0);
    }
    let exact = LayoutBudget::with_limits(LayoutLimits {
        metadata_bytes: usage.peak_metadata,
        allocation_work: usage.allocation_work,
    });
    let actual = Paginator::try_new_in(text, &layout(), &TextMetrics::default(), &exact).unwrap();
    assert_eq!(actual.pages, expected.pages);
    assert_eq!(exact.usage().peak_metadata, usage.peak_metadata);
    for limits in [
        LayoutLimits {
            metadata_bytes: usage.peak_metadata - 1,
            ..LayoutLimits::default()
        },
        LayoutLimits {
            allocation_work: usage.allocation_work - 1,
            ..LayoutLimits::default()
        },
    ] {
        let budget = LayoutBudget::with_limits(limits);
        assert!(Paginator::try_new_in(text, &layout(), &TextMetrics::default(), &budget).is_err());
        assert_eq!(budget.usage().live_metadata, 0);
        assert_eq!(budget.usage().rejected, 1);
    }
}

#[test]
fn short_nonempty_lines_refuse_the_first_tuple_buffer_before_text_copy() {
    let text = "x\n";
    let scalars = text.chars().count() * std::mem::size_of::<char>();
    let first_slots = 4 * std::mem::size_of::<(Line, u32)>();
    let budget = LayoutBudget::with_limits(LayoutLimits {
        metadata_bytes: scalars + first_slots - 1,
        ..LayoutLimits::default()
    });
    let error =
        Paginator::try_new_in(text, &layout(), &TextMetrics::default(), &budget).unwrap_err();
    assert_eq!(error.kind, LayoutLimitKind::Metadata);
    assert_eq!(error.requested, first_slots);
    assert_eq!(budget.usage().allocations, 1);
    assert_eq!(budget.usage().vector_pushes, text.chars().count());
    assert_eq!(budget.usage().text_copies, 0);
    assert_eq!(budget.usage().live_metadata, 0);
}

#[test]
fn released_temporary_storage_never_refunds_spent_allocation_work() {
    let budget = LayoutBudget::default();
    let first = LayoutVec::<u32>::with_capacity(&budget, 8).unwrap();
    let first_work = budget.usage().allocation_work;
    assert_eq!(budget.usage().live_metadata, 8 * std::mem::size_of::<u32>());
    drop(first);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(budget.usage().allocation_work, first_work);
    let second = LayoutVec::<u32>::with_capacity(&budget, 8).unwrap();
    assert_eq!(budget.usage().allocation_work, first_work * 2);
    drop(second);
    assert_eq!(budget.usage().live_metadata, 0);
}

#[test]
fn capacity_overflow_is_rejected_without_an_allocation_request() {
    let budget = LayoutBudget::default();
    assert!(LayoutVec::<Line>::with_capacity(&budget, usize::MAX).is_err());
    assert_eq!(budget.usage().allocations, 0);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(budget.usage().allocation_work, 0);
}

#[test]
fn image_clone_slots_are_admitted_before_copy_and_keep_equal_anchor_order() {
    let text = "甲\n乙\n";
    let images = [
        InlineImage {
            id: "first".into(),
            anchor: 2,
            width: 80.0,
            height: 40.0,
        },
        InlineImage {
            id: "second".into(),
            anchor: 2,
            width: 80.0,
            height: 40.0,
        },
    ];
    let pagination =
        Paginator::try_new_with_images(text, &layout(), &TextMetrics::default(), 48.0, &images)
            .unwrap();
    let order: Vec<_> = pagination
        .pages
        .iter()
        .flat_map(|page| &page.images)
        .map(|image| image.id.as_str())
        .collect();
    assert_eq!(order, ["first", "second"]);
    assert!(
        pagination
            .pages
            .iter()
            .flat_map(|page| &page.images)
            .all(|image| image.anchor == 2 && image.height > 0.0)
    );
    let scalars = text.chars().count() * std::mem::size_of::<char>();
    let slots = 4 * std::mem::size_of::<(usize, InlineImage)>();
    let budget = LayoutBudget::with_limits(LayoutLimits {
        metadata_bytes: scalars + slots - 1,
        ..LayoutLimits::default()
    });
    let error = Paginator::try_new_with_images_in(
        text,
        &layout(),
        &TextMetrics::default(),
        48.0,
        &images,
        &budget,
    )
    .unwrap_err();
    assert_eq!(error.kind, LayoutLimitKind::Metadata);
    assert_eq!(error.requested, slots);
    assert_eq!(budget.usage().text_copies, 0);
    assert_eq!(budget.usage().allocations, 1);
}

#[test]
fn failed_reader_reflow_keeps_pages_layout_and_character_anchor() {
    let mut pagination =
        Paginator::try_new("甲\n乙\n丙\n", &layout(), &TextMetrics::default()).unwrap();
    pagination.seek(2);
    let pages = pagination.pages.clone();
    let old_layout = pagination.layout;
    let old_page = pagination.current_index();
    let error = with_layout_test_limits(
        LayoutLimits {
            metadata_bytes: 0,
            ..LayoutLimits::default()
        },
        || pagination.try_reflow("x\nx\n", &layout(), &TextMetrics::default(), 0),
    )
    .unwrap_err();
    assert_eq!(error.kind, LayoutLimitKind::Metadata);
    assert_eq!(pagination.pages, pages);
    assert_eq!(pagination.layout, old_layout);
    assert_eq!(pagination.current_index(), old_page);
}

#[test]
fn real_web_text_and_article_pages_share_one_allocation_operation() {
    let html = "<h2>甲e\u{301}</h2><p>x</p><p>x</p><blockquote>乙</blockquote>";
    let text = crate::source::content::html_to_text(html);
    let budget = LayoutBudget::default();
    let document = crate::article::ArticleDocument::parse_checked_in(html, &text, &budget).unwrap();
    let pages = document
        .try_paginate_with_options_in(
            &layout(),
            &TextMetrics::default(),
            |_| false,
            layout().height,
            &crate::article::ArticleLayoutOptions::default(),
            &budget,
        )
        .unwrap();
    assert!(pages.iter().flatten().any(|block| block.text == "x"));
    assert!(pages.iter().flatten().any(|block| block.kind == "heading"));
    assert!(pages.iter().flatten().any(|block| block.kind == "quote"));
    let usage = budget.usage();
    assert!(usage.peak_metadata >= document.requested_metadata() + pages.requested_metadata());
    drop(pages);
    drop(document);
    assert_eq!(budget.usage().live_metadata, 0);
    assert_eq!(budget.usage().allocation_work, usage.allocation_work);
    let tiny = LayoutBudget::with_limits(LayoutLimits {
        metadata_bytes: 0,
        ..LayoutLimits::default()
    });
    let error = crate::article::ArticleDocument::parse_checked_in(html, &text, &tiny).unwrap_err();
    assert!(matches!(
        error,
        crate::article::ArticleParseError::Layout(_)
    ));
    assert_eq!(tiny.usage().allocations, 0);
}

#[test]
fn admitted_reader_and_article_outputs_remain_send() {
    fn require_send<T: Send>() {}
    require_send::<Paginator>();
    require_send::<crate::article::ArticleDocument>();
    require_send::<crate::article::ArticlePages>();
}
