use super::{GrayMode, GrayRenderer};
use crate::dithering::{
    DitheringControl, PhysicalImageRegion, RawImageRegion, physical_image_regions,
};
use crate::refresh::PhysicalRect;
use slint::Rgb8Pixel;
use std::ops::Range;

// Independently evaluated from a Gray110 input and the reviewed 4x4 palette.
// The Bayer phase belongs to the final-size image, before presentation rotation.
const GRAY_110: [[u8; 4]; 4] = [
    [102, 102, 102, 119],
    [119, 102, 119, 102],
    [102, 119, 102, 119],
    [119, 102, 119, 102],
];

fn gray(value: u8) -> Rgb8Pixel {
    Rgb8Pixel {
        r: value,
        g: value,
        b: value,
    }
}

fn image(x: u32, y: u32, width: u32, height: u32) -> RawImageRegion {
    RawImageRegion {
        x,
        y,
        width,
        height,
    }
}

fn contains(rect: RawImageRegion, x: usize, y: usize) -> bool {
    x >= rect.x as usize
        && x < (rect.x + rect.width) as usize
        && y >= rect.y as usize
        && y < (rect.y + rect.height) as usize
}

fn put_image(rgb: &mut [Rgb8Pixel], width: usize, rect: RawImageRegion, value: u8) {
    for y in rect.y as usize..(rect.y + rect.height) as usize {
        for x in rect.x as usize..(rect.x + rect.width) as usize {
            rgb[y * width + x] = gray(value);
        }
    }
}

struct Panel {
    width: usize,
    height: usize,
    stride: usize,
    bytes: Vec<u8>,
    writes: Vec<(usize, Range<usize>)>,
}

impl Panel {
    fn new(width: usize, height: usize) -> Self {
        let stride = width + 3;
        Self {
            width,
            height,
            stride,
            bytes: vec![0x5d; stride * height],
            writes: Vec::new(),
        }
    }

    fn write(&mut self, row: usize, columns: Range<usize>, pixels: &[u8]) {
        assert!(row < self.height);
        assert!(columns.start <= columns.end && columns.end <= self.width);
        assert_eq!(columns.len(), pixels.len());
        let start = row * self.stride + columns.start;
        self.bytes[start..start + pixels.len()].copy_from_slice(pixels);
        self.writes.push((row, columns));
    }

    fn full(
        &mut self,
        renderer: &mut GrayRenderer,
        rgb: &[Rgb8Pixel],
        mode: GrayMode,
        images: &[PhysicalImageRegion],
    ) {
        let size = (self.width as u32, self.height as u32);
        renderer.write_frame_with_image_regions(
            rgb,
            size,
            None,
            mode,
            true,
            images,
            |row, columns, pixels| self.write(row, columns, pixels),
        );
    }

    fn rewrite(
        &mut self,
        renderer: &mut GrayRenderer,
        rgb: &[Rgb8Pixel],
        mode: GrayMode,
        images: &[PhysicalImageRegion],
    ) {
        let size = (self.width as u32, self.height as u32);
        renderer.rewrite_image_regions(rgb, size, images, mode, |row, columns, pixels| {
            self.write(row, columns, pixels);
        });
    }

    fn pixel(&self, x: usize, y: usize) -> u8 {
        self.bytes[y * self.stride + x]
    }

    fn write_counts(&self) -> Vec<usize> {
        let mut counts = vec![0; self.width * self.height];
        for (row, columns) in &self.writes {
            for column in columns.clone() {
                counts[*row * self.width + column] += 1;
            }
        }
        counts
    }

    fn assert_padding_untouched(&self) {
        for row in self.bytes.chunks_exact(self.stride) {
            assert_eq!(&row[self.width..], &[0x5d; 3]);
        }
    }
}

#[test]
fn software_images_do_not_quantize_antialiased_glyphs_outside_their_regions() {
    let size = (6, 6);
    let raw = image(1, 1, 4, 4);
    let mut rgb = vec![gray(128); 36];
    put_image(&mut rgb, 6, raw, 110);
    rgb[0] = gray(127);
    rgb[5] = gray(129);
    rgb[30] = gray(73);
    let original = rgb.clone();
    let images = physical_image_regions(&[raw], size, 0);
    let mut renderer = GrayRenderer::new(6, GrayMode::default());
    let mut panel = Panel::new(6, 6);
    panel.full(&mut renderer, &rgb, GrayMode::default(), &images);
    for y in 0..6 {
        for x in 0..6 {
            let expected = if contains(raw, x, y) {
                GRAY_110[y - 1][x - 1]
            } else {
                rgb[y * 6 + x].r
            };
            assert_eq!(panel.pixel(x, y), expected, "pixel {x},{y}");
        }
    }
    assert_eq!(rgb, original);
    panel.assert_padding_untouched();
}

