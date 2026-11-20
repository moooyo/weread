use std::{
    env,
    fmt::Write,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(feature = "bundled-font-preflight")]
#[allow(dead_code)]
#[path = "src/main_font_budget.rs"]
mod font_budget;
#[cfg(feature = "bundled-font-preflight")]
#[allow(dead_code)]
#[path = "src/main_glyph_budget.rs"]
mod glyph_budget;
#[cfg(feature = "bundled-font-preflight")]
#[path = "src/main_glyph_cff.rs"]
mod glyph_cff;

const BUNDLED_FONT_FILES: &[(&str, &str)] = &[
    ("SANS_REGULAR", "NotoSansSC-Regular.otf"),
    ("SANS_MEDIUM", "NotoSansSC-Medium.otf"),
    ("SANS_BOLD", "NotoSansSC-Bold.otf"),
    ("SERIF_LIGHT", "NotoSerifSC-Light.otf"),
    ("SERIF_REGULAR", "NotoSerifSC-Regular.otf"),
    ("SERIF_MEDIUM", "NotoSerifSC-Medium.otf"),
    ("SERIF_BOLD", "NotoSerifSC-Bold.otf"),
];

fn prepare_bundled_fonts(output: &Path) {
    let directory = output.join("fonts");
    fs::create_dir_all(&directory).expect("Create generated font directory");
    let mut declarations = String::new();
    #[cfg(feature = "bundled-font-preflight")]
    let (manifest, mut charges) = {
        for path in [
            "assets/fonts/manifest.json",
            "src/main_font_budget.rs",
            "src/main_glyph_budget.rs",
            "src/main_glyph_cff.rs",
        ] {
            println!("cargo:rerun-if-changed={path}");
        }
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read("assets/fonts/manifest.json").expect("Read the bundled font manifest"),
        )
        .expect("Parse the bundled font manifest");
        (manifest, String::new())
    };
    for &(symbol, name) in BUNDLED_FONT_FILES {
        let source = format!("assets/fonts/{name}.gz");
        println!("cargo:rerun-if-changed={source}");
        let compressed = fs::File::open(&source).expect("Read a bundled font source");
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(compressed)
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .expect("Expand a bundled font at build time");
        assert!(
            bytes.len() <= 64 * 1024 * 1024,
            "Bundled font exceeds the size limit"
        );
        let target = directory.join(name);
        if fs::read(&target).ok().as_deref() != Some(bytes.as_slice()) {
            fs::write(&target, &bytes).expect("Write a generated font payload");
        }
        // A named array has one storage identity even through const tables in
        // other codegen units; a promoted include_bytes! slice need not.
        writeln!(declarations, "static {symbol}_BYTES: [u8; {}] = *include_bytes!(concat!(env!(\"OUT_DIR\"), \"/fonts/{name}\"));\nconst {symbol}: &[u8] = &{symbol}_BYTES;", bytes.len()).unwrap();
        #[cfg(feature = "bundled-font-preflight")]
        {
            use sha2::{Digest, Sha256};
            println!("cargo:rerun-if-changed={}", target.display());
            let entry = manifest["fonts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|font| font["decompressed_file"] == name)
                .expect("Missing bundled font manifest entry");
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                entry["decompressed_sha256"].as_str().unwrap(),
                "Unreviewed bundled font {name}"
            );
            assert!(
                !bytes.starts_with(b"ttcf"),
                "Expected a standalone bundled CFF font: {name}"
            );
            let charge = glyph_budget::admit_font_glyphs_with_cff(
                &bytes,
                0,
                &mut font_budget::FontBudget::default(),
                None,
            )
            .unwrap_or_else(|e| panic!("Bundled glyph preflight failed for {name}: {e}"))
            .expect("Expected a CFF1 font in the bundled manifest");
            writeln!(charges, "    ({symbol}, font_budget::CffLoopCharge {{ work: {}, visits: {}, points: {}, memory: {} }}),", charge.work, charge.visits, charge.points, charge.memory).unwrap();
        }
    }
    #[cfg(feature = "bundled-font-preflight")]
    write!(
        declarations,
        "static BUNDLED_CFF_RECEIPTS: &[(&[u8], font_budget::CffLoopCharge)] = &[\n{charges}];\n"
    )
    .unwrap();
    let generated = output.join("bundled_fonts.rs");
    if fs::read(&generated).ok().as_deref() != Some(declarations.as_bytes()) {
        fs::write(&generated, declarations).expect("Write bundled font declarations");
    }
    #[cfg(feature = "bundled-font-preflight")]
    println!("cargo:rerun-if-changed={}", generated.display());
}

fn main() {
    println!("cargo:rerun-if-changed=assets/i18n/zh-CN.json");
    println!("cargo:rerun-if-changed=assets/design-tokens.json");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    prepare_bundled_fonts(&output);
    let labels: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(
        &fs::read("assets/i18n/zh-CN.json").expect("Read interface translations"),
    )
    .expect("Parse interface translations");
    let mut translations = String::from("export global Strings {\n");
    for (key, value) in labels {
        let value = serde_json::to_string(value.as_str().expect("Translation must be text"))
            .expect("Escape translation");
        translations.push_str(&format!(
            "    out property <string> {}: {};\n",
            key.replace('_', "-"),
            value
        ));
    }
    translations.push_str("}\n");
    fs::write(output.join("strings.slint"), translations).expect("Write generated translations");
    let configuration = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![output])
        .with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", configuration)
        .expect("Compile the application interface");
}
