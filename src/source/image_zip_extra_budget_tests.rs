//! Real small ZIP constructors expose derived extra-field copies and legacy bytes.

use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    io::Write,
};
use zip::read::{MetadataRetentionLimits, MetadataRetentionUsage};
use zip::{HasZipMetadata, ZipArchive, ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Copy, Default)]
struct RejectedPayloadTarget {
    enabled: bool,
    armed: bool,
    size: usize,
    calls: usize,
}

thread_local! {
    static RETENTION_LIMITS: Cell<Option<MetadataRetentionLimits>> = const { Cell::new(None) };
    static RETENTION_USAGE: Cell<MetadataRetentionUsage> = const { Cell::new(MetadataRetentionUsage {
        metadata_bytes: 0, peak_metadata_bytes: 0, extra_preflights: 0, extra_records: 0,
        extra_pushes: 0, vector_reservations: 0, vector_requested_slots: 0,
        vector_capacity_delta_slots: 0, maximum_old_buffer_bytes: 0,
        candidate_attempts: 0, denied: false, operations_after_denial: 0,
    }) };
    static REJECTED_PAYLOAD: Cell<RejectedPayloadTarget> = const {
        Cell::new(RejectedPayloadTarget { enabled: false, armed: false, size: 0, calls: 0 })
    };
}

pub(super) fn with_retention_limits<T>(
    limits: MetadataRetentionLimits,
    payload_size: usize,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(
        Option<MetadataRetentionLimits>,
        MetadataRetentionUsage,
        RejectedPayloadTarget,
    );
    impl Drop for Restore {
        fn drop(&mut self) {
            RETENTION_LIMITS.with(|slot| slot.set(self.0));
            RETENTION_USAGE.with(|slot| slot.set(self.1));
            REJECTED_PAYLOAD.with(|slot| slot.set(self.2));
        }
    }
    let _restore = Restore(
        RETENTION_LIMITS.with(|slot| slot.replace(Some(limits))),
        RETENTION_USAGE.with(|slot| slot.replace(MetadataRetentionUsage::default())),
        REJECTED_PAYLOAD.with(|slot| {
            slot.replace(RejectedPayloadTarget {
                enabled: payload_size > 0,
                armed: false,
                size: payload_size,
                calls: 0,
            })
        }),
    );
    run()
}

pub(super) fn retention_usage() -> MetadataRetentionUsage {
    RETENTION_USAGE.with(Cell::get)
}

pub(super) fn rejected_payload_copies() -> usize {
    REJECTED_PAYLOAD.with(|slot| slot.get().calls)
}

pub(super) fn construct_archive<R: std::io::Read + std::io::Seek>(
    reader: R,
    max_files: usize,
) -> zip::result::ZipResult<ZipArchive<R>> {
    let Some(limits) = RETENTION_LIMITS.with(Cell::get) else {
        return ZipArchive::new_with_file_limit(reader, max_files);
    };
    REJECTED_PAYLOAD.with(|slot| {
        let mut target = slot.get();
        target.armed = false;
        slot.set(target);
    });
    let mut usage = MetadataRetentionUsage::default();
    let result = ZipArchive::new_with_retention_limits(
        reader,
        max_files,
        zip::read::DEFAULT_METADATA_READ_LIMIT,
        zip::read::DEFAULT_METADATA_CANDIDATE_LIMIT,
        limits,
        &mut usage,
    );
    RETENTION_USAGE.with(|slot| slot.set(usage));
    if usage.denied {
        REJECTED_PAYLOAD.with(|slot| {
            let mut target = slot.get();
            target.armed = true;
            slot.set(target);
        });
    }
    result
}

#[derive(Clone, Copy, Default)]
struct AllocationTarget {
    enabled: bool,
    size: usize,
    calls: usize,
    bytes: usize,
}

