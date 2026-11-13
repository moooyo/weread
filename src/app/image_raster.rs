//! Bounded, final-size picture preparation independent of display submission.

use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, imageops::FilterType};

use crate::preferences::ImageDithering;

const MAX_ENCODED_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SOURCE_EDGE: u32 = 8192;
const MAX_SOURCE_PIXELS: u64 = 16_000_000;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_VIEWPORT_PIXELS: u64 = 8_000_000;
const MAX_SCENE_RGB_BYTES: u64 = 64 * 1024 * 1024;
const RGB_BYTES: u64 = std::mem::size_of::<slint::Rgb8Pixel>() as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PictureFit {
    Contain,
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("image raster {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub(super) struct RasterLimitError {
    pub resource: &'static str,
    pub limit: u64,
    pub requested: u64,
}

fn limited(resource: &'static str, requested: u64, limit: u64) -> Result<()> {
    if requested > limit {
        return Err(RasterLimitError {
            resource,
            limit,
            requested,
        }
        .into());
    }
    Ok(())
}

/// A staged scene pays for every output picture, including its hardware fallback.
/// Failed preparation consumes no retained-picture allowance.
pub(super) struct RasterBudget {
    maximum_pixels: u64,
    pixels: u64,
    bytes: u64,
}

impl RasterBudget {
    pub(super) fn new(viewport: (u32, u32)) -> Result<Self> {
        ensure!(viewport.0 > 0 && viewport.1 > 0, "invalid image viewport");
        let pixels = u64::from(viewport.0) * u64::from(viewport.1);
        limited("viewport pixels", pixels, MAX_VIEWPORT_PIXELS)?;
        Ok(Self {
            maximum_pixels: pixels,
            pixels: 0,
            bytes: 0,
        })
    }

    fn forecast(&self, dimensions: (u32, u32), method: ImageDithering) -> Result<(u64, u64)> {
        let pixels = u64::from(dimensions.0) * u64::from(dimensions.1);
        let models = if method == ImageDithering::Hardware {
            2
        } else {
            1
        };
        let bytes = pixels
            .checked_mul(RGB_BYTES)
            .and_then(|bytes| bytes.checked_mul(models))
            .context("image raster storage overflow")?;
        let total_pixels = self
            .pixels
            .checked_add(pixels)
            .context("image pixel sum overflow")?;
        let total_bytes = self
            .bytes
            .checked_add(bytes)
            .context("image storage sum overflow")?;
        limited("scene pixels", total_pixels, self.maximum_pixels)?;
        limited("scene RGB bytes", total_bytes, MAX_SCENE_RGB_BYTES)?;
        Ok((total_pixels, total_bytes))
    }
}

pub(super) struct PreparedPicture {
    pub primary: slint::Image,
    pub software: slint::Image,
    pub dimensions: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RasterPlan {
    crop: (u32, u32, u32, u32),
    output: (u32, u32),
}

fn source_dimensions(dimensions: (u32, u32)) -> Result<()> {
    ensure!(
        dimensions.0 > 0 && dimensions.1 > 0,
        "invalid canonical image dimensions"
    );
    limited(
        "source width",
        u64::from(dimensions.0),
        u64::from(MAX_SOURCE_EDGE),
    )?;
    limited(
        "source height",
        u64::from(dimensions.1),
        u64::from(MAX_SOURCE_EDGE),
    )?;
    limited(
        "source pixels",
        u64::from(dimensions.0) * u64::from(dimensions.1),
        MAX_SOURCE_PIXELS,
    )
}

fn rounded_ratio(value: u32, numerator: u32, denominator: u32) -> u32 {
    ((u64::from(value) * u64::from(numerator) + u64::from(denominator) / 2)
        / u64::from(denominator)) as u32
}

fn raster_plan(source: (u32, u32), frame: (u32, u32), fit: PictureFit) -> Result<RasterPlan> {
    ensure!(frame.0 > 0 && frame.1 > 0, "invalid picture frame");
    limited(
        "frame width",
        u64::from(frame.0),
        u64::from(MAX_SOURCE_EDGE),
    )?;
    limited(
        "frame height",
        u64::from(frame.1),
        u64::from(MAX_SOURCE_EDGE),
    )?;
    let (width, height) = source;
    let source_is_wider =
        u64::from(width) * u64::from(frame.1) >= u64::from(height) * u64::from(frame.0);
    let plan = match fit {
        PictureFit::Contain => {
            let output = if source_is_wider {
                (
                    frame.0,
                    rounded_ratio(height, frame.0, width).clamp(1, frame.1),
                )
            } else {
                (
                    rounded_ratio(width, frame.1, height).clamp(1, frame.0),
                    frame.1,
                )
            };
            RasterPlan {
                crop: (0, 0, width, height),
                output,
            }
        }
        PictureFit::Cover => {
            // Crop in source coordinates before resizing. Resizing an extreme
            // aspect ratio to cover first could allocate an enormous temporary.
            let (crop_width, crop_height) = if source_is_wider {
                (
                    rounded_ratio(height, frame.0, frame.1).clamp(1, width),
                    height,
                )
            } else {
                (
                    width,
                    rounded_ratio(width, frame.1, frame.0).clamp(1, height),
                )
            };
            RasterPlan {
                crop: (
                    (width - crop_width) / 2,
                    (height - crop_height) / 2,
                    crop_width,
                    crop_height,
                ),
                output: frame,
            }
        }
    };
    Ok(plan)
}

fn encoded_image(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).context("open canonical picture")?;
    let length = file.metadata()?.len();
    limited("encoded bytes", length, MAX_ENCODED_BYTES)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize + 1)
        .context("reserve canonical picture bytes")?;
    file.take(MAX_ENCODED_BYTES + 1).read_to_end(&mut bytes)?;
    limited("encoded bytes", bytes.len() as u64, MAX_ENCODED_BYTES)?;
    Ok(bytes)
}

