//! Real fonts and masks exercise downstream fill admission and refusal.

use super::*;
use flate2::read::GzDecoder;
use std::io::Read;
use zeno::{Command, Fill, PathBuilder};

#[path = "../../../../src/reader_caret_font.rs"]
mod original_font;

fn word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn integer(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, part| {
        let mut word = [0; 4];
        word[..part.len()].copy_from_slice(part);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn table(bytes: &[u8], tag: &[u8; 4]) -> (usize, usize, usize) {
    let count = usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
    for index in 0..count {
        let record = 12 + index * 16;
        if &bytes[record..record + 4] == tag {
            let offset = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap());
            let length = u32::from_be_bytes(bytes[record + 12..record + 16].try_into().unwrap());
            return (usize::try_from(offset).unwrap(), usize::try_from(length).unwrap(), record);
        }
    }
    panic!("The original tiny font is missing a required table");
}

fn rectangle(width: i16, height: i16) -> [u8; 34] {
    let mut glyph = [0; 34];
    word(&mut glyph, 0, 1);
    glyph[6..8].copy_from_slice(&width.to_be_bytes());
    glyph[8..10].copy_from_slice(&height.to_be_bytes());
    word(&mut glyph, 10, 3);
    glyph[14..18].fill(1);
    for (index, delta) in [0, width, 0, -width].into_iter().enumerate() {
        glyph[18 + 2 * index..20 + 2 * index].copy_from_slice(&delta.to_be_bytes());
    }
    for (index, delta) in [0, 0, height, 0].into_iter().enumerate() {
        glyph[26 + 2 * index..28 + 2 * index].copy_from_slice(&delta.to_be_bytes());
    }
    glyph
}

fn rectangle_font(width: i16, height: i16) -> Vec<u8> {
    let mut bytes = original_font::bytes();
    let (head, _, _) = table(&bytes, b"head");
    word(&mut bytes, head + 18, 16);
    word(&mut bytes, head + 40, width as u16);
    word(&mut bytes, head + 42, height as u16);
    integer(&mut bytes, head + 8, 0);
    let (glyf, _, _) = table(&bytes, b"glyf");
    for offset in [10, 44, 78] {
        bytes[glyf + offset..glyf + offset + 34].copy_from_slice(&rectangle(width, height));
    }
    let (hhea, _, _) = table(&bytes, b"hhea");
    word(&mut bytes, hhea + 4, 16);
    word(&mut bytes, hhea + 6, 0);
    word(&mut bytes, hhea + 10, 16);
    let (hmtx, _, _) = table(&bytes, b"hmtx");
    for glyph in [0, 1, 3] {
        word(&mut bytes, hmtx + glyph * 4, 16);
    }
    for tag in [*b"head", *b"glyf", *b"hhea", *b"hmtx"] {
        let (offset, length, record) = table(&bytes, &tag);
        let sum = checksum(&bytes[offset..offset + length]);
        integer(&mut bytes, record + 4, sum);
    }
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&bytes));
    integer(&mut bytes, head + 8, adjustment);
    bytes
}

fn tiny_font() -> Vec<u8> {
    rectangle_font(4, 4)
}

struct RenderObservation {
    image: Option<Image>,
    usage: FillRasterUsage,
    error: Option<FillRasterLimitError>,
    target_usage: RasterUsage,
    target_error: Option<RasterLimitError>,
}

fn render_font(bytes: &[u8], format: Format, limits: FillRasterLimits) -> RenderObservation {
    let font = FontRef::from_index(bytes, 0).expect("The original four-point font is valid");
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(16.0).hint(false).normalized_coords([123i16]).build();
    let image = Render::new(&[Source::Outline])
        .format(format)
        .fill_raster_limits(limits)
        .render(&mut scaler, 1);
    RenderObservation {
        image,
        usage: scaler.fill_raster_usage(),
        error: scaler.fill_raster_error(),
        target_usage: scaler.raster_usage(),
        target_error: scaler.raster_error(),
    }
}

fn assert_same_image(actual: &Image, expected: &Image) {
    assert_eq!(actual.data, expected.data);
    assert_eq!(actual.content, expected.content);
    assert_eq!(
        (actual.placement.left, actual.placement.top, actual.placement.width, actual.placement.height),
        (expected.placement.left, expected.placement.top, expected.placement.width, expected.placement.height),
    );
    assert!(matches!(actual.source, Source::Outline));
}