thread_local! {
    // Const, destructor-free TLS and Cell reads cannot allocate or reenter the
    // allocator. Tracking is enabled only on the constructor's current thread.
    static ALLOCATION_TARGET: Cell<AllocationTarget> = const {
        Cell::new(AllocationTarget { enabled: false, size: 0, calls: 0, bytes: 0 })
    };
}

struct ObservedSystem;

fn record_allocation(size: usize, alignment: usize) {
    let _ = ALLOCATION_TARGET.try_with(|target| {
        let mut state = target.get();
        if state.enabled && alignment == 1 && size == state.size {
            state.calls = state.calls.saturating_add(1);
            state.bytes = state.bytes.saturating_add(size);
            target.set(state);
        }
    });
    let _ = REJECTED_PAYLOAD.try_with(|slot| {
        let mut target = slot.get();
        if target.enabled && target.armed && alignment == 1 && target.size == size {
            target.calls = target.calls.saturating_add(1);
            slot.set(target);
        }
    });
}

// This module exists only in the root library test binary. Production keeps
// its existing allocator. Constructor accounting itself is small numeric data.
#[global_allocator]
static TEST_ALLOCATOR: ObservedSystem = ObservedSystem;

unsafe impl GlobalAlloc for ObservedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size(), layout.align());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size(), layout.align());
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            record_allocation(size, layout.align());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

struct AllocationScope {
    active: bool,
}

impl AllocationScope {
    fn start(size: usize) -> Self {
        assert!(size > 2048 && size % 2 == 1 && !size.is_power_of_two());
        ALLOCATION_TARGET.with(|target| {
            assert!(!target.get().enabled);
            target.set(AllocationTarget {
                enabled: true,
                size,
                calls: 0,
                bytes: 0,
            });
        });
        Self { active: true }
    }

    fn finish(mut self) -> AllocationTarget {
        let result = ALLOCATION_TARGET.with(|target| {
            let result = target.get();
            target.set(AllocationTarget::default());
            result
        });
        self.active = false;
        result
    }
}

impl Drop for AllocationScope {
    fn drop(&mut self) {
        if self.active {
            let _ = ALLOCATION_TARGET.try_with(|target| target.set(AllocationTarget::default()));
        }
    }
}

