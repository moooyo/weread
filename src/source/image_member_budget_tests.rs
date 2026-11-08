//! Bounded real archive/digest fixtures for first-seen asset membership.

use super::*;
use std::io::Write;

fn raster(shade: u8) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([shade])))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn zip_bytes(names: &[String], bytes: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in names {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(bytes.len() < 512 * 1024);
    bytes
}

fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    for (name, bytes) in entries {
        let (prefix, base) = name.rsplit_once('/').unwrap_or(("", name));
        assert!(base.len() <= 100 && prefix.len() <= 155);
        let mut header = [0u8; 512];
        header[..base.len()].copy_from_slice(base.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let checksum = header.iter().map(|byte| u64::from(*byte)).sum::<u64>();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        output.extend_from_slice(&header);
        output.extend_from_slice(bytes);
        output.resize(output.len() + (512 - bytes.len() % 512) % 512, 0);
    }
    output.resize(output.len() + 1024, 0);
    assert!(output.len() < 512 * 1024);
    output
}

fn assert_resource_error<T>(result: Result<T>) {
    let error = match result {
        Ok(_) => panic!("member quota was bypassed"),
        Err(error) => error,
    };
    assert!(matches!(&error, SourceError::ResponseTooLarge));
    assert!(!error.requires_login() && !error.retryable());
}

#[test]
fn zip_long_prefix_aliases_merge_once_in_original_directory_order() {
    let names = (0..96)
        .map(|index| format!("{}-{index:04}.png", "p".repeat(1800)))
        .collect::<Vec<_>>();
    let bytes = raster(123);
    let decoded = decode_archive(&zip_bytes(&names, &bytes)).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].bytes, bytes);
    assert_eq!(decoded[0].source_names, names);
}

#[test]
fn tar_repeated_digest_aliases_keep_first_seen_order_and_complete_names() {
    let prefix = "p".repeat(120);
    let a = format!("{prefix}/a.png");
    let b = format!("{prefix}/b.png");
    let c = format!("{prefix}/c.png");
    let image = raster(44);
    let tar = tar_bytes(&[
        (&a, &image),
        (&b, &image),
        (&a, &image),
        (&c, &image),
        (&b, &image),
    ]);
    let decoded = decode_archive(&tar).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].source_names, [a, b, c]);
}

#[test]
fn one_source_path_with_distinct_digests_still_has_ambiguous_identity() {
    let first = raster(12);
    let second = raster(230);
    let decoded = decode_archive(&tar_bytes(&[
        ("images/a.png", &first),
        ("images/a.png", &second),
    ]))
    .unwrap();
    assert_eq!(decoded.len(), 2);
    assert!(matches!(
        ArchiveIndex::new(&decoded).unwrap().resolve("images/a.png"),
        Err(SourceError::Unsupported("ambiguous_chapter_image"))
    ));
}

#[test]
fn member_count_bytes_and_work_are_admitted_before_copy_or_hash() {
    let image = raster(120);
    let mut collected = AssetAccumulator {
        limits: MemberLimits {
            names: 2,
            ..MemberLimits::default()
        },
        ..AssetAccumulator::default()
    };
    for name in ["first.png", "second.png"] {
        let name = collected.admit_name(&[name]).unwrap();
        collected.add_image(image.clone(), vec![name]).unwrap();
    }
    let before = (
        collected.usage.names,
        collected.usage.name_bytes,
        collected.usage.name_work,
        collected.usage.hash_work,
    );
    assert_resource_error(collected.admit_name(&[&format!("{}-third.png", "p".repeat(1024))]));
    assert_eq!(
        (
            collected.usage.names,
            collected.usage.name_bytes,
            collected.usage.name_work,
            collected.usage.hash_work
        ),
        before
    );
    assert_eq!(
        collected.assets[0].source_names,
        ["first.png", "second.png"]
    );
    assert_eq!(collected.members[0].len(), 2);
    for limits in [
        MemberLimits {
            name_bytes: 3,
            ..MemberLimits::default()
        },
        MemberLimits {
            name_work: 0,
            ..MemberLimits::default()
        },
    ] {
        let mut collected = AssetAccumulator {
            limits,
            ..AssetAccumulator::default()
        };
        assert_resource_error(collected.admit_name(&["ab", "/", "cd"]));
        assert_eq!(
            (
                collected.usage.names,
                collected.usage.name_bytes,
                collected.usage.name_work
            ),
            (0, 0, 0)
        );
        assert!(collected.assets.is_empty());
    }
}

