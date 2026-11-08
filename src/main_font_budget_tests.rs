//! Genuine small-font initialization and admission, without oversized font samples.

#[path = "main_glyph_budget_tests.rs"]
mod glyph_budget_tests;

use super::font_budget::{FontLimits, FontUsage};
use super::*;
use std::{
    io::{self, Read, Write},
    sync::atomic::{AtomicUsize, Ordering},
};

struct FontTestPlatform;

impl slint::platform::Platform for FontTestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(
            slint::platform::software_renderer::MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            ),
        )
    }
}

std::thread_local! {
    // Initialize Slint's actual font context once on each libtest thread.
    // This software-window platform does not create a native display or event loop.
    static FONT_TEST_PLATFORM: () = {
        slint::platform::set_platform(Box::new(FontTestPlatform))
            .expect("Install the software font-test platform");
    };
}

fn initialize_font_test_platform() {
    FONT_TEST_PLATFORM.with(|_| ());
}

mod fixture {

    use std::collections::BTreeMap;

    fn u16_at(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn rectangle(width: i16, height: i16) -> Vec<u8> {
        let mut glyph = vec![0; 34];
        u16_at(&mut glyph, 0, 1);
        u16_at(&mut glyph, 6, width as u16);
        u16_at(&mut glyph, 8, height as u16);
        u16_at(&mut glyph, 10, 3);
        glyph[14..18].fill(1);
        for (index, delta) in [0, width, 0, -width].into_iter().enumerate() {
            u16_at(&mut glyph, 18 + 2 * index, delta as u16);
        }
        for (index, delta) in [0, 0, height, 0].into_iter().enumerate() {
            u16_at(&mut glyph, 26 + 2 * index, delta as u16);
        }
        glyph
    }

    fn names(family: &str, weight: u16, axes: usize) -> Vec<u8> {
        let style = if weight >= 700 { "Bold" } else { "Regular" };
        let full = format!("{family} {style}");
        let postscript = format!("{}-{style}", family.replace(' ', "-"));
        let mut names = vec![
            (1u16, family.to_owned()),
            (2, style.to_owned()),
            (4, full),
            (6, postscript),
            (16, family.to_owned()),
        ];
        for axis in 0..axes {
            names.push((256 + axis as u16, format!("Fixture Axis {axis}")));
        }
        let start = 6 + names.len() * 12;
        let mut table = vec![0; start];
        u16_at(&mut table, 2, names.len() as u16);
        u16_at(&mut table, 4, start as u16);
        for (index, (id, value)) in names.into_iter().enumerate() {
            let encoded = value
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            let record = 6 + index * 12;
            for (offset, word) in [
                (0, 3),
                (2, 1),
                (4, 0x0409),
                (6, id),
                (8, encoded.len() as u16),
                (10, (table.len() - start) as u16),
            ] {
                u16_at(&mut table, record + offset, word);
            }
            table.extend_from_slice(&encoded);
        }
        table
    }

    fn checksum(bytes: &[u8]) -> u32 {
        bytes.chunks(4).fold(0u32, |sum, part| {
            let mut word = [0; 4];
            word[..part.len()].copy_from_slice(part);
            sum.wrapping_add(u32::from_be_bytes(word))
        })
    }

    pub(super) fn bytes(family: &str, weight: u16, advance: u16) -> Vec<u8> {
        build(family, weight, advance, 0, None)
    }

    pub(super) fn variable_bytes(family: &str, axes: usize) -> Vec<u8> {
        build(family, 400, 500, axes, None)
    }

    pub(super) fn named_bytes(specs: &[(u16, u16, u16, u16, &str)]) -> Vec<u8> {
        let string_base = 6 + specs.len() * 12;
        let mut table = vec![0; string_base];
        u16_at(&mut table, 2, specs.len() as u16);
        u16_at(&mut table, 4, string_base as u16);
        let mut spans = BTreeMap::new();
        for (index, &(name_id, platform, encoding, language, text)) in specs.iter().enumerate() {
            let encoded = text
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            let offset = if let Some(offset) = spans.get(&encoded) {
                *offset
            } else {
                let offset = table.len() - string_base;
                spans.insert(encoded.clone(), offset);
                table.extend_from_slice(&encoded);
                offset
            };
            let record = 6 + index * 12;
            for (field, value) in [
                (0, platform),
                (2, encoding),
                (4, language),
                (6, name_id),
                (8, u16::try_from(encoded.len()).unwrap()),
                (10, u16::try_from(offset).unwrap()),
            ] {
                u16_at(&mut table, record + field, value);
            }
        }
        build("Unused fixture name", 400, 500, 0, Some(table))
    }

    pub(super) fn work_bytes(family: &str, record_count: usize) -> Vec<u8> {
        assert!((4..=32).contains(&record_count));
        let mut names = vec![(4, 3, 1, 0x0409, "Short fixture title"); record_count - 2];
        names.push((1, 3, 1, 0x0409, family));
        names.push((16, 3, 1, 0x0409, family));
        named_bytes(&names)
    }

    /// Rebuild only a tiny fixture directory. NAME entries either alias the same
    /// physical payload or point at separate copies with identical selection data.
    pub(super) fn name_table_copies(font: &[u8], copies: usize, shared: bool) -> Vec<u8> {
        assert!((1..=8).contains(&copies));
        let count = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
        let old_directory_end = 12 + count * 16;
        let extra_directory_bytes = (copies - 1) * 16;
        let name_record = font[12..old_directory_end]
            .chunks_exact(16)
            .find(|record| &record[..4] == b"name")
            .unwrap();
        let name_offset = u32::from_be_bytes(name_record[8..12].try_into().unwrap()) as usize;
        let name_length = u32::from_be_bytes(name_record[12..16].try_into().unwrap()) as usize;
        let name_data = &font[name_offset..name_offset + name_length];
        let expanded_count = count + copies - 1;
        let mut bytes = vec![0; old_directory_end + extra_directory_bytes];
        bytes[..12].copy_from_slice(&font[..12]);
        u16_at(&mut bytes, 4, u16::try_from(expanded_count).unwrap());
        let selector = usize::BITS - 1 - expanded_count.leading_zeros();
        let search_range = (1usize << selector) * 16;
        u16_at(&mut bytes, 6, u16::try_from(search_range).unwrap());
        u16_at(&mut bytes, 8, u16::try_from(selector).unwrap());
        u16_at(
            &mut bytes,
            10,
            u16::try_from(expanded_count * 16 - search_range).unwrap(),
        );
        bytes.extend_from_slice(&font[old_directory_end..]);
        let mut directory_index = 0;
        let mut head_offset = 0;
        for record in font[12..old_directory_end].chunks_exact(16) {
            let is_name = &record[..4] == b"name";
            let old_offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
            let repetitions = if is_name { copies } else { 1 };
            for copy in 0..repetitions {
                let offset = if is_name && !shared && copy > 0 {
                    while bytes.len() % 4 != 0 {
                        bytes.push(0);
                    }
                    let offset = bytes.len();
                    bytes.extend_from_slice(name_data);
                    offset
                } else {
                    old_offset + extra_directory_bytes
                };
                let position = 12 + directory_index * 16;
                bytes[position..position + 16].copy_from_slice(record);
                u32_at(&mut bytes, position + 8, u32::try_from(offset).unwrap());
                if &record[..4] == b"head" {
                    head_offset = offset;
                }
                directory_index += 1;
            }
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        u32_at(&mut bytes, head_offset + 8, 0);
        let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&bytes));
        u32_at(&mut bytes, head_offset + 8, adjustment);
        bytes
    }