fn raster() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(2, 2, image::Luma([123])))
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn field(tag: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&tag.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn repeated_extra(count: usize) -> Vec<u8> {
    // The unknown payload and resulting odd capacity differ from normal
    // metadata slots, the 2 KiB search window, and both original extra copies.
    let mut bytes = field(0xcafe, &vec![0x31; 2053]);
    for _ in 0..count {
        bytes.extend_from_slice(&field(0x0001, &[]));
    }
    bytes
}

fn zip_bytes(extras: &[Vec<u8>], body: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for ordinal in 0..extras.len() {
        writer
            .start_file(
                format!("images/{ordinal}.png"),
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(body).unwrap();
    }
    let original = writer.finish().unwrap().into_inner();
    let footer = original.len() - 22;
    assert_eq!(&original[footer..footer + 4], b"PK\x05\x06");
    assert_eq!(&original[footer + 20..], &[0, 0]);
    assert_eq!(
        u16::from_le_bytes(original[footer + 8..footer + 10].try_into().unwrap()) as usize,
        extras.len()
    );
    assert_eq!(
        u16::from_le_bytes(original[footer + 10..footer + 12].try_into().unwrap()) as usize,
        extras.len()
    );
    let central =
        u32::from_le_bytes(original[footer + 16..footer + 20].try_into().unwrap()) as usize;
    assert_eq!(
        u32::from_le_bytes(original[footer + 12..footer + 16].try_into().unwrap()) as usize,
        footer - central
    );
    let mut output = original[..central].to_vec();
    let mut offset = central;
    for extra in extras {
        assert!(extra.len() < 4096);
        assert_eq!(&original[offset..offset + 4], b"PK\x01\x02");
        let name_bytes =
            u16::from_le_bytes(original[offset + 28..offset + 30].try_into().unwrap()) as usize;
        let extra_bytes =
            u16::from_le_bytes(original[offset + 30..offset + 32].try_into().unwrap()) as usize;
        let comment_bytes =
            u16::from_le_bytes(original[offset + 32..offset + 34].try_into().unwrap()) as usize;
        assert_eq!(extra_bytes, 0);
        let mut header = original[offset..offset + 46].to_vec();
        header[30..32].copy_from_slice(&u16::try_from(extra.len()).unwrap().to_le_bytes());
        output.extend_from_slice(&header);
        output.extend_from_slice(&original[offset + 46..offset + 46 + name_bytes]);
        output.extend_from_slice(extra);
        output.extend_from_slice(
            &original[offset + 46 + name_bytes..offset + 46 + name_bytes + comment_bytes],
        );
        offset += 46 + name_bytes + comment_bytes;
    }
    assert_eq!(offset, footer);
    let directory_bytes = output.len() - central;
    let mut footer = original[footer..].to_vec();
    footer[12..16].copy_from_slice(&u32::try_from(directory_bytes).unwrap().to_le_bytes());
    output.extend_from_slice(&footer);
    assert!(output.len() < 32 * 1024);
    output
}

fn timestamp_field(flags: u8, values: &[u32]) -> Vec<u8> {
    let mut payload = vec![flags];
    for value in values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    field(0x5455, &payload)
}

fn empty_timestamps(count: usize) -> Vec<u8> {
    let mut extra = Vec::new();
    for _ in 0..count {
        extra.extend_from_slice(&timestamp_field(0, &[]));
    }
    extra
}

fn retention_constructor<'a>(
    bytes: &'a [u8],
    limits: MetadataRetentionLimits,
    legacy: bool,
) -> (
    zip::result::ZipResult<ZipArchive<Cursor<&'a [u8]>>>,
    MetadataRetentionUsage,
) {
    let mut usage = MetadataRetentionUsage::default();
    let result = if legacy {
        ZipArchive::with_config_and_retention_limits(
            zip::read::Config::default(),
            Cursor::new(bytes),
            limits,
            &mut usage,
        )
    } else {
        ZipArchive::new_with_retention_limits(
            Cursor::new(bytes),
            MAX_ARCHIVE_ENTRIES,
            zip::read::DEFAULT_METADATA_READ_LIMIT,
            zip::read::DEFAULT_METADATA_CANDIDATE_LIMIT,
            limits,
            &mut usage,
        )
    };
    (result, usage)
}

fn assert_retention_error<T>(
    result: zip::result::ZipResult<T>,
    usage: MetadataRetentionUsage,
    pushed: usize,
) {
    assert!(matches!(
        result,
        Err(zip::result::ZipError::UnsupportedArchive(
            zip::result::ZipError::METADATA_LIMIT_EXCEEDED
        ))
    ));
    assert!(usage.denied);
    assert_eq!(usage.extra_pushes, pushed);
    assert_eq!(usage.extra_records, pushed);
    assert_eq!(usage.operations_after_denial, 0);
}

fn append_absolute_archive(older: &[u8], later: &[u8]) -> Vec<u8> {
    let prefix = u32::try_from(older.len()).unwrap();
    let mut later = later.to_vec();
    let footer = later.len() - 22;
    let central = u32::from_le_bytes(later[footer + 16..footer + 20].try_into().unwrap()) as usize;
    let count = usize::from(u16::from_le_bytes(
        later[footer + 10..footer + 12].try_into().unwrap(),
    ));
    let mut offset = central;
    for _ in 0..count {
        assert_eq!(&later[offset..offset + 4], b"PK\x01\x02");
        let local = u32::from_le_bytes(later[offset + 42..offset + 46].try_into().unwrap());
        later[offset + 42..offset + 46]
            .copy_from_slice(&local.checked_add(prefix).unwrap().to_le_bytes());
        let name = usize::from(u16::from_le_bytes(
            later[offset + 28..offset + 30].try_into().unwrap(),
        ));
        let extra = usize::from(u16::from_le_bytes(
            later[offset + 30..offset + 32].try_into().unwrap(),
        ));
        let comment = usize::from(u16::from_le_bytes(
            later[offset + 32..offset + 34].try_into().unwrap(),
        ));
        offset += 46 + name + extra + comment;
    }
    assert_eq!(offset, footer);
    later[footer + 16..footer + 20].copy_from_slice(
        &u32::try_from(central)
            .unwrap()
            .checked_add(prefix)
            .unwrap()
            .to_le_bytes(),
    );
    // Absolute offsets ensure the pinned optimistic CDF hint reaches the later
    // directory instead of accidentally accepting the earlier directory first.
    let mut bytes = older.to_vec();
    bytes.extend_from_slice(&later);
    bytes
}

#[test]
fn retained_records_are_shared_across_two_complete_central_entries() {
    let body = raster();
    let extra = empty_timestamps(3);
    assert_eq!(extra.len(), 15);
    let bytes = zip_bytes(&[extra.clone(), extra.clone()], &body);
    for legacy in [false, true] {
        let (parsed, usage) = retention_constructor(
            &bytes,
            MetadataRetentionLimits {
                extra_records: 6,
                ..Default::default()
            },
            legacy,
        );
        let mut archive = parsed.unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(
            (
                usage.extra_preflights,
                usage.extra_records,
                usage.extra_pushes
            ),
            (6, 6, 6)
        );
        assert_eq!(
            (
                usage.vector_reservations,
                usage.vector_requested_slots,
                usage.vector_capacity_delta_slots
            ),
            (2, 8, 8)
        );
        assert!(!usage.denied);
        for ordinal in 0..2 {
            let mut entry = archive.by_index(ordinal).unwrap();
            assert_eq!(entry.extra_data(), Some(extra.as_slice()));
            let timestamps = entry
                .extra_data_fields()
                .map(|field| match field {
                    zip::extra_fields::ExtraField::ExtendedTimestamp(value) => {
                        (value.mod_time(), value.ac_time(), value.cr_time())
                    }
                    _ => panic!("A legal timestamp record changed its type"),
                })
                .collect::<Vec<_>>();
            assert_eq!(timestamps, vec![(None, None, None); 3]);
            let mut actual = Vec::new();
            entry.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, body);
        }
        let (denied, usage) = retention_constructor(
            &bytes,
            MetadataRetentionLimits {
                extra_records: 5,
                ..Default::default()
            },
            legacy,
        );
        assert_eq!(usage.extra_preflights, 6);
        assert_eq!(usage.candidate_attempts, 1);
        assert_retention_error(denied, usage, 5);
    }
    let assets = decode_archive(&bytes).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].bytes, body);
    assert_eq!(assets[0].source_names, ["images/0.png", "images/1.png"]);
    // A failed borrowed constructor must release its adapter/borrow rather than
    // consume or replace the caller's original reader object.
    let mut original = Cursor::new(bytes.as_slice());
    let mut usage = MetadataRetentionUsage::default();
    let denied = ZipArchive::new_with_retention_limits(
        &mut original,
        2,
        zip::read::DEFAULT_METADATA_READ_LIMIT,
        zip::read::DEFAULT_METADATA_CANDIDATE_LIMIT,
        MetadataRetentionLimits {
            extra_records: 5,
            ..Default::default()
        },
        &mut usage,
    );
    assert_retention_error(denied, usage, 5);
    original.set_position(0);
    let mut restored = Vec::new();
    original.read_to_end(&mut restored).unwrap();
    assert_eq!(restored, bytes);
}

