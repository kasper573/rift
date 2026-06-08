//! Embeds every file under `assets/` as the FILES table behind `core::assets`.

use std::path::Path;
use std::{env, fs};

fn main() {
    println!("cargo::rerun-if-changed=assets");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut entries = Vec::new();
    collect(&root, &root, &mut entries);
    entries.sort();
    let mut table = String::from("static FILES: &[(&str, &[u8])] = &[\n");
    for (name, path) in &entries {
        table.push_str(&format!("    ({name:?}, include_bytes!({path:?})),\n"));
    }
    table.push_str("];\n");
    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("assets.rs");
    fs::write(out, table).expect("write assets table");
}

fn collect(root: &Path, dir: &Path, entries: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("readable assets dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(root, &path, entries);
        } else {
            let name = path
                .strip_prefix(root)
                .expect("path under assets")
                .components()
                .map(|c| c.as_os_str().to_str().expect("utf-8 asset path"))
                .collect::<Vec<_>>()
                .join("/");
            entries.push((name, path.to_str().expect("utf-8 asset path").to_owned()));
        }
    }
}
