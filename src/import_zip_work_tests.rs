//! Small ZIP fixtures measure cumulative metadata work through public APIs.

use std::{
    cell::{Cell, RefCell},
    io::{Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

use anyhow::Result;
use zip::{ZipArchive, ZipWriter, result::ZipError, write::SimpleFileOptions};

const GENEROUS_BYTES: u64 = 1024 * 1024;

#[derive(Default)]
struct Work {
    bytes: Cell<u64>,
    seeks: Cell<usize>,
    backwards: Cell<usize>,
    repeated: Cell<u64>,
    central_headers: Cell<usize>,
    visited: RefCell<Vec<bool>>,
}

struct Metered<'a> {
    inner: Cursor<&'a [u8]>,
    central: u64,
    work: Rc<Work>,
    maximum_chunk: usize,
}

impl<'a> Metered<'a> {
    fn new(bytes: &'a [u8], central: u64) -> (Self, Rc<Work>) {
        let work = Rc::new(Work {
            visited: RefCell::new(vec![false; bytes.len()]),
            ..Work::default()
        });
        (
            Self {
                inner: Cursor::new(bytes),
                central,
                work: work.clone(),
                maximum_chunk: usize::MAX,
            },
            work,
        )
    }

    fn short_reads(mut self, bytes: usize) -> Self {
        assert!(bytes > 0);
        self.maximum_chunk = bytes;
        self
    }
}

impl Read for Metered<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let start = self.inner.position();
        let requested = output.len();
        let length = output.len().min(self.maximum_chunk);
        let count = self.inner.read(&mut output[..length])?;
        self.work.bytes.set(self.work.bytes.get() + count as u64);
        if start == self.central && requested == 46 && count == 46 {
            // The pinned ZIP ABI reads a fixed central header separately from
            // EOCD scans and signature probes. Count real directory attempts.
            self.work
                .central_headers
                .set(self.work.central_headers.get() + 1);
        }
        let mut visited = self.work.visited.borrow_mut();
        for offset in start as usize..start as usize + count {
            if visited[offset] {
                self.work.repeated.set(self.work.repeated.get() + 1);
            } else {
                visited[offset] = true;
            }
        }
        Ok(count)
    }
}

impl Seek for Metered<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let before = self.inner.position();
        let after = self.inner.seek(position)?;
        self.work.seeks.set(self.work.seeks.get() + 1);
        if after < before {
            self.work.backwards.set(self.work.backwards.get() + 1);
        }
        Ok(after)
    }
}

fn stored_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in entries {
        writer.start_file(*name, options)?;
        writer.write_all(body)?;
    }
    Ok(writer.finish()?.into_inner())
}

fn footer(bytes: &[u8]) -> usize {
    let offset = bytes
        .len()
        .checked_sub(22)
        .expect("The fixture has no EOCD");
    assert_eq!(&bytes[offset..offset + 4], b"PK\x05\x06");
    assert_eq!(&bytes[offset + 20..offset + 22], &[0, 0]);
    offset
}

fn central_offset(bytes: &[u8]) -> u64 {
    let footer = footer(bytes);
    u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as u64
}

struct Candidates {
    bytes: Vec<u8>,
    central: u64,
    comment: Vec<u8>,
}

fn candidates(fake_count: usize) -> Result<Candidates> {
    let mut bytes = stored_zip(&[
        ("first.txt", b"First payload"),
        ("second.txt", b"Second payload"),
    ])?;
    let real = footer(&bytes);
    let central = central_offset(&bytes);
    let mut fake = bytes[real..].to_vec();
    // Every fake reaches the real two-record directory, but its third record
    // fails. The valid EOCD remains before all fakes in its own ZIP comment.
    fake[8..10].copy_from_slice(&3_u16.to_le_bytes());
    fake[10..12].copy_from_slice(&3_u16.to_le_bytes());
    let comment_bytes = u16::try_from(fake_count * fake.len())?;
    bytes[real + 20..real + 22].copy_from_slice(&comment_bytes.to_le_bytes());
    let mut comment = Vec::new();
    for _ in 0..fake_count {
        bytes.extend_from_slice(&fake);
        comment.extend_from_slice(&fake);
    }
    assert!(bytes.len() < 4096);
    assert_eq!(
        bytes
            .windows(4)
            .filter(|window| *window == b"PK\x05\x06")
            .count(),
        fake_count + 1
    );
    Ok(Candidates {
        bytes,
        central,
        comment,
    })
}

fn expect_metadata_limit<T>(result: zip::result::ZipResult<T>) {
    let error = match result {
        Ok(_) => panic!("The metadata-work limit was not enforced"),
        Err(error) => error,
    };
    assert!(
        matches!(
            &error,
            ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
        ),
        "Expected a stable metadata-work refusal, got {error}"
    );
}

#[test]
fn metadata_read_quotas_bound_actual_underlying_bytes_even_on_short_reads() -> Result<()> {
    let bytes = stored_zip(&[("body.txt", b"A complete small payload")])?;
    let central = central_offset(&bytes);
    let (reader, _) = Metered::new(&bytes, central);
    let mut admitted = ZipArchive::new_with_limits(reader.short_reads(7), 1, GENEROUS_BYTES, 8)?;
    let mut body = String::new();
    admitted.by_name("body.txt")?.read_to_string(&mut body)?;
    assert_eq!(body, "A complete small payload");
    drop(admitted);
    for quota in [0_u64, 1, 4, 16] {
        let (reader, work) = Metered::new(&bytes, central);
        expect_metadata_limit(ZipArchive::new_with_limits(
            reader.short_reads(7),
            1,
            quota,
            8,
        ));
        assert!(
            work.bytes.get() <= quota,
            "The reader consumed {} bytes with quota {quota}",
            work.bytes.get()
        );
        assert_eq!(work.central_headers.get(), 0);
    }
    Ok(())
}