#[test]
fn retained_metadata_exact_fee_rejects_before_the_first_excess_push() {
    let body = raster();
    let bytes = zip_bytes(&[empty_timestamps(3), empty_timestamps(3)], &body);
    let (baseline, admitted) =
        retention_constructor(&bytes, MetadataRetentionLimits::default(), false);
    assert_eq!(baseline.unwrap().len(), 2);
    assert_eq!(admitted.metadata_bytes, admitted.peak_metadata_bytes);
    assert!(admitted.metadata_bytes >= 6 * std::mem::size_of::<zip::extra_fields::ExtraField>());
    let exact = MetadataRetentionLimits {
        metadata_bytes: admitted.peak_metadata_bytes,
        ..Default::default()
    };
    let (parsed, usage) = retention_constructor(&bytes, exact, false);
    assert_eq!(parsed.unwrap().len(), 2);
    assert_eq!(usage.extra_pushes, 6);
    let (denied, usage) = retention_constructor(
        &bytes,
        MetadataRetentionLimits {
            metadata_bytes: exact.metadata_bytes - 1,
            ..exact
        },
        false,
    );
    assert_eq!(usage.extra_preflights, 6);
    assert_eq!(usage.vector_reservations, 2);
    assert_retention_error(denied, usage, 5);
}