fn opaque_gray(decoded: DynamicImage) -> GrayImage {
    match decoded {
        DynamicImage::ImageLuma8(gray) => gray,
        decoded => {
            let mut rgba = decoded.into_rgba8();
            for pixel in rgba.pixels_mut() {
                let alpha = u32::from(pixel[3]);
                for channel in &mut pixel.0[..3] {
                    *channel =
                        ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
                }
                pixel[3] = 255;
            }
            DynamicImage::ImageRgba8(rgba).into_luma8()
        }
    }
}

fn quantize(value: u8, x: u32, y: u32, software: bool) -> u8 {
    const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let bias = if software {
        BAYER[(y % 4) as usize][(x % 4) as usize] - 8
    } else {
        0
    };
    let value = (i16::from(value) + bias).clamp(0, 255) as u16;
    (((value + 8) / 17).min(15) * 17) as u8
}

fn picture_models(gray: &GrayImage, method: ImageDithering) -> PreparedPicture {
    let (width, height) = gray.dimensions();
    let mut primary = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    let mut fallback = (method == ImageDithering::Hardware)
        .then(|| slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height));
    for (index, (destination, source)) in primary
        .make_mut_slice()
        .iter_mut()
        .zip(gray.pixels())
        .enumerate()
    {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let value = match method {
            ImageDithering::Hardware => source[0],
            ImageDithering::Software => quantize(source[0], x, y, true),
            ImageDithering::Off => quantize(source[0], x, y, false),
        };
        *destination = slint::Rgb8Pixel {
            r: value,
            g: value,
            b: value,
        };
    }
    if let Some(fallback) = fallback.as_mut() {
        for (index, (destination, source)) in fallback
            .make_mut_slice()
            .iter_mut()
            .zip(gray.pixels())
            .enumerate()
        {
            let value = quantize(source[0], index as u32 % width, index as u32 / width, true);
            *destination = slint::Rgb8Pixel {
                r: value,
                g: value,
                b: value,
            };
        }
    }
    let primary = slint::Image::from_rgb8(primary);
    let software = fallback
        .map(slint::Image::from_rgb8)
        .unwrap_or_else(|| primary.clone());
    PreparedPicture {
        primary,
        software,
        dimensions: (width, height),
    }
}