#[test]
fn overlapping_image_regions_rewrite_each_pixel_once_without_writing_a_glyph() {
    let raw = image(1, 1, 4, 4);
    let mut rgb = vec![gray(128); 36];
    put_image(&mut rgb, 6, raw, 110);
    let original = rgb.clone();
    let overlapping = image(3, 1, 2, 4);
    let images = physical_image_regions(&[raw, overlapping, overlapping], (6, 6), 0);
    let mode = GrayMode {
        dark_mode: true,
        black_and_white: false,
    };
    let mut renderer = GrayRenderer::new(6, mode);
    let mut panel = Panel::new(6, 6);
    panel.full(&mut renderer, &rgb, mode, &[]);
    for _ in 0..2 {
        panel.writes.clear();
        panel.rewrite(&mut renderer, &rgb, mode, &images);
        let counts = panel.write_counts();
        for y in 0..6 {
            for x in 0..6 {
                let expected = if contains(raw, x, y) {
                    assert_eq!(counts[y * 6 + x], 1, "image {x},{y} must be rewritten once");
                    let local_x = if contains(overlapping, x, y) {
                        x - 3
                    } else {
                        x - 1
                    };
                    255 - GRAY_110[y - 1][local_x]
                } else {
                    assert_eq!(counts[y * 6 + x], 0, "glyph {x},{y} was rewritten");
                    127
                };
                assert_eq!(panel.pixel(x, y), expected);
            }
        }
    }
    assert_eq!(rgb, original);
    panel.assert_padding_untouched();
}

#[test]
fn image_rewrites_apply_day_or_native_compensation_once_from_canonical_rgb() {
    let raw = image(1, 1, 4, 4);
    let mut rgb = vec![gray(128); 36];
    put_image(&mut rgb, 6, raw, 110);
    let original = rgb.clone();
    let images = physical_image_regions(&[raw], (6, 6), 0);
    let mut renderer = GrayRenderer::new(6, GrayMode::default());
    let mut panel = Panel::new(6, 6);
    for inverted in [false, true, false] {
        let mode = GrayMode {
            dark_mode: inverted,
            black_and_white: false,
        };
        panel.full(&mut renderer, &rgb, mode, &[]);
        panel.writes.clear();
        panel.rewrite(&mut renderer, &rgb, mode, &images);
        let counts = panel.write_counts();
        for y in 0..6 {
            for x in 0..6 {
                let logical = if contains(raw, x, y) {
                    assert_eq!(counts[y * 6 + x], 1);
                    GRAY_110[y - 1][x - 1]
                } else {
                    assert_eq!(counts[y * 6 + x], 0);
                    128
                };
                assert_eq!(
                    panel.pixel(x, y),
                    if inverted { 255 - logical } else { logical }
                );
            }
        }
        assert_eq!(rgb, original);
    }
    panel.assert_padding_untouched();
}

#[test]
fn bilevel_image_rewrites_keep_the_binary_threshold_without_an_extra_bayer_pass() {
    let raw = image(1, 1, 4, 4);
    let images = physical_image_regions(&[raw], (6, 6), 0);
    let mut renderer = GrayRenderer::new(6, GrayMode::default());
    let mut panel = Panel::new(6, 6);
    for image_value in [127, 128] {
        let mut rgb = vec![gray(128); 36];
        put_image(&mut rgb, 6, raw, image_value);
        let original = rgb.clone();
        for inverted in [false, true] {
            let mode = GrayMode {
                dark_mode: inverted,
                black_and_white: true,
            };
            panel.full(&mut renderer, &rgb, mode, &[]);
            panel.writes.clear();
            panel.rewrite(&mut renderer, &rgb, mode, &images);
            for y in 0..6 {
                for x in 0..6 {
                    let logical = if contains(raw, x, y) && image_value < 128 {
                        0
                    } else {
                        255
                    };
                    assert_eq!(
                        panel.pixel(x, y),
                        if inverted { 255 - logical } else { logical }
                    );
                }
            }
            assert_eq!(rgb, original);
        }
    }
    panel.assert_padding_untouched();
}

#[test]
fn image_local_bayer_phase_rotates_with_the_image_and_keeps_body_gray8() {
    let size = (8, 6);
    let source = image(1, 2, 3, 2);
    let cases: [(u32, RawImageRegion, &[u8]); 4] = [
        (0, image(1, 2, 3, 2), &[102, 102, 102, 119, 102, 119]),
        (90, image(4, 1, 2, 3), &[119, 102, 102, 102, 119, 102]),
        (180, image(4, 2, 3, 2), &[119, 102, 119, 102, 102, 102]),
        (270, image(2, 2, 2, 3), &[102, 119, 102, 102, 102, 119]),
    ];
    for (rotation, expected_rect, expected_image) in cases {
        let images = physical_image_regions(&[source], size, rotation);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].region, expected_rect);
        let mut rgb = vec![gray(128); 48];
        put_image(&mut rgb, 8, expected_rect, 110);
        let original = rgb.clone();
        let mut renderer = GrayRenderer::new(8, GrayMode::default());
        let mut panel = Panel::new(8, 6);
        panel.full(&mut renderer, &rgb, GrayMode::default(), &images);
        for y in 0..6 {
            for x in 0..8 {
                let expected = if contains(expected_rect, x, y) {
                    let index = (y - expected_rect.y as usize) * expected_rect.width as usize + x
                        - expected_rect.x as usize;
                    expected_image[index]
                } else {
                    128
                };
                assert_eq!(
                    panel.pixel(x, y),
                    expected,
                    "rotation {rotation}, pixel {x},{y}"
                );
            }
        }
        assert_eq!(rgb, original);
        panel.assert_padding_untouched();
    }
}

