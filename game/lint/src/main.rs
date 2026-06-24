//! Custom lint rules for the game crates. A crate opts in with
//!
//! ```toml
//! [package.metadata.lint]
//! game = true
//! ```
//!
//! and is then held to the layering convention:
//!
//! 1. Rust source lives only under `src/core/`, `src/systems/`, or `src/bin/` (plus the crate root
//!    `src/lib.rs` / `src/main.rs`). Tests live in `tests/`, outside `src/`.
//! 2. `tests/` and `src/bin/` may depend on both `core` and `systems`.
//! 3. `core` may not depend on *any* systems layer — neither its own crate's `systems/` nor another
//!    crate's (e.g. `world::systems` from the client). Code in `core` must be abstract enough that
//!    systems plug into it, not the other way round.
//!
//! Run from the workspace root (`cargo run -p lint`); exits non-zero on any violation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::MetadataCommand;
use syn::UseTree;
use syn::visit::{self, Visit};

fn main() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("run `cargo metadata`");

    let mut violations = Vec::new();
    for pkg in &metadata.packages {
        if !opted_in(&pkg.metadata) {
            continue;
        }
        let src = pkg
            .manifest_path
            .parent()
            .expect("a manifest has a parent directory")
            .as_std_path()
            .join("src");
        check_crate(&pkg.name, &src, &mut violations);
    }

    if violations.is_empty() {
        println!("lint: layering OK");
        return;
    }
    for violation in &violations {
        eprintln!("{violation}");
    }
    eprintln!("\nlint: {} layering violation(s)", violations.len());
    std::process::exit(1);
}

fn opted_in(metadata: &serde_json::Value) -> bool {
    metadata
        .get("lint")
        .and_then(|lint| lint.get("game"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn check_crate(name: &str, src: &Path, out: &mut Vec<String>) {
    let mut files = Vec::new();
    collect_rs(src, &mut files);
    for file in files {
        let rel = file
            .strip_prefix(src)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");

        let in_core = rel.starts_with("core/");
        let placed = in_core || rel.starts_with("systems/") || rel.starts_with("bin/");
        if !placed && rel != "lib.rs" && rel != "main.rs" {
            out.push(format!(
                "[{name}] src/{rel}: source must live under src/core/, src/systems/, or src/bin/ (only lib.rs/main.rs may sit at the src root)"
            ));
        }

        if in_core {
            for reference in core_to_systems(&file) {
                out.push(format!(
                    "[{name}] src/{rel}: `{reference}` — core may not depend on a systems layer (any crate's)"
                ));
            }
        }
    }
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every distinct `…::systems` reference (in any path or `use`, any crate) in a `core/` file.
fn core_to_systems(file: &Path) -> BTreeSet<String> {
    let mut found = SystemsRefs::default();
    if let Ok(source) = fs::read_to_string(file)
        && let Ok(ast) = syn::parse_file(&source)
    {
        found.visit_file(&ast);
    }
    found.hits
}

#[derive(Default)]
struct SystemsRefs {
    hits: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for SystemsRefs {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        // `<crate>::systems::…` for any crate root (crate / world / client / …) — core must not reach
        // into a systems layer, in its own crate or across a crate boundary.
        if segments.len() >= 2 && segments[1] == "systems" {
            self.hits.insert(segments.join("::"));
        }
        visit::visit_path(self, path);
    }

    fn visit_use_tree(&mut self, tree: &'ast UseTree) {
        if let UseTree::Path(path) = tree
            && use_reaches_systems(&path.tree)
        {
            self.hits.insert(format!("use {}::systems::…", path.ident));
        }
        visit::visit_use_tree(self, tree);
    }
}

fn use_reaches_systems(tree: &UseTree) -> bool {
    match tree {
        UseTree::Path(path) => path.ident == "systems",
        UseTree::Name(name) => name.ident == "systems",
        UseTree::Rename(rename) => rename.ident == "systems",
        UseTree::Group(group) => group.items.iter().any(use_reaches_systems),
        UseTree::Glob(_) => false,
    }
}
