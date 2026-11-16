//! Bounded, final-size picture preparation independent of display submission.

use std::{
    fs::{Metadata, OpenOptions},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, imageops::FilterType};
use sha2::{Digest, Sha256};

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

/// A candidate cache object, not retained proof that its bytes are still valid.
/// The UI verifies the current source again before consuming a ready scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PictureRequest {
    pub path: PathBuf,
    pub frame: (u32, u32),
    pub fit: PictureFit,
    pub method: ImageDithering,
}

/// Immutable final-size payloads are shared across the worker/UI boundary.
/// Slint images are constructed only on the UI thread, without copying pixels.
#[derive(Clone, Debug)]
pub(super) struct PreparedPixels {
    primary: slint::SharedPixelBuffer<slint::Rgb8Pixel>,
    software: Option<slint::SharedPixelBuffer<slint::Rgb8Pixel>>,
    pub dimensions: (u32, u32),
}

impl PreparedPixels {
    pub(super) fn into_picture(self) -> PreparedPicture {
        let primary = slint::Image::from_rgb8(self.primary);
        let software = self
            .software
            .map(slint::Image::from_rgb8)
            .unwrap_or_else(|| primary.clone());
        PreparedPicture {
            primary,
            software,
            dimensions: self.dimensions,
        }
    }

    #[cfg(test)]
    pub(super) fn storage_pointer(&self) -> *const u8 {
        self.primary.as_bytes().as_ptr()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("image preparation was cancelled")]
pub(super) struct PreparationCancelled;

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Acquire), PreparationCancelled);
    Ok(())
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

fn encoded_image(path: &Path, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    check_cancelled(cancelled)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).context("open canonical picture")?;
    ensure!(
        file.metadata()?.is_file(),
        "canonical picture is not a regular file"
    );
    let length = file.metadata()?.len();
    limited("encoded bytes", length, MAX_ENCODED_BYTES)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize + 1)
        .context("reserve canonical picture bytes")?;
    let mut buffer = [0; 32 * 1024];
    loop {
        check_cancelled(cancelled)?;
        let remaining = (MAX_ENCODED_BYTES + 1).saturating_sub(bytes.len() as u64);
        if remaining == 0 {
            break;
        }
        let count = match file.read(&mut buffer[..remaining.min(32 * 1024) as usize]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    limited("encoded bytes", bytes.len() as u64, MAX_ENCODED_BYTES)?;
    Ok(bytes)
}

fn opaque_gray(decoded: DynamicImage, cancelled: &AtomicBool) -> Result<GrayImage> {
    check_cancelled(cancelled)?;
    match decoded {
        DynamicImage::ImageLuma8(gray) => Ok(gray),
        decoded => {
            let mut rgba = decoded.into_rgba8();
            let width = rgba.width() as usize;
            for row in rgba.as_mut().chunks_exact_mut(width * 4) {
                check_cancelled(cancelled)?;
                for pixel in row.chunks_exact_mut(4) {
                    let alpha = u32::from(pixel[3]);
                    for channel in &mut pixel[..3] {
                        *channel =
                            ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
                    }
                    pixel[3] = 255;
                }
            }
            check_cancelled(cancelled)?;
            Ok(DynamicImage::ImageRgba8(rgba).into_luma8())
        }
    }
}

fn quantize(value: u8, x: u32, y: u32, software: bool) -> u8 {
    const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let bias = if software {
        BAYER[(y & 3) as usize][(x & 3) as usize] - 8
    } else {
        0
    };
    let value = (i16::from(value) + bias).clamp(0, 255) as u16;
    (((value + 8) / 17).min(15) * 17) as u8
}

fn picture_pixels(
    gray: &GrayImage,
    method: ImageDithering,
    cancelled: &AtomicBool,
) -> Result<PreparedPixels> {
    check_cancelled(cancelled)?;
    let (width, height) = gray.dimensions();
    let mut primary = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    let mut fallback = (method == ImageDithering::Hardware)
        .then(|| slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height));
    if let Some(fallback) = fallback.as_mut() {
        for (y, ((primary, software), source)) in primary
            .make_mut_slice()
            .chunks_exact_mut(width as usize)
            .zip(fallback.make_mut_slice().chunks_exact_mut(width as usize))
            .zip(gray.as_raw().chunks_exact(width as usize))
            .enumerate()
        {
            check_cancelled(cancelled)?;
            for (x, ((primary, software), source)) in primary
                .iter_mut()
                .zip(software.iter_mut())
                .zip(source.iter().copied())
                .enumerate()
            {
                *primary = slint::Rgb8Pixel {
                    r: source,
                    g: source,
                    b: source,
                };
                let value = quantize(source, x as u32, y as u32, true);
                *software = slint::Rgb8Pixel {
                    r: value,
                    g: value,
                    b: value,
                };
            }
        }
    } else {
        let software = method == ImageDithering::Software;
        for (y, (destination, source)) in primary
            .make_mut_slice()
            .chunks_exact_mut(width as usize)
            .zip(gray.as_raw().chunks_exact(width as usize))
            .enumerate()
        {
            check_cancelled(cancelled)?;
            for (x, (destination, source)) in destination
                .iter_mut()
                .zip(source.iter().copied())
                .enumerate()
            {
                let value = quantize(source, x as u32, y as u32, software);
                *destination = slint::Rgb8Pixel {
                    r: value,
                    g: value,
                    b: value,
                };
            }
        }
    }
    check_cancelled(cancelled)?;
    Ok(PreparedPixels {
        primary,
        software: fallback,
        dimensions: (width, height),
    })
}