    fn build(
        family: &str,
        weight: u16,
        advance: u16,
        axes: usize,
        name_override: Option<Vec<u8>>,
    ) -> Vec<u8> {
        let mut tables = BTreeMap::new();
        let mut head = vec![0; 54];
        u32_at(&mut head, 0, 0x0001_0000);
        u32_at(&mut head, 4, 0x0001_0000);
        u32_at(&mut head, 12, 0x5f0f_3cf5);
        u16_at(&mut head, 18, 1000);
        u16_at(&mut head, 40, 600);
        u16_at(&mut head, 42, 700);
        u16_at(&mut head, 46, 8);
        u16_at(&mut head, 48, 2);
        tables.insert(*b"head", head);
        let mut hhea = vec![0; 36];
        u32_at(&mut hhea, 0, 0x0001_0000);
        u16_at(&mut hhea, 4, 800);
        u16_at(&mut hhea, 6, (-200i16) as u16);
        u16_at(&mut hhea, 10, 600);
        u16_at(&mut hhea, 18, 1);
        u16_at(&mut hhea, 34, 4);
        tables.insert(*b"hhea", hhea);
        let mut hmtx = vec![0; 16];
        for (glyph, advance) in [500, advance, 0, 600].into_iter().enumerate() {
            u16_at(&mut hmtx, glyph * 4, advance);
        }
        tables.insert(*b"hmtx", hmtx);
        let mut maxp = vec![0; 32];
        u32_at(&mut maxp, 0, 0x0001_0000);
        u16_at(&mut maxp, 4, 4);
        u16_at(&mut maxp, 6, 4);
        u16_at(&mut maxp, 8, 1);
        u16_at(&mut maxp, 14, 1);
        tables.insert(*b"maxp", maxp);
        let mut cmap = vec![0; 60];
        u16_at(&mut cmap, 2, 1);
        u16_at(&mut cmap, 4, 3);
        u16_at(&mut cmap, 6, 1);
        u32_at(&mut cmap, 8, 12);
        for (offset, value) in [(12, 4), (14, 48), (18, 8), (20, 8), (22, 2)] {
            u16_at(&mut cmap, offset, value);
        }
        for (index, code) in [0x0041u16, 0x0042, 0x0301, 0xffff].into_iter().enumerate() {
            u16_at(&mut cmap, 26 + index * 2, code);
            u16_at(&mut cmap, 36 + index * 2, code);
            let glyph = [1u16, 3, 2, 0][index];
            u16_at(&mut cmap, 44 + index * 2, glyph.wrapping_sub(code));
        }
        tables.insert(*b"cmap", cmap);
        let mut kern = vec![0; 36];
        u16_at(&mut kern, 2, 1);
        u16_at(&mut kern, 6, 32);
        u16_at(&mut kern, 8, 1);
        u16_at(&mut kern, 10, 3);
        u16_at(&mut kern, 12, 12);
        u16_at(&mut kern, 14, 1);
        u16_at(&mut kern, 16, 6);
        for (index, (left, right, value)) in [(1, 2, 5i16), (2, 2, 10), (2, 3, -25)]
            .into_iter()
            .enumerate()
        {
            let offset = 18 + index * 6;
            u16_at(&mut kern, offset, left);
            u16_at(&mut kern, offset + 2, right);
            u16_at(&mut kern, offset + 4, value as u16);
        }
        tables.insert(*b"kern", kern);
        let mut glyf = vec![0; 12];
        glyf.extend(rectangle(400, 700));
        glyf.extend(rectangle(300, 200));
        glyf.extend(rectangle(550, 700));
        tables.insert(*b"glyf", glyf);
        let mut loca = vec![0; 10];
        for (index, offset) in [0, 6, 23, 40, 57].into_iter().enumerate() {
            u16_at(&mut loca, index * 2, offset);
        }
        tables.insert(*b"loca", loca);
        tables.insert(
            *b"name",
            name_override.unwrap_or_else(|| names(family, weight, axes)),
        );
        let mut os2 = vec![0; 78];
        u16_at(&mut os2, 2, 500);
        u16_at(&mut os2, 4, weight);
        u16_at(&mut os2, 6, 5);
        os2[58..62].copy_from_slice(b"TEST");
        u16_at(&mut os2, 62, if weight >= 700 { 0x0020 } else { 0x0040 });
        u16_at(&mut os2, 64, 0x0041);
        u16_at(&mut os2, 66, 0x0301);
        u16_at(&mut os2, 68, 800);
        u16_at(&mut os2, 70, (-200i16) as u16);
        u16_at(&mut os2, 74, 800);
        u16_at(&mut os2, 76, 200);
        tables.insert(*b"OS/2", os2);
        let mut post = vec![0; 32];
        u32_at(&mut post, 0, 0x0003_0000);
        u16_at(&mut post, 8, (-75i16) as u16);
        u16_at(&mut post, 10, 50);
        tables.insert(*b"post", post);
        if axes > 0 {
            let mut fvar = vec![0; 16 + axes * 20];
            u16_at(&mut fvar, 0, 1);
            u16_at(&mut fvar, 4, 16);
            u16_at(&mut fvar, 6, 2);
            u16_at(&mut fvar, 8, axes as u16);
            u16_at(&mut fvar, 10, 20);
            u16_at(&mut fvar, 14, (4 + axes * 4) as u16);
            for axis in 0..axes {
                let record = 16 + axis * 20;
                fvar[record..record + 4].copy_from_slice(format!("X{axis:03}").as_bytes());
                u32_at(&mut fvar, record + 12, 0x0001_0000);
                u16_at(&mut fvar, record + 18, 256 + axis as u16);
            }
            tables.insert(*b"fvar", fvar);
        }
        let count = tables.len();
        let mut font = vec![0; 12 + count * 16];
        u32_at(&mut font, 0, 0x0001_0000);
        u16_at(&mut font, 4, count as u16);
        u16_at(&mut font, 6, 128);
        u16_at(&mut font, 8, 3);
        u16_at(&mut font, 10, (count * 16 - 128) as u16);
        let mut head_offset = 0;
        for (index, (tag, data)) in tables.into_iter().enumerate() {
            while font.len() % 4 != 0 {
                font.push(0);
            }
            let offset = font.len();
            let record = 12 + index * 16;
            font[record..record + 4].copy_from_slice(&tag);
            u32_at(&mut font, record + 4, checksum(&data));
            u32_at(&mut font, record + 8, offset as u32);
            u32_at(&mut font, record + 12, data.len() as u32);
            if tag == *b"head" {
                head_offset = offset;
            }
            font.extend(data);
        }
        while font.len() % 4 != 0 {
            font.push(0);
        }
        let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&font));
        u32_at(&mut font, head_offset + 8, adjustment);
        font
    }
}