#[test]
fn requested_extra_vector_growth_prepays_the_old_buffer_overlap() {
    let body = raster();
    let bytes = zip_bytes(&[empty_timestamps(9), Vec::new()], &body);
    let (parsed, admitted) =
        retention_constructor(&bytes, MetadataRetentionLimits::default(), false);
    assert_eq!(parsed.unwrap().len(), 2);
    assert_eq!(
        (
            admitted.vector_reservations,
            admitted.vector_requested_slots,
            admitted.vector_capacity_delta_slots
        ),
        (3, 28, 16)
    );
    assert_eq!(
        admitted.maximum_old_buffer_bytes,
        8 * std::mem::size_of::<zip::extra_fields::ExtraField>()
    );
    let (parsed, exact) = retention_constructor(
        &bytes,
        MetadataRetentionLimits {
            metadata_bytes: admitted.peak_metadata_bytes,
            ..Default::default()
        },
        false,
    );
    assert_eq!(parsed.unwrap().len(), 2);
    assert_eq!(exact.extra_pushes, 9);
    let (denied, usage) = retention_constructor(
        &bytes,
        MetadataRetentionLimits {
            metadata_bytes: admitted.peak_metadata_bytes - 1,
            ..Default::default()
        },
        false,
    );
    assert_eq!(usage.extra_preflights, 9);
    assert_eq!(
        (usage.vector_reservations, usage.vector_requested_slots),
        (2, 12)
    );
    assert_retention_error(denied, usage, 8);
}

#[test]
fn retained_record_denial_cannot_fall_back_to_an_earlier_valid_directory() {
    let body = raster();
    let older = zip_bytes(&[empty_timestamps(1), empty_timestamps(1)], &body);
    let mut latest_extra = empty_timestamps(3);
    latest_extra.extend_from_slice(&field(0x000a, &[]));
    let latest = zip_bytes(&[latest_extra, Vec::new()], &body);
    let control = append_absolute_archive(
        &older,
        &zip_bytes(&[empty_timestamps(3), Vec::new()], &body),
    );
    let (parsed, control_usage) =
        retention_constructor(&control, MetadataRetentionLimits::default(), false);
    let mut calibrated = parsed.unwrap();
    assert_eq!(
        (control_usage.candidate_attempts, control_usage.extra_pushes),
        (1, 3)
    );
    let mut entry = calibrated.by_index(0).unwrap();
    assert_eq!(entry.extra_data_fields().count(), 3);
    let mut payload = Vec::new();
    entry.read_to_end(&mut payload).unwrap();
    assert_eq!(payload, body);
    drop(entry);
    let bytes = append_absolute_archive(&older, &latest);
    assert!(bytes.len() < 4096);
    for legacy in [false, true] {
        let (parsed, usage) =
            retention_constructor(&bytes, MetadataRetentionLimits::default(), legacy);
        let mut archive =
            parsed.expect("The malformed latest candidate must retain ordinary EOCD fallback");
        assert_eq!(archive.len(), 2);
        assert_eq!(usage.candidate_attempts, 2);
        assert_eq!(
            (usage.extra_records, usage.extra_pushes),
            (5, 5),
            "Failed candidates must not refund their admitted records"
        );
        assert_eq!(archive.by_index(0).unwrap().extra_data_fields().count(), 1);
        let (denied, usage) = retention_constructor(
            &bytes,
            MetadataRetentionLimits {
                extra_records: 2,
                ..Default::default()
            },
            legacy,
        );
        assert_eq!(usage.extra_preflights, 3);
        assert_eq!(usage.candidate_attempts, 1);
        assert_retention_error(denied, usage, 2);
    }
}