#[test]
fn clipped_images_keep_the_full_source_phase_instead_of_restarting_at_the_visible_edge() {
    let cases: [(RawImageRegion, u32, RawImageRegion, &[u8]); 4] = [
        (image(5, 6, 4, 4), 90, image(0, 5, 2, 1), &[119, 102]),
        (image(5, 6, 4, 4), 270, image(6, 0, 2, 1), &[102, 119]),
        (image(5, 5, 4, 4), 90, image(0, 5, 3, 1), &[102, 119, 102]),
        (image(5, 5, 4, 4), 270, image(5, 0, 3, 1), &[102, 119, 102]),
    ];
    for (source, rotation, expected_rect, expected_image) in cases {
        let images = physical_image_regions(&[source], (8, 6), rotation);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].region, expected_rect);
        assert_eq!(images[0].source, source);
        let mut rgb = vec![gray(128); 48];
        put_image(&mut rgb, 8, expected_rect, 110);
        let mut renderer = GrayRenderer::new(8, GrayMode::default());
        let mut panel = Panel::new(8, 6);
        panel.full(&mut renderer, &rgb, GrayMode::default(), &images);
        for y in 0..6 {
            for x in 0..8 {
                let expected = if contains(expected_rect, x, y) {
                    expected_image[x - expected_rect.x as usize]
                } else {
                    128
                };
                assert_eq!(
                    panel.pixel(x, y),
                    expected,
                    "rotation {rotation}, pixel {x},{y}"
                );
            }
        }
        panel.assert_padding_untouched();
    }
}

#[test]
fn partial_redraws_do_not_restart_image_phase_at_the_dirty_rectangle_origin() {
    let raw = image(1, 1, 4, 4);
    let images = physical_image_regions(&[raw], (6, 6), 0);
    let mut rgb = vec![gray(128); 36];
    put_image(&mut rgb, 6, raw, 110);
    let original = rgb.clone();
    let mode = GrayMode::default();
    let mut renderer = GrayRenderer::new(6, mode);
    let mut panel = Panel::new(6, 6);
    panel.full(&mut renderer, &rgb, mode, &[]);
    panel.writes.clear();
    renderer.write_frame_with_image_regions(
        &rgb,
        (6, 6),
        Some(PhysicalRect {
            x: 3,
            y: 2,
            width: 2,
            height: 2,
        }),
        mode,
        false,
        &images,
        |row, columns, pixels| panel.write(row, columns, pixels),
    );
    let counts = panel.write_counts();
    for y in 0..6 {
        for x in 0..6 {
            let changed = (3..5).contains(&x) && (2..4).contains(&y);
            assert_eq!(counts[y * 6 + x], usize::from(changed));
            let expected = if changed {
                GRAY_110[y - 1][x - 1]
            } else {
                rgb[y * 6 + x].r
            };
            assert_eq!(panel.pixel(x, y), expected);
        }
    }
    assert_eq!(rgb, original);
    panel.assert_padding_untouched();
}

#[test]
fn native_full_snapshots_finish_retained_images_without_modifying_a_glyph_or_the_frame() {
    let raw = image(1, 1, 4, 4);
    let control = DitheringControl::new(true);
    control.publish(vec![raw]).unwrap();
    let retained = control.snapshot().unwrap();
    control.publish(vec![image(0, 0, 1, 1)]).unwrap();
    let images = physical_image_regions(&retained.regions, (6, 6), 0);
    let mut rgb = vec![gray(128); 36];
    put_image(&mut rgb, 6, raw, 110);
    let original = rgb.clone();
    let retained_regions = retained.regions.clone();
    let mut renderer = GrayRenderer::new(6, GrayMode::default());
    let mut panel = Panel::new(6, 6);
    for native_inverse in [false, true, false] {
        let native_mode = GrayMode {
            dark_mode: native_inverse,
            black_and_white: false,
        };
        panel.full(&mut renderer, &rgb, native_mode, &images);
        for y in 0..6 {
            for x in 0..6 {
                let logical = if contains(raw, x, y) {
                    GRAY_110[y - 1][x - 1]
                } else {
                    128
                };
                let expected = if native_inverse {
                    255 - logical
                } else {
                    logical
                };
                assert_eq!(panel.pixel(x, y), expected);
            }
        }
        assert_eq!(rgb, original);
        assert_eq!(retained.regions, retained_regions);
    }
    assert_eq!(retained.regions.as_ref(), &[raw]);
    panel.assert_padding_untouched();
}