fn collection(fonts: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0; 12 + fonts.len() * 4];
    bytes[..4].copy_from_slice(b"ttcf");
    bytes[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&(fonts.len() as u32).to_be_bytes());
    for (index, font) in fonts.iter().enumerate() {
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let base = bytes.len();
        bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(base as u32).to_be_bytes());
        let mut font = font.clone();
        let tables = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
        for record in 0..tables {
            let position = 12 + record * 16 + 8;
            let offset = u32::from_be_bytes(font[position..position + 4].try_into().unwrap());
            font[position..position + 4].copy_from_slice(&(offset + base as u32).to_be_bytes());
        }
        bytes.extend(font);
    }
    bytes
}

fn shared_name_collection(font: &[u8], faces: usize) -> Vec<u8> {
    assert!((2..=4).contains(&faces));
    let fonts = (0..faces).map(|_| font.to_vec()).collect::<Vec<_>>();
    let mut bytes = collection(&fonts);
    let name_tag = owned_ttf_parser::Tag::from_bytes(b"name");
    let first_name = owned_ttf_parser::RawFace::parse(&bytes, 0)
        .unwrap()
        .table_records
        .into_iter()
        .find(|record| record.tag == name_tag)
        .unwrap()
        .offset;
    for index in 1..faces {
        let base =
            u32::from_be_bytes(bytes[12 + index * 4..16 + index * 4].try_into().unwrap()) as usize;
        let count = u16::from_be_bytes(bytes[base + 4..base + 6].try_into().unwrap()) as usize;
        let position = (0..count)
            .map(|record| base + 12 + record * 16)
            .find(|position| &bytes[*position..*position + 4] == b"name")
            .unwrap();
        bytes[position + 8..position + 12].copy_from_slice(&first_name.to_be_bytes());
    }
    bytes
}

fn assert_name_fixture_is_supported(bytes: &[u8], family: &str, face_count: usize) {
    for index in 0..face_count {
        owned_ttf_parser::Face::parse(bytes, index as u32).unwrap();
    }
    let mut temporary = fontique::Collection::new(fontique::CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let registered = temporary.register_fonts(fontique::Blob::new(Arc::new(bytes.to_vec())), None);
    let mut indices = BTreeSet::new();
    for (id, fonts) in registered {
        assert_eq!(temporary.family_name(id), Some(family));
        indices.extend(fonts.into_iter().map(|font| font.index()));
    }
    assert_eq!(indices, (0..face_count as u32).collect::<BTreeSet<_>>());
}

fn name_preflight(bytes: &[u8], index: u32) -> FontBudget {
    let mut budget = FontBudget::default();
    admit_font_names(bytes, index, &mut budget).unwrap();
    budget
}

fn assert_no_font_registration(budget: &FontBudget, family: &str) {
    assert_eq!(
        (
            budget.temporary_registrations,
            budget.metric_copies,
            budget.shared_registrations
        ),
        (0, 0, 0)
    );
    assert!(!shared_has_family(family));
}

fn shared_has_family(name: &str) -> bool {
    slint::fontique_010::shared_collection()
        .family_id(name)
        .is_some()
}

fn record_snapshot(records: &[FontRecord], choices: &BTreeMap<String, f32>) -> Vec<String> {
    let mut result = records
        .iter()
        .map(|record| {
            format!(
                "{}:{}:{}:{}:{:?}",
                record.id,
                record.family,
                record.available,
                record.metrics.advance('A', 1000.0),
                record
                    .weights
                    .iter()
                    .map(|(weight, metrics)| (*weight, metrics.advance('A', 1000.0)))
                    .collect::<Vec<_>>()
            )
        })
        .collect::<Vec<_>>();
    result.push(format!("{choices:?}"));
    result
}

#[test]
fn a_valid_collection_is_rejected_before_shared_registration_or_metric_copy() {
    initialize_font_test_platform();
    let names = ["Budget Raw TTC Alpha", "Budget Raw TTC Beta"];
    let bytes = Arc::new(collection(&[
        fixture::bytes(names[0], 400, 500),
        fixture::bytes(names[1], 400, 600),
    ]));
    for index in 0..2 {
        owned_ttf_parser::Face::parse(bytes.as_slice(), index).unwrap();
    }
    assert!(names.iter().all(|name| !shared_has_family(name)));
    let mut budget = FontBudget::new(FontLimits {
        per_file_faces: 1,
        ..Default::default()
    });
    let error = register_font_data(bytes.clone(), &mut budget)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::Faces)
    );
    assert_eq!((budget.metric_copies, budget.shared_registrations), (0, 0));
    assert_eq!(budget.usage().faces, 0);
    assert_eq!(budget.usage().payload_and_metrics, 0);
    assert_eq!(budget.usage().metadata, 0);
    assert_eq!(budget.usage().read_work, bytes.len());
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert!(names.iter().all(|name| !shared_has_family(name)));
}