struct MaskObservation {
    data: Vec<u8>,
    result: Result<Placement, FillRasterLimitError>,
    usage: FillRasterUsage,
}

fn render_mask(path: &[Command], format: Format, width: u32, height: u32, limits: FillRasterLimits) -> MaskObservation {
    // These explicit fixture targets never exceed 32 by 8 pixels.
    assert!(width <= 32 && height <= 8);
    let mut data = vec![0; format.buffer_size(width, height)];
    let mut scratch = Scratch::new();
    let mut budget = FillRasterBudget::new(limits);
    let result = Mask::with_scratch(path, &mut scratch)
        .format(format)
        .origin(Origin::BottomLeft)
        .style(Fill::NonZero)
        .size(width, height)
        .render_into_with_budget(&mut data, None, &mut budget);
    MaskObservation { data, result, usage: budget.usage() }
}

fn strips(ascending: bool) -> Vec<Command> {
    let mut path = Vec::new();
    for index in 0..16 {
        let index = if ascending { index } else { 15 - index };
        let x = index as f32 * 2.0 + 0.25;
        path.move_to([x, 0.25])
            .line_to([x + 1.0, 0.25])
            .line_to([x + 1.0, 3.75])
            .line_to([x, 3.75])
            .close();
    }
    path
}

#[test]
fn actual_tiny_ttf_cells_storage_and_work_exact_and_one_less() {
    let bytes = tiny_font();
    let baseline = render_font(&bytes, Format::Alpha, FillRasterLimits::default());
    let expected = baseline.image.as_ref().unwrap();
    assert_eq!((expected.placement.width, expected.placement.height), (4, 4));
    assert_eq!(baseline.usage.indices, 4);
    assert!(baseline.usage.cells > 0);
    assert_eq!(baseline.usage.maximum_live_cells, baseline.usage.cells);
    assert_eq!(
        baseline.usage.storage_bytes,
        baseline.usage.cell_slots * FillRasterLimits::cell_bytes()
            + baseline.usage.index_slots * FillRasterLimits::index_bytes(),
    );
    let exact = FillRasterLimits {
        cells: baseline.usage.cells,
        storage_bytes: baseline.usage.storage_bytes.max(baseline.usage.maximum_peak_storage_bytes),
        work_accesses: baseline.usage.work_accesses,
    };
    let admitted = render_font(&bytes, Format::Alpha, exact);
    assert_same_image(admitted.image.as_ref().unwrap(), expected);
    assert_eq!(admitted.error, None);
    assert_eq!(admitted.usage.cells, exact.cells);
    assert_eq!(admitted.usage.storage_bytes, exact.storage_bytes);
    assert_eq!(admitted.usage.work_accesses, exact.work_accesses);
    for (limits, kind, outer_kind, requested) in [
        (FillRasterLimits { cells: exact.cells - 1, ..exact }, FillRasterLimitKind::Cells, RasterLimitKind::FillCells, exact.cells),
        (FillRasterLimits { storage_bytes: exact.storage_bytes - 1, ..exact }, FillRasterLimitKind::Storage, RasterLimitKind::FillStorage, exact.storage_bytes),
        (FillRasterLimits { work_accesses: exact.work_accesses - 1, ..exact }, FillRasterLimitKind::Work, RasterLimitKind::FillWork, exact.work_accesses),
    ] {
        let denied = render_font(&bytes, Format::Alpha, limits);
        assert!(denied.image.is_none());
        assert_eq!(denied.error.unwrap().kind, kind);
        assert_eq!(denied.error.unwrap().requested, requested as u64);
        assert_eq!(denied.target_error.unwrap().kind, outer_kind);
        assert!(denied.usage.cells <= limits.cells);
        assert!(denied.usage.storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.maximum_peak_storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.work_accesses <= limits.work_accesses);
        assert_eq!(denied.target_usage.resize_calls, 1);
        assert_eq!(denied.target_usage.maximum_buffer_bytes, 16);
    }
    let denied = render_font(&bytes, Format::Alpha, FillRasterLimits { cells: 0, ..Default::default() });
    assert_eq!(denied.error.unwrap().kind, FillRasterLimitKind::Cells);
    assert_eq!(denied.usage.cells, 0);
    assert_eq!(denied.usage.maximum_live_cells, 0);
    assert_eq!(denied.usage.cell_growths, 0);
    // The zero-cell refusal identifies the real first insertion frontier.
    // Its two writes and one allocator request must fit before reserve or link.
    let first_growth_work = denied.usage.work_accesses.checked_add(3).unwrap();
    let admitted_first_growth = render_font(&bytes, Format::Alpha, FillRasterLimits {
        work_accesses: first_growth_work,
        ..Default::default()
    });
    assert_eq!(admitted_first_growth.usage.maximum_live_cells, 1);
    assert_eq!(admitted_first_growth.usage.cell_growths, 1);
    assert_eq!(admitted_first_growth.usage.cell_slots, 4);
    assert_eq!(admitted_first_growth.usage.storage_bytes, 4 * FillRasterLimits::cell_bytes() + 4 * FillRasterLimits::index_bytes());
    assert!(admitted_first_growth.image.is_none());
    let denied_first_growth = render_font(&bytes, Format::Alpha, FillRasterLimits {
        work_accesses: first_growth_work - 1,
        ..Default::default()
    });
    assert_eq!(denied_first_growth.error.unwrap().kind, FillRasterLimitKind::Work);
    assert_eq!(denied_first_growth.usage.maximum_live_cells, 0);
    assert_eq!(denied_first_growth.usage.cell_growths, 0);
    assert_eq!(denied_first_growth.usage.cell_reserve_attempts, 0);
    assert_eq!(denied_first_growth.usage.cell_slots, 4);

    // A real four-point font produces 64 rows through the current Render path.
    // Its single edge isolates vector growth from long sorted-row prefixes.
    let tall_bytes = rectangle_font(4, 64);
    let tall = render_font(&tall_bytes, Format::Alpha, FillRasterLimits::default());
    let expected = tall.image.as_ref().unwrap();
    assert_eq!((expected.placement.width, expected.placement.height), (4, 64));
    assert_eq!(tall.usage.cells, 64);
    assert_eq!(tall.usage.cell_slots, 64);
    assert_eq!(tall.usage.index_slots, 64);
    assert_eq!(tall.usage.cell_growths, 5);
    assert_eq!(tall.usage.cell_reserve_attempts, 5);
    assert_eq!(tall.usage.index_reserve_attempts, 1);
    assert_eq!(tall.usage.potential_relocation_bytes, (4 + 8 + 16 + 32) * FillRasterLimits::cell_bytes());
    assert_eq!(tall.usage.storage_bytes, 64 * FillRasterLimits::cell_bytes() + 64 * FillRasterLimits::index_bytes());
    assert_eq!(tall.usage.maximum_peak_storage_bytes, (32 + 64) * FillRasterLimits::cell_bytes() + 64 * FillRasterLimits::index_bytes());
    let exact = FillRasterLimits {
        cells: tall.usage.cells,
        storage_bytes: tall.usage.storage_bytes.max(tall.usage.maximum_peak_storage_bytes),
        work_accesses: tall.usage.work_accesses,
    };
    let admitted = render_font(&tall_bytes, Format::Alpha, exact);
    assert_same_image(admitted.image.as_ref().unwrap(), expected);
    for (limits, kind, requested) in [
        (FillRasterLimits { cells: exact.cells - 1, ..exact }, FillRasterLimitKind::Cells, exact.cells),
        (FillRasterLimits { storage_bytes: exact.storage_bytes - 1, ..exact }, FillRasterLimitKind::Storage, exact.storage_bytes),
        (FillRasterLimits { work_accesses: exact.work_accesses - 1, ..exact }, FillRasterLimitKind::Work, exact.work_accesses),
    ] {
        let denied = render_font(&tall_bytes, Format::Alpha, limits);
        assert!(denied.image.is_none());
        assert_eq!(denied.error.unwrap().kind, kind);
        assert_eq!(denied.error.unwrap().requested, requested as u64);
        assert!(denied.usage.storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.maximum_peak_storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.work_accesses <= limits.work_accesses);
    }
    let denied_peak = render_font(&tall_bytes, Format::Alpha, FillRasterLimits { storage_bytes: exact.storage_bytes - 1, ..exact });
    assert_eq!(denied_peak.error.unwrap().kind, FillRasterLimitKind::Storage);
    assert_eq!(denied_peak.usage.cell_growths, 4);
    assert_eq!(denied_peak.usage.maximum_live_cells, 32);

    // The fifth node requires the first nonempty relocation. Prepay the old
    // four-cell layout's read/write upper bound before the second reserve.
    let before_move = render_font(&tall_bytes, Format::Alpha, FillRasterLimits { cells: 4, ..Default::default() });
    assert_eq!(before_move.error.unwrap().kind, FillRasterLimitKind::Cells);
    assert_eq!(before_move.usage.maximum_live_cells, 4);
    let move_work = 4usize.checked_mul(FillRasterLimits::cell_bytes()).unwrap().checked_mul(2).unwrap().checked_add(1).unwrap();
    let growth_work = before_move.usage.work_accesses.checked_add(2).unwrap().checked_add(move_work).unwrap();
    let admitted_move = render_font(&tall_bytes, Format::Alpha, FillRasterLimits { work_accesses: growth_work, ..Default::default() });
    assert_eq!(admitted_move.error.unwrap().kind, FillRasterLimitKind::Work);
    assert_eq!(admitted_move.usage.maximum_live_cells, 5);
    assert_eq!(admitted_move.usage.cell_growths, 2);
    assert_eq!(admitted_move.usage.potential_relocation_bytes, 4 * FillRasterLimits::cell_bytes());
    let denied_move = render_font(&tall_bytes, Format::Alpha, FillRasterLimits { work_accesses: growth_work - 1, ..Default::default() });
    assert_eq!(denied_move.error.unwrap().kind, FillRasterLimitKind::Work);
    assert_eq!(denied_move.error.unwrap().requested, growth_work as u64);
    assert_eq!(denied_move.usage.maximum_live_cells, 4);
    assert_eq!(denied_move.usage.cell_growths, 1);
    assert_eq!(denied_move.usage.cell_reserve_attempts, 1);
    assert_eq!(denied_move.usage.potential_relocation_bytes, 0);
    assert_eq!(denied_move.usage.cell_slots, 8);
}