#[test]
fn retained_timestamp_and_ntfs_fields_keep_duplicates_values_and_original_order() {
    let body = raster();
    let mut first = empty_timestamps(2);
    first.extend_from_slice(&timestamp_field(1, &[101]));
    let mut ntfs = Vec::new();
    ntfs.extend_from_slice(&0_u32.to_le_bytes());
    ntfs.extend_from_slice(&1_u16.to_le_bytes());
    ntfs.extend_from_slice(&24_u16.to_le_bytes());
    for value in [201_u64, 202, 203] {
        ntfs.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(ntfs.len(), 32);
    first.extend_from_slice(&field(0x000a, &ntfs));
    let mut second = timestamp_field(3, &[301, 302]);
    second.extend_from_slice(&timestamp_field(0x80, &[303]));
    let bytes = zip_bytes(&[first.clone(), second.clone()], &body);
    let (parsed, usage) = retention_constructor(
        &bytes,
        MetadataRetentionLimits {
            extra_records: 6,
            ..Default::default()
        },
        false,
    );
    let mut archive = parsed.unwrap();
    assert_eq!((usage.extra_records, usage.extra_pushes), (6, 6));
    for (ordinal, expected) in [(0, &first), (1, &second)] {
        let mut entry = archive.by_index(ordinal).unwrap();
        assert_eq!(entry.extra_data(), Some(expected.as_slice()));
        let actual = entry
            .extra_data_fields()
            .map(|field| match field {
                zip::extra_fields::ExtraField::ExtendedTimestamp(value) => (
                    0,
                    value.mod_time().map(u64::from),
                    value.ac_time().map(u64::from),
                    value.cr_time().map(u64::from),
                ),
                zip::extra_fields::ExtraField::Ntfs(value) => (
                    1,
                    Some(value.mtime()),
                    Some(value.atime()),
                    Some(value.ctime()),
                ),
            })
            .collect::<Vec<_>>();
        if ordinal == 0 {
            assert_eq!(
                actual,
                [
                    (0, None, None, None),
                    (0, None, None, None),
                    (0, Some(101), None, None),
                    (1, Some(201), Some(202), Some(203))
                ]
            );
        } else {
            assert_eq!(
                actual,
                [(0, Some(301), Some(302), None), (0, Some(303), None, None)]
            );
        }
        let mut read = Vec::new();
        entry.read_to_end(&mut read).unwrap();
        assert_eq!(read, body);
    }
    let malformed = zip_bytes(&[field(0x5455, &[]), Vec::new()], &body);
    let (result, usage) = retention_constructor(
        &malformed,
        MetadataRetentionLimits {
            extra_records: 0,
            ..Default::default()
        },
        false,
    );
    assert!(
        matches!(result, Err(zip::result::ZipError::InvalidArchive(reason))
        if reason.as_ref() == "Could not find EOCD")
    );
    assert_eq!(usage.extra_preflights, 0);
    assert!(
        !usage.denied,
        "The existing len=0 error must not become a quota error"
    );
}

#[test]
fn actual_acquisition_keeps_retention_denial_typed_before_any_image_payload_copy() {
    let body = raster();
    let bytes = zip_bytes(&[empty_timestamps(3), empty_timestamps(3)], &body);
    let plan = make_plan(
        "fixture-book",
        "fixture-chapter",
        "<img src='images/0.png'><img src='images/1.png'>",
        Some("/fixture.zip"),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let normal = runtime
        .block_on(acquire_with(&plan, |_, _| {
            std::future::ready(Ok(bytes.clone()))
        }))
        .unwrap();
    assert_eq!(normal.assets.len(), 1);
    assert_eq!(normal.source_to_asset.len(), 2);
    assert_eq!(normal.assets[0].bytes, body);
    with_retention_limits(
        MetadataRetentionLimits {
            extra_records: 5,
            ..Default::default()
        },
        body.len(),
        || {
            for _ in 0..2 {
                let calls = Cell::new(0);
                let error = runtime
                    .block_on(acquire_with(&plan, |_, maximum| {
                        calls.set(calls.get() + 1);
                        assert_eq!(maximum, MAX_ARCHIVE_BYTES);
                        std::future::ready(Ok(bytes.clone()))
                    }))
                    .unwrap_err();
                assert!(matches!(error, SourceError::ResponseTooLarge));
                assert!(!error.retryable() && !error.requires_login());
                let usage = retention_usage();
                assert_eq!((usage.extra_preflights, usage.extra_pushes), (6, 5));
                assert!(usage.denied);
                assert_eq!(usage.operations_after_denial, 0);
                assert_eq!(calls.get(), 1);
                assert_eq!(rejected_payload_copies(), 0);
            }
        },
    );
}

#[test]
fn repeated_zip64_fields_rebuild_once_per_real_central_record() {
    let body = raster();
    let extra = repeated_extra(64);
    let expected = extra[..extra.len() - 4].to_vec();
    let bytes = zip_bytes(&vec![extra.clone(); 4], &body);
    let observed = AllocationScope::start(expected.len());
    let result =
        ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), MAX_ARCHIVE_ENTRIES);
    let allocations = observed.finish();
    let mut archive = result.unwrap();
    assert_eq!(allocations.calls, 4);
    assert_eq!(allocations.bytes, 4 * expected.len());
    assert_eq!(archive.len(), 4);
    for ordinal in 0..4 {
        let mut entry = archive.by_index(ordinal).unwrap();
        assert_eq!(entry.extra_data(), Some(expected.as_slice()));
        assert!(entry.get_metadata().large_file);
        let mut actual = Vec::new();
        entry.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, body);
    }
    let assets = decode_archive(&bytes).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].bytes, body);
    assert_eq!(assets[0].source_names.len(), 4);
}