#[test]
fn encoded_hash_work_is_reserved_before_format_and_digest_processing() {
    let image = raster(41);
    let mut collected = AssetAccumulator {
        limits: MemberLimits {
            hash_work: image.len() - 1,
            ..MemberLimits::default()
        },
        ..AssetAccumulator::default()
    };
    let name = collected.admit_name(&["image.png"]).unwrap();
    assert_resource_error(collected.add_image(image, vec![name]));
    assert_eq!(collected.usage.hash_work, 0);
    assert!(
        collected.assets.is_empty()
            && collected.by_digest.is_empty()
            && collected.members.is_empty()
    );
}

#[test]
fn pruning_moves_existing_membership_and_updates_the_digest_index() {
    let mut collected = AssetAccumulator::default();
    let first = collected.admit_name(&["first.png"]).unwrap();
    collected.add_image(raster(10), vec![first]).unwrap();
    let second = collected.admit_name(&["second.png"]).unwrap();
    let id = collected
        .add_image(raster(200), vec![second])
        .unwrap()
        .unwrap();
    let pointer = collected.members[1].get("second.png").unwrap() as *const String as usize;
    let before = (
        collected.usage.names,
        collected.usage.name_bytes,
        collected.usage.name_work,
    );
    collected.retain(|asset| asset.id == id);
    assert_eq!(
        collected.members[0].get("second.png").unwrap() as *const String as usize,
        pointer
    );
    assert_eq!(
        (
            collected.usage.names,
            collected.usage.name_bytes,
            collected.usage.name_work
        ),
        before
    );
    let alias = collected.admit_name(&["second-alias.png"]).unwrap();
    collected.add_image(raster(200), vec![alias]).unwrap();
    assert_eq!(collected.assets.len(), 1);
    assert_eq!(collected.by_digest.get(&id), Some(&0));
    assert_eq!(
        collected.assets[0].source_names,
        ["second.png", "second-alias.png"]
    );
}

#[test]
fn acquisition_reuses_pruned_archive_membership_for_equal_remote_bytes() {
    let first = raster(11);
    let second = raster(220);
    let archive = tar_bytes(&[
        ("fig/a.png", &first),
        ("fig/b.png", &second),
        ("fig/b-alias.png", &second),
    ]);
    let html = "<img src='fig/b.png'><img src='https://cdn.example.test/b.png'>";
    let plan = make_plan("book", "chapter", html, Some("/archive")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime
        .block_on(acquire_with(&plan, |request, _| {
            std::future::ready(Ok(if request.source.is_empty() {
                archive.clone()
            } else {
                second.clone()
            }))
        }))
        .unwrap();
    assert_eq!(result.assets.len(), 1);
    assert_eq!(result.assets[0].bytes, second);
    assert_eq!(
        result.assets[0].source_names,
        [
            "fig/b.png",
            "fig/b-alias.png",
            "https://cdn.example.test/b.png"
        ]
    );
    assert_eq!(result.source_to_asset.len(), 2);
    assert!(
        result
            .source_to_asset
            .values()
            .all(|id| id == &result.assets[0].id)
    );
}

#[test]
fn genuine_zip_and_tar_members_share_the_small_count_admission_policy() {
    let image = raster(77);
    let names = ["a.png".to_owned(), "b.png".to_owned(), "c.png".to_owned()];
    let limits = MemberLimits {
        names: 2,
        ..MemberLimits::default()
    };
    assert_resource_error(decode_zip_accumulator(
        &zip_bytes(&names, &image),
        MAX_COMPRESSED_WORK_BYTES as u64,
        limits,
    ));
    assert_resource_error(decode_tar_accumulator(
        &tar_bytes(&[("a.png", &image), ("b.png", &image), ("c.png", &image)]),
        limits,
    ));
}
