use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use cargo_metadata::{Metadata, MetadataCommand};
use engine::core::content::{Content, FilesystemFolder, LoadWarnings, write};
use engine::systems::actor::model::{ActorModel, ModelDef};
use engine::systems::actor::skeleton::{SkeletonDef, SkeletonId};
use engine::systems::actor::stance;
use engine::systems::item::ItemDef;
use engine::systems::{GameSettings, load_content, registry};
use syn::UseTree;
use syn::visit::{self, Visit};
use walkdir::{DirEntry, WalkDir};

fn main() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("run `cargo metadata`");

    let mut violations = Vec::new();
    let engine = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == env!("CARGO_PKG_NAME"))
        .expect("the lint's own package in the workspace");
    let src = engine
        .manifest_path
        .parent()
        .expect("a manifest has a parent directory")
        .as_std_path()
        .join("src");
    check_crate(&engine.name, &src, &mut violations);
    for package in metadata
        .packages
        .iter()
        .filter(|package| CONTENT_CODE.contains(&package.name.as_str()))
    {
        let src = package
            .manifest_path
            .parent()
            .expect("a manifest has a parent directory")
            .as_std_path()
            .join("src");
        check_json_keys(&package.name, &src, &mut violations);
    }
    let root = metadata.workspace_root.as_std_path();
    check_standalone(&metadata, &mut violations);
    let scratch = metadata.target_directory.as_std_path().join("lint");
    for game in SHIPPED_GAMES {
        check_shipped_game(game, &root.join(game), &scratch.join(game), &mut violations);
    }

    if violations.is_empty() {
        println!("lint: layering, standalone crates, content keys, JSON keys and shipped games OK");
        return;
    }
    for violation in &violations {
        eprintln!("{violation}");
    }
    eprintln!("\nlint: {} violation(s)", violations.len());
    std::process::exit(1);
}

fn check_crate(name: &str, src: &Path, out: &mut Vec<String>) {
    let files = WalkDir::new(src)
        .into_iter()
        .filter_map(Result::ok)
        .map(DirEntry::into_path)
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"));
    for file in files {
        let rel = file
            .strip_prefix(src)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");

        if rel.starts_with("core/") {
            for reference in core_to_systems(&file) {
                out.push(format!(
                    "[{name}] src/{rel}: `{reference}` — core may not depend on any systems layer"
                ));
            }
        }
        if !rel.starts_with("core/content/") {
            for (line, call) in content_keys(&file) {
                out.push(format!(
                    "[{name}] src/{rel}:{line}: `{call}` — code names content: use a fixture, or a setting in game.config.json"
                ));
            }
        }
    }
}

/// Code reads and writes JSON through the types that declare its fields; only code that walks a
/// JSON Schema, and so takes its keys from the schema or names the schema's own, handles JSON by
/// key.
fn check_json_keys(name: &str, src: &Path, out: &mut Vec<String>) {
    let files = WalkDir::new(src)
        .into_iter()
        .filter_map(Result::ok)
        .map(DirEntry::into_path)
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"));
    for file in files {
        let rel = file
            .strip_prefix(src)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if SCHEMA_WALKERS.contains(&(name, rel.as_str())) {
            continue;
        }
        for (line, access) in json_keys(&file) {
            out.push(format!(
                "[{name}] src/{rel}:{line}: `{access}` — JSON by literal key: read and write it as the type that declares the field"
            ));
        }
    }
}

/// The packages whose code reads and writes game content. Tests and the testkit declare their games
/// as JSON literals, as the files hold them: the loader reads every one and refuses fields its
/// types don't have, so a stale literal fails its test loudly.
const CONTENT_CODE: &[&str] = &["engine", "editor", "showcase", "driver"];

/// The files that walk a JSON Schema, keyed by the package they are in.
const SCHEMA_WALKERS: &[(&str, &str)] = &[("editor", "schema.rs"), ("editor", "forms.rs")];

/// The folders of the crates that know nothing of rift, so any game or tool could use them.
const STANDALONE: &[&str] = &["ui", "bevy"];

