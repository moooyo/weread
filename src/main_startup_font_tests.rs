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