#[test]
fn real_collection_copies_are_prepaid_before_the_byte_boundary() {
    initialize_font_test_platform();
    let names = ["Budget Byte TTC Alpha", "Budget Byte TTC Beta"];
    let bytes = Arc::new(collection(&[
        fixture::bytes(names[0], 400, 500),
        fixture::bytes(names[1], 700, 600),
    ]));
    let charge = bytes.len() * 3;
    let mut denied = FontBudget::new(FontLimits {
        payload_and_metrics: charge - 1,
        ..Default::default()
    });
    assert!(register_font_data(bytes.clone(), &mut denied).is_err());
    assert_eq!((denied.metric_copies, denied.shared_registrations), (0, 0));
    assert!(names.iter().all(|name| !shared_has_family(name)));
    let mut admitted = FontBudget::new(FontLimits {
        payload_and_metrics: charge,
        ..Default::default()
    });
    let prepared = prepare_font(bytes, &mut admitted).unwrap();
    assert_eq!(
        (admitted.metric_copies, admitted.shared_registrations),
        (2, 0)
    );
    assert_eq!(admitted.usage().payload_and_metrics, charge);
    assert_eq!(prepared.faces.len(), 2);
    publish_font(&prepared, &mut admitted);
    assert_eq!(admitted.shared_registrations, 1);
    assert!(names.iter().all(|name| shared_has_family(name)));
}

#[test]
fn ordinary_metrics_lookups_share_the_prepared_font_without_blob_recopies() {
    initialize_font_test_platform();
    let family = "Budget Ordinary Single Face";
    let bytes = Arc::new(fixture::bytes(family, 400, 530));
    let mut budget = FontBudget::default();
    let prepared = register_font_data(bytes, &mut budget).unwrap();
    assert_eq!((budget.metric_copies, budget.shared_registrations), (1, 1));
    let first = registered_metrics(&prepared, &prepared.faces[0]).unwrap();
    let second = registered_metrics(&prepared, &prepared.faces[0]).unwrap();
    assert!(first.is_font_backed() && second.is_font_backed());
    assert_eq!(first.advance('A', 1000.0), 530.0);
    assert_eq!(second.advance('A', 1000.0), 530.0);
    assert_eq!(budget.metric_copies, 1);
    assert!(shared_has_family(family));
}

#[test]
fn custom_weights_and_defaults_commit_only_after_the_whole_file_prepares() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let family = "Budget Custom Weight Family";
    let regular = directory.path().join("regular.ttf");
    let bold = directory.path().join("bold.otf");
    fs::write(&regular, fixture::bytes(family, 400, 500)).unwrap();
    fs::write(&bold, fixture::bytes(family, 700, 640)).unwrap();
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    let mut budget = FontBudget::default();
    load_custom_font(&regular, &mut records, &mut choices, &mut budget).unwrap();
    load_custom_font(&bold, &mut records, &mut choices, &mut budget).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].weights.keys().copied().collect::<Vec<_>>(),
        [400, 700]
    );
    assert_eq!(records[0].metrics.advance('A', 1000.0), 500.0);
    assert_eq!(records[0].weights[&700].advance('A', 1000.0), 640.0);
    let before = record_snapshot(&records, &choices);
    let charged = budget.usage().payload_and_metrics;
    let mut invalid = collection(&[
        fixture::bytes("Budget Never Commit Alpha", 400, 500),
        fixture::bytes("Budget Never Commit Beta", 400, 500),
    ]);
    let second = u32::from_be_bytes(invalid[16..20].try_into().unwrap()) as usize;
    invalid[second..second + 4].fill(0xff);
    let invalid_path = directory.path().join("invalid.ttf");
    fs::write(&invalid_path, invalid).unwrap();
    assert!(load_custom_font(&invalid_path, &mut records, &mut choices, &mut budget).is_err());
    assert_eq!(record_snapshot(&records, &choices), before);
    assert_eq!(budget.shared_registrations, 2);
    assert!(budget.usage().payload_and_metrics > charged);
    assert!(!shared_has_family("Budget Never Commit Alpha"));
}

#[test]
fn overwriting_a_registered_weight_consumes_new_source_capacity_without_refund() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let family = "Budget Overwrite Single Weight";
    let first = directory.path().join("first.ttf");
    let next = directory.path().join("next.ttf");
    let first_bytes = fixture::bytes(family, 400, 500);
    let next_bytes = fixture::bytes(family, 400, 620);
    fs::write(&first, &first_bytes).unwrap();
    fs::write(&next, &next_bytes).unwrap();
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    let mut budget = FontBudget::default();
    load_custom_font(&first, &mut records, &mut choices, &mut budget).unwrap();
    load_custom_font(&next, &mut records, &mut choices, &mut budget).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].metrics_for_weight(400).advance('A', 1000.0),
        620.0
    );
    assert_eq!(records[0].metrics.advance('A', 1000.0), 500.0);
    assert_eq!(budget.usage().faces, 2);
    assert_eq!(
        budget.usage().payload_and_metrics,
        2 * (first_bytes.len() + next_bytes.len())
    );
    assert_eq!((budget.metric_copies, budget.shared_registrations), (2, 2));
}

