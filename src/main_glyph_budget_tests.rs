//! Small legal outline producers and admission before registration or copies.

use super::*;

fn set_word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn set_dword(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, bytes| {
        let mut word = [0; 4];
        word[..bytes.len()].copy_from_slice(bytes);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn rectangle() -> Vec<u8> {
    let mut glyph = vec![0; 34];
    set_word(&mut glyph, 0, 1);
    set_word(&mut glyph, 6, 100);
    set_word(&mut glyph, 8, 100);
    set_word(&mut glyph, 10, 3);
    glyph[14..18].fill(1);
    for (index, delta) in [0i16, 100, 0, -100].into_iter().enumerate() {
        set_word(&mut glyph, 18 + index * 2, delta as u16);
    }
    for (index, delta) in [0i16, 0, 100, 0].into_iter().enumerate() {
        set_word(&mut glyph, 26 + index * 2, delta as u16);
    }
    glyph
}

fn compound(children: &[u16]) -> Vec<u8> {
    assert!(!children.is_empty() && children.len() <= 4);
    let mut glyph = vec![0; 10];
    set_word(&mut glyph, 0, u16::MAX);
    for (index, child) in children.iter().enumerate() {
        let flags: u16 = 3 | if index + 1 < children.len() { 32 } else { 0 };
        glyph.extend_from_slice(&flags.to_be_bytes());
        glyph.extend_from_slice(&child.to_be_bytes());
        glyph.extend_from_slice(&[0; 4]);
    }
    glyph
}

fn rebuild(
    base: &[u8],
    mut replacements: BTreeMap<[u8; 4], Vec<u8>>,
    removed: &[[u8; 4]],
) -> Vec<u8> {
    let raw = owned_ttf_parser::RawFace::parse(base, 0).unwrap();
    for record in raw.table_records {
        let tag = record.tag.to_bytes();
        if removed.contains(&tag) || replacements.contains_key(&tag) {
            continue;
        }
        let start = record.offset as usize;
        replacements.insert(tag, base[start..start + record.length as usize].to_vec());
    }
    let count = replacements.len();
    let mut bytes = vec![0; 12 + count * 16];
    let cff = replacements.contains_key(b"CFF ") || replacements.contains_key(b"CFF2");
    bytes[..4].copy_from_slice(if cff { b"OTTO" } else { &[0, 1, 0, 0] });
    set_word(&mut bytes, 4, count as u16);
    let selector = usize::BITS - 1 - count.leading_zeros();
    let search = (1usize << selector) * 16;
    set_word(&mut bytes, 6, search as u16);
    set_word(&mut bytes, 8, selector as u16);
    set_word(&mut bytes, 10, (count * 16 - search) as u16);
    let mut head_start = 0;
    for (index, (tag, mut data)) in replacements.into_iter().enumerate() {
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let start = bytes.len();
        if tag == *b"head" {
            head_start = start;
            set_dword(&mut data, 8, 0);
        }
        let record = 12 + index * 16;
        bytes[record..record + 4].copy_from_slice(&tag);
        set_dword(&mut bytes, record + 4, checksum(&data));
        set_dword(&mut bytes, record + 8, start as u32);
        set_dword(&mut bytes, record + 12, data.len() as u32);
        bytes.extend(data);
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&bytes));
    set_dword(&mut bytes, head_start + 8, adjustment);
    assert!(bytes.len() < 8192, "Outline fixtures must stay small");
    bytes
}

fn glyf_font(family: &str, glyphs: &[Vec<u8>], extra: BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    assert!((4..=40).contains(&glyphs.len()));
    let base = super::fixture::variable_bytes(family, 1);
    let raw = owned_ttf_parser::RawFace::parse(&base, 0).unwrap();
    let mut tables = extra;
    let mut head = raw
        .table(owned_ttf_parser::Tag::from_bytes(b"head"))
        .unwrap()
        .to_vec();
    set_word(&mut head, 50, 1);
    tables.insert(*b"head", head);
    let mut maxp = vec![0; 32];
    set_dword(&mut maxp, 0, 0x0001_0000);
    set_word(&mut maxp, 4, glyphs.len() as u16);
    set_word(&mut maxp, 6, 4);
    set_word(&mut maxp, 8, 1);
    set_word(&mut maxp, 10, 65531);
    set_word(&mut maxp, 12, 16383);
    set_word(&mut maxp, 14, 1);
    set_word(&mut maxp, 28, 4);
    set_word(&mut maxp, 30, 33);
    tables.insert(*b"maxp", maxp);
    let mut hhea = raw
        .table(owned_ttf_parser::Tag::from_bytes(b"hhea"))
        .unwrap()
        .to_vec();
    set_word(&mut hhea, 34, glyphs.len() as u16);
    tables.insert(*b"hhea", hhea);
    let mut hmtx = vec![0; glyphs.len() * 4];
    for glyph in 0..glyphs.len() {
        set_word(&mut hmtx, glyph * 4, 500);
    }
    tables.insert(*b"hmtx", hmtx);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend(glyph);
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"loca", loca);
    rebuild(&base, tables, &[])
}

fn dag(family: &str) -> Vec<u8> {
    glyf_font(
        family,
        &[
            vec![0; 12],
            rectangle(),
            compound(&[1, 1]),
            compound(&[2, 2]),
        ],
        BTreeMap::new(),
    )
}

fn assert_before_publication(budget: &FontBudget, family: &str) {
    assert_eq!(budget.metric_copies, 0);
    assert_eq!(budget.temporary_registrations, 0);
    assert_eq!(budget.shared_registrations, 0);
    assert!(!super::shared_has_family(family));
}

#[test]
fn a_small_compound_dag_counts_repeated_refs_without_expanding_a_point_buffer() {
    let bytes = dag("Glyph counted DAG");
    assert_eq!(
        owned_ttf_parser::Face::parse(&bytes, 0)
            .unwrap()
            .number_of_glyphs(),
        4
    );
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_graph_nodes, 4);
    assert_eq!(budget.glyph_component_records, 4);
    assert_eq!(budget.usage().glyph_visits, 12);
    assert_eq!(budget.usage().glyph_points, 44);
    assert_eq!(budget.usage().glyph_memory, 256);
    assert_eq!(budget.glyph_state_allocations, 1);
    assert_eq!(budget.metric_copies, 0);
}

#[test]
fn compound_point_and_visit_one_less_limits_reject_before_fontique_or_metrics() {
    super::initialize_font_test_platform();
    let family = "Glyph point and visit gate";
    let bytes = dag(family);
    for limits in [
        FontLimits {
            per_glyph_points: 19,
            ..FontLimits::default()
        },
        FontLimits {
            per_glyph_visits: 6,
            ..FontLimits::default()
        },
    ] {
        let mut budget = FontBudget::new(limits);
        let error = prepare_font(Arc::new(bytes.clone()), &mut budget)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphComplexity)
        );
        assert_before_publication(&budget, family);
        assert!(budget.glyph_component_records > 0);
    }
    let mut budget = FontBudget::new(FontLimits {
        per_glyph_points: 20,
        per_glyph_visits: 7,
        ..FontLimits::default()
    });
    let prepared = prepare_font(Arc::new(bytes), &mut budget).unwrap();
    assert_eq!(prepared.faces.len(), 1);
    assert_eq!(budget.metric_copies, 1);
    assert_eq!(budget.shared_registrations, 0);
}