#[test]
fn actual_empty_mask_index_bytes_exact_and_one_less_precede_growth() {
    let baseline = render_mask(&[], Format::Alpha, 4, 4, FillRasterLimits::default());
    assert!(baseline.result.is_ok());
    assert_eq!(baseline.usage.cells, 0);
    assert_eq!(baseline.usage.indices, 4);
    let bytes = 4 * FillRasterLimits::index_bytes();
    assert_eq!(baseline.usage.storage_bytes, bytes);
    let exact = FillRasterLimits { cells: 0, storage_bytes: bytes, work_accesses: baseline.usage.work_accesses };
    let admitted = render_mask(&[], Format::Alpha, 4, 4, exact);
    assert!(admitted.result.is_ok());
    assert_eq!(admitted.data, [0; 16]);
    assert_eq!(admitted.usage.index_growths, 1);
    let denied = render_mask(&[], Format::Alpha, 4, 4, FillRasterLimits { storage_bytes: bytes - 1, ..exact });
    assert_eq!(denied.result.unwrap_err().kind, FillRasterLimitKind::Storage);
    assert_eq!(denied.usage.index_growths, 0);
    assert_eq!(denied.usage.maximum_live_indices, 0);
    assert_eq!(denied.usage.index_reserve_attempts, 0);
    let denied = render_mask(&[], Format::Alpha, 4, 4, FillRasterLimits { work_accesses: 4, ..exact });
    assert_eq!(denied.result.unwrap_err().kind, FillRasterLimitKind::Work);
    assert_eq!(denied.usage.storage_bytes, bytes);
    assert_eq!(denied.usage.index_growths, 0);
    assert_eq!(denied.usage.maximum_live_indices, 0);
    let admitted_initialization = render_mask(&[], Format::Alpha, 4, 4, FillRasterLimits { work_accesses: 5, ..exact });
    assert_eq!(admitted_initialization.result.unwrap_err().kind, FillRasterLimitKind::Work);
    assert_eq!(admitted_initialization.usage.index_growths, 1);
    assert_eq!(admitted_initialization.usage.maximum_live_indices, 4);

    // A real second Mask pass grows retained row-head capacity from four to
    // eight slots. Its old layout can move even after indices.clear().
    let index_storage = (4 + 8) * FillRasterLimits::index_bytes();
    let move_bytes = 4 * FillRasterLimits::index_bytes();
    let index_growth_work = baseline.usage.work_accesses.checked_add(8).unwrap()
        .checked_add(move_bytes.checked_mul(2).unwrap()).unwrap().checked_add(1).unwrap();
    for (limits, expected_error, expected_growths, expected_live) in [
        (FillRasterLimits { cells: 0, storage_bytes: index_storage, ..Default::default() }, None, 2, 8),
        (FillRasterLimits { cells: 0, storage_bytes: index_storage - 1, ..Default::default() }, Some((FillRasterLimitKind::Storage, index_storage)), 1, 4),
        (FillRasterLimits { cells: 0, storage_bytes: index_storage, work_accesses: index_growth_work }, Some((FillRasterLimitKind::Work, index_growth_work + 1)), 2, 8),
        (FillRasterLimits { cells: 0, storage_bytes: index_storage, work_accesses: index_growth_work - 1 }, Some((FillRasterLimitKind::Work, index_growth_work)), 1, 4),
    ] {
        let mut scratch = Scratch::new();
        let mut budget = FillRasterBudget::new(limits);
        let mut mask = Mask::with_scratch(&[] as &[Command], &mut scratch);
        let mut data = [0; 32];
        mask.size(4, 4);
        assert!(mask.render_into_with_budget(&mut data[..16], None, &mut budget).is_ok());
        mask.size(4, 8);
        let result = mask.render_into_with_budget(&mut data, None, &mut budget);
        if let Some((kind, requested)) = expected_error {
            let error = result.unwrap_err();
            assert_eq!(error.kind, kind);
            assert_eq!(error.requested, requested as u64);
        } else {
            assert!(result.is_ok());
        }
        assert_eq!(budget.usage().index_growths, expected_growths);
        assert_eq!(budget.usage().index_reserve_attempts, expected_growths);
        assert_eq!(budget.usage().maximum_live_indices, expected_live);
        assert_eq!(budget.usage().potential_relocation_bytes, if expected_growths == 2 { move_bytes } else { 0 });
        assert!(budget.usage().storage_bytes <= limits.storage_bytes);
        assert!(budget.usage().maximum_peak_storage_bytes <= limits.storage_bytes);
        assert!(budget.usage().work_accesses <= limits.work_accesses);
        assert_eq!(data, [0; 32]);
    }
}

