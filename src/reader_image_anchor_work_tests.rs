//! Small typed-image regressions for operation-local anchor projection.

use super::*;

fn image(id: &str, anchor: u32) -> InlineImage {
    InlineImage {
        id: id.into(),
        anchor,
        width: 48.0,
        height: 24.0,
    }
}

fn paginate(text: &str, images: &[InlineImage]) -> Paginator {
    let layout = Layout {
        width: 240.0,
        height: 360.0,
        font_size: 20.0,
        indent: 0,
        justify: false,
        ..Default::default()
    };
    Paginator::new_with_images(
        text,
        &layout,
        &TextMetrics::default(),
        layout.height,
        images,
    )
}

fn placed(paginator: &Paginator) -> Vec<(String, u32)> {
    paginator
        .pages
        .iter()
        .flat_map(|page| page.images.iter())
        .map(|image| (image.id.clone(), image.anchor))
        .collect()
}

#[test]
fn repeated_typed_anchors_share_one_zero_width_run_scan() {
    let text = format!("{}Tail.", "\u{200b}".repeat(96));
    let images = (0..32)
        .map(|index| image(&format!("image-{index}"), 0))
        .collect::<Vec<_>>();
    reset_image_anchor_work();
    let paginator = paginate(&text, &images);
    let work = image_anchor_work();
    assert_eq!(work.projections, 1);
    assert_eq!(work.images, images.len());
    assert_eq!(work.scalar_checks, 97);
    assert!(work.scalar_checks <= text.chars().count() + images.len());
    assert_eq!(
        placed(&paginator),
        images
            .iter()
            .map(|image| (image.id.clone(), 96))
            .collect::<Vec<_>>(),
    );
    assert_eq!(paginator.char_count as usize, text.chars().count());
    assert_eq!(paginator.pages.last().unwrap().end, paginator.char_count);
}

#[test]
fn distinct_anchors_in_one_run_keep_original_order_after_collapsing() {
    let text = format!("A{}B\r\nC", "\u{301}".repeat(32));
    let mut images = (1..=32)
        .rev()
        .map(|anchor| image(&format!("run-{anchor}"), anchor))
        .collect::<Vec<_>>();
    images.extend([
        image("at-run-end", 33),
        image("inside-crlf", 35),
        image("final-boundary", 37),
    ]);
    reset_image_anchor_work();
    let paginator = paginate(&text, &images);
    let mut expected = images[..32]
        .iter()
        .map(|image| (image.id.clone(), 33))
        .collect::<Vec<_>>();
    expected.extend([
        (String::from("at-run-end"), 33),
        (String::from("inside-crlf"), 36),
        (String::from("final-boundary"), 37),
    ]);
    assert_eq!(placed(&paginator), expected);
    let work = image_anchor_work();
    assert_eq!(work.images, images.len());
    assert_eq!(work.scalar_checks, 35);
    assert!(work.scalar_checks <= text.chars().count() + images.len());
    assert_eq!(paginator.char_count, 37);
    assert_eq!(paginator.pages.last().unwrap().end, 37);
}

#[test]
fn invalid_images_do_not_enter_anchor_projection_or_change_text_fallback() {
    let text = "A\u{301}B\r\nC";
    let images = [
        image("", 0),
        image("outside-body", 100),
        InlineImage {
            width: f32::NAN,
            ..image("nan-width", 0)
        },
        InlineImage {
            height: 0.0,
            ..image("zero-height", 0)
        },
    ];
    reset_image_anchor_work();
    let paginator = paginate(text, &images);
    assert_eq!(image_anchor_work(), ImageAnchorWork::default());
    assert!(placed(&paginator).is_empty());
    assert_eq!(paginator.char_count as usize, text.chars().count());
    assert_eq!(paginator.pages.last().unwrap().end, paginator.char_count);
}