#[test]
fn glyf_temporary_memory_exact_boundary_precedes_registry_and_metric_copy() {
    super::initialize_font_test_platform();
    let family = "Glyph temporary memory gate";
    let bytes = dag(family);
    let mut denied = FontBudget::new(FontLimits {
        per_glyph_memory: 255,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphMemory)
    );
    assert_before_publication(&denied, family);
    let mut admitted = FontBudget::new(FontLimits {
        per_glyph_memory: 256,
        ..FontLimits::default()
    });
    let font = prepare_font(Arc::new(bytes), &mut admitted).unwrap();
    assert_eq!(admitted.usage().glyph_memory, 256);
    assert_eq!(admitted.metric_copies, 1);
    publish_font(&font, &mut admitted);
    assert_eq!(admitted.shared_registrations, 1);
    assert!(super::shared_has_family(family));
}

#[test]
fn zero_output_component_refs_spend_visits_and_present_simple_scratch() {
    super::initialize_font_test_platform();
    let family = "Glyph zero output graph";
    let bytes = glyf_font(
        family,
        &[
            vec![0; 12],
            compound(&[0, 0]),
            compound(&[1, 1]),
            compound(&[2, 2]),
        ],
        BTreeMap::new(),
    );
    let mut budget = FontBudget::new(FontLimits {
        per_glyph_visits: 14,
        per_glyph_points: 4,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut budget)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphComplexity)
    );
    assert_before_publication(&budget, family);
    let mut admitted = FontBudget::new(FontLimits {
        per_glyph_visits: 15,
        per_glyph_points: 4,
        ..FontLimits::default()
    });
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut admitted).unwrap();
    assert_eq!(admitted.usage().glyph_visits, 26);
    assert_eq!(admitted.usage().glyph_points, 16);
    assert_eq!(admitted.usage().glyph_memory, 72);
}

#[test]
fn glyph_header_and_state_admission_happen_before_any_graph_cache_allocation() {
    let bytes = dag("Glyph work and cache gate");
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    let state_bytes = measured.usage().glyph_state_bytes;
    let mut no_header = FontBudget::new(FontLimits {
        glyph_work: 31,
        ..FontLimits::default()
    });
    assert_eq!(
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut no_header),
        Err(FontLimitError::GlyphWork)
    );
    assert_eq!(no_header.glyph_state_allocations, 0);
    assert_eq!(no_header.glyph_graph_nodes, 0);
    let mut denied = FontBudget::new(FontLimits {
        glyph_state_bytes: state_bytes - 1,
        ..FontLimits::default()
    });
    assert_eq!(
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut denied),
        Err(FontLimitError::GlyphMemory)
    );
    assert_eq!(denied.glyph_state_allocations, 0);
    assert_eq!(denied.glyph_graph_nodes, 0);
    let mut exact = FontBudget::new(FontLimits {
        glyph_state_bytes: state_bytes,
        ..FontLimits::default()
    });
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut exact).unwrap();
    assert_eq!(exact.glyph_state_allocations, 1);
}

#[test]
fn cyclic_deep_and_truncated_compounds_keep_old_choices_and_successful_fees() {
    super::initialize_font_test_platform();
    let directory = tempfile::tempdir().unwrap();
    let old_path = directory.path().join("old.ttf");
    fs::write(&old_path, dag("Glyph previous choice")).unwrap();
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    let mut budget = FontBudget::default();
    load_custom_font(&old_path, &mut records, &mut choices, &mut budget).unwrap();
    let before = super::record_snapshot(&records, &choices);
    let registrations = budget.shared_registrations;
    let mut deep = vec![vec![0; 12], rectangle(), vec![0; 12], vec![0; 12]];
    let mut previous = 1;
    for _ in 0..33 {
        deep.push(compound(&[previous]));
        previous = (deep.len() - 1) as u16;
    }
    let mut short = compound(&[1]);
    short.truncate(13);
    for (index, glyphs) in [
        vec![vec![0; 12], rectangle(), compound(&[3]), compound(&[2])],
        deep,
        vec![vec![0; 12], rectangle(), short, vec![0; 12]],
    ]
    .into_iter()
    .enumerate()
    {
        let family = format!("Glyph rejected graph {index}");
        let path = directory.path().join(format!("rejected-{index}.ttf"));
        fs::write(&path, glyf_font(&family, &glyphs, BTreeMap::new())).unwrap();
        let usage = budget.usage();
        let error = load_custom_font(&path, &mut records, &mut choices, &mut budget).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<FontLimitError>(),
            Some(FontLimitError::GlyphProgram | FontLimitError::GlyphComplexity)
        ));
        assert_eq!(super::record_snapshot(&records, &choices), before);
        assert_eq!(budget.shared_registrations, registrations);
        assert!(budget.usage().glyph_work > usage.glyph_work);
        assert!(budget.usage().read_work > usage.read_work);
        assert!(!super::shared_has_family(&family));
    }
    deep = vec![vec![0; 12], rectangle(), vec![0; 12], vec![0; 12]];
    previous = 1;
    for _ in 0..32 {
        deep.push(compound(&[previous]));
        previous = (deep.len() - 1) as u16;
    }
    glyph_budget::admit_font_glyphs(
        &glyf_font("Glyph depth thirty two", &deep, BTreeMap::new()),
        0,
        &mut budget,
    )
    .unwrap();
}

#[test]
fn all_collection_faces_and_failed_attempts_share_one_glyph_work_allowance() {
    super::initialize_font_test_platform();
    let family = "Glyph collection work gate";
    let bytes = dag(family);
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    let work = measured.usage().glyph_work;
    let collection = super::collection(&[bytes.clone(), bytes]);
    let mut budget = FontBudget::new(FontLimits {
        glyph_work: work * 2 - 1,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(collection), &mut budget)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphWork)
    );
    assert_before_publication(&budget, family);
    assert!(budget.usage().glyph_work >= work);
    let paid = budget.usage().glyph_work;
    assert_eq!(
        budget.reserve_glyph_work(usize::MAX),
        Err(FontLimitError::GlyphWork)
    );
    assert_eq!(budget.usage().glyph_work, paid);
}

fn cff_index(programs: &[Vec<u8>], cff2: bool) -> Vec<u8> {
    let mut bytes = if cff2 {
        (programs.len() as u32).to_be_bytes().to_vec()
    } else {
        (programs.len() as u16).to_be_bytes().to_vec()
    };
    if programs.is_empty() {
        return bytes;
    }
    bytes.push(2);
    let mut offset = 1u16;
    bytes.extend_from_slice(&offset.to_be_bytes());
    for program in programs {
        offset = offset.checked_add(program.len() as u16).unwrap();
        bytes.extend_from_slice(&offset.to_be_bytes());
    }
    for program in programs {
        bytes.extend(program);
    }
    bytes
}