#[test]
fn denied_partial_sweep_preserves_old_image_metrics_and_coordinates() {
    let bytes = tiny_font();
    let font = FontRef::from_index(&bytes, 0).unwrap();
    for format in [Format::Alpha, Format::CustomSubpixel([0.0, 0.0, 0.0])] {
        let baseline = render_font(&bytes, format, FillRasterLimits::default());
        let advance = font.glyph_metrics(&[]).advance_width(1);
        let mut context = ScaleContext::new();
        let mut scaler = context.builder(font).size(16.0).hint(false).normalized_coords([123i16]).build();
        let coordinates = scaler.coords.to_vec();
        let outline = scaler.scale_outline(1).unwrap();
        let points = outline.points().to_vec();
        let verbs = outline.verbs().to_vec();
        let mut image = Image {
            source: Source::ColorBitmap(StrikeWith::Index(9)),
            content: Content::Color,
            placement: Placement { left: 7, top: 9, width: 3, height: 1 },
            data: vec![11, 22, 33],
        };
        let limits = FillRasterLimits { work_accesses: baseline.usage.work_accesses - 1, ..Default::default() };
        assert!(!Render::new(&[Source::Outline]).format(format).fill_raster_limits(limits).render_into(&mut scaler, 1, &mut image));
        assert_eq!(scaler.raster_error().unwrap().kind, RasterLimitKind::FillWork);
        assert_eq!(scaler.fill_raster_usage().cells, baseline.usage.cells);
        assert!(scaler.fill_raster_usage().maximum_live_cells > 0);
        assert_eq!(scaler.raster_usage().resize_calls, 1);
        assert_eq!(image.data, [11, 22, 33]);
        assert_eq!((image.placement.left, image.placement.top, image.placement.width, image.placement.height), (7, 9, 3, 1));
        assert_eq!(image.content, Content::Color);
        assert!(matches!(image.source, Source::ColorBitmap(StrikeWith::Index(9))));
        assert_eq!(scaler.coords, coordinates);
        assert_eq!(font.glyph_metrics(&[]).advance_width(1), advance);
        let restored_outline = scaler.scale_outline(1).unwrap();
        assert_eq!(restored_outline.points(), points);
        assert_eq!(restored_outline.verbs(), verbs);
        assert!(Render::new(&[Source::Outline]).format(format).render_into(&mut scaler, 1, &mut image));
        assert_same_image(&image, baseline.image.as_ref().unwrap());
        assert_eq!(scaler.raster_error(), None);
        assert_eq!(scaler.fill_raster_error(), None);
        assert_eq!(scaler.coords, coordinates);
    }
}

