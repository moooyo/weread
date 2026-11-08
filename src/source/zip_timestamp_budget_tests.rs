//! Small real ZIP timestamp errors and a production acquisition bridge for resource tests.

use super::*;
use std::{io::Write, panic::catch_unwind};
use zip::{ZipArchive, ZipWriter, extra_fields::ExtraField, write::SimpleFileOptions};

const TIMESTAMP_ERROR: &str = "flags and len don't match in extended timestamp field";

/// Substitute only the fixture transport. The real plan, acquisition, archive
/// decoder, membership, payload and source-binding policies still execute.
pub(crate) async fn acquire_for_test<F, Fut>(
    plan: &ChapterImagePlan,
    fetch: F,
) -> Result<DownloadedChapterImages>
where
    F: FnMut(ImageRequest, usize) -> Fut,
    Fut: Future<Output = Result<Vec<u8>>>,
{
    acquire_with(plan, fetch).await
}

pub(crate) fn raster(shade: u8) -> anyhow::Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([shade])))
        .write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

pub(crate) fn archive(names: &[String], body: &[u8]) -> Vec<u8> {
    archive_assets(
        &names
            .iter()
            .map(|name| (name.clone(), body.to_vec()))
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn archive_assets(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in entries {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(body).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub(crate) fn field(tag: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&tag.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn with_extra(original: &[u8], extra: &[u8]) -> Vec<u8> {
    let footer = original.len() - 22;
    assert_eq!(&original[footer..footer + 4], b"PK\x05\x06");
    assert_eq!(&original[footer + 20..], &[0, 0]);
    assert_eq!(
        u16::from_le_bytes(original[footer + 10..footer + 12].try_into().unwrap()),
        1
    );
    let central =
        u32::from_le_bytes(original[footer + 16..footer + 20].try_into().unwrap()) as usize;
    assert_eq!(&original[central..central + 4], b"PK\x01\x02");
    let name_bytes =
        u16::from_le_bytes(original[central + 28..central + 30].try_into().unwrap()) as usize;
    let old_extra =
        u16::from_le_bytes(original[central + 30..central + 32].try_into().unwrap()) as usize;
    let comment =
        u16::from_le_bytes(original[central + 32..central + 34].try_into().unwrap()) as usize;
    assert_eq!(old_extra, 0);
    assert_eq!(central + 46 + name_bytes + comment, footer);
    assert!(extra.len() < 128);
    let mut output = original[..central].to_vec();
    let mut header = original[central..central + 46].to_vec();
    header[30..32].copy_from_slice(&u16::try_from(extra.len()).unwrap().to_le_bytes());
    output.extend_from_slice(&header);
    output.extend_from_slice(&original[central + 46..central + 46 + name_bytes]);
    output.extend_from_slice(extra);
    output.extend_from_slice(&original[central + 46 + name_bytes..footer]);
    let mut footer = original[footer..].to_vec();
    footer[12..16].copy_from_slice(&u32::try_from(output.len() - central).unwrap().to_le_bytes());
    output.extend_from_slice(&footer);
    assert!(output.len() < 1024);
    output
}

fn timestamp(flags: u8, values: &[u32]) -> Vec<u8> {
    let mut payload = vec![flags];
    for value in values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    field(0x5455, &payload)
}

#[test]
fn zero_timestamp_length_rejects_before_reading_the_single_following_byte() {
    for data in [&[][..], &[0x00][..]] {
        let mut reader = Cursor::new(data);
        let result = zip::extra_fields::ExtendedTimestamp::try_from_reader(&mut reader, 0);
        assert!(matches!(
            result,
            Err(zip::result::ZipError::UnsupportedArchive(TIMESTAMP_ERROR))
        ));
        assert_eq!(reader.position(), 0);
    }
}

#[test]
fn malformed_zero_timestamp_is_an_error_in_real_zip_constructors_and_source_decode()
-> anyhow::Result<()> {
    let body = raster(93)?;
    let original = archive(&["images/fixture.png".into()], &body);
    let mut extra = field(0x5455, &[]);
    // This is one following byte, not an additional complete extra record.
    extra.push(0x00);
    let bytes = with_extra(&original, &extra);
    for bounded in [false, true] {
        let outcome = catch_unwind(|| {
            if bounded {
                ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1)
            } else {
                ZipArchive::new(Cursor::new(bytes.as_slice()))
            }
        });
        assert!(
            outcome.is_ok(),
            "Malformed timestamp must not panic inside the actual constructor"
        );
        assert!(outcome.unwrap().is_err());
    }
    let outcome = catch_unwind(|| decode_archive(&bytes));
    assert!(
        outcome.is_ok(),
        "Actual source ZIP decoding must return its typed failure"
    );
    assert!(matches!(
        outcome.unwrap(),
        Err(SourceError::InvalidResponse("chapter_zip"))
    ));
    Ok(())
}

#[test]
fn normal_and_length_five_compatibility_timestamps_keep_values_and_crc_reads() -> anyhow::Result<()>
{
    let body = raster(121)?;
    let original = archive(&["images/fixture.png".into()], &body);
    for (extra, expected) in [
        (timestamp(0, &[]), (None, None, None)),
        (timestamp(1, &[101]), (Some(101), None, None)),
        (timestamp(0, &[102]), (Some(102), None, None)),
        (timestamp(0x80, &[103]), (Some(103), None, None)),
        (timestamp(3, &[104, 105]), (Some(104), Some(105), None)),
        (
            timestamp(7, &[106, 107, 108]),
            (Some(106), Some(107), Some(108)),
        ),
    ] {
        let bytes = with_extra(&original, &extra);
        let mut archive = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1)?;
        let mut entry = archive.by_index(0)?;
        let actual = entry
            .extra_data_fields()
            .find_map(|field| match field {
                ExtraField::ExtendedTimestamp(timestamp) => Some((
                    timestamp.mod_time(),
                    timestamp.ac_time(),
                    timestamp.cr_time(),
                )),
                _ => None,
            })
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(entry.extra_data(), Some(extra.as_slice()));
        let mut read = Vec::new();
        entry.read_to_end(&mut read)?;
        assert_eq!(read, body);
        let decoded = decode_archive(&bytes)?;
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].bytes, body);
    }
    Ok(())
}

#[test]
fn valid_timestamp_keeps_unknown_fields_and_the_existing_last_zip64_removal() -> anyhow::Result<()>
{
    let body = raster(143)?;
    let original = archive(&["images/fixture.png".into()], &body);
    let mut extra = field(0x0001, &[]);
    extra.extend_from_slice(&timestamp(1, &[201]));
    extra.extend_from_slice(&field(0xcafe, &[1, 2, 3]));
    let expected = extra.clone();
    extra.extend_from_slice(&field(0x0001, &[]));
    let bytes = with_extra(&original, &extra);
    let mut archive = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1)?;
    let mut entry = archive.by_index(0)?;
    assert_eq!(entry.extra_data(), Some(expected.as_slice()));
    assert!(zip::HasZipMetadata::get_metadata(&entry).large_file);
    let timestamp = entry.extra_data_fields().find_map(|field| match field {
        ExtraField::ExtendedTimestamp(timestamp) => timestamp.mod_time(),
        _ => None,
    });
    assert_eq!(timestamp, Some(201));
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual)?;
    assert_eq!(actual, body);
    assert_eq!(decode_archive(&bytes)?[0].bytes, body);
    Ok(())
}

#[test]
fn valid_timestamp_does_not_hide_a_payload_crc_failure() -> anyhow::Result<()> {
    let body = raster(163)?;
    let original = archive(&["images/fixture.png".into()], &body);
    let mut bytes = with_extra(&original, &timestamp(1, &[301]));
    let footer = bytes.len() - 22;
    let central = u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
    bytes[central + 16] ^= 1;
    let mut archive = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1)?;
    let mut entry = archive.by_index(0)?;
    let mut read = Vec::new();
    assert!(entry.read_to_end(&mut read).is_err());
    assert!(matches!(
        decode_archive(&bytes),
        Err(SourceError::Integrity)
    ));
    Ok(())
}