fn dict_integer(bytes: &mut Vec<u8>, value: usize) {
    bytes.push(29);
    bytes.extend_from_slice(&(value as i32).to_be_bytes());
}

fn private_dict() -> Vec<u8> {
    private_dict_extra(&[])
}

fn private_dict_extra(extra: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    dict_integer(&mut bytes, 6 + extra.len());
    bytes.push(19);
    bytes.extend_from_slice(extra);
    bytes
}

fn cff_table(programs: &[Vec<u8>], locals: &[Vec<Vec<u8>>], cff2: bool, cid: bool) -> Vec<u8> {
    cff_table_extra(programs, locals, cff2, cid, &[])
}

fn cff_table_extra(
    programs: &[Vec<u8>],
    locals: &[Vec<Vec<u8>>],
    cff2: bool,
    cid: bool,
    extra: &[u8],
) -> Vec<u8> {
    cff_table_layout(programs, locals, cff2, cid, extra, 0)
}

fn cff_table_layout(
    programs: &[Vec<u8>],
    locals: &[Vec<Vec<u8>>],
    cff2: bool,
    cid: bool,
    extra: &[u8],
    select_format: u8,
) -> Vec<u8> {
    cff_table_dict_payloads(programs, locals, cff2, cid, extra, select_format, &[], &[])
}

fn cff_table_dict_payloads(
    programs: &[Vec<u8>],
    locals: &[Vec<Vec<u8>>],
    cff2: bool,
    cid: bool,
    extra: &[u8],
    select_format: u8,
    fd_extra: &[u8],
    private_extra: &[u8],
) -> Vec<u8> {
    assert_eq!(programs.len(), 4);
    assert!(!locals.is_empty() && locals.len() <= 2);
    let names = cff_index(&[b"OutlineBudgetFixture".to_vec()], false);
    let strings = cff_index(&[b"Adobe".to_vec(), b"Identity".to_vec()], false);
    let globals = cff_index(&[], cff2);
    let use_fd_array = cff2 || cid;
    let use_select = use_fd_array && (cid || locals.len() > 1);
    let top_len = 6
        + if cff2 { 0 } else { 6 }
        + if cid && !cff2 { 17 } else { 0 }
        + if use_fd_array { 7 } else { 11 }
        + if use_select { 7 } else { 0 }
        + extra.len();
    let prefix = if cff2 {
        5 + top_len + globals.len()
    } else {
        4 + names.len()
            + cff_index(&[vec![0; top_len]], false).len()
            + strings.len()
            + globals.len()
    };
    let charset_offset = prefix;
    let charset = if cff2 {
        vec![]
    } else if cid {
        vec![0, 0, 1, 0, 2, 0, 3]
    } else {
        vec![0, 0, 34, 0, 35, 0, 36]
    };
    let charstrings_offset = charset_offset + charset.len();
    let charstrings = cff_index(programs, cff2);
    let mut after = charstrings_offset + charstrings.len();
    let fd_array_offset = after;
    let fd_length = if use_fd_array {
        cff_index(&vec![vec![0; 11 + fd_extra.len()]; locals.len()], cff2).len()
    } else {
        0
    };
    after += fd_length;
    let select_offset = after;
    let select = if use_select {
        let other = if locals.len() > 1 { 1 } else { 0 };
        match select_format {
            0 => vec![0, 0, 0, other, other],
            3 => vec![3, 0, 2, 0, 0, 0, 0, 2, other, 0, 4],
            4 => vec![
                4, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, other, 0, 0, 0, 4,
            ],
            _ => panic!("Unsupported fixture FDSelect format"),
        }
    } else {
        vec![]
    };
    after += select.len();
    let mut private_offsets = Vec::new();
    let mut private_payloads = Vec::new();
    for local in locals {
        private_offsets.push(after);
        let mut bytes = if private_extra.is_empty() {
            private_dict()
        } else {
            private_dict_extra(private_extra)
        };
        bytes.extend(cff_index(local, cff2));
        after += bytes.len();
        private_payloads.push(bytes);
    }
    let mut top = Vec::new();
    dict_integer(&mut top, charstrings_offset);
    top.push(17);
    if !cff2 {
        dict_integer(&mut top, charset_offset);
        top.push(15);
    }
    if cid && !cff2 {
        dict_integer(&mut top, 391);
        dict_integer(&mut top, 392);
        dict_integer(&mut top, 0);
        top.extend([12, 30]);
    }
    if use_fd_array {
        dict_integer(&mut top, fd_array_offset);
        top.extend([12, 36]);
    } else {
        dict_integer(&mut top, 6 + private_extra.len());
        dict_integer(&mut top, private_offsets[0]);
        top.push(18);
    }
    if use_select {
        dict_integer(&mut top, select_offset);
        top.extend([12, 37]);
    }
    top.extend_from_slice(extra);
    assert_eq!(top.len(), top_len);
    let mut bytes = if cff2 {
        let mut bytes = vec![2, 0, 5];
        bytes.extend_from_slice(&(top.len() as u16).to_be_bytes());
        bytes.extend(top);
        bytes.extend(globals);
        bytes
    } else {
        let mut bytes = vec![1, 0, 4, 4];
        bytes.extend(names);
        bytes.extend(cff_index(&[top], false));
        bytes.extend(strings);
        bytes.extend(globals);
        bytes
    };
    assert_eq!(bytes.len(), charset_offset);
    bytes.extend(charset);
    bytes.extend(charstrings);
    if use_fd_array {
        let dictionaries = private_offsets
            .iter()
            .map(|&offset| {
                let mut bytes = Vec::new();
                dict_integer(&mut bytes, 6 + private_extra.len());
                dict_integer(&mut bytes, offset);
                bytes.push(18);
                bytes.extend_from_slice(fd_extra);
                bytes
            })
            .collect::<Vec<_>>();
        bytes.extend(cff_index(&dictionaries, cff2));
        bytes.extend(select);
    }
    for private in private_payloads {
        bytes.extend(private);
    }
    assert_eq!(bytes.len(), after);
    bytes
}

fn cff_font(family: &str, table: Vec<u8>, cff2: bool) -> Vec<u8> {
    let base = super::fixture::bytes(family, 400, 500);
    let mut maxp = vec![0; 6];
    set_dword(&mut maxp, 0, 0x0000_5000);
    set_word(&mut maxp, 4, 4);
    rebuild(
        &base,
        BTreeMap::from([
            (if cff2 { *b"CFF2" } else { *b"CFF " }, table),
            (*b"maxp", maxp),
        ]),
        &[*b"glyf", *b"loca", *b"kern"],
    )
}

fn line_program(endchar: bool) -> Vec<u8> {
    let mut program = vec![139, 139, 21, 189, 139, 5];
    if endchar {
        program.push(14);
    }
    program
}

