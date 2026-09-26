fn main() {
    check_livearea_assets();
    use std::{collections::BTreeMap, fmt::Write};
    let mut source = String::from("const MESSAGES: &[nivora_ui::StaticLocale] = &[\n");
    let mut keys = None;
    for locale in ["zh", "en"] {
        let path = format!("locales/{locale}.json");
        println!("cargo:rerun-if-changed={path}");
        let values: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let current: Vec<_> = values.keys().cloned().collect();
        if let Some(previous) = &keys {
            assert_eq!(previous, &current, "Translation keys differ");
        }
        keys = Some(current);
        writeln!(
            source,
            "nivora_ui::StaticLocale {{ locale: {locale:?}, messages: &["
        )
        .unwrap();
        for (key, value) in values {
            assert!(!value.trim().is_empty());
            writeln!(source, "({key:?},{value:?}),").unwrap();
        }
        source.push_str("] },\n");
    }
    source.push_str("];\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("locales.rs"),
        source,
    )
    .unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("vita") {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-search=native={root}/target/native/user");
        println!("cargo:rustc-link-lib=static=VitaSaveMountUser_stub_weak");
        println!("cargo:rustc-link-lib=static=taihen_stub");
        println!("cargo:rustc-link-arg=-Wl,--undefined=sceLibcHeapSize");
        println!("cargo:rustc-link-arg=-Wl,--undefined=sceUserMainThreadStackSize");
    }
}

// Vita's installer rejects unsupported LiveArea PNG encodings with 0x8010113D.
fn check_livearea_assets() {
    for (name, width, height) in [
        ("icon0.png", 128u32, 128u32),
        ("livearea/contents/bg0.png", 840, 500),
        ("livearea/contents/startup.png", 280, 158),
    ] {
        let path = format!("runtime/sce_sys/{name}");
        println!("cargo:rerun-if-changed={path}");
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("Read {path}: {e}"));
        assert!(
            data.len() >= 33
                && &data[..8] == b"\x89PNG\r\n\x1a\n"
                && data[8..12] == 13u32.to_be_bytes()
                && &data[12..16] == b"IHDR",
            "{path}: invalid PNG header"
        );
        assert_eq!(&data[16..20], &width.to_be_bytes(), "{path}: invalid width");
        assert_eq!(
            &data[20..24],
            &height.to_be_bytes(),
            "{path}: invalid height"
        );
        assert_eq!(
            &data[24..29],
            &[8, 3, 0, 0, 0],
            "{path}: LiveArea requires a non-interlaced 8-bit indexed PNG; convert with pngquant"
        );
    }
}