#[test]
fn ordered_strip_prefix_visits_consume_work_without_extra_cells() {
    let ascending = strips(true);
    let descending = strips(false);
    let forward = render_mask(&ascending, Format::Alpha, 32, 4, FillRasterLimits::default());
    let reverse = render_mask(&descending, Format::Alpha, 32, 4, FillRasterLimits::default());
    assert!(forward.result.is_ok() && reverse.result.is_ok());
    assert_eq!(forward.data, reverse.data);
    assert_eq!(forward.usage.cells, reverse.usage.cells);
    assert_eq!(forward.usage.storage_bytes, reverse.usage.storage_bytes);
    assert!(forward.usage.work_accesses > reverse.usage.work_accesses);
    let limits = FillRasterLimits { work_accesses: reverse.usage.work_accesses, ..Default::default() };
    let denied = render_mask(&ascending, Format::Alpha, 32, 4, limits);
    assert_eq!(denied.result.unwrap_err().kind, FillRasterLimitKind::Work);
    assert!(denied.usage.work_accesses <= limits.work_accesses);
    assert!(denied.usage.cells <= forward.usage.cells);
    // This small original oracle has a fixed 128-byte target and 64 points.
    let (expected, _) = Mask::new(&ascending).format(Format::Alpha).origin(Origin::BottomLeft).size(32, 4).render();
    assert_eq!(forward.data, expected);
}