#[test]
fn cff_local_subroutines_preserve_the_real_fd_and_argument_context() {
    let root = vec![32, 10, 14];
    let first = vec![139, 139, 21, 189, 139, 5, 11];
    let second = vec![139, 139, 21, 189, 139, 139, 189, 89, 139, 8, 11];
    let table = cff_table(
        &[vec![14], root.clone(), root.clone(), root],
        &[vec![first], vec![second]],
        false,
        true,
    );
    let bytes = cff_font("Glyph CFF FD context", table, false);
    assert!(owned_ttf_parser::Face::parse(&bytes, 0).is_ok());
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_subroutine_calls, 3);
    assert_eq!(budget.usage().glyph_visits, 7);
    assert_eq!(budget.usage().glyph_points, 26);
}

#[test]
fn cff_child_endchar_cannot_skip_the_callers_following_real_geometry() {
    super::initialize_font_test_platform();
    let family = "Glyph CFF frame endchar";
    let root = [vec![32, 10], line_program(true)].concat();
    let bytes = cff_font(
        family,
        cff_table(
            &[vec![14], root, vec![14], vec![14]],
            &[vec![vec![14]]],
            false,
            false,
        ),
        false,
    );
    let mut denied = FontBudget::new(FontLimits {
        per_glyph_points: 5,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphComplexity)
    );
    assert_before_publication(&denied, family);
    assert_eq!(denied.glyph_subroutine_calls, 1);
    let mut admitted = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut admitted).unwrap();
    assert_eq!(admitted.usage().glyph_points, 18);
}

#[test]
fn cff_fdselect_ranges_3_and_4_preserve_small_local_program_selection() {
    for (format, cff2) in [(3, false), (4, true)] {
        let root = if cff2 { vec![32, 10] } else { vec![32, 10, 14] };
        let mut first = vec![139, 139, 21, 189, 139, 5];
        let mut second = vec![139, 139, 21, 189, 139, 139, 189, 89, 139, 8];
        if !cff2 {
            first.push(11);
            second.push(11);
        }
        let empty = if cff2 { vec![] } else { vec![14] };
        let table = cff_table_layout(
            &[empty, root.clone(), root.clone(), root],
            &[vec![first], vec![second]],
            cff2,
            true,
            &[],
            format,
        );
        let bytes = cff_font(&format!("Glyph CFF FDSelect {format}"), table, cff2);
        let mut budget = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
        assert_eq!(budget.glyph_subroutine_calls, 3);
        assert_eq!(budget.usage().glyph_points, 26);
    }
}

#[test]
fn cff_implicit_endchar_and_unsupported_generation_refuse_before_publication() {
    super::initialize_font_test_platform();
    for (index, program) in [
        vec![139, 139, 204, 205],
        vec![139, 139, 204, 205, 14],
        vec![139, 139, 12, 16, 14],
    ]
    .into_iter()
    .enumerate()
    {
        let family = format!("Glyph CFF unsupported {index}");
        let bytes = cff_font(
            &family,
            cff_table(
                &[
                    line_program(true),
                    line_program(true),
                    line_program(true),
                    program,
                ],
                &[vec![]],
                false,
                false,
            ),
            false,
        );
        let mut budget = FontBudget::default();
        let error = prepare_font(Arc::new(bytes), &mut budget).err().unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphFormat)
        );
        assert_before_publication(&budget, &family);
    }
}

#[test]
fn cff2_fixed_programs_are_counted_and_unproved_blend_is_typed() {
    let table = cff_table(
        &[
            line_program(false),
            line_program(false),
            line_program(false),
            line_program(false),
        ],
        &[vec![]],
        true,
        false,
    );
    let bytes = cff_font("Glyph CFF2 fixed program", table, true);
    assert!(owned_ttf_parser::Face::parse(&bytes, 0).is_ok());
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.usage().glyph_points, 24);
    let bytes = cff_font(
        "Glyph CFF2 unproved blend",
        cff_table(
            &[vec![139, 16], vec![], vec![], vec![]],
            &[vec![]],
            true,
            false,
        ),
        true,
    );
    assert_eq!(
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut FontBudget::default()),
        Err(FontLimitError::GlyphFormat)
    );
}

#[test]
fn a_varc_source_is_explicitly_refused_instead_of_admitting_only_its_base_glyf() {
    super::initialize_font_test_platform();
    let family = "Glyph VARC boundary";
    let bytes = glyf_font(
        family,
        &[vec![0; 12], rectangle(), rectangle(), rectangle()],
        BTreeMap::from([(*b"VARC", vec![0; 24])]),
    );
    let mut budget = FontBudget::default();
    let error = prepare_font(Arc::new(bytes), &mut budget).err().unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphFormat)
    );
    assert_before_publication(&budget, family);
    assert_eq!(budget.glyph_state_allocations, 0);
}

#[test]
fn cff_child_width_observation_keeps_the_callers_operand_and_mask_state() {
    let bytes = cff_font(
        "Glyph CFF retained width operand",
        cff_table(
            &[vec![14], vec![32, 10, 10, 14], vec![14], vec![14]],
            &[vec![vec![32, 14]]],
            false,
            false,
        ),
        false,
    );
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_subroutine_calls, 2);
    assert_eq!(budget.usage().glyph_points, 16);
    let local = vec![149, 159, 3, 19, 0xc0, 11];
    let root = [vec![149, 159, 1, 32, 10], line_program(true)].concat();
    let bytes = cff_font(
        "Glyph CFF shared mask state",
        cff_table(
            &[vec![14], root, vec![14], vec![14]],
            &[vec![local]],
            false,
            false,
        ),
        false,
    );
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_subroutine_calls, 1);
    assert_eq!(budget.usage().glyph_points, 18);
}

#[test]
fn cff_binary_calls_and_more_than_48_operands_are_bounded_before_publication() {
    super::initialize_font_test_platform();
    let family = "Glyph CFF small binary calls";
    let mut subrs = vec![vec![139, 139, 21, 189, 139, 5, 11]];
    for index in 1..6 {
        let child = 32 + index as u8 - 1;
        subrs.push(vec![child, 10, child, 10, 11]);
    }
    let bytes = cff_font(
        family,
        cff_table(
            &[vec![14], vec![37, 10, 14], vec![14], vec![14]],
            &[subrs],
            false,
            false,
        ),
        false,
    );
    let mut denied = FontBudget::new(FontLimits {
        per_glyph_visits: 63,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphComplexity)
    );
    assert_before_publication(&denied, family);
    let mut admitted = FontBudget::new(FontLimits {
        per_glyph_visits: 64,
        ..FontLimits::default()
    });
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut admitted).unwrap();
    assert_eq!(admitted.glyph_subroutine_calls, 63);
    let mut curve = vec![139; 60];
    curve.extend([8, 14]);
    let bytes = cff_font(
        "Glyph CFF sixty operands",
        cff_table(
            &[vec![14], curve, vec![14], vec![14]],
            &[vec![]],
            false,
            false,
        ),
        false,
    );
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
    assert_eq!(budget.usage().glyph_points, 47);
}

