//! Generic caret capacity checks using real, original TrueType data.

use super::*;

fn metrics() -> TextMetrics {
    TextMetrics::from_font_owned(caret_test_font::bytes(), 0)
        .expect("Parse the original tiny TrueType fixture")
}

fn line(text: String) -> Line {
    Line {
        end: text.chars().count() as u32,
        text,
        paragraph_end: true,
        ..Default::default()
    }
}

fn layout() -> Layout {
    Layout {
        indent: 0,
        justify: false,
        font_size: 20.0,
        ..Default::default()
    }
}

fn fee(bytes: usize) -> usize {
    bytes * (bytes + 1) / 2 + bytes * 8 + 1
}

#[test]
fn caret_real_font_supplies_zero_width_and_nonzero_cluster_kerning() {
    let metrics = metrics();
    assert!(metrics.is_font_backed());
    assert_eq!(metrics.advance('\u{301}', 20.0), 0.0);
    assert!((metrics.advance('A', 20.0) - 10.0).abs() < 0.0001);
    assert!((metrics.advance('B', 20.0) - 12.0).abs() < 0.0001);
    assert!((metrics.kerning('A', '\u{301}', 20.0) - 0.1).abs() < 0.0001);
    assert!((metrics.kerning('\u{301}', '\u{301}', 20.0) - 0.2).abs() < 0.0001);
    let line = line(format!("A{}B", "\u{301}".repeat(16)));
    let mut budget = CaretBudget::default();
    line.caret_geometry(&layout(), &metrics, &mut budget)
        .unwrap();
    assert_eq!(budget.stats().measured, 18);
    assert_eq!(budget.stats().boundaries, 170);
    assert_eq!(budget.stats().allocations, 1);
}

#[test]
fn caret_work_is_admitted_before_any_measure_allocation_or_history_scan() {
    let metrics = metrics();
    let line = line(format!("A{}B", "\u{301}".repeat(16)));
    let mut budget = CaretBudget::with_work_limit(fee(line.text.len()) - 1);
    assert!(matches!(
        line.caret_geometry(&layout(), &metrics, &mut budget),
        Err(CaretError::Work)
    ));
    assert_eq!(
        budget.stats(),
        CaretStats {
            rejected: 1,
            ..Default::default()
        }
    );
    let mut exact = CaretBudget::with_work_limit(fee(line.text.len()));
    assert!(line.caret_geometry(&layout(), &metrics, &mut exact).is_ok());
    assert_eq!(exact.stats().measured, 18);
    assert_eq!(exact.stats().allocations, 1);
}

#[test]
fn caret_public_wrappers_and_large_requested_budgets_have_no_unbounded_bypass() {
    let metrics = metrics();
    let long = line(format!("A{}B", "\u{301}".repeat(6000)));
    assert_eq!(long.x_at_offset(1, &layout(), &metrics), None);
    assert_eq!(long.offset_at_x(1.0, &layout(), &metrics), None);
    assert_eq!(long.range_at_x(1.0, &layout(), &metrics), None);
    let mut budget = CaretBudget::with_work_limit(usize::MAX);
    assert!(matches!(
        long.caret_geometry(&layout(), &metrics, &mut budget),
        Err(CaretError::Work)
    ));
    assert_eq!(budget.stats().allocations, 0);
    assert_eq!(budget.stats().measured, 0);
    let page = Page {
        lines: vec![long],
        ..Default::default()
    };
    assert_eq!(
        page.text_range_at_point(1.0, 1.0, &layout(), &metrics),
        None
    );
    let oversized = line("A".repeat(MAX_CARET_INPUT_BYTES + 1));
    let mut budget = CaretBudget::default();
    assert!(matches!(
        oversized.caret_geometry(&layout(), &metrics, &mut budget),
        Err(CaretError::Input)
    ));
    assert_eq!(budget.stats().measured, 0);
    assert_eq!(budget.stats().allocations, 0);
}

#[test]
fn caret_invalid_scalar_anchors_and_nan_points_are_unavailable() {
    let metrics = metrics();
    let invalid = Line {
        text: "AB".into(),
        start: 8,
        end: 12,
        ..Default::default()
    };
    let mut budget = CaretBudget::default();
    assert!(matches!(
        invalid.caret_geometry(&layout(), &metrics, &mut budget),
        Err(CaretError::Anchors)
    ));
    assert_eq!(budget.stats().allocations, 0);
    assert_eq!(invalid.x_at_offset(8, &layout(), &metrics), None);
    assert_eq!(invalid.offset_at_x(1.0, &layout(), &metrics), None);
    assert_eq!(invalid.range_at_x(1.0, &layout(), &metrics), None);
    let ordinary = line("AB".into());
    assert_eq!(ordinary.offset_at_x(f32::NAN, &layout(), &metrics), None);
    assert_eq!(ordinary.range_at_x(f32::NAN, &layout(), &metrics), None);
    assert_eq!(
        ordinary.offset_at_x(f32::NEG_INFINITY, &layout(), &metrics),
        Some(0)
    );
}

#[test]
fn caret_snapshots_reuse_endpoints_and_keep_one_operation_allowance() {
    let metrics = metrics();
    let cluster = line(format!("A{}B", "\u{301}".repeat(16)));
    let mut budget = CaretBudget::with_work_limit(fee(cluster.text.len()));
    let geometry = cluster
        .caret_geometry(&layout(), &metrics, &mut budget)
        .unwrap();
    let stats = budget.stats();
    for _ in 0..64 {
        assert_eq!(geometry.x_at_offset(1), geometry.x_at_offset(17));
        assert_eq!(geometry.range_at_x(5.0), Some((0, 17)));
    }
    assert_eq!(budget.stats(), stats);
    assert!(matches!(
        cluster.caret_geometry(&layout(), &metrics, &mut budget),
        Err(CaretError::Work)
    ));
    assert_eq!(budget.stats().allocations, 1);
    let ordinary = line("A".repeat(4096));
    let mut budget = CaretBudget::default();
    ordinary
        .caret_geometry(&layout(), &TextMetrics::default(), &mut budget)
        .unwrap();
    assert!(matches!(
        ordinary.caret_geometry(&layout(), &TextMetrics::default(), &mut budget),
        Err(CaretError::Positions)
    ));
    assert_eq!(budget.stats().allocations, 1);
}

#[test]
fn caret_real_kerning_preserves_cluster_coordinates_unicode_and_scalar_ranges() {
    let metrics = metrics();
    let cluster = line(format!("A{}B", "\u{301}".repeat(16)));
    let mut budget = CaretBudget::default();
    let geometry = cluster
        .caret_geometry(&layout(), &metrics, &mut budget)
        .unwrap();
    assert!((geometry.x_at_offset(1) - 12.6).abs() < 0.0001);
    assert_eq!(geometry.x_at_offset(1), geometry.x_at_offset(17));
    assert!((geometry.x_at_offset(18) - metrics.measure(&cluster.text, 20.0)).abs() < 0.0001);
    assert_eq!(geometry.range_at_x(5.0), Some((0, 17)));
    assert_eq!(geometry.range_at_x(20.0), Some((17, 18)));
    let unicode = Line {
        start: 10,
        end: 14,
        ..line("A\u{1f642}\u{4e2d}B".into())
    };
    let geometry = unicode
        .caret_geometry(&layout(), &metrics, &mut CaretBudget::default())
        .unwrap();
    assert_eq!(geometry.offset_at_x(geometry.x_at_offset(12)), Some(12));
    assert!(geometry.x_at_offset(13) > geometry.x_at_offset(12));
}
