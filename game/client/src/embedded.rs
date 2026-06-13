//! Embeds the `assets/` tree into the shipped binary and materializes it on disk at startup, so a
//! distributed client is self-contained — no `assets/` folder or `RIFT_ASSETS` needed. On disk
//! rather than an in-memory Bevy asset source because the `world` crate reads content tables and
//! Tiled maps straight from the filesystem; one extracted tree serves both them and Bevy.

use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../assets");

/// Writes the embedded tree to a temp directory and returns its path, for `RIFT_ASSETS`. Re-extracts
/// each run so a freshly built binary's content always wins over a stale extraction.
pub fn extract() -> PathBuf {
    let root = std::env::temp_dir().join(concat!("rift-assets-", env!("CARGO_PKG_VERSION")));
    let _ = std::fs::remove_dir_all(&root);
    write(&ASSETS, &root);
    root
}

fn write(dir: &Dir, root: &Path) {
    for file in dir.files() {
        let path = root.join(file.path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create asset directory");
        }
        std::fs::write(&path, file.contents()).expect("write embedded asset");
    }
    for sub in dir.dirs() {
        write(sub, root);
    }
}