#[test]
fn cff_stack_513_exact_boundary_and_one_more_refuse_before_any_font_copy() {
    super::initialize_font_test_platform();
    let program = |count| {
        let mut bytes = vec![139, 139, 21];
        bytes.extend(vec![139; count]);
        bytes.push(6);
        bytes
    };
    let bytes = cff_font(
        "Glyph CFF2 exact stack",
        cff_table(
            &[vec![], program(513), vec![], vec![]],
            &[vec![]],
            true,
            false,
        ),
        true,
    );
    assert!(owned_ttf_parser::Face::parse(&bytes, 0).is_ok());
    let mut admitted = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut admitted).unwrap();
    assert_eq!(admitted.usage().glyph_points, 530);
    let family = "Glyph CFF2 refused stack";
    let bytes = cff_font(
        family,
        cff_table(
            &[vec![], program(514), vec![], vec![]],
            &[vec![]],
            true,
            false,
        ),
        true,
    );
    let mut denied = FontBudget::default();
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphComplexity)
    );
    assert_before_publication(&denied, family);
    let directory = tempfile::tempdir().unwrap();
    let old_path = directory.path().join("previous.ttf");
    fs::write(&old_path, dag("Glyph previous stack choice")).unwrap();
    let mut records = Vec::new();
    let mut choices = BTreeMap::new();
    let mut budget = FontBudget::default();
    load_custom_font(&old_path, &mut records, &mut choices, &mut budget).unwrap();
    let previous = super::record_snapshot(&records, &choices);
    let counters = (
        budget.metric_copies,
        budget.temporary_registrations,
        budget.shared_registrations,
    );
    let path = directory.path().join("refused.otf");
    fs::write(&path, bytes).unwrap();
    let error = load_custom_font(&path, &mut records, &mut choices, &mut budget).unwrap_err();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphComplexity)
    );
    assert_eq!(super::record_snapshot(&records, &choices), previous);
    assert_eq!(
        (
            budget.metric_copies,
            budget.temporary_registrations,
            budget.shared_registrations
        ),
        counters
    );
}

fn matrix_prefix() -> Vec<u8> {
    let mut bytes = vec![30, 0x0a, 0x00, 0x10];
    bytes.extend([0; 80]);
    bytes.push(0x0f);
    bytes.extend([139, 139, 30, 0x0a, 0x00, 0x1f, 139, 139, 12, 7]);
    bytes
}

#[test]
fn cff_repeated_matrix_prefixes_pay_each_glyph_projection_before_registration() {
    super::initialize_font_test_platform();
    let family = "Glyph CFF matrix work gate";
    let extra = matrix_prefix().repeat(3);
    let bytes = cff_font(
        family,
        cff_table_extra(
            &[
                line_program(true),
                line_program(true),
                line_program(true),
                line_program(true),
            ],
            &[vec![]],
            false,
            false,
            &extra,
        ),
        false,
    );
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    let work = measured.usage().glyph_work;
    // Three matrices re-read the same 93-byte six-number prefix. Each glyph
    // still scans the whole 308-byte Top DICT and six-byte Private DICT.
    assert_eq!(
        measured.glyph_work_usage.dict_projection,
        4 * (308 + 3 * 93 + 3 * 64 + 6 + 64)
    );
    assert_eq!(measured.glyph_work_usage.dict_parse, 308 + 3 * 93 + 6);
    assert_eq!(measured.glyph_work_usage.total(), work);
    let mut denied = FontBudget::new(FontLimits {
        glyph_work: work - 1,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphWork)
    );
    assert_before_publication(&denied, family);
    let mut exact = FontBudget::new(FontLimits {
        glyph_work: work,
        ..FontLimits::default()
    });
    prepare_font(Arc::new(bytes), &mut exact).unwrap();
    assert_eq!(exact.metric_copies, 1);
    assert_eq!(exact.shared_registrations, 0);
}

fn gvar_table(glyph: usize, body: &[u8], private_points: bool) -> Vec<u8> {
    let mut variation = vec![0; 10];
    set_word(&mut variation, 0, 1);
    set_word(&mut variation, 2, 10);
    set_word(&mut variation, 4, body.len() as u16);
    set_word(
        &mut variation,
        6,
        0x8000 | if private_points { 0x2000 } else { 0 },
    );
    // A zero embedded peak is active at the default variation location. The
    // fixture exercises the real work path without requiring an App axis edit.
    set_word(&mut variation, 8, 0);
    variation.extend_from_slice(body);
    if variation.len() % 2 != 0 {
        variation.push(0);
    }
    let mut table = vec![0; 30];
    set_word(&mut table, 0, 1);
    set_word(&mut table, 4, 1);
    set_word(&mut table, 12, 4);
    set_dword(&mut table, 16, 30);
    for index in glyph + 1..=4 {
        set_word(&mut table, 20 + index * 2, (variation.len() / 2) as u16);
    }
    table.extend(variation);
    table
}

#[test]
fn gvar_private_dense_sparse_and_bad_point_ids_keep_the_same_raw_budget() {
    super::initialize_font_test_platform();
    let glyphs = [vec![0; 12], rectangle(), rectangle(), rectangle()];
    for (index, body) in [vec![0, 0x87, 0x87], vec![2, 1, 0, 3, 0x81, 0x81]]
        .into_iter()
        .enumerate()
    {
        let bytes = glyf_font(
            &format!("Glyph gvar valid list {index}"),
            &glyphs,
            BTreeMap::from([(*b"gvar", gvar_table(1, &body, true))]),
        );
        let mut budget = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
        assert!(budget.usage().glyph_work > body.len());
        assert!(budget.usage().glyph_memory >= 136);
    }
    let family = "Glyph gvar invalid sparse point";
    let bytes = glyf_font(
        family,
        &glyphs,
        BTreeMap::from([(*b"gvar", gvar_table(1, &[1, 0, 8, 0x80, 0x80], true))]),
    );
    let mut budget = FontBudget::default();
    let error = prepare_font(Arc::new(bytes), &mut budget).err().unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphProgram)
    );
    assert_before_publication(&budget, family);
}

#[test]
fn gvar_missing_point_i32_runs_pay_zero_output_iteration_before_any_copy() {
    super::initialize_font_test_platform();
    let family = "Glyph gvar missing point work";
    // The I32 payload resembles a long byte run if its four bytes are not
    // consumed. Every component is empty, so this fixture creates no mask.
    let mut body = vec![0xc0, 0x3f, 0, 0, 0];
    body.extend([0xbf; 61]);
    let glyphs = [vec![0; 12], compound(&[0]), vec![0; 12], compound(&[0])];
    let bytes = glyf_font(
        family,
        &glyphs,
        BTreeMap::from([(*b"gvar", gvar_table(3, &body, false))]),
    );
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    let work = measured.usage().glyph_work;
    assert!(
        work > 15_000,
        "Real sparse iteration must not collapse to a byte-run prefix"
    );
    assert_eq!(measured.usage().glyph_points, 16);
    let mut denied = FontBudget::new(FontLimits {
        glyph_work: work - 1,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphWork)
    );
    assert_before_publication(&denied, family);
    let mut exact = FontBudget::new(FontLimits {
        glyph_work: work,
        ..FontLimits::default()
    });
    prepare_font(Arc::new(bytes), &mut exact).unwrap();
    assert_eq!(exact.metric_copies, 1);
    assert_eq!(exact.shared_registrations, 0);
}