#[test]
fn a_single_complete_removal_returns_none_and_unknown_fields_keep_their_bytes() {
    let body = raster();
    let single = zip_bytes(&[field(0x0001, &[])], &body);
    let mut archive = ZipArchive::new_with_file_limit(Cursor::new(single.as_slice()), 1).unwrap();
    let mut entry = archive.by_index(0).unwrap();
    assert!(entry.extra_data().is_none());
    assert!(entry.get_metadata().large_file);
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, body);
    drop(entry);
    let unknown = field(0xcafe, &vec![0x31; 2053]);
    let bytes = zip_bytes(&[unknown.clone()], &body);
    let mut archive = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1).unwrap();
    let entry = archive.by_index(0).unwrap();
    assert_eq!(entry.extra_data(), Some(unknown.as_slice()));
    assert!(!entry.get_metadata().large_file);
}

#[test]
fn only_the_last_removal_is_applied_and_later_zip64_values_keep_their_order() {
    let body = raster();
    let zip64 = |size: u64| {
        let mut payload = Vec::new();
        payload.extend_from_slice(&size.to_le_bytes());
        payload.extend_from_slice(&(body.len() as u64).to_le_bytes());
        payload.extend_from_slice(&0_u64.to_le_bytes());
        field(0x0001, &payload)
    };
    let first = zip64(body.len() as u64 + 1);
    let last = zip64(body.len() as u64);
    let unknown = field(0xcafe, &vec![0x31; 2053]);
    let mut extra = first.clone();
    extra.extend_from_slice(&unknown);
    let last_start = extra.len();
    extra.extend_from_slice(&last);
    extra.extend_from_slice(&field(0xbeef, &[1, 2, 3]));
    let mut expected = extra[..last_start].to_vec();
    expected.extend_from_slice(&extra[last_start + last.len()..]);
    // A seven-byte tail makes the reconstruction capacity even, so this case
    // checks bytes and effects separately from the odd allocation observation.
    let bytes = zip_bytes(&[extra], &body);
    for bounded in [false, true] {
        let mut archive = if bounded {
            ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1).unwrap()
        } else {
            ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap()
        };
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(entry.extra_data(), Some(expected.as_slice()));
        assert_eq!(
            &entry.extra_data().unwrap()[..first.len()],
            first.as_slice()
        );
        assert!(entry.get_metadata().large_file);
        assert_eq!(entry.size(), body.len() as u64);
        assert_eq!(entry.compressed_size(), body.len() as u64);
        assert_eq!(entry.header_start(), 0);
        let mut actual = Vec::new();
        entry.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, body);
    }
}