/// The game folders the repo ships: the example game, and the template the editor starts new games
/// from.
const SHIPPED_GAMES: &[&str] = &["example_game", "editor/templates/new_game"];

fn check_standalone(metadata: &Metadata, out: &mut Vec<String>) {
    let root = metadata.workspace_root.as_std_path();
    let members: BTreeSet<&str> = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    for package in &metadata.packages {
        let folder = package
            .manifest_path
            .as_std_path()
            .strip_prefix(root)
            .expect("a member inside the workspace");
        if !STANDALONE
            .iter()
            .any(|standalone| folder.starts_with(standalone))
        {
            continue;
        }
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| members.contains(dependency.name.as_str()))
        {
            out.push(format!(
                "[{}] depends on `{}` — a standalone crate may not depend on another crate in this repo",
                package.name, dependency.name
            ));
        }
    }
}

/// A shipped game loads as a whole game, and every file of a kind the editor writes reads exactly
/// as the editor writes it, so saving it in the editor changes only what was edited.
fn check_shipped_game(name: &str, folder: &Path, scratch: &Path, out: &mut Vec<String>) {
    let content = match load_content(Arc::new(FilesystemFolder(folder.to_path_buf())), "lint") {
        Ok(content) => content,
        Err(report) => {
            out.push(format!("[{name}] does not load:\n{report}"));
            return;
        }
    };
    check_bodies(name, &content, out);
    if scratch.exists() {
        fs::remove_dir_all(scratch).expect("clear the lint's scratch folder");
    }
    let writer = write(registry(), content, scratch).expect("write the game to the scratch folder");
    let written: BTreeSet<String> = writer.written().iter().cloned().collect();
    let shipped = written_kinds(folder);
    for file in shipped.difference(&written) {
        out.push(format!("[{name}] {file}: a file the editor never writes"));
    }
    for file in written.intersection(&shipped) {
        let ours = fs::read(folder.join(file)).expect("a shipped file");
        let editors = fs::read(scratch.join(file)).expect("a written file");
        if ours != editors {
            out.push(format!(
                "[{name}] {file}: not as the editor writes it; see {}",
                scratch.join(file).display()
            ));
        }
    }
}

/// What a shipped game's bodies must be beyond what loads: every skeleton worn by a model, and a
/// set on the player's skeleton for every stance an item names, so no shipped weapon falls back to
/// the base set. The loader's warnings are printed, the designer's to accept.
fn check_bodies(name: &str, content: &Content, out: &mut Vec<String>) {
    for warning in &content.built::<LoadWarnings>().0 {
        println!("[{name}] warning: {warning}");
    }
    let worn: BTreeSet<SkeletonId> = content
        .table::<ModelDef>()
        .rows()
        .map(|model| model.skeleton)
        .collect();
    for skeleton in content.table::<SkeletonDef>().ids() {
        if !worn.contains(&skeleton) {
            out.push(format!(
                "[{name}] {}: no model wears this skeleton",
                skeleton.path(content)
            ));
        }
    }
    let player = content.built::<GameSettings>().player.get(content);
    let body = ActorModel::get(content, player.class.get(content).model).skeleton();
    for item in content.table::<ItemDef>().ids() {
        let Some(stance) = stance::of_item(content, item) else {
            continue;
        };
        if body.def.set(stance).is_none() {
            out.push(format!(
                "[{name}] {}: its stance {} has no set on the player's skeleton",
                item.path(content),
                stance.path(content)
            ));
        }
    }
}

/// Every file of the kinds the editor writes.
fn written_kinds(folder: &Path) -> BTreeSet<String> {
    WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry.path().strip_prefix(folder).unwrap_or(entry.path());
            relative.to_string_lossy().replace('\\', "/")
        })
        .filter(|file| registry().writes(file))
        .collect()
}

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