fn declared_prefix_font(
    base: &[u8],
    long: bool,
    mapped: u16,
    padding: &[u8],
    glyf_padding: &[u8],
) -> Vec<u8> {
    let raw = owned_ttf_parser::RawFace::parse(base, 0).unwrap();
    let table = |tag: &[u8; 4]| raw.table(owned_ttf_parser::Tag::from_bytes(tag)).unwrap();
    let mut maxp = table(b"maxp").to_vec();
    set_word(&mut maxp, 4, 4);
    let mut head = table(b"head").to_vec();
    set_word(&mut head, 50, if long { 1 } else { 0 });
    let mut hhea = table(b"hhea").to_vec();
    set_word(&mut hhea, 34, 4);
    let hmtx = table(b"hmtx")[..16].to_vec();
    let mut cmap = table(b"cmap").to_vec();
    set_word(&mut cmap, 44, mapped.wrapping_sub(0x0041));
    let mut loca = Vec::new();
    for offset in table(b"loca").chunks_exact(4) {
        let offset = u32::from_be_bytes(offset.try_into().unwrap());
        if long {
            loca.extend_from_slice(&offset.to_be_bytes());
        } else {
            assert_eq!(offset % 2, 0);
            loca.extend_from_slice(&(offset as u16 / 2).to_be_bytes());
        }
    }
    loca.extend_from_slice(padding);
    let mut glyf = table(b"glyf").to_vec();
    glyf.extend_from_slice(glyf_padding);
    rebuild(
        base,
        BTreeMap::from([
            (*b"maxp", maxp),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"cmap", cmap),
            (*b"loca", loca),
            (*b"glyf", glyf),
        ]),
        &[],
    )
}

fn loca_tail_font(family: &str, long: bool, empty: bool) -> Vec<u8> {
    let glyphs = if empty {
        vec![
            vec![0; 12],
            compound(&[0, 0]),
            compound(&[1, 1]),
            compound(&[2, 2]),
            compound(&[3, 3]),
        ]
    } else {
        vec![
            vec![0; 12],
            rectangle(),
            compound(&[1, 1]),
            compound(&[2, 2]),
            compound(&[3, 3]),
        ]
    };
    let bytes = declared_prefix_font(
        &glyf_font(family, &glyphs, BTreeMap::new()),
        long,
        4,
        &[],
        &[],
    );
    let face = owned_ttf_parser::Face::parse(&bytes, 0).unwrap();
    assert_eq!(face.number_of_glyphs(), 4);
    // This checks only the real cmap lookup. The actual renderer's whole-table
    // loca contract is covered by the primary source review, not by this
    // parser's maxp-truncated outline API.
    assert_eq!(face.glyph_index('A'), Some(owned_ttf_parser::GlyphId(4)));
    bytes
}

fn actual_tail_outline(bytes: &[u8], expected_points: usize, expected_memory: usize) {
    use skrifa::MetadataProvider;
    let font = swash::FontRef::from_index(bytes, 0).expect("Read the real small Swash font");
    let glyph = font.charmap().map('A');
    assert_eq!(
        glyph, 4,
        "The real Swash cmap must select the loca tail root"
    );
    let font_ref = skrifa::FontRef::from_index(bytes, 0).expect("Read the locked Skrifa font");
    let outlines = font_ref.outline_glyphs();
    let tail = outlines
        .get(skrifa::GlyphId::from(glyph))
        .expect("The actual Skrifa collection exposes the tail GID");
    assert_eq!(
        tail.draw_memory_size(skrifa::outline::Hinting::None),
        expected_memory
    );
    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context.builder(font).size(16.0).hint(false).build();
    let actual = scaler
        .scale_outline(glyph)
        .expect("Actual Swash/Skrifa expands only this small tail outline");
    assert_eq!(actual.points().len(), expected_points);
}

#[test]
fn cmap_tail_roots_pay_points_memory_and_exact_one_less_before_publication() {
    super::initialize_font_test_platform();
    for long in [false, true] {
        let family = format!("Glyph cmap tail points {long}");
        let bytes = loca_tail_font(&family, long, false);
        // The producer oracle reaches the real locked scaler before admission
        // assertions. This sample has only 32 real points and no raster mask.
        actual_tail_outline(&bytes, 32, 408);
        for points in [20, 35] {
            let mut denied = FontBudget::new(FontLimits {
                per_glyph_points: points,
                ..FontLimits::default()
            });
            let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
                .err()
                .unwrap();
            assert_eq!(
                error.downcast_ref::<FontLimitError>(),
                Some(&FontLimitError::GlyphComplexity)
            );
            assert_before_publication(&denied, &family);
        }
        let mut memory_denied = FontBudget::new(FontLimits {
            per_glyph_memory: 407,
            ..FontLimits::default()
        });
        let error = prepare_font(Arc::new(bytes.clone()), &mut memory_denied)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphMemory)
        );
        assert_before_publication(&memory_denied, &family);
        let mut exact = FontBudget::new(FontLimits {
            per_glyph_points: 36,
            per_glyph_visits: 15,
            per_glyph_memory: 408,
            ..FontLimits::default()
        });
        let font = prepare_font(Arc::new(bytes.clone()), &mut exact).unwrap();
        assert_eq!(exact.glyph_graph_nodes, 5);
        assert_eq!(exact.glyph_component_records, 6);
        assert_eq!(exact.usage().glyph_points, 80);
        assert_eq!(exact.usage().glyph_visits, 27);
        assert_eq!(exact.usage().glyph_memory, 408);
        assert_eq!(exact.metric_copies, 1);
        assert_eq!(exact.shared_registrations, 0);
        publish_font(&font, &mut exact);
        assert_eq!(exact.shared_registrations, 1);
        let directory = tempfile::tempdir().unwrap();
        let old_path = directory.path().join("old.ttf");
        fs::write(&old_path, dag(&format!("Glyph old tail choices {long}"))).unwrap();
        let mut records = Vec::new();
        let mut choices = BTreeMap::new();
        let mut budget = FontBudget::default();
        load_custom_font(&old_path, &mut records, &mut choices, &mut budget).unwrap();
        let previous = super::record_snapshot(&records, &choices);
        let counters = (
            budget.metric_copies,
            budget.temporary_registrations,
            budget.shared_registrations,
        );
        budget.limits.per_glyph_points = 20;
        let path = directory.path().join("tail.ttf");
        fs::write(&path, bytes).unwrap();
        let error = load_custom_font(&path, &mut records, &mut choices, &mut budget).unwrap_err();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphComplexity)
        );
        assert_eq!(super::record_snapshot(&records, &choices), previous);
        assert_eq!(
            (
                budget.metric_copies,
                budget.temporary_registrations,
                budget.shared_registrations
            ),
            counters
        );
    }
}