#[test]
fn invalid_font_attempts_keep_their_charge_and_cannot_modify_old_choices() {
    initialize_font_test_platform();
    let mut budget = FontBudget::default();
    let invalid = Arc::new(b"invalid font bytes".to_vec());
    assert!(register_font_data(invalid.clone(), &mut budget).is_err());
    assert_eq!(budget.usage().payload_and_metrics, invalid.len() * 2);
    assert_eq!(budget.usage().faces, 1);
    assert_eq!((budget.metric_copies, budget.shared_registrations), (0, 0));
    let before = budget.usage();
    assert!(budget.reserve_font(usize::MAX, 2).is_err());
    assert_eq!(budget.usage(), before);
    assert!(budget.reserve_metadata(usize::MAX).is_err());
    assert_eq!(budget.usage(), before);
}

#[test]
fn staging_exhaustion_keeps_old_records_and_shared_registration_unchanged() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let family = "Budget Staging Must Not Publish";
    let data = fixture::bytes(family, 400, 500);
    let path = directory.path().join("staging.ttf");
    fs::write(&path, &data).unwrap();
    let mut records = vec![FontRecord {
        id: "old".repeat(100),
        label: "Old selection".repeat(100),
        family: "Old fixture".into(),
        metrics: TextMetrics::default(),
        weights: BTreeMap::new(),
        available: false,
    }];
    let mut choices = BTreeMap::new();
    let before = record_snapshot(&records, &choices);
    let mut envelope = FontBudget::default();
    prepare_font(Arc::new(data.clone()), &mut envelope).unwrap();
    let mut budget = FontBudget::new(FontLimits {
        metadata: envelope.usage().metadata,
        ..Default::default()
    });
    assert!(load_custom_font(&path, &mut records, &mut choices, &mut budget).is_err());
    assert_eq!(record_snapshot(&records, &choices), before);
    assert_eq!((budget.metric_copies, budget.shared_registrations), (1, 0));
    assert!(!shared_has_family(family));
}

struct CountedFile {
    file: fs::File,
    bytes: Arc<AtomicUsize>,
}

impl Read for CountedFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(buffer)?;
        self.bytes.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

#[test]
fn a_real_file_growing_after_metadata_stops_at_the_read_allowance() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("growing.ttf");
    fs::write(&path, b"seed").unwrap();
    let file = fs::File::open(&path).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 4);
    let mut budget = FontBudget::new(FontLimits {
        payload_and_metrics: 32,
        ..Default::default()
    });
    assert!(budget.can_read_file(file.metadata().unwrap().len() as usize));
    let maximum = budget.payload_read_limit(MAX_FONT_BYTES);
    assert_eq!(maximum, 16);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[b'x'; 64])
        .unwrap();
    let bytes = Arc::new(AtomicUsize::new(0));
    let counted = CountedFile {
        file,
        bytes: bytes.clone(),
    };
    assert!(read_font_payload(counted, maximum, &mut budget).is_err());
    assert_eq!(bytes.load(Ordering::SeqCst), 17);
    assert_eq!(budget.usage().read_work, 17);
    assert_eq!(
        read_font_payload(io::Cursor::new([b'x'; 16]), 16, &mut budget)
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn excessive_inventory_preserves_existing_records_without_loading_any_file() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    for name in ["a.ttf", "b.ttf", "c.ttf"] {
        fs::write(directory.path().join(name), b"not a font").unwrap();
    }
    let limits = FontLimits {
        inventory_entries: 2,
        ..Default::default()
    };
    assert!(font_inventory(directory.path(), limits).is_err());
    let mut records = Vec::new();
    let mut budget = FontBudget::new(limits);
    load_custom_fonts(directory.path(), &mut records, &mut budget).unwrap();
    assert!(records.is_empty());
    assert_eq!(budget.usage(), FontUsage::default());
    assert_eq!((budget.metric_copies, budget.shared_registrations), (0, 0));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}

#[test]
fn inventory_path_bytes_are_admitted_before_full_prefix_paths_accumulate() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    for name in ["font-a.ttf", "font-b.ttf"] {
        fs::write(directory.path().join(name), b"not a font").unwrap();
    }
    let one_path = directory
        .path()
        .join("font-a.ttf")
        .as_os_str()
        .as_encoded_bytes()
        .len();
    let limits = FontLimits {
        inventory_path_bytes: one_path * 2 - 1,
        ..Default::default()
    };
    let error = font_inventory(directory.path(), limits).err().unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::Inventory)
    );
}

#[test]
fn the_seven_real_bundled_payloads_fit_the_default_envelope_and_copy_once() {
    initialize_font_test_platform();
    let fonts = BundledFonts::load().unwrap();
    let payloads = [
        &fonts.sans_regular,
        &fonts.sans_medium,
        &fonts.sans_bold,
        &fonts.serif_light,
        &fonts.serif_regular,
        &fonts.serif_medium,
        &fonts.serif_bold,
    ];
    let total = payloads.iter().map(|font| font.len()).sum::<usize>();
    assert_eq!(total, 64_104_120);
    let mut budget = FontBudget::default();
    println!(
        "stock_glyph_work,index,index_read,index_span,dict_parse,projection,program,operand,output,general,total"
    );
    for (index, bytes) in payloads.into_iter().enumerate() {
        let previous_work = budget.glyph_work_usage;
        let prepared = register_font_data(bytes.clone(), &mut budget).unwrap_or_else(|error| {
            panic!(
                "Bundled font {index} failed admission: {error:#}; usage: {:?}; categories: {:?}; font_delta: {:?}",
                budget.usage(), budget.glyph_work_usage,
                budget.glyph_work_usage.difference(previous_work)
            )
        });
        let work = budget.glyph_work_usage.difference(previous_work);
        assert_eq!(budget.glyph_work_usage.total(), budget.usage().glyph_work);
        println!(
            "stock_glyph_work,{index},{},{},{},{},{},{},{},{},{}",
            work.index_read,
            work.index_span,
            work.dict_parse,
            work.dict_projection,
            work.program,
            work.operand,
            work.output,
            work.general,
            work.total()
        );
        assert_eq!(prepared.faces.len(), 1);
        let metrics = registered_metrics(&prepared, &prepared.faces[0]).unwrap();
        assert!(metrics.is_font_backed());
    }
    assert_eq!(budget.usage().payload_and_metrics, 2 * total);
    assert_eq!(budget.usage().faces, 7);
    assert_eq!(budget.usage().read_work, total);
    assert_eq!((budget.metric_copies, budget.shared_registrations), (7, 7));
    assert!(budget.usage().payload_and_metrics < budget.limits.payload_and_metrics);
    assert!(budget.usage().name_work > 0 && budget.usage().name_work <= budget.limits.name_work);
    assert!(budget.glyph_graph_nodes > 7);
    assert!(budget.usage().glyph_points > 7 * 4);
    assert!(budget.usage().glyph_work > 0 && budget.usage().glyph_work <= budget.limits.glyph_work);
}

