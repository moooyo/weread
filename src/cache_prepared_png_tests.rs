//! Worker preparations preserve the established cache bytes and ownership boundary.

use super::{FileCache, PreparationCancelled, PreparedPng};
use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma, LumaA, Rgb, Rgba};
use std::{fs, io::Cursor, sync::atomic::AtomicBool};

fn encode(image: DynamicImage) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn sources() -> Vec<DynamicImage> {
    vec![
        DynamicImage::ImageLuma8(GrayImage::from_fn(47, 31, |x, y| {
            Luma([((x * 19 + y * 31) % 256) as u8])
        })),
        DynamicImage::ImageLumaA8(image::ImageBuffer::from_fn(31, 47, |x, y| {
            LumaA([
                ((x * 13 + y * 29) % 256) as u8,
                [0, 64, 128, 255][((x + y) % 4) as usize],
            ])
        })),
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(47, 31, |x, y| {
            Rgb([
                ((x * 37 + y * 5) % 256) as u8,
                ((x * 7 + y * 23) % 256) as u8,
                ((x * 13 + y * 17) % 256) as u8,
            ])
        })),
        DynamicImage::ImageRgba8(image::RgbaImage::from_fn(31, 47, |x, y| {
            Rgba([
                ((x * 11 + y * 17) % 256) as u8,
                ((x * 23 + y * 7) % 256) as u8,
                ((x * 5 + y * 37) % 256) as u8,
                [0, 64, 128, 255][((x + y) % 4) as usize],
            ])
        })),
        DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(47, 31, |x, y| {
            Luma([((x * 1181 + y * 2081) % 65536) as u16])
        })),
    ]
}

/// Keep the previous synchronous algorithm as an independent byte reference.
fn reference_cover(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut gray = image::load_from_memory(bytes)?
        .thumbnail(width, height)
        .to_luma8();
    for pixel in gray.pixels_mut() {
        pixel[0] = (((u16::from(pixel[0]) + 8) / 17).min(15) * 17) as u8;
    }
    encode(DynamicImage::ImageLuma8(gray))
}

fn reference_gray8(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut rgba = image::load_from_memory(bytes)?
        .thumbnail(width, height)
        .into_rgba8();
    for pixel in rgba.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
    let gray = DynamicImage::ImageRgba8(rgba).into_luma8();
    encode(DynamicImage::ImageLuma8(gray))
}

#[test]
fn worker_cover_preparation_matches_existing_png_bytes_and_pixels() -> Result<()> {
    for source in sources() {
        let encoded = encode(source)?;
        for (width, height) in [(1, 1), (13, 17), (47, 31), (83, 79)] {
            let prepared = FileCache::prepare_cover_png(&encoded, width, height)?;
            let reference = reference_cover(&encoded, width, height)?;
            assert_eq!(prepared.encoded(), reference);
            assert_eq!(
                prepared.gray(),
                &image::load_from_memory(&reference)?.into_luma8()
            );
            assert!(prepared.gray().pixels().all(|pixel| pixel[0] % 17 == 0));
        }
    }
    Ok(())
}

#[test]
fn worker_gray8_preparation_matches_existing_alpha_resize_and_png_bytes() -> Result<()> {
    for source in sources() {
        let encoded = encode(source)?;
        for (width, height) in [(1, 1), (13, 17), (47, 31), (83, 79)] {
            let prepared = FileCache::prepare_gray8_png(&encoded, width, height)?;
            let reference = reference_gray8(&encoded, width, height)?;
            assert_eq!(prepared.encoded(), reference);
            assert_eq!(
                prepared.gray(),
                &image::load_from_memory(&reference)?.into_luma8()
            );
        }
    }
    Ok(())
}

#[test]
fn cancelled_preparations_have_a_typed_error_and_leave_existing_objects_untouched() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let cache = FileCache::open(directory.path(), 1024 * 1024)?;
    let retained = cache.put("retained", b"retained")?;
    let usage = cache.usage()?;
    let encoded = encode(DynamicImage::new_luma8(3, 5))?;
    let cancelled = AtomicBool::new(true);
    for result in [
        FileCache::prepare_cover_png_cancellable(&encoded, 400, 560, &cancelled),
        FileCache::prepare_gray8_png_cancellable(&encoded, 400, 560, &cancelled),
    ] {
        let error = result.unwrap_err();
        assert!(error.downcast_ref::<PreparationCancelled>().is_some());
    }
    assert_eq!(cache.get_for_commit("retained")?, Some(retained.clone()));
    assert_eq!(fs::read(retained)?, b"retained");
    assert_eq!(cache.usage()?, usage);
    Ok(())
}

#[test]
fn a_prepared_payload_moves_from_a_worker_to_an_owner_without_a_cache_connection() -> Result<()> {
    fn assert_send<T: Send>() {}
    assert_send::<PreparedPng>();
    let encoded = encode(DynamicImage::ImageLuma8(GrayImage::from_fn(
        256,
        1,
        |x, _| Luma([x as u8]),
    )))?;
    let prepared = std::thread::spawn(move || FileCache::prepare_gray8_png(&encoded, 256, 1))
        .join()
        .expect("The pure image-preparation worker panicked")?;
    let directory = tempfile::tempdir()?;
    let cache = FileCache::open(directory.path(), 1024 * 1024)?;
    let path = cache.put_prepared_png("worker-gray8", &prepared)?;
    assert_eq!(fs::read(path)?, prepared.encoded());
    assert_eq!(prepared.gray().as_raw(), &(0_u8..=255).collect::<Vec<_>>());
    Ok(())
}
