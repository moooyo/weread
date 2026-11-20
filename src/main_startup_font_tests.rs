use super::*;

const FONTS: &[(&str, &[u8])] = &[
    ("NotoSansSC-Regular.otf", SANS_REGULAR),
    ("NotoSansSC-Medium.otf", SANS_MEDIUM),
    ("NotoSansSC-Bold.otf", SANS_BOLD),
    ("NotoSerifSC-Light.otf", SERIF_LIGHT),
    ("NotoSerifSC-Regular.otf", SERIF_REGULAR),
    ("NotoSerifSC-Medium.otf", SERIF_MEDIUM),
    ("NotoSerifSC-Bold.otf", SERIF_BOLD),
];

#[test]
fn expanded_fonts_match_the_accepted_font_manifest() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../assets/fonts/manifest.json")).unwrap();
    let entries = manifest["fonts"].as_array().unwrap();
    for (name, bytes) in FONTS {
        let entry = entries
            .iter()
            .find(|entry| entry["decompressed_file"] == *name)
            .unwrap();
        assert_eq!(
            bytes.len() as u64,
            entry["decompressed_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            hex::encode(Sha256::digest(bytes)),
            entry["decompressed_sha256"].as_str().unwrap()
        );
    }
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["available"] == true)
            .count(),
        FONTS.len()
    );
}

#[test]
fn prepared_bundled_font_retains_static_bytes_and_resource_admission() {
    super::font_budget_tests::initialize_font_test_platform();
    let mut budget = FontBudget::default();
    let prepared = prepare_bundled_font(SANS_REGULAR, &mut budget).unwrap();
    let bytes = prepared.bytes.as_ref().as_ref();
    assert_eq!(bytes.as_ptr(), SANS_REGULAR.as_ptr());
    assert_eq!(bytes.len(), SANS_REGULAR.len());
    assert!(!prepared.faces.is_empty());
    assert_eq!(budget.temporary_registrations, 1);
    assert_eq!(budget.shared_registrations, 0);
    assert!(prepare_bundled_font(b"invalid font", &mut FontBudget::default()).is_err());
}

#[test]
fn static_metrics_preserve_owned_font_advances_kerning_and_fallbacks() {
    for (_, bytes) in FONTS {
        let borrowed = TextMetrics::from_static_font(bytes, 0).unwrap();
        let owned = TextMetrics::from_font(bytes, 0).unwrap();
        for size in [16.0, 24.0, 36.0] {
            for text in [
                "AV To WeRead",
                "\u{4e2d}\u{6587}\u{3002}",
                "a\u{301}\t \u{10ffff}",
            ] {
                assert_eq!(borrowed.measure(text, size), owned.measure(text, size));
            }
        }
        assert_eq!(
            TextMetrics::from_static_font(bytes, 1).is_err(),
            TextMetrics::from_font(bytes, 1).is_err()
        );
    }
}

#[cfg(feature = "bundled-font-preflight")]
#[test]
fn bundled_preflight_replays_only_the_exact_static_faces_with_identical_charges() {
    super::font_budget_tests::initialize_font_test_platform();
    let mut fast = FontBudget::default();
    let mut full = FontBudget::default();
    for &(name, bytes) in FONTS {
        let charge = bundled_cff_charge(bytes).expect(name);
        assert!(bundled_cff_charge(&bytes[..bytes.len() - 1]).is_none());
        let previous = fast.glyph_work_usage.precomputed;
        let font = register_bundled_font_data(bytes, &mut fast).unwrap();
        full.record_read(bytes.len()).unwrap();
        let reference =
            prepare_font_payload(Arc::new(bytes), Some(bytes), None, &mut full).unwrap();
        publish_font(&reference, &mut full);
        assert_eq!(fast.usage(), full.usage(), "{name}");
        assert_eq!(fast.glyph_work_usage.total(), fast.usage().glyph_work);
        assert_eq!(fast.glyph_work_usage.precomputed - previous, charge.work);
        assert_eq!(fast.glyph_graph_nodes, 0);
        assert!(full.glyph_graph_nodes > 0);
        assert_eq!(fast.glyph_state_allocations, full.glyph_state_allocations);
        assert_eq!(font.faces[0].family, reference.faces[0].family);
        assert_eq!(font.faces[0].weight, reference.faces[0].weight);
        assert!(std::ptr::eq(font.bytes.as_ref().as_ref(), bytes));
    }
    assert_eq!(
        (
            fast.shared_registrations,
            fast.temporary_registrations,
            fast.metric_copies
        ),
        (7, 7, 7)
    );

    let owned = Arc::new(SANS_REGULAR.to_vec());
    assert!(bundled_cff_charge(&owned).is_none());
    let mut custom = FontBudget::default();
    prepare_font(owned, &mut custom).unwrap();
    assert!(custom.glyph_graph_nodes > 0);
    assert_eq!(custom.glyph_work_usage.precomputed, 0);
}

#[cfg(feature = "bundled-font-preflight")]
#[test]
fn bundled_preflight_preserves_insufficient_budget_errors_and_partial_charges() {
    use font_budget::FontLimits;
    let mut measured = FontBudget::default();
    glyph_budget::admit_font_glyphs(SANS_REGULAR, 0, &mut measured).unwrap();
    let usage = measured.usage();
    let charge = bundled_cff_charge(SANS_REGULAR).unwrap();
    for limits in [
        FontLimits {
            glyph_work: usage.glyph_work,
            ..FontLimits::default()
        },
        FontLimits {
            glyph_visits: usage.glyph_visits - 1,
            ..FontLimits::default()
        },
        FontLimits {
            glyph_points: usage.glyph_points - 1,
            ..FontLimits::default()
        },
        FontLimits {
            glyph_state_bytes: usage.glyph_state_bytes - 1,
            ..FontLimits::default()
        },
        FontLimits {
            per_glyph_memory: usage.glyph_memory - 1,
            ..FontLimits::default()
        },
        FontLimits {
            per_glyph_visits: 1,
            ..FontLimits::default()
        },
        FontLimits {
            per_glyph_points: 4,
            ..FontLimits::default()
        },
    ] {
        let mut fast = FontBudget::new(limits);
        let mut full = FontBudget::new(limits);
        for budget in [&mut fast, &mut full] {
            budget.reserve_glyph_work(1).unwrap();
        }
        let expected = glyph_budget::admit_font_glyphs(SANS_REGULAR, 0, &mut full).unwrap_err();
        assert_eq!(
            glyph_budget::admit_font_glyphs_with_cff(SANS_REGULAR, 0, &mut fast, Some(charge))
                .unwrap_err(),
            expected
        );
        assert_eq!(fast.usage(), full.usage());
        assert_eq!(fast.glyph_work_usage, full.glyph_work_usage);
        assert_eq!(fast.glyph_graph_nodes, full.glyph_graph_nodes);
        assert_eq!(fast.glyph_work_usage.precomputed, 0);
    }
}