#[test]
fn cmap_zero_output_tail_refs_spend_visits_beyond_the_maxp_prefix() {
    super::initialize_font_test_platform();
    let family = "Glyph cmap zero output tail";
    let bytes = loca_tail_font(family, true, true);
    actual_tail_outline(&bytes, 0, 72);
    for visits in [15, 30] {
        let mut budget = FontBudget::new(FontLimits {
            per_glyph_visits: visits,
            per_glyph_points: 4,
            ..FontLimits::default()
        });
        let error = prepare_font(Arc::new(bytes.clone()), &mut budget)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphComplexity)
        );
        assert_before_publication(&budget, family);
    }
    let mut exact = FontBudget::new(FontLimits {
        per_glyph_visits: 31,
        per_glyph_points: 4,
        ..FontLimits::default()
    });
    prepare_font(Arc::new(bytes), &mut exact).unwrap();
    assert_eq!(exact.glyph_graph_nodes, 5);
    assert_eq!(exact.usage().glyph_visits, 57);
    assert_eq!(exact.usage().glyph_points, 20);
    assert_eq!(exact.metric_copies, 1);
    assert_eq!(exact.shared_registrations, 0);
}

#[test]
fn whole_loca_tail_state_is_admitted_before_allocating_the_graph_cache() {
    let base = dag("Glyph declared state measure");
    let mut baseline = FontBudget::default();
    glyph_budget::admit_font_glyphs(&base, 0, &mut baseline).unwrap();
    let bytes = loca_tail_font("Glyph whole loca state fee", true, false);
    let mut denied = FontBudget::new(FontLimits {
        glyph_state_bytes: baseline.usage().glyph_state_bytes,
        ..FontLimits::default()
    });
    assert_eq!(
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut denied),
        Err(FontLimitError::GlyphMemory)
    );
    assert_eq!(denied.glyph_state_allocations, 0);
    assert_eq!(denied.glyph_graph_nodes, 0);
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    assert_eq!(
        measured.usage().glyph_state_bytes * 4,
        baseline.usage().glyph_state_bytes * 5
    );
    let mut one_less = FontBudget::new(FontLimits {
        glyph_state_bytes: measured.usage().glyph_state_bytes - 1,
        ..FontLimits::default()
    });
    assert_eq!(
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut one_less),
        Err(FontLimitError::GlyphMemory)
    );
    assert_eq!(one_less.glyph_state_allocations, 0);
    let mut exact = FontBudget::new(FontLimits {
        glyph_state_bytes: measured.usage().glyph_state_bytes,
        ..FontLimits::default()
    });
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut exact).unwrap();
    assert_eq!(exact.glyph_state_allocations, 1);
    assert_eq!(exact.glyph_graph_nodes, 5);
}

#[test]
fn whole_loca_keeps_empty_padding_and_gvar_uses_its_own_declared_count() {
    let base = dag("Glyph harmless loca padding");
    let raw = owned_ttf_parser::RawFace::parse(&base, 0).unwrap();
    let loca = raw
        .table(owned_ttf_parser::Tag::from_bytes(b"loca"))
        .unwrap();
    let end = u32::from_be_bytes(loca[loca.len() - 4..].try_into().unwrap());
    for (padding, glyf_padding) in [
        (vec![0, 0], vec![]),
        ((end as u16 / 2).to_be_bytes().to_vec(), vec![]),
        (((end as u16 + 2) / 2).to_be_bytes().to_vec(), vec![0, 0]),
    ] {
        let bytes = declared_prefix_font(&base, false, 1, &padding, &glyf_padding);
        let mut budget = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
        assert_eq!(budget.glyph_graph_nodes, 5);
        assert_eq!(budget.usage().glyph_points, 48);
    }
    let partial = declared_prefix_font(&base, true, 1, &[0, 0], &[]);
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&partial, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_graph_nodes, 4);
    let mut gvar = vec![0; 30];
    set_word(&mut gvar, 0, 1);
    set_word(&mut gvar, 4, 1);
    set_word(&mut gvar, 12, 4);
    set_dword(&mut gvar, 16, 30);
    let tail = loca_tail_font("Glyph tail with declared gvar", true, false);
    let tail = rebuild(&tail, BTreeMap::from([(*b"gvar", gvar)]), &[]);
    actual_tail_outline(&tail, 32, 680);
    let mut budget = FontBudget::default();
    glyph_budget::admit_font_glyphs(&tail, 0, &mut budget).unwrap();
    assert_eq!(budget.glyph_graph_nodes, 5);
    assert_eq!(budget.usage().glyph_points, 80);
    assert_eq!(budget.usage().glyph_memory, 680);
}

fn actual_cff_line(bytes: &[u8]) {
    let font = swash::FontRef::from_index(bytes, 0).expect("Read the small real CFF producer");
    let glyph = font.charmap().map('A');
    assert_eq!(glyph, 1);
    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context.builder(font).size(16.0).hint(false).build();
    let outline = scaler
        .scale_outline(glyph)
        .expect("The real CFF producer remains drawable");
    assert_eq!(outline.points().len(), 2);
}

#[test]
fn cff_unused_index_payload_spans_pay_constant_bounds_work_without_token_scan() {
    let mut usages = Vec::new();
    for count in [4, 512] {
        let mut unused = vec![139; count];
        unused.extend([6, 11]);
        let bytes = cff_font(
            &format!("Glyph CFF unused span {count}"),
            cff_table(
                &[
                    line_program(true),
                    line_program(true),
                    line_program(true),
                    line_program(true),
                ],
                &[vec![unused]],
                false,
                false,
            ),
            false,
        );
        actual_cff_line(&bytes);
        let mut budget = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
        assert_eq!(budget.glyph_subroutine_calls, 0);
        assert_eq!(budget.usage().glyph_points, 24);
        assert_eq!(budget.glyph_work_usage.total(), budget.usage().glyph_work);
        usages.push(budget.glyph_work_usage);
    }
    assert_eq!(
        usages[0], usages[1],
        "Unvisited subroutine bytes are not a program scan"
    );
    assert_eq!(usages[0].index_span, 5 * 4);
    assert!(usages[0].index_read > usages[0].index_span);
    assert!(usages[0].program > 0 && usages[0].operand > 0 && usages[0].output > 0);
}

