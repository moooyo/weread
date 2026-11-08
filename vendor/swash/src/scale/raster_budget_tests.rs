//! Real outline builders exercise physical admission before allocation.

use super::*;
use super::raster_budget::Budget;
use flate2::read::GzDecoder;
use std::io::Read;

#[path = "../../../../src/reader_caret_font.rs"]
mod original_font;

fn word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn integer(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, bytes| {
        let mut word = [0; 4];
        word[..bytes.len()].copy_from_slice(bytes);
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

fn rectangle(edge: i16) -> [u8; 34] {
    let mut glyph = [0; 34];
    word(&mut glyph, 0, 1);
    glyph[6..8].copy_from_slice(&edge.to_be_bytes());
    glyph[8..10].copy_from_slice(&edge.to_be_bytes());
    word(&mut glyph, 10, 3);
    glyph[14..18].fill(1);
    for (index, delta) in [0, edge, 0, -edge].into_iter().enumerate() {
        glyph[18 + 2 * index..20 + 2 * index].copy_from_slice(&delta.to_be_bytes());
    }
    for (index, delta) in [0, 0, edge, 0].into_iter().enumerate() {
        glyph[26 + 2 * index..28 + 2 * index].copy_from_slice(&delta.to_be_bytes());
    }
    glyph
}

fn font(edge: i16, upem: u16) -> Vec<u8> {
    let mut bytes = original_font::bytes();
    let (head, _, _) = table(&bytes, b"head");
    word(&mut bytes, head + 18, upem);
    bytes[head + 40..head + 42].copy_from_slice(&edge.to_be_bytes());
    bytes[head + 42..head + 44].copy_from_slice(&edge.to_be_bytes());
    integer(&mut bytes, head + 8, 0);
    let (glyf, _, _) = table(&bytes, b"glyf");
    for offset in [10, 44, 78] {
        bytes[glyf + offset..glyf + offset + 34].copy_from_slice(&rectangle(edge));
    }
    let (hhea, _, _) = table(&bytes, b"hhea");
    word(&mut bytes, hhea + 4, upem);
    word(&mut bytes, hhea + 6, 0);
    word(&mut bytes, hhea + 10, upem);
    let (hmtx, _, _) = table(&bytes, b"hmtx");
    for glyph in [0, 1, 3] {
        word(&mut bytes, hmtx + glyph * 4, upem);
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

fn render(bytes: &[u8], size: f32, limits: RasterLimits, format: Format) -> (Option<Image>, RasterUsage, Option<RasterLimitError>) {
    let font = FontRef::from_index(bytes, 0).expect("The original four-point font is valid");
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(size).hint(false).build();
    let image = Render::new(&[Source::Outline])
        .format(format)
        .raster_limits(limits)
        .render(&mut scaler, 1);
    (image, scaler.raster_usage(), scaler.raster_error())
}

#[test]
fn actual_four_point_alpha_exact_and_one_less_limits_precede_growth() {
    let bytes = font(4, 16);
    let exact = RasterLimits { dimension: 4, pixels: 16, buffer_bytes: 16, work_bytes: 16 };
    let (image, usage, error) = render(&bytes, 16.0, exact, Format::Alpha);
    let image = image.expect("The exact physical mask fits its allowance");
    assert_eq!((image.placement.width, image.placement.height), (4, 4));
    assert_eq!(image.data.len(), 16);
    assert_eq!(usage.stride, 4);
    assert_eq!(usage.work_bytes, 16);
    assert_eq!(usage.resize_calls, 1);
    assert_eq!(usage.growth_allocations, 1);
    assert_eq!(error, None);
    for limits in [
        RasterLimits { dimension: 3, ..exact },
        RasterLimits { pixels: 15, ..exact },
        RasterLimits { buffer_bytes: 15, ..exact },
        RasterLimits { work_bytes: 15, ..exact },
    ] {
        let (image, usage, error) = render(&bytes, 16.0, limits, Format::Alpha);
        assert!(image.is_none());
        assert_eq!((usage.width, usage.height), (4, 4));
        assert_eq!(usage.resize_calls, 0);
        assert_eq!(usage.growth_allocations, 0);
        assert_eq!(usage.maximum_buffer_bytes, 0);
        assert!(error.is_some());
    }
}

#[test]
fn menu_physical_size_refuses_large_mask_without_allocating_or_rebinding_metrics() {
    let bytes = font(16_000, 16);
    let font = FontRef::from_index(&bytes, 0).unwrap();
    let advance = font.glyph_metrics(&[]).advance_width(1);
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(30.0).hint(false).build();
    let image = Render::new(&[Source::Outline]).format(Format::Alpha).render(&mut scaler, 1);
    // A 15-logical-pixel menu at scale two reaches this actual physical builder.
    // The renderer's later small viewport clip cannot pre-admit the mask.
    assert!(image.is_none());
    assert_eq!((scaler.raster_usage().width, scaler.raster_usage().height), (30_000, 30_000));
    assert_eq!(scaler.raster_usage().work_bytes, 0);
    assert_eq!(scaler.raster_usage().resize_calls, 0);
    assert_eq!(scaler.raster_usage().growth_allocations, 0);
    assert_eq!(scaler.raster_error().unwrap().kind, RasterLimitKind::Dimensions);
    assert_eq!(font.glyph_metrics(&[]).advance_width(1), advance);
    assert_eq!(advance, 16.0);
}

#[test]
fn resource_refusal_preserves_the_callers_existing_image() {
    let bytes = font(16_000, 16);
    let font = FontRef::from_index(&bytes, 0).unwrap();
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(30.0).hint(false).build();
    let mut image = Image {
        source: Source::Outline,
        content: Content::Mask,
        placement: Placement { left: 7, top: 9, width: 3, height: 1 },
        data: vec![11, 22, 33],
    };
    assert!(!Render::new(&[Source::Outline]).format(Format::Alpha).render_into(&mut scaler, 1, &mut image));
    assert_eq!(image.data, [11, 22, 33]);
    assert_eq!((image.placement.left, image.placement.top, image.placement.width, image.placement.height), (7, 9, 3, 1));
    assert_eq!(image.content, Content::Mask);
    assert!(matches!(image.source, Source::Outline));
    assert_eq!(scaler.raster_usage().growth_allocations, 0);
}

#[test]
fn shared_format_math_prices_real_alpha_and_rgba_subpixel_bytes() {
    let bytes = font(4, 16);
    for (format, channels) in [
        (Format::Alpha, 1usize),
        (Format::Subpixel, 4usize),
        (Format::CustomSubpixel([0.3, 0.0, -0.3]), 4usize),
    ] {
        let length = 16 * channels;
        let exact = RasterLimits { buffer_bytes: length, work_bytes: length, ..Default::default() };
        let (image, usage, _) = render(&bytes, 16.0, exact, format);
        assert_eq!(image.unwrap().data.len(), length);
        assert_eq!(usage.stride, 4 * channels);
        let (image, usage, error) = render(&bytes, 16.0, RasterLimits { buffer_bytes: length - 1, ..exact }, format);
        assert!(image.is_none());
        assert_eq!(usage.growth_allocations, 0);
        assert_eq!(error.unwrap().kind, RasterLimitKind::Bytes);
    }
}

#[test]
fn checked_layout_and_work_overflow_cannot_raise_project_ceilings() {
    let mut budget = Budget::new(RasterLimits {
        dimension: u32::MAX,
        pixels: u64::MAX,
        buffer_bytes: usize::MAX,
        work_bytes: usize::MAX,
    });
    assert!(budget.layout(u32::MAX, u32::MAX, 4).is_err());
    assert!(budget.layout(2049, 1, 1).is_err());
    assert!(budget.layout(4, 4, 5).is_err());
    let rgb = budget.layout(4, 4, 3).unwrap();
    assert_eq!((rgb.stride, rgb.bytes), (12, 48));
    let mut reservation = budget.prepay(1).unwrap();
    let mut data = Vec::new();
    budget.resize(&mut data, 1, &mut reservation).unwrap();
    assert!(budget.prepay(usize::MAX).is_err());
    assert_eq!(budget.usage.work_bytes, 1);
    assert_eq!(data, [0]);
}

#[test]
fn normal_noto_masks_match_original_zeno_bytes_placement_and_metrics() {
    let fonts: &[&[u8]] = &[
        include_bytes!("../../../../assets/fonts/NotoSansSC-Regular.otf.gz"),
        include_bytes!("../../../../assets/fonts/NotoSerifSC-Bold.otf.gz"),
    ];
    for compressed in fonts {
        let mut bytes = Vec::new();
        GzDecoder::new(*compressed).take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 64 * 1024 * 1024);
        let font = FontRef::from_index(&bytes, 0).unwrap();
        for size in [30.0, 76.0, 152.0] {
            for character in ['A', '\u{4e2d}'] {
                let glyph = font.charmap().map(character);
                assert_ne!(glyph, 0);
                let advance = font.glyph_metrics(&[]).advance_width(glyph);
                let mut context = ScaleContext::new();
                let mut scaler = context.builder(font).size(size).hint(false).build();
                let outline = scaler.scale_outline(glyph).unwrap();
                // Only these known-small normal masks use the original oracle.
                let mut oracle = Mask::new(outline.path());
                oracle.format(Format::Alpha).origin(Origin::BottomLeft).inspect(|_, width, height| {
                    // Match the original Swash inspect-before-render placement.
                    // Keep the unguarded oracle within this small fixed fixture.
                    assert!(width <= 2048 && height <= 2048);
                });
                let (expected, placement) = oracle.render();
                let image = Render::new(&[Source::Outline]).format(Format::Alpha).render(&mut scaler, glyph).unwrap();
                assert_eq!(image.data, expected);
                assert_eq!((image.placement.left, image.placement.top, image.placement.width, image.placement.height), (placement.left, placement.top, placement.width, placement.height));
                assert_eq!(font.glyph_metrics(&[]).advance_width(glyph), advance);
                assert_eq!(scaler.raster_error(), None);
                assert_eq!(scaler.raster_usage().resize_calls, 1);
                assert_eq!(scaler.raster_usage().work_bytes, image.data.len());
            }
        }
    }
}