#[test]
fn scratch_reset_and_denied_attempts_never_refund_the_shared_budget() {
    let path = strips(true);
    let baseline = render_mask(&path, Format::Alpha, 32, 4, FillRasterLimits::default());
    assert!(baseline.result.is_ok());
    for (limits, kind) in [
        (FillRasterLimits { cells: baseline.usage.cells, ..Default::default() }, FillRasterLimitKind::Cells),
        (FillRasterLimits { storage_bytes: baseline.usage.storage_bytes.max(baseline.usage.maximum_peak_storage_bytes), ..Default::default() }, FillRasterLimitKind::Storage),
        (FillRasterLimits { work_accesses: baseline.usage.work_accesses, ..Default::default() }, FillRasterLimitKind::Work),
    ] {
        let mut scratch = Scratch::new();
        let mut budget = FillRasterBudget::new(limits);
        let mut mask = Mask::with_scratch(&path, &mut scratch);
        mask.format(Format::Alpha).origin(Origin::BottomLeft).size(32, 4);
        let mut first = vec![0; 128];
        assert!(mask.render_into_with_budget(&mut first, None, &mut budget).is_ok());
        assert_eq!(first, baseline.data);
        let first_usage = budget.usage();
        let mut second = vec![0; 128];
        let error = mask.render_into_with_budget(&mut second, None, &mut budget).unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(budget.usage().cells >= first_usage.cells);
        assert!(budget.usage().storage_bytes >= first_usage.storage_bytes);
        assert!(budget.usage().work_accesses >= first_usage.work_accesses);
        assert_eq!(budget.usage().cell_growths, first_usage.cell_growths);
        assert_eq!(budget.usage().index_growths, first_usage.index_growths);
        let denied_usage = budget.usage();
        assert_eq!(mask.render_into_with_budget(&mut second, None, &mut budget).unwrap_err(), error);
        assert_eq!(budget.usage(), denied_usage);
        assert_eq!(first, baseline.data);
    }
}

