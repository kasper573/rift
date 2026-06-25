use std::path::Path;

// The asset tree is baked into the binary with `include_dir!`, which doesn't tell Cargo to rebuild
// when those files change — so an edited map or a newly added tileset would otherwise be served stale
// from a cached build. Emit a rerun trigger for every asset file so the embed always tracks `assets/`.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    watch(Path::new("../../assets"));
}

fn watch(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            watch(&path);
        }
    }
}