/// Prepare complete pixels before the owner can publish either model.
pub(super) fn prepare_picture(
    path: &Path,
    frame: (u32, u32),
    fit: PictureFit,
    method: ImageDithering,
    budget: &mut RasterBudget,
) -> Result<PreparedPicture> {
    let bytes = encoded_image(path)?;
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    ensure!(
        reader.format() == Some(image::ImageFormat::Png),
        "canonical picture is not PNG"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits.clone());
    let source = reader
        .into_dimensions()
        .context("inspect canonical picture dimensions")?;
    source_dimensions(source)?;
    let plan = raster_plan(source, frame, fit)?;
    let charge = budget.forecast(plan.output, method)?;
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    reader.limits(limits);
    let gray = opaque_gray(reader.decode().context("decode canonical picture")?);
    ensure!(
        gray.dimensions() == source,
        "canonical picture dimensions changed during decode"
    );
    let raster = if plan.crop == (0, 0, source.0, source.1) && plan.output == source {
        gray
    } else {
        let cropped =
            image::imageops::crop_imm(&gray, plan.crop.0, plan.crop.1, plan.crop.2, plan.crop.3);
        image::imageops::resize(
            &*cropped,
            plan.output.0,
            plan.output.1,
            FilterType::Triangle,
        )
    };
    let prepared = picture_models(&raster, method);
    budget.pixels = charge.0;
    budget.bytes = charge.1;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, io::Write};

    fn fixture(image: DynamicImage) -> Result<tempfile::NamedTempFile> {
        let mut file = tempfile::NamedTempFile::new()?;
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, image::ImageFormat::Png)?;
        file.write_all(output.get_ref())?;
        Ok(file)
    }

    fn luminances(image: &slint::Image) -> Vec<u8> {
        image
            .to_rgb8()
            .expect("The picture is an in-memory RGB buffer")
            .as_bytes()
            .chunks_exact(3)
            .map(|pixel| {
                assert_eq!(pixel[0], pixel[1]);
                assert_eq!(pixel[0], pixel[2]);
                pixel[0]
            })
            .collect()
    }

    #[test]
    fn hardware_preserves_every_gray8_input_and_prepares_software_fallback() -> Result<()> {
        let image = GrayImage::from_fn(256, 1, |x, _| image::Luma([x as u8]));
        let source = fixture(DynamicImage::ImageLuma8(image))?;
        let mut budget = RasterBudget::new((256, 1))?;
        let prepared = prepare_picture(
            source.path(),
            (256, 1),
            PictureFit::Contain,
            ImageDithering::Hardware,
            &mut budget,
        )?;
        assert_eq!(prepared.dimensions, (256, 1));
        assert_eq!(
            luminances(&prepared.primary),
            (0..=255).collect::<Vec<u8>>()
        );
        let fallback = luminances(&prepared.software);
        assert!(fallback.iter().all(|value| value % 17 == 0));
        assert_eq!(fallback.iter().copied().collect::<BTreeSet<_>>().len(), 16);
        assert_eq!((budget.pixels, budget.bytes), (256, 256 * RGB_BYTES * 2));
        Ok(())
    }

    #[test]
    fn software_and_off_are_distinct_final_size_sixteen_level_treatments() -> Result<()> {
        let source = fixture(DynamicImage::ImageLuma8(GrayImage::from_pixel(
            4,
            4,
            image::Luma([8]),
        )))?;
        let mut off_budget = RasterBudget::new((4, 4))?;
        let off = prepare_picture(
            source.path(),
            (4, 4),
            PictureFit::Contain,
            ImageDithering::Off,
            &mut off_budget,
        )?;
        assert_eq!(luminances(&off.primary), vec![0; 16]);
        assert_eq!(luminances(&off.primary), luminances(&off.software));
        let mut software_budget = RasterBudget::new((4, 4))?;
        let software = prepare_picture(
            source.path(),
            (4, 4),
            PictureFit::Contain,
            ImageDithering::Software,
            &mut software_budget,
        )?;
        let expected = (0..4)
            .flat_map(|y| (0..4).map(move |x| quantize(8, x, y, true)))
            .collect::<Vec<_>>();
        assert_eq!(luminances(&software.primary), expected);
        assert_ne!(luminances(&software.primary), luminances(&off.primary));
        assert_eq!(
            luminances(&software.primary),
            luminances(&software.software)
        );
        Ok(())
    }

    #[test]
    fn cover_crops_the_source_while_contain_keeps_its_aspect_ratio() -> Result<()> {
        assert_eq!(
            raster_plan((8192, 1), (1024, 1024), PictureFit::Cover)?,
            RasterPlan {
                crop: (4095, 0, 1, 1),
                output: (1024, 1024)
            },
        );
        let image = GrayImage::from_fn(8, 2, |x, _| image::Luma([(x * 20) as u8]));
        let source = fixture(DynamicImage::ImageLuma8(image))?;
        let mut budget = RasterBudget::new((4, 4))?;
        let cover = prepare_picture(
            source.path(),
            (2, 2),
            PictureFit::Cover,
            ImageDithering::Hardware,
            &mut budget,
        )?;
        assert_eq!(cover.dimensions, (2, 2));
        assert_eq!(luminances(&cover.primary), vec![60, 80, 60, 80]);
        let contain = prepare_picture(
            source.path(),
            (4, 4),
            PictureFit::Contain,
            ImageDithering::Hardware,
            &mut budget,
        )?;
        assert_eq!(contain.dimensions, (4, 1));
        assert_eq!(budget.pixels, 8);
        Ok(())
    }

    #[test]
    fn alpha_is_composited_over_white_before_gray8_publication() -> Result<()> {
        let source = fixture(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([0, 0, 0, 128]),
        )))?;
        let mut budget = RasterBudget::new((1, 1))?;
        let picture = prepare_picture(
            source.path(),
            (1, 1),
            PictureFit::Contain,
            ImageDithering::Hardware,
            &mut budget,
        )?;
        assert_eq!(luminances(&picture.primary), vec![127]);
        Ok(())
    }

    #[test]
    fn output_admission_is_cumulative_and_failed_work_cannot_consume_it() -> Result<()> {
        let source = fixture(DynamicImage::ImageLuma8(GrayImage::new(2, 2)))?;
        let mut budget = RasterBudget::new((2, 2))?;
        prepare_picture(
            source.path(),
            (2, 2),
            PictureFit::Contain,
            ImageDithering::Hardware,
            &mut budget,
        )?;
        let usage = (budget.pixels, budget.bytes);
        let error = prepare_picture(
            source.path(),
            (1, 1),
            PictureFit::Cover,
            ImageDithering::Hardware,
            &mut budget,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.downcast_ref::<RasterLimitError>().unwrap().resource,
            "scene pixels"
        );
        assert_eq!((budget.pixels, budget.bytes), usage);
        let mut invalid = tempfile::NamedTempFile::new()?;
        invalid.write_all(b"not a PNG")?;
        let mut empty = RasterBudget::new((2, 2))?;
        assert!(
            prepare_picture(
                invalid.path(),
                (2, 2),
                PictureFit::Cover,
                ImageDithering::Off,
                &mut empty
            )
            .is_err()
        );
        assert_eq!((empty.pixels, empty.bytes), (0, 0));
        Ok(())
    }

    #[test]
    fn encoded_source_and_viewport_limits_are_independent() -> Result<()> {
        assert!(RasterBudget::new((0, 1)).is_err());
        assert!(RasterBudget::new((8192, 8192)).is_err());
        assert!(source_dimensions((8193, 1)).is_err());
        assert!(source_dimensions((4000, 4001)).is_err());
        let source = tempfile::NamedTempFile::new()?;
        source.as_file().set_len(MAX_ENCODED_BYTES + 1)?;
        let mut budget = RasterBudget::new((1, 1))?;
        let error = prepare_picture(
            source.path(),
            (1, 1),
            PictureFit::Cover,
            ImageDithering::Off,
            &mut budget,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.downcast_ref::<RasterLimitError>().unwrap().resource,
            "encoded bytes"
        );
        let mut full_bytes = RasterBudget {
            maximum_pixels: 4,
            pixels: 0,
            bytes: MAX_SCENE_RGB_BYTES - 1,
        };
        assert_eq!(
            full_bytes
                .forecast((1, 1), ImageDithering::Hardware)
                .err()
                .unwrap()
                .downcast_ref::<RasterLimitError>()
                .unwrap()
                .resource,
            "scene RGB bytes"
        );
        full_bytes.bytes = 0;
        assert!(
            full_bytes
                .forecast((1, 1), ImageDithering::Hardware)
                .is_ok()
        );
        Ok(())
    }
}