struct ErrorAfterPrefix {
    file: fs::File,
    returned_prefix: bool,
}

impl Read for ErrorAfterPrefix {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.returned_prefix {
            return Err(io::Error::other(
                "Synthetic read failure after real file bytes",
            ));
        }
        self.returned_prefix = true;
        let take = buffer.len().min(5);
        self.file.read(&mut buffer[..take])
    }
}

#[test]
fn actual_read_work_survives_rejected_files_probe_bytes_and_partial_io_errors() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let names = ["Budget Read Rejected Alpha", "Budget Read Rejected Beta"];
    let data = collection(&[
        fixture::bytes(names[0], 400, 500),
        fixture::bytes(names[1], 400, 500),
    ]);
    let paths = [
        directory.path().join("reject-a.ttf"),
        directory.path().join("reject-b.ttf"),
        directory.path().join("reject-c.ttf"),
    ];
    for path in &paths {
        fs::write(path, &data).unwrap();
    }
    let mut budget = FontBudget::new(FontLimits {
        per_file_faces: 1,
        read_work: data.len() * 2,
        ..Default::default()
    });
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    for (index, path) in paths.iter().enumerate() {
        assert!(load_custom_font(path, &mut records, &mut choices, &mut budget).is_err());
        assert_eq!(budget.usage().read_work, data.len() * (index + 1).min(2));
    }
    assert!(records.is_empty() && choices.is_empty());
    assert_eq!((budget.metric_copies, budget.shared_registrations), (0, 0));
    assert!(names.iter().all(|name| !shared_has_family(name)));

    let partial_path = directory.path().join("partial.ttf");
    fs::write(&partial_path, b"A real partial prefix").unwrap();
    let partial = ErrorAfterPrefix {
        file: fs::File::open(&partial_path).unwrap(),
        returned_prefix: false,
    };
    let mut partial_budget = FontBudget::new(FontLimits {
        read_work: 16,
        ..Default::default()
    });
    assert!(read_font_payload(partial, 64, &mut partial_budget).is_err());
    assert_eq!(partial_budget.usage().read_work, 5);
    let observed = Arc::new(AtomicUsize::new(0));
    let counted = CountedFile {
        file: fs::File::open(&partial_path).unwrap(),
        bytes: observed.clone(),
    };
    assert!(read_font_payload(counted, 64, &mut partial_budget).is_err());
    assert_eq!(observed.load(Ordering::SeqCst), 12);
    assert_eq!(partial_budget.usage().read_work, 17);
    let observed = Arc::new(AtomicUsize::new(0));
    let blocked = CountedFile {
        file: fs::File::open(&partial_path).unwrap(),
        bytes: observed.clone(),
    };
    assert!(read_font_payload(blocked, 64, &mut partial_budget).is_err());
    assert_eq!(observed.load(Ordering::SeqCst), 0);
}

#[test]
fn variation_axis_counts_are_admitted_before_fontique_retains_axis_vectors() {
    initialize_font_test_platform();
    let family = "Budget Many Axis Source";
    let bytes = Arc::new(fixture::variable_bytes(family, MAX_VARIATION_AXES + 1));
    let borrowed = owned_ttf_parser::Face::parse(bytes.as_slice(), 0).unwrap();
    assert_eq!(
        borrowed.variation_axes().len() as usize,
        MAX_VARIATION_AXES + 1
    );
    let mut budget = FontBudget::default();
    let error = register_font_data(bytes, &mut budget).err().unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::Metadata)
    );
    assert_eq!((budget.metric_copies, budget.shared_registrations), (0, 0));
    assert!(!shared_has_family(family));
    let mut budget = FontBudget::default();
    let ordinary = register_font_data(
        Arc::new(fixture::variable_bytes("Budget Three Axis Source", 3)),
        &mut budget,
    )
    .unwrap();
    assert_eq!(ordinary.faces.len(), 1);
    assert_eq!((budget.metric_copies, budget.shared_registrations), (1, 1));
}