fn prepare_encoded_picture(
    bytes: &[u8],
    frame: (u32, u32),
    fit: PictureFit,
    method: ImageDithering,
    budget: &mut RasterBudget,
    cancelled: &AtomicBool,
) -> Result<PreparedPixels> {
    check_cancelled(cancelled)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
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
    check_cancelled(cancelled)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    let gray = opaque_gray(
        reader.decode().context("decode canonical picture")?,
        cancelled,
    )?;
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
    check_cancelled(cancelled)?;
    let prepared = picture_pixels(&raster, method, cancelled)?;
    budget.pixels = charge.0;
    budget.bytes = charge.1;
    Ok(prepared)
}

fn same_object(before: &Metadata, after: &Metadata) -> bool {
    if !before.is_file()
        || !after.is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A queued path is untrusted until this worker reads the current immutable
/// object and checks its content-addressed digest. The UI independently
/// resolves and verifies current source ownership before reusing its output.
pub(super) fn prepare_cached_picture(
    request: &PictureRequest,
    budget: &mut RasterBudget,
    cancelled: &AtomicBool,
) -> Result<PreparedPixels> {
    check_cancelled(cancelled)?;
    let filename = request
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .context("canonical picture filename is unavailable")?;
    let stem = filename.strip_suffix(".png").unwrap_or("");
    ensure!(
        stem.len() == 129
            && stem.as_bytes()[64] == b'-'
            && stem.bytes().enumerate().all(|(index, byte)| {
                index == 64 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }),
        "canonical picture has no current content-addressed identity"
    );
    let before = std::fs::symlink_metadata(&request.path)?;
    ensure!(before.is_file(), "canonical picture is not a regular file");
    let bytes = encoded_image(&request.path, cancelled)?;
    check_cancelled(cancelled)?;
    ensure!(
        hex::encode(Sha256::digest(&bytes)) == stem[65..129],
        "canonical picture content digest changed"
    );
    ensure!(
        same_object(&before, &std::fs::symlink_metadata(&request.path)?),
        "canonical picture changed while reading"
    );
    prepare_encoded_picture(
        &bytes,
        request.frame,
        request.fit,
        request.method,
        budget,
        cancelled,
    )
}

#[cfg(test)]
fn prepare_picture(
    path: &Path,
    frame: (u32, u32),
    fit: PictureFit,
    method: ImageDithering,
    budget: &mut RasterBudget,
) -> Result<PreparedPicture> {
    let cancelled = AtomicBool::new(false);
    let bytes = encoded_image(path, &cancelled)?;
    Ok(prepare_encoded_picture(&bytes, frame, fit, method, budget, &cancelled)?.into_picture())
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

    fn cached_fixture(bytes: &[u8]) -> Result<(tempfile::TempDir, PathBuf)> {
        let directory = tempfile::tempdir()?;
        let name = format!(
            "{}-{}.png",
            "a".repeat(64),
            hex::encode(Sha256::digest(bytes))
        );
        let path = directory.path().join(name);
        std::fs::write(&path, bytes)?;
        Ok((directory, path))
    }

    fn encoded_gray(gray: GrayImage) -> Result<Vec<u8>> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(gray).write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(bytes.into_inner())
    }

    fn legacy_luminances(gray: &GrayImage, method: ImageDithering) -> Vec<u8> {
        const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        gray.as_raw()
            .iter()
            .enumerate()
            .map(|(index, source)| {
                if method == ImageDithering::Hardware {
                    *source
                } else {
                    let x = index % gray.width() as usize;
                    let y = index / gray.width() as usize;
                    let bias = if method == ImageDithering::Software {
                        BAYER[y % 4][x % 4] - 8
                    } else {
                        0
                    };
                    let value = (i16::from(*source) + bias).clamp(0, 255) as u16;
                    (((value + 8) / 17).min(15) * 17) as u8
                }
            })
            .collect()
    }

    #[test]
    fn row_preparation_preserves_the_legacy_palette_and_bayer_phase_for_odd_widths() -> Result<()> {
        for width in [1, 3, 7, 9, 13, 256] {
            for height in [1, 5, 7] {
                let gray = GrayImage::from_fn(width, height, |x, y| {
                    image::Luma([(x.wrapping_mul(31).wrapping_add(y * 47) % 256) as u8])
                });
                for method in [
                    ImageDithering::Off,
                    ImageDithering::Software,
                    ImageDithering::Hardware,
                ] {
                    let pixels = picture_pixels(&gray, method, &AtomicBool::new(false))?;
                    let pointer = pixels.storage_pointer();
                    let cloned = pixels.clone();
                    assert_eq!(pointer, cloned.storage_pointer());
                    let picture = cloned.into_picture();
                    assert_eq!(
                        luminances(&picture.primary),
                        legacy_luminances(&gray, method)
                    );
                    let fallback = if method == ImageDithering::Hardware {
                        ImageDithering::Software
                    } else {
                        method
                    };
                    assert_eq!(
                        luminances(&picture.software),
                        legacy_luminances(&gray, fallback)
                    );
                    assert_eq!(
                        picture.primary.to_rgb8().unwrap().as_bytes().as_ptr(),
                        pointer
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn cached_worker_preparation_keeps_triangle_pixels_and_all_processing_choices() -> Result<()> {
        let source = GrayImage::from_fn(8, 6, |x, y| image::Luma([(x * 19 + y * 17) as u8]));
        let expected = image::imageops::resize(&source, 5, 4, FilterType::Triangle);
        let bytes = encoded_gray(source)?;
        let (_directory, path) = cached_fixture(&bytes)?;
        for method in [
            ImageDithering::Off,
            ImageDithering::Software,
            ImageDithering::Hardware,
        ] {
            let mut budget = RasterBudget::new((5, 5))?;
            let picture = prepare_cached_picture(
                &PictureRequest {
                    path: path.clone(),
                    frame: (5, 5),
                    fit: PictureFit::Contain,
                    method,
                },
                &mut budget,
                &AtomicBool::new(false),
            )?
            .into_picture();
            assert_eq!(picture.dimensions, (5, 4));
            assert_eq!(
                luminances(&picture.primary),
                legacy_luminances(&expected, method)
            );
            let fallback = if method == ImageDithering::Hardware {
                ImageDithering::Software
            } else {
                method
            };
            assert_eq!(
                luminances(&picture.software),
                legacy_luminances(&expected, fallback)
            );
            assert_eq!(budget.pixels, 20);
            assert_eq!(
                budget.bytes,
                20 * RGB_BYTES
                    * if method == ImageDithering::Hardware {
                        2
                    } else {
                        1
                    }
            );
        }
        Ok(())
    }

    #[test]
    fn queued_candidate_is_reverified_instead_of_trusting_its_old_path() -> Result<()> {
        let original = encoded_gray(GrayImage::from_pixel(2, 2, image::Luma([17])))?;
        let (_directory, path) = cached_fixture(&original)?;
        let replacement = encoded_gray(GrayImage::from_pixel(2, 2, image::Luma([238])))?;
        std::fs::write(&path, replacement)?;
        let mut budget = RasterBudget::new((2, 2))?;
        let result = prepare_cached_picture(
            &PictureRequest {
                path,
                frame: (2, 2),
                fit: PictureFit::Contain,
                method: ImageDithering::Hardware,
            },
            &mut budget,
            &AtomicBool::new(false),
        );
        assert!(result.unwrap_err().to_string().contains("digest changed"));
        assert_eq!((budget.pixels, budget.bytes), (0, 0));
        Ok(())
    }

    #[test]
    fn cancelled_work_never_opens_its_candidate_or_charges_output() -> Result<()> {
        let mut budget = RasterBudget::new((2, 2))?;
        let result = prepare_cached_picture(
            &PictureRequest {
                path: PathBuf::from("absent-canonical-picture.png"),
                frame: (2, 2),
                fit: PictureFit::Contain,
                method: ImageDithering::Hardware,
            },
            &mut budget,
            &AtomicBool::new(true),
        );
        assert!(result.unwrap_err().is::<PreparationCancelled>());
        assert_eq!((budget.pixels, budget.bytes), (0, 0));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_queued_cache_path_cannot_be_replaced_with_a_symlink() -> Result<()> {
        let bytes = encoded_gray(GrayImage::from_pixel(2, 2, image::Luma([17])))?;
        let (directory, path) = cached_fixture(&bytes)?;
        let replacement = directory.path().join("replacement.png");
        std::fs::rename(&path, &replacement)?;
        std::os::unix::fs::symlink(&replacement, &path)?;
        let mut budget = RasterBudget::new((2, 2))?;
        let result = prepare_cached_picture(
            &PictureRequest {
                path,
                frame: (2, 2),
                fit: PictureFit::Contain,
                method: ImageDithering::Hardware,
            },
            &mut budget,
            &AtomicBool::new(false),
        );
        assert!(result.unwrap_err().to_string().contains("regular file"));
        assert_eq!((budget.pixels, budget.bytes), (0, 0));
        Ok(())
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