#[test]
fn three_real_subpixel_passes_share_cells_storage_and_work_allowances() {
    let bytes = tiny_font();
    let alpha = render_font(&bytes, Format::Alpha, FillRasterLimits::default());
    let format = Format::CustomSubpixel([0.0, 0.0, 0.0]);
    let observed = render_font(&bytes, format, FillRasterLimits::default());
    assert!(observed.image.is_some());
    let exact = FillRasterLimits {
        cells: observed.usage.cells,
        storage_bytes: observed.usage.storage_bytes.max(observed.usage.maximum_peak_storage_bytes),
        work_accesses: observed.usage.work_accesses,
    };
    let admitted = render_font(&bytes, format, exact);
    let image = admitted.image.as_ref().unwrap();
    assert_eq!(admitted.usage.cells, exact.cells);
    assert_eq!(admitted.usage.cells, alpha.usage.cells * 3);
    assert_eq!(admitted.usage.indices, alpha.usage.indices * 3);
    assert_eq!(admitted.usage.cell_slots, alpha.usage.cell_slots * 3);
    assert_eq!(admitted.usage.index_slots, alpha.usage.index_slots * 3);
    assert_eq!(admitted.usage.storage_bytes, exact.storage_bytes);
    assert_eq!(admitted.usage.storage_bytes, alpha.usage.storage_bytes * 3);
    assert_eq!(admitted.usage.work_accesses, exact.work_accesses);
    // Each pass reserves its capacity without refund, but the same scratch
    // vectors grow only on the first pass. Later passes owe no new reserve fee.
    let reserve_attempts = alpha.usage.cell_reserve_attempts + alpha.usage.index_reserve_attempts;
    assert_eq!(admitted.usage.cell_reserve_attempts, alpha.usage.cell_reserve_attempts);
    assert_eq!(admitted.usage.index_reserve_attempts, alpha.usage.index_reserve_attempts);
    assert_eq!(admitted.usage.potential_relocation_bytes, 0);
    assert_eq!(admitted.usage.work_accesses, alpha.usage.work_accesses * 3 - reserve_attempts * 2);
    assert_eq!(admitted.usage.maximum_live_cells, alpha.usage.maximum_live_cells);
    for (pixel, alpha) in image.data.chunks_exact(4).zip(&alpha.image.as_ref().unwrap().data) {
        assert_eq!(pixel, &[*alpha, *alpha, *alpha, 0]);
    }
    for (limits, kind) in [
        (FillRasterLimits { cells: exact.cells - 1, ..exact }, FillRasterLimitKind::Cells),
        (FillRasterLimits { storage_bytes: exact.storage_bytes - 1, ..exact }, FillRasterLimitKind::Storage),
        (FillRasterLimits { work_accesses: exact.work_accesses - 1, ..exact }, FillRasterLimitKind::Work),
    ] {
        let denied = render_font(&bytes, format, limits);
        assert!(denied.image.is_none());
        assert_eq!(denied.error.unwrap().kind, kind);
        assert!(denied.usage.indices > alpha.usage.indices);
        assert!(denied.usage.cells <= limits.cells);
        assert!(denied.usage.storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.maximum_peak_storage_bytes <= limits.storage_bytes);
        assert!(denied.usage.work_accesses <= limits.work_accesses);
    }
}

#[test]
fn clipped_line_and_curve_loops_refuse_before_unbudgeted_continuation() {
    let mut vertical = Vec::new();
    vertical.move_to([16.0, 0.0]).line_to([16.0, 128.0]).line_to([17.0, 0.0]).close();
    let mut diagonal = Vec::new();
    diagonal.move_to([1.0, 0.0]).line_to([7.0, 128.0]).line_to([1.0, 0.0]).close();
    let mut quadratic = Vec::new();
    quadratic.move_to([1.0, 1.0]).quad_to([7.0, 7.0], [7.0, 1.0]).line_to([1.0, 1.0]).close();
    let mut cubic = Vec::new();
    cubic.move_to([1.0, 1.0]).curve_to([1.0, 7.0], [7.0, 7.0], [7.0, 1.0]).line_to([1.0, 1.0]).close();
    let limits = FillRasterLimits { work_accesses: 18, ..Default::default() };
    for path in [&vertical, &diagonal, &quadratic, &cubic] {
        let denied = render_mask(path, Format::Alpha, 8, 8, limits);
        assert_eq!(denied.result.unwrap_err().kind, FillRasterLimitKind::Work);
        assert!(denied.usage.work_accesses <= limits.work_accesses);
        assert_eq!(denied.data, [0; 64]);
    }
    let denied = render_mask(&vertical, Format::Alpha, 8, 8, limits);
    assert_eq!(denied.usage.cells, 0);
    assert_eq!(denied.usage.cell_growths, 0);
}