#[test]
fn selected_name_spans_are_prepaid_before_temporary_decoding_without_changing_english_choice() {
    initialize_font_test_platform();
    // One shared, valid UTF-16 span expands past the old fixed metadata fee,
    // while the complete fixture remains only tens of KiB.
    let long = "\u{4e2d}".repeat(22_000);
    let data = Arc::new(fixture::named_bytes(&[
        (1, 3, 1, 0x0409, long.as_str()),
        (16, 3, 1, 0x0409, long.as_str()),
        (2, 3, 1, 0x0409, "Regular"),
    ]));
    owned_ttf_parser::Face::parse(data.as_slice(), 0).unwrap();
    let mut calibration = fontique::Collection::new(fontique::CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let (id, _) = calibration
        .register_fonts(fontique::Blob::new(data.clone()), None)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(calibration.family_name(id).unwrap().len(), long.len());
    let mut fixed = FontBudget::default();
    fixed.reserve_font(data.len(), 1).unwrap();
    assert!(long.len() > fixed.usage().metadata);
    let mut budget = FontBudget::new(FontLimits {
        metadata: fixed.usage().metadata,
        ..Default::default()
    });
    let error = register_font_data(data, &mut budget).err().unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::Metadata)
    );
    assert_eq!(
        (
            budget.temporary_registrations,
            budget.metric_copies,
            budget.shared_registrations
        ),
        (0, 0, 0)
    );
    assert!(!shared_has_family(&long));

    // Enumeration is physical before ID filtering. Only the English selected
    // TYPO/FAMILY candidates are prepaid; the unselected long name is untouched.
    let english = "Budget Selected English Name";
    let data = Arc::new(fixture::named_bytes(&[
        (4, 3, 1, 0x0409, "Budget Full Name"),
        (1, 3, 1, 0x0404, long.as_str()),
        (16, 3, 1, 0, "Budget Alternate Name"),
        (16, 3, 1, 0x0409, english),
        (1, 3, 1, 0x0409, "Budget Selected Family Name"),
        (2, 3, 1, 0x0409, "Regular"),
    ]));
    let mut budget = FontBudget::new(FontLimits {
        metadata: fixed.usage().metadata + 16 * 1024,
        ..Default::default()
    });
    let prepared = register_font_data(data, &mut budget).unwrap();
    assert_eq!(prepared.faces[0].family, english);
    assert_eq!(
        (
            budget.temporary_registrations,
            budget.metric_copies,
            budget.shared_registrations
        ),
        (1, 1, 1)
    );
}

#[test]
fn name_directory_work_is_admitted_before_raw_table_visits() {
    initialize_font_test_platform();
    let family = "Budget Name Directory Gate";
    let bytes = Arc::new(fixture::work_bytes(family, 8));
    assert_name_fixture_is_supported(bytes.as_slice(), family, 1);
    for allowance in [0, 32] {
        let mut budget = FontBudget::new(FontLimits {
            name_work: allowance,
            ..Default::default()
        });
        let error = register_font_data(bytes.clone(), &mut budget)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::NameWork)
        );
        assert_eq!(budget.name_root_headers, if allowance == 0 { 0 } else { 1 });
        assert_eq!(budget.name_directory_records, 0);
        assert_eq!(budget.name_table_headers, 0);
        assert_eq!(budget.name_record_visits, 0);
        assert_eq!(budget.usage().name_work, allowance);
        assert_eq!(budget.usage().read_work, bytes.len());
        assert_eq!(Arc::strong_count(&bytes), 1);
        assert_no_font_registration(&budget, family);
    }
}

#[test]
fn repeated_name_directories_charge_records_before_any_search() {
    initialize_font_test_platform();
    let family = "Budget Repeated Name Search Gate";
    let plain = fixture::work_bytes(family, 16);
    let one = name_preflight(&plain, 0);
    let repeated = Arc::new(fixture::name_table_copies(&plain, 4, true));
    assert_name_fixture_is_supported(repeated.as_slice(), family, 1);
    let mut budget = FontBudget::new(FontLimits {
        name_work: one.usage().name_work,
        ..Default::default()
    });
    let error = register_font_data(repeated.clone(), &mut budget)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::NameWork)
    );
    assert_eq!(budget.name_root_headers, 1);
    assert!(budget.name_directory_records > 0);
    assert_eq!(budget.name_table_headers, 1);
    assert_eq!(budget.name_record_visits, 0);
    let mut fixed = FontBudget::default();
    fixed.reserve_font(repeated.len(), 1).unwrap();
    assert_eq!(budget.usage().metadata, fixed.usage().metadata);
    assert_eq!(budget.usage().read_work, repeated.len());
    assert_eq!(Arc::strong_count(&repeated), 1);
    assert_no_font_registration(&budget, family);
}

#[test]
fn repeated_and_distinct_name_tables_share_one_work_allowance() {
    initialize_font_test_platform();
    for shared in [true, false] {
        let family = if shared {
            "Budget Same Name Work"
        } else {
            "Budget Distinct Name Work"
        };
        let plain = fixture::work_bytes(family, 8);
        let one = name_preflight(&plain, 0);
        let repeated = Arc::new(fixture::name_table_copies(&plain, 3, shared));
        assert_name_fixture_is_supported(repeated.as_slice(), family, 1);
        let whole = name_preflight(repeated.as_slice(), 0);
        assert_eq!(whole.name_record_visits, one.name_record_visits * 3);
        let allowance = whole.usage().name_work - 1;
        let mut budget = FontBudget::new(FontLimits {
            name_work: allowance,
            ..Default::default()
        });
        let error = register_font_data(repeated.clone(), &mut budget)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::NameWork)
        );
        assert_eq!(budget.name_table_headers, 3);
        assert_eq!(budget.name_record_visits, one.name_record_visits * 2);
        assert!(budget.usage().name_work < allowance);
        let mut fixed = FontBudget::default();
        fixed.reserve_font(repeated.len(), 1).unwrap();
        assert_eq!(
            budget.usage().metadata,
            fixed.usage().metadata + one.usage().metadata * 2
        );
        let spent = budget.usage().name_work;
        assert!(register_font_data(repeated.clone(), &mut budget).is_err());
        assert!(budget.usage().name_work > spent);
        assert_eq!(budget.name_record_visits, one.name_record_visits * 2);
        assert_eq!(budget.usage().read_work, repeated.len() * 2);
        assert_no_font_registration(&budget, family);

        if !shared {
            // The first-match Fontique source remains usable, but an invalid
            // later raw NAME entry must still fail closed before publication.
            let mut invalid = repeated.as_ref().clone();
            let count = u16::from_be_bytes(invalid[4..6].try_into().unwrap()) as usize;
            let last_name = (0..count)
                .map(|record| 12 + record * 16)
                .filter(|position| &invalid[*position..*position + 4] == b"name")
                .last()
                .unwrap();
            let invalid_offset = u32::try_from(invalid.len())
                .unwrap()
                .checked_add(4)
                .unwrap();
            invalid[last_name + 8..last_name + 12].copy_from_slice(&invalid_offset.to_be_bytes());
            assert_name_fixture_is_supported(&invalid, family, 1);
            let mut denied = FontBudget::default();
            let error = register_font_data(Arc::new(invalid), &mut denied)
                .err()
                .unwrap();
            assert!(
                error
                    .to_string()
                    .contains("The font name table is out of bounds")
            );
            assert_eq!(denied.name_record_visits, one.name_record_visits * 2);
            assert_no_font_registration(&denied, family);
        }
    }
}