#[test]
fn repeated_seeks_and_eocd_fallback_share_one_cumulative_read_budget() -> Result<()> {
    let fixture = candidates(6)?;
    let (reader, work) = Metered::new(&fixture.bytes, fixture.central);
    let admitted = ZipArchive::new_with_limits(reader, 3, GENEROUS_BYTES, 7)?;
    assert_eq!(admitted.len(), 2);
    assert_eq!(admitted.comment(), fixture.comment.as_slice());
    assert!(work.central_headers.get() >= 7);
    assert!(work.backwards.get() > 0);
    assert!(work.repeated.get() > 0);
    assert!(work.bytes.get() > fixture.bytes.len() as u64);
    drop(admitted);

    // A unique-byte or seek-reset budget would admit this archive: the whole
    // file fits, but repeated reads of its metadata must still be charged.
    let quota = fixture.bytes.len() as u64 + 64;
    let (reader, limited) = Metered::new(&fixture.bytes, fixture.central);
    expect_metadata_limit(ZipArchive::new_with_limits(reader, 3, quota, 64));
    assert!(limited.bytes.get() <= quota);
    assert!(limited.repeated.get() > 0);
    assert!(limited.seeks.get() > 0);
    Ok(())
}

#[test]
fn every_eocd_candidate_counts_even_when_its_raw_file_count_is_rejected() -> Result<()> {
    let fixture = candidates(6)?;
    for max_files in [2, 3] {
        let (reader, work) = Metered::new(&fixture.bytes, fixture.central);
        expect_metadata_limit(ZipArchive::new_with_limits(
            reader,
            max_files,
            GENEROUS_BYTES,
            6,
        ));
        assert!(work.bytes.get() <= GENEROUS_BYTES);
        if max_files == 2 {
            assert_eq!(
                work.central_headers.get(),
                0,
                "All six candidates should be rejected before central records"
            );
        } else {
            assert!(work.central_headers.get() >= 6);
        }
        let (reader, _) = Metered::new(&fixture.bytes, fixture.central);
        let admitted = ZipArchive::new_with_limits(reader, max_files, GENEROUS_BYTES, 7)?;
        assert_eq!(admitted.len(), 2);
        assert_eq!(admitted.comment(), fixture.comment.as_slice());
    }
    let (reader, work) = Metered::new(&fixture.bytes, fixture.central);
    expect_metadata_limit(ZipArchive::new_with_limits(reader, 3, GENEROUS_BYTES, 0));
    assert_eq!(work.central_headers.get(), 0);
    Ok(())
}

#[test]
fn body_reads_use_the_restored_reader_after_metadata_has_been_admitted() -> Result<()> {
    let body = b"Stored body bytes. ".repeat(200);
    assert!(body.len() < 4 * 1024);
    let bytes = stored_zip(&[("body.txt", body.as_slice())])?;
    assert!(bytes.len() < 4 * 1024);
    let central = central_offset(&bytes);
    let (reader, baseline) = Metered::new(&bytes, central);
    let measured = ZipArchive::new_with_limits(reader, 1, GENEROUS_BYTES, 8)?;
    let metadata_bytes = baseline.bytes.get();
    assert!(metadata_bytes > 0);
    drop(measured);

    let quota = metadata_bytes + 8;
    let (reader, work) = Metered::new(&bytes, central);
    let mut admitted = ZipArchive::new_with_limits(reader, 1, quota, 8)?;
    assert_eq!(work.bytes.get(), metadata_bytes);
    assert!(body.len() as u64 > quota - work.bytes.get());
    let mut actual = Vec::new();
    admitted.by_name("body.txt")?.read_to_end(&mut actual)?;
    assert_eq!(actual, body);
    assert!(
        work.bytes.get() > quota,
        "The extraction reader should no longer carry the initialization quota"
    );
    Ok(())
}

#[test]
fn bounded_defaults_do_not_change_unbounded_legacy_constructor_paths() -> Result<()> {
    // This is only a few KiB and each fake asks for three records. It safely
    // distinguishes the bounded default of 64 candidates from legacy behavior.
    let fixture = candidates(65)?;
    let legacy = ZipArchive::new(Cursor::new(fixture.bytes.as_slice()))?;
    let configured = ZipArchive::with_config(
        zip::read::Config::default(),
        Cursor::new(fixture.bytes.as_slice()),
    )?;
    assert_eq!(legacy.len(), 2);
    assert_eq!(configured.len(), 2);
    assert_eq!(legacy.comment(), fixture.comment.as_slice());
    assert_eq!(configured.comment(), legacy.comment());
    // Initialization of append retains the same unlimited metadata path. Its
    // in-memory writer is finalized by Drop; no filesystem output is created.
    drop(ZipWriter::new_append(Cursor::new(fixture.bytes.clone()))?);
    expect_metadata_limit(ZipArchive::new_with_file_limit(
        Cursor::new(fixture.bytes.as_slice()),
        3,
    ));
    let explicitly_admitted =
        ZipArchive::new_with_limits(Cursor::new(fixture.bytes.as_slice()), 3, GENEROUS_BYTES, 66)?;
    assert_eq!(explicitly_admitted.len(), 2);
    Ok(())
}