#[test]
fn guarded_public_target_math_is_checked_and_limits_only_tighten() {
    let mut budget = FillRasterBudget::new(FillRasterLimits {
        cells: usize::MAX,
        storage_bytes: usize::MAX,
        work_accesses: usize::MAX,
    });
    assert_eq!(budget.limits(), FillRasterLimits::default());
    let mut mask = Mask::new(&[] as &[Command]);
    mask.size(u32::MAX, 1);
    assert_eq!(mask.render_into_with_budget(&mut [], None, &mut budget).unwrap_err().kind, FillRasterLimitKind::Coordinates);
    assert_eq!(budget.usage().storage_bytes, 0);
    for (length, pitch) in [(15, None), (16, Some(3)), (16, Some(usize::MAX))] {
        let mut budget = FillRasterBudget::default();
        let mut data = vec![0; length];
        let mut mask = Mask::new(&[] as &[Command]);
        mask.size(4, 4);
        assert_eq!(mask.render_into_with_budget(&mut data, pitch, &mut budget).unwrap_err().kind, FillRasterLimitKind::Target);
        assert_eq!(budget.usage().index_growths, 0);
        assert_eq!(budget.usage().cell_growths, 0);
        assert_eq!(budget.usage().work_accesses, 0);
    }
    let mut budget = FillRasterBudget::new(FillRasterLimits { cells: 0, storage_bytes: 0, work_accesses: 1 });
    let mut mask = Mask::new(&[] as &[Command]);
    mask.size(0, 0);
    assert!(mask.render_into_with_budget(&mut [], None, &mut budget).is_ok());
}

#[test]
fn normal_noto_fill_formats_match_the_small_original_mask_oracle() {
    let fonts: &[&[u8]] = &[
        include_bytes!("../../../../assets/fonts/NotoSansSC-Regular.otf.gz"),
        include_bytes!("../../../../assets/fonts/NotoSerifSC-Bold.otf.gz"),
    ];
    for compressed in fonts {
        let mut bytes = Vec::new();
        GzDecoder::new(*compressed).take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 64 * 1024 * 1024);
        let font = FontRef::from_index(&bytes, 0).unwrap();
        for size in [30.0, 152.0] {
            for character in ['A', '\u{4e2d}'] {
                let glyph = font.charmap().map(character);
                assert_ne!(glyph, 0);
                let advance = font.glyph_metrics(&[]).advance_width(glyph);
                for fill in [Fill::NonZero, Fill::EvenOdd] {
                    for format in [Format::Alpha, Format::Subpixel, Format::subpixel_bgra()] {
                        let mut context = ScaleContext::new();
                        let mut scaler = context.builder(font).size(size).hint(false).build();
                        let coordinates = scaler.coords.to_vec();
                        let outline = scaler.scale_outline(glyph).unwrap();
                        assert!(outline.points().len() < 4096);
                        let offset = Vector::new(2.25, -1.5);
                        let mut oracle = Mask::new(outline.path());
                        oracle.format(format).origin(Origin::BottomLeft).style(fill).offset(offset).render_offset(offset)
                            .inspect(|_, width, height| {
                                // Only fixed, known-small Noto paths reach this unguarded oracle.
                                assert!(width <= 256 && height <= 256);
                            });
                        let (expected, placement) = oracle.render();
                        let image = Render::new(&[Source::Outline]).format(format).style(fill).offset(offset)
                            .render(&mut scaler, glyph).unwrap();
                        assert_eq!(image.data, expected);
                        assert_eq!(
                            (image.placement.left, image.placement.top, image.placement.width, image.placement.height),
                            (placement.left, placement.top, placement.width, placement.height),
                        );
                        assert_eq!(font.glyph_metrics(&[]).advance_width(glyph), advance);
                        assert_eq!(scaler.coords, coordinates);
                        assert_eq!(scaler.raster_error(), None);
                        assert_eq!(scaler.fill_raster_error(), None);
                        assert_eq!(scaler.raster_usage().work_bytes, image.data.len());
                        let passes = if format == Format::Alpha { 1 } else { 3 };
                        assert_eq!(scaler.fill_raster_usage().indices, image.placement.height as usize * passes);
                        assert!(scaler.fill_raster_usage().work_accesses > 0);
                    }
                }
            }
        }
    }
}