#[test]
fn cff_top_fd_and_private_matrix_prefixes_keep_whole_scan_and_runtime_projection() {
    super::initialize_font_test_platform();
    let family = "Glyph CFF three DICT fee scopes";
    let extra = matrix_prefix();
    assert_eq!(extra.len(), 95);
    // Top already supplies the global 0.001 font scale. Keep the FD matrix
    // local and identity so concatenation does not quantize the line away.
    // Its encoded length and six-number prefix remain identical for fee checks.
    let mut fd_extra = vec![30, 0x1a, 0, 0];
    fd_extra.extend([0; 80]);
    fd_extra.extend([0x0f, 139, 139, 30, 0x1a, 0, 0x0f, 139, 139, 12, 7]);
    assert_eq!(fd_extra.len(), 95);
    let bytes = cff_font(
        family,
        cff_table_dict_payloads(
            &[
                line_program(true),
                line_program(true),
                line_program(true),
                line_program(true),
            ],
            &[vec![], vec![]],
            false,
            true,
            &extra,
            0,
            &fd_extra,
            &extra,
        ),
        false,
    );
    actual_cff_line(&bytes);
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
    // Top is scanned once; selected FD/Private descriptors twice. The real
    // renderer still scans all three on each of the four glyph misses.
    assert_eq!(
        measured.glyph_work_usage.dict_parse,
        (138 + 93) + 2 * (106 + 93) + 2 * (101 + 93)
    );
    assert_eq!(
        measured.glyph_work_usage.dict_projection,
        4 * ((138 + 93 + 64) + (106 + 93 + 64) + (101 + 93 + 64) + 64)
    );
    let work = measured.usage().glyph_work;
    assert_eq!(measured.glyph_work_usage.total(), work);
    let mut denied = FontBudget::new(FontLimits {
        glyph_work: work - 1,
        ..FontLimits::default()
    });
    let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<FontLimitError>(),
        Some(&FontLimitError::GlyphWork)
    );
    assert_before_publication(&denied, family);
    assert_eq!(denied.glyph_work_usage.total(), denied.usage().glyph_work);
    let mut exact = FontBudget::new(FontLimits {
        glyph_work: work,
        ..FontLimits::default()
    });
    prepare_font(Arc::new(bytes), &mut exact).unwrap();
    assert_eq!(exact.metric_copies, 1);
    assert_eq!(exact.shared_registrations, 0);
}

#[test]
fn cff_matrix_reparse_cursor_tracks_known_fields_and_ignores_unknown_opcodes() {
    let short_matrix = vec![
        30, 0x0a, 0, 0x1f, 139, 139, 30, 0x0a, 0, 0x1f, 139, 139, 12, 7,
    ];
    let mut known = matrix_prefix()[..85].to_vec();
    known.extend([12, 9]);
    known.extend_from_slice(&short_matrix);
    let unknown = [vec![12, 16], short_matrix].concat();
    for (index, extra, projection, parse) in [(0, known, 1080, 142), (1, unknown, 696, 46)] {
        let bytes = cff_font(
            &format!("Glyph CFF matrix cursor {index}"),
            cff_table_extra(
                &[
                    line_program(true),
                    line_program(true),
                    line_program(true),
                    line_program(true),
                ],
                &[vec![]],
                false,
                false,
                &extra,
            ),
            false,
        );
        actual_cff_line(&bytes);
        let mut budget = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut budget).unwrap();
        assert_eq!(budget.glyph_work_usage.dict_projection, projection);
        assert_eq!(budget.glyph_work_usage.dict_parse, parse);
        assert_eq!(budget.glyph_work_usage.total(), budget.usage().glyph_work);
        assert_eq!(budget.usage().glyph_points, 24);
    }
}

#[test]
fn cff_bcd_phase_endings_keep_real_matrix_tokens_and_prefix_fees() {
    super::initialize_font_test_platform();
    let fraction = vec![
        30, 0x0a, 0, 0x1a, 139, 139, 30, 0x0a, 0, 0x1a, 139, 139, 12, 7, 30, 0xf0,
    ];
    let integer = vec![30, 0x1d, 139, 139, 30, 0x1d, 139, 139, 12, 7, 30, 0xf0];
    let exponent = vec![
        30, 0x1c, 0x3a, 139, 139, 30, 0x1c, 0x3a, 139, 139, 12, 7, 30, 0xf0,
    ];
    let overflow = vec![
        30, 0x1c, 0x10, 0, 0, 139, 139, 30, 0x0a, 0, 0x1f, 139, 139, 12, 7, 30, 0xf0,
    ];
    for (index, extra, projection, parse) in [
        (0, fraction, 740, 57),
        (1, integer, 708, 49),
        (2, exponent, 724, 53),
        (3, overflow, 748, 59),
    ] {
        let family = format!("Glyph CFF BCD phase {index}");
        let bytes = cff_font(
            &family,
            cff_table_extra(
                &[
                    line_program(true),
                    line_program(true),
                    line_program(true),
                    line_program(true),
                ],
                &[vec![]],
                false,
                false,
                &extra,
            ),
            false,
        );
        actual_cff_line(&bytes);
        let mut measured = FontBudget::default();
        glyph_budget::admit_font_glyphs(&bytes, 0, &mut measured).unwrap();
        assert_eq!(measured.glyph_work_usage.dict_projection, projection);
        assert_eq!(measured.glyph_work_usage.dict_parse, parse);
        let work = measured.usage().glyph_work;
        let mut denied = FontBudget::new(FontLimits {
            glyph_work: work - 1,
            ..FontLimits::default()
        });
        let error = prepare_font(Arc::new(bytes.clone()), &mut denied)
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphWork)
        );
        assert_before_publication(&denied, &family);
        let mut exact = FontBudget::new(FontLimits {
            glyph_work: work,
            ..FontLimits::default()
        });
        prepare_font(Arc::new(bytes), &mut exact).unwrap();
        assert_eq!(exact.metric_copies, 1);
        assert_eq!(exact.shared_registrations, 0);
        let directory = tempfile::tempdir().unwrap();
        let previous_path = directory.path().join("previous.ttf");
        fs::write(
            &previous_path,
            dag(&format!("Glyph BCD prior choice {index}")),
        )
        .unwrap();
        let mut records = Vec::new();
        let mut choices = BTreeMap::new();
        let mut guarded = FontBudget::default();
        load_custom_font(&previous_path, &mut records, &mut choices, &mut guarded).unwrap();
        let previous = super::record_snapshot(&records, &choices);
        let counters = (
            guarded.metric_copies,
            guarded.temporary_registrations,
            guarded.shared_registrations,
        );
        guarded.limits.glyph_work = guarded.usage().glyph_work + work - 1;
        let path = directory.path().join("bcd.otf");
        fs::write(
            &path,
            cff_font(
                &family,
                cff_table_extra(
                    &[
                        line_program(true),
                        line_program(true),
                        line_program(true),
                        line_program(true),
                    ],
                    &[vec![]],
                    false,
                    false,
                    &extra,
                ),
                false,
            ),
        )
        .unwrap();
        let error = load_custom_font(&path, &mut records, &mut choices, &mut guarded).unwrap_err();
        assert_eq!(
            error.downcast_ref::<FontLimitError>(),
            Some(&FontLimitError::GlyphWork)
        );
        assert_eq!(super::record_snapshot(&records, &choices), previous);
        assert_eq!(
            (
                guarded.metric_copies,
                guarded.temporary_registrations,
                guarded.shared_registrations
            ),
            counters
        );
    }
}