#[test]
fn shared_name_tables_across_collection_faces_keep_cumulative_work() {
    initialize_font_test_platform();
    let family = "Budget Shared TTC Name Work";
    let plain = fixture::work_bytes(family, 8);
    let bytes = Arc::new(shared_name_collection(&plain, 2));
    assert_name_fixture_is_supported(bytes.as_slice(), family, 2);
    let name_tag = owned_ttf_parser::Tag::from_bytes(b"name");
    let first = owned_ttf_parser::RawFace::parse(bytes.as_slice(), 0)
        .unwrap()
        .table(name_tag)
        .unwrap();
    let second = owned_ttf_parser::RawFace::parse(bytes.as_slice(), 1)
        .unwrap()
        .table(name_tag)
        .unwrap();
    assert_eq!(
        (first.as_ptr(), first.len()),
        (second.as_ptr(), second.len())
    );
    let first_work = name_preflight(bytes.as_slice(), 0);
    let second_work = name_preflight(bytes.as_slice(), 1);
    let allowance = first_work.usage().name_work + second_work.usage().name_work - 1;
    let mut budget = FontBudget::new(FontLimits {
        name_work: allowance,
        ..Default::default()
    });
    let error = register_font_data(bytes.clone(), &mut budget)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::NameWork)
    );
    assert_eq!(budget.name_root_headers, 2);
    assert_eq!(budget.name_table_headers, 2);
    assert_eq!(budget.name_record_visits, first_work.name_record_visits);
    let mut fixed = FontBudget::default();
    fixed.reserve_font(bytes.len(), 2).unwrap();
    assert_eq!(
        budget.usage().metadata,
        fixed.usage().metadata + first_work.usage().metadata
    );
    assert_eq!(budget.usage().read_work, bytes.len());
    assert_eq!(budget.usage().faces, 2);
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert_no_font_registration(&budget, family);
}

#[test]
fn different_name_arrays_keep_old_choices_and_share_file_work() {
    initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let first_family = "Budget Name File Original";
    let second_family = "Budget Name File Denied";
    let first_data = fixture::work_bytes(first_family, 4);
    let second_data = fixture::work_bytes(second_family, 32);
    assert_name_fixture_is_supported(&first_data, first_family, 1);
    assert_name_fixture_is_supported(&second_data, second_family, 1);
    let first_work = name_preflight(&first_data, 0);
    let second_work = name_preflight(&second_data, 0);
    let first_path = directory.path().join("first.ttf");
    let second_path = directory.path().join("second.ttf");
    fs::write(&first_path, &first_data).unwrap();
    fs::write(&second_path, &second_data).unwrap();
    let allowance = first_work.usage().name_work + second_work.usage().name_work - 1;
    let mut budget = FontBudget::new(FontLimits {
        name_work: allowance,
        ..Default::default()
    });
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    load_custom_font(&first_path, &mut records, &mut choices, &mut budget).unwrap();
    let before = record_snapshot(&records, &choices);
    let registrations = (
        budget.temporary_registrations,
        budget.metric_copies,
        budget.shared_registrations,
    );
    for attempt in 1..=2 {
        let spent = budget.usage().name_work;
        let error = load_custom_font(&second_path, &mut records, &mut choices, &mut budget)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::NameWork)
        );
        assert_eq!(record_snapshot(&records, &choices), before);
        assert!(budget.usage().name_work > spent);
        assert_eq!(budget.name_record_visits, first_work.name_record_visits);
        assert_eq!(
            budget.usage().read_work,
            first_data.len() + second_data.len() * attempt
        );
        assert_eq!(
            (
                budget.temporary_registrations,
                budget.metric_copies,
                budget.shared_registrations
            ),
            registrations
        );
        assert!(!shared_has_family(second_family));
    }
    assert_eq!(registrations, (1, 1, 1));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].family, first_family);
    assert_eq!(records[0].metrics.advance('A', 1000.0), 500.0);
    assert!(shared_has_family(first_family));
}

#[test]
fn name_work_overflow_and_failed_headers_do_not_refund_prior_work() {
    initialize_font_test_platform();
    let family = "Budget Failed Name Header Work";
    let mut data = fixture::work_bytes(family, 8);
    let name_tag = owned_ttf_parser::Tag::from_bytes(b"name");
    let name_offset = owned_ttf_parser::RawFace::parse(&data, 0)
        .unwrap()
        .table_records
        .into_iter()
        .find(|record| record.tag == name_tag)
        .unwrap()
        .offset as usize;
    data[name_offset + 2..name_offset + 4].copy_from_slice(&u16::MAX.to_be_bytes());
    owned_ttf_parser::Face::parse(&data, 0).unwrap();
    let bytes = Arc::new(data);
    let mut budget = FontBudget::default();
    let error = register_font_data(bytes.clone(), &mut budget)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("The font name records are incomplete")
    );
    let before = budget.usage();
    assert!(before.name_work > 0);
    assert_eq!(before.read_work, bytes.len());
    assert_eq!(budget.name_root_headers, 1);
    assert_eq!(budget.name_table_headers, 1);
    assert_eq!(budget.name_record_visits, 0);
    assert_eq!(
        budget.reserve_name_work(usize::MAX),
        Err(FontLimitError::NameWork)
    );
    assert_eq!(budget.usage(), before);
    assert!(register_font_data(bytes.clone(), &mut budget).is_err());
    assert_eq!(budget.usage().name_work, before.name_work * 2);
    assert_eq!(budget.usage().read_work, bytes.len() * 2);
    assert_eq!(budget.name_record_visits, 0);
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert_no_font_registration(&budget, family);
}