fn content_keys(file: &Path) -> Vec<(usize, String)> {
    let mut found = KeyLookups::default();
    if let Ok(source) = fs::read_to_string(file)
        && let Ok(ast) = syn::parse_file(&source)
    {
        found.visit_file(&ast);
    }
    found.hits
}

const KEY_LOOKUPS: &[&str] = &["by_path", "id_of"];

#[derive(Default)]
struct KeyLookups {
    hits: Vec<(usize, String)>,
}

impl<'ast> Visit<'ast> for KeyLookups {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if KEY_LOOKUPS.contains(&call.method.to_string().as_str())
            && let Some(key) = call.args.first().and_then(string_literal)
        {
            self.hits.push((
                call.method.span().start().line,
                format!("{}({key:?})", call.method),
            ));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && let Some(last) = path.path.segments.last()
            && KEY_LOOKUPS.contains(&last.ident.to_string().as_str())
            && let Some(key) = call.args.iter().find_map(string_literal)
        {
            self.hits.push((
                last.ident.span().start().line,
                format!("{}({key:?})", last.ident),
            ));
        }
        visit::visit_expr_call(self, call);
    }
}

fn json_keys(file: &Path) -> Vec<(usize, String)> {
    let mut found = JsonKeys::default();
    if let Ok(source) = fs::read_to_string(file)
        && let Ok(ast) = syn::parse_file(&source)
    {
        found.visit_file(&ast);
    }
    found.hits
}

const KEYED_ACCESS: &[&str] = &[
    "get",
    "get_mut",
    "pointer",
    "pointer_mut",
    "remove",
    "insert",
    "contains_key",
    "entry",
];

#[derive(Default)]
struct JsonKeys {
    hits: Vec<(usize, String)>,
}

impl<'ast> Visit<'ast> for JsonKeys {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !walks_a_schema(&item.sig) {
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !walks_a_schema(&item.sig) {
            visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_expr_index(&mut self, index: &'ast syn::ExprIndex) {
        if let Some(key) = string_literal(&index.index) {
            self.hits.push((
                index.bracket_token.span.open().start().line,
                format!("[{key:?}]"),
            ));
        }
        visit::visit_expr_index(self, index);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if KEYED_ACCESS.contains(&call.method.to_string().as_str())
            && let Some(key) = call.args.first().and_then(string_literal)
        {
            self.hits.push((
                call.method.span().start().line,
                format!(".{}({key:?})", call.method),
            ));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let builds_object = mac.path.segments.last().is_some_and(|last| last.ident == "json")
            && mac.tokens.clone().into_iter().next().is_some_and(|token| {
                matches!(token, proc_macro2::TokenTree::Group(group) if group.delimiter() == proc_macro2::Delimiter::Brace)
            });
        if builds_object && let Some(last) = mac.path.segments.last() {
            self.hits
                .push((last.ident.span().start().line, "json!({…})".to_owned()));
        }
        visit::visit_macro(self, mac);
    }
}

/// Whether a function takes or makes a JSON Schema, whose keys are the schema's own vocabulary.
fn walks_a_schema(sig: &syn::Signature) -> bool {
    let names_a_schema = |ty: &syn::Type| {
        let mut found = SchemaTypes::default();
        found.visit_type(ty);
        found.0
    };
    let output = match &sig.output {
        syn::ReturnType::Type(_, ty) => names_a_schema(ty),
        syn::ReturnType::Default => false,
    };
    output
        || sig.inputs.iter().any(|input| match input {
            syn::FnArg::Typed(typed) => names_a_schema(&typed.ty),
            syn::FnArg::Receiver(_) => false,
        })
}

#[derive(Default)]
struct SchemaTypes(bool);

impl<'ast> Visit<'ast> for SchemaTypes {
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        if segment.ident == "Schema" || segment.ident == "SchemaGenerator" {
            self.0 = true;
        }
        visit::visit_path_segment(self, segment);
    }
}

fn string_literal(expr: &syn::Expr) -> Option<String> {
    let expr = match expr {
        syn::Expr::Reference(reference) => &*reference.expr,
        other => other,
    };
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}