#[test]
fn truncated_extra_fields_keep_original_bytes_and_zip64_side_effects_without_rebuild() {
    let body = raster();
    let mut extra = repeated_extra(24);
    // Preserve the existing central parser's tolerated I/O-error behavior.
    extra.extend_from_slice(&0xbeef_u16.to_le_bytes());
    extra.extend_from_slice(&4_u16.to_le_bytes());
    extra.extend_from_slice(&[1, 2]);
    let bytes = zip_bytes(&[extra.clone()], &body);
    let observed = AllocationScope::start(extra.len() - 4);
    let result = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1);
    let allocations = observed.finish();
    let mut archive = result.unwrap();
    let mut entry = archive.by_index(0).unwrap();
    assert_eq!(entry.extra_data(), Some(extra.as_slice()));
    assert!(entry.get_metadata().large_file);
    assert_eq!(allocations.calls, 0);
    assert_eq!(allocations.bytes, 0);
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, body);
}

#[test]
fn unsupported_fields_after_removals_still_reject_before_any_reconstruction() {
    let body = raster();
    let mut extra = repeated_extra(24);
    extra.extend_from_slice(&field(0x000a, &[]));
    let bytes = zip_bytes(&[extra.clone()], &body);
    let observed = AllocationScope::start(extra.len() - 4);
    let result = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1);
    let allocations = observed.finish();
    assert!(result.is_err());
    assert_eq!(allocations.calls, 0);
    assert_eq!(allocations.bytes, 0);
    assert!(ZipArchive::new(Cursor::new(bytes.as_slice())).is_err());
    assert!(decode_archive(&bytes).is_err());
}

#[test]
fn self_extracting_prefixes_and_legacy_configuration_preserve_final_extra_bytes() {
    let body = raster();
    let extra = repeated_extra(24);
    let expected = extra[..extra.len() - 4].to_vec();
    let prefix = b"#!/bin/sh\nexit 0\n";
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(&zip_bytes(&[extra], &body));
    let observed = AllocationScope::start(expected.len());
    let result = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1);
    let allocations = observed.finish();
    let mut archive = result.unwrap();
    assert_eq!(allocations.calls, 1);
    assert_eq!(allocations.bytes, expected.len());
    let mut entry = archive.by_index(0).unwrap();
    assert_eq!(entry.extra_data(), Some(expected.as_slice()));
    assert_eq!(entry.header_start(), prefix.len() as u64);
    assert!(entry.get_metadata().large_file);
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, body);
    drop(entry);
    let mut legacy =
        ZipArchive::with_config(zip::read::Config::default(), Cursor::new(bytes.as_slice()))
            .unwrap();
    assert_eq!(
        legacy.by_index(0).unwrap().extra_data(),
        Some(expected.as_slice())
    );
    assert_eq!(legacy.len(), archive.len());
}
