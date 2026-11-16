use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
};

const BUNDLED_FONT_FILES: &[&str] = &[
    "NotoSansSC-Regular.otf",
    "NotoSansSC-Medium.otf",
    "NotoSansSC-Bold.otf",
    "NotoSerifSC-Light.otf",
    "NotoSerifSC-Regular.otf",
    "NotoSerifSC-Medium.otf",
    "NotoSerifSC-Bold.otf",
];

fn prepare_bundled_fonts(output: &Path) {
    let directory = output.join("fonts");
    fs::create_dir_all(&directory).expect("Create generated font directory");
    for name in BUNDLED_FONT_FILES {
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
            fs::write(target, bytes).expect("Write a generated font payload");
        }
    }
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
