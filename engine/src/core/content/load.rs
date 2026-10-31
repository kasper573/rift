use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use bevy::tasks::block_on;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::bundle::{Bundle, PartSpec, Parts};
use super::check::RowCheck;
use super::files::{
    CONFIG_FILE, FileType, Listing, SETTINGS_FILE, Shape, bundle_of, fields_file, fixture_dir,
    table_of,
};
use super::fixture::{Fixture, FixtureIds, FixtureLock};
use super::fresh::Nearby;
use super::id::{ContentId, ContentScope, Leaf, LeafKind, nearest, with_content};
use super::locate::{IndexedValue, JsonIndex, escape};
use super::report::{ContentProblem, ErrorPlace, LoadError, LoadReport, VERSION_RULE, article};
use super::serial::TypeTag;
use super::table::{
    Content, ContentBuilder, ContentFile, MODULES, ModuleRule, ModuleSet, NamedContent, ReadRow,
    Table,
};
use super::version::ContentVersion;
use crate::core::pace::Pace;

// The steps a load runs its hooks at, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoadStep {
    // Every row parsed and checked on its own.
    Rows,
    // Every file read and hashed.
    Files,
    // Rows that stand for others, such as templates, expanded.
    Expand,
    // What's put together from expanded rows.
    Assemble,
    // Checks across tables.
    Across,
    // Checks of the game's settings, once they parsed.
    Settings,
    // Only once everything else loaded without an error.
    Compile,
}

/// A step of a load: an `async fn(&mut Loading)` that breathes between the pieces of its work, so a
/// paced load never holds a frame.
type Hook =
    Box<dyn for<'a> Fn(&'a mut Loading) -> Pin<Box<dyn Future<Output = ()> + 'a>> + Send + Sync>;
type FixtureCheck = Box<dyn Fn(&mut Loading, bool) + Send + Sync>;
struct FixtureEntry {
    table: &'static str,
    folder: Option<&'static str>,
    whole_table: bool,
    paths: Vec<(String, Vec<FixtureLock>)>,
    check: FixtureCheck,
    fresh: FreshFixture,
}

/// The fresh file of the fixture at a path, when one of the entry's fixtures is there.
type FreshFixture = Box<dyn Fn(&str, &Nearby) -> Option<Result<AnyRow, String>> + Send + Sync>;

type AnyRow = Box<dyn Any + Send + Sync>;

pub type FolderWriter = Box<dyn Fn(&Content, &mut FileWriter) -> io::Result<()> + Send + Sync>;

#[derive(Default)]
pub struct ContentRegistry {
    rows: Vec<Box<dyn TableLoader>>,
    fixtures: Vec<FixtureEntry>,
    fixtures_required: bool,
    hooks: Vec<(LoadStep, Hook)>,
    settings: Option<Box<dyn SettingsLoader>>,
    writers: Vec<FolderWriter>,
    module_sets: Vec<ModuleSetOf>,
    named: Vec<(&'static str, ShownNameOf)>,
    list_kinds: Vec<ListKind>,
    json_parts: Vec<JsonPart>,
    // Hashing every shape takes a while, and a registry is read far more often than it changes.
    version: OnceLock<ContentVersion>,
}

/// The shape of every table and of the game's settings, for tools that edit content.
#[derive(Serialize)]
pub struct ContentShapes {
    pub tables: Vec<TableShape>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct TableShape {
    pub table: String,
    /// Whether each file is a bundle of its config and parts.
    pub bundle: bool,
    pub shape: serde_json::Value,
    /// The path of every fixture of the type.
    pub fixtures: Vec<String>,
    /// What each fixture's file keeps as the engine reads it, by path.
    pub locks: BTreeMap<String, Vec<FixtureLock>>,
    /// Whether the type holds its fixtures and nothing else.
    pub closed: bool,
}

/// What the entries of a JSON list field are called in shape errors: a list of `field` whose
/// entry names an unknown variant reports `rule`, and asks for `expected` in its place.
#[derive(Clone, Copy, Debug)]
pub(super) struct ListKind {
    field: &'static str,
    rule: &'static str,
    expected: &'static str,
}

/// The name a player reads a file of one type by, from its fields as JSON.
type ShownNameOf = fn(&serde_json::Value) -> Option<String>;

/// A type of file holding modules, as a tool that edits its files as JSON reaches them: each
/// function runs in the scope the JSON's references are read in.
struct ModuleSetOf {
    table: &'static str,
    rule: fn(&serde_json::Value) -> Option<ModuleRule>,
    read: fn(&serde_json::Value) -> Result<Vec<serde_json::Value>, String>,
    write: fn(&serde_json::Value, &[serde_json::Value]) -> Result<serde_json::Value, String>,
}

impl ModuleSetOf {
    fn of<Row: ModuleSet>() -> ModuleSetOf {
        ModuleSetOf {
            table: Row::TABLE,
            rule: |json| {
                let (parsed, _) = with_content(ContentScope::new(), || {
                    serde_json::from_value::<Row::Module>(json.clone())
                });
                parsed.ok().map(|module| Row::rule(&module))
            },
            read: |json| {
                let row = parse_value::<Row>(json).map_err(|error| error.to_string())?;
                row.modules()
                    .iter()
                    .map(|module| super::serial::json_of(module).map_err(|e| e.to_string()))
                    .collect()
            },
            write: |json, modules| {
                let mut row = parse_value::<Row>(json).map_err(|error| error.to_string())?;
                *row.modules_mut() = modules
                    .iter()
                    .enumerate()
                    .map(|(index, module)| {
                        parse_value::<Row::Module>(module).map_err(|error| {
                            format!("at /{MODULES}/{index}{}: {}", error.pointer, error.inner)
                        })
                    })
                    .collect::<Result<_, _>>()?;
                super::serial::json_of(&row).map_err(|error| error.to_string())
            },
        }
    }
}

/// A JSON part of a type's bundles that names content, read the way the bundle's type reads it, so
/// its references are found where the editor looks for them.
pub struct JsonPart {
    pub table: &'static str,
    pub name: &'static str,
    /// Reads `value`, the part of the bundle at the given path, in the scope it is called in.
    pub read: fn(&Content, &str, &serde_json::Value) -> Result<(), ParseError>,
}

impl ContentRegistry {
    pub fn file<Row: ContentFile>(&mut self) -> &mut ContentRegistry {
        assert_eq!(Row::SHAPE, Shape::Json, "{} is a bundle type", Row::TABLE);
        self.version.take();
        self.rows.push(Box::new(RowLoader::<Row> {
            parts: None,
            marker: PhantomData,
        }));
        self
    }

    pub fn bundle<Row: Bundle>(&mut self) -> &mut ContentRegistry {
        assert_eq!(
            Row::SHAPE,
            Shape::Bundle,
            "{} isn't a bundle type",
            Row::TABLE
        );
        self.version.take();
        self.rows.push(Box::new(RowLoader::<Row> {
            parts: Some(BundleParts {
                specs: Row::PARTS,
                written: Row::WRITTEN,
                read: Row::read_parts,
                write: Row::write_parts,
            }),
            marker: PhantomData,
        }));
        self
    }

    pub fn modules<Row: ModuleSet>(&mut self) -> &mut ContentRegistry {
        self.version.take();
        self.module_sets.push(ModuleSetOf::of::<Row>());
        self.hook(LoadStep::Rows, module_rules::<Row>)
    }

    /// Marks the files of `Row` as carrying a name, the one `NamedContent` shows.
    pub fn named<Row: NamedContent>(&mut self) -> &mut ContentRegistry {
        self.named.push((Row::TABLE, |json| {
            let (parsed, _) = with_content(ContentScope::new(), || parse_value::<Row>(json).ok());
            parsed.map(|row| row.shown_name().to_owned())
        }));
        self
    }

    /// The name `row`, a file of `table` as JSON, shows, when its type carries one and it reads.
    pub fn shown_name(&self, table: &str, row: &serde_json::Value) -> Option<String> {
        let (_, name) = self.named.iter().find(|(named, _)| *named == table)?;
        name(row)
    }

    pub fn module_rule(&self, table: &str, json: &serde_json::Value) -> Option<ModuleRule> {
        self.module_set(table).and_then(|set| (set.rule)(json))
    }

    /// Whether the files of `table` hold modules.
    pub fn holds_modules(&self, table: &str) -> bool {
        self.module_set(table).is_some()
    }

    /// The modules of `row`, a file of `table` as JSON, each as its type writes it, read in the
    /// scope the caller runs in.
    pub(super) fn modules_of_json(
        &self,
        table: &str,
        row: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, String> {
        let set = self.module_set(table).ok_or_else(|| no_modules(table))?;
        (set.read)(row)
    }

    /// `row`, a file of `table` as JSON, holding `modules` in place of its own, read and written
    /// as their types in the scope the caller runs in.
    pub(super) fn with_modules_json(
        &self,
        table: &str,
        row: &serde_json::Value,
        modules: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        let set = self.module_set(table).ok_or_else(|| no_modules(table))?;
        (set.write)(row, modules)
    }

    fn module_set(&self, table: &str) -> Option<&ModuleSetOf> {
        self.module_sets.iter().find(|set| set.table == table)
    }

    /// Whether a row of `table` holding `modules` may take `module` as well: not when it holds
    /// at most one of that kind and has one already.
    pub fn takes_module(
        &self,
        table: &str,
        modules: &[serde_json::Value],
        module: &serde_json::Value,
    ) -> bool {
        let tag = TypeTag::of_json(module);
        let held = modules.iter().any(|known| TypeTag::of_json(known) == tag);
        !held || self.module_rule(table, module) != Some(ModuleRule::AtMostOne)
    }

    pub fn list_kind(
        &mut self,
        field: &'static str,
        rule: &'static str,
        expected: &'static str,
    ) -> &mut ContentRegistry {
        self.version.take();
        self.list_kinds.push(ListKind {
            field,
            rule,
            expected,
        });
        self
    }

    pub fn json_part(&mut self, part: JsonPart) -> &mut ContentRegistry {
        self.json_parts.push(part);
        self
    }

    pub fn fixture<F: Fixture>(&mut self) -> &mut ContentRegistry {
        self.version.take();
        self.fixtures.push(FixtureEntry {
            table: F::Row::TABLE,
            folder: F::FOLDER,
            whole_table: F::WHOLE_TABLE,
            paths: F::VARIANTS
                .iter()
                .map(|fixture| (fixture.path(), fixture.locks()))
                .collect(),
            check: Box::new(check_fixture::<F>),
            fresh: Box::new(|path, near| {
                let fixture = F::VARIANTS.iter().find(|fixture| fixture.path() == path)?;
                Some(fixture.fresh(near).map(|row| Box::new(row) as AnyRow))
            }),
        });
        self
    }

    /// The fresh file of the content at `path`: the fixture's when a fixture belongs there, else
    /// its type's.
    pub(super) fn fresh_row(
        &self,
        loader: &dyn TableLoader,
        path: &str,
        near: &Nearby,
    ) -> Result<AnyRow, String> {
        self.fixtures
            .iter()
            .find_map(|fixture| (fixture.fresh)(path, near))
            .unwrap_or_else(|| loader.fresh_row(near))
    }

    /// Holds a game to every fixture: without this, a game's fixtures are found wherever it has
    /// them, and the ones it lacks are no error.
    pub fn require_fixtures(&mut self) -> &mut ContentRegistry {
        self.fixtures_required = true;
        self
    }

    pub fn settings<G: Settings>(&mut self) -> &mut ContentRegistry {
        self.version.take();
        self.settings = Some(Box::new(SettingsOf::<G>(PhantomData)));
        self
    }

    pub fn hook<F>(&mut self, step: LoadStep, hook: F) -> &mut ContentRegistry
    where
        F: AsyncFn(&mut Loading) + Send + Sync + 'static,
    {
        let hook = Arc::new(hook);
        self.hooks.push((
            step,
            Box::new(
                move |loading: &mut Loading| -> Pin<Box<dyn Future<Output = ()> + '_>> {
                    let hook = Arc::clone(&hook);
                    Box::pin(async move { hook(loading).await })
                },
            ),
        ));
        self
    }

    pub fn writer(
        &mut self,
        writer: impl Fn(&Content, &mut FileWriter) -> io::Result<()> + Send + Sync + 'static,
    ) -> &mut ContentRegistry {
        self.writers.push(Box::new(writer));
        self
    }

    /// The table of every type kept as a bundle.
    pub fn bundle_tables(&self) -> Vec<&'static str> {
        super::files::bundle_tables(&self.file_types())
    }

    /// The bundle `file` sits in, and its path inside the bundle, when it is a part of one.
    pub fn bundle_of<'f>(&self, file: &'f str) -> Option<(&'f str, &'f str)> {
        bundle_of(file, &self.bundle_tables())
    }

    pub fn table_names(&self) -> Vec<&'static str> {
        self.rows.iter().map(|row| row.name()).collect()
    }

    /// Every type the folder may hold, as the walk of a folder sorts its files.
    pub fn file_types(&self) -> Vec<FileType> {
        self.rows
            .iter()
            .map(|row| FileType {
                table: row.name(),
                shape: row.shape(),
                parts: row.parts(),
            })
            .collect()
    }

    /// Whether `file` is one the writer writes: the JSON of settings and content, the CSV of
    /// grids, or a bundle part its type writes back. Every other file is kept as it is.
    pub fn writes(&self, file: &str) -> bool {
        if file.ends_with(".json") || file.ends_with(".csv") {
            return true;
        }
        let Some((bundle, part)) = self.bundle_of(file) else {
            return false;
        };
        let Some(table) = table_of(bundle) else {
            return false;
        };
        self.rows
            .iter()
            .filter(|row| row.name() == table)
            .flat_map(|row| row.written_parts())
            .any(|spec| spec.matches(part))
    }

    pub fn shape_of(&self, table: &str) -> Option<Shape> {
        self.rows
            .iter()
            .find(|row| row.name() == table)
            .map(|row| row.shape())
    }

    /// Whether every file of the type is in use by being there, as
    /// `ContentFile::PICKED_FROM_TABLE` declares.
    pub fn picked_from_table(&self, table: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.name() == table && row.picked_from_table())
    }

    /// Whether code names the content file at `path` through a fixture, which keeps it where the
    /// engine looks.
    pub fn is_fixture(&self, path: &str) -> bool {
        self.fixtures
            .iter()
            .any(|fixture| fixture.paths.iter().any(|(fixed, _)| fixed == path))
    }

    /// The path of every fixture, so the editor knows what stays where it is.
    pub fn fixture_paths(&self) -> impl Iterator<Item = &str> {
        self.fixtures
            .iter()
            .flat_map(|fixture| fixture.paths.iter().map(|(path, _)| path.as_str()))
    }

    /// Whether the type's files are its fixtures and nothing else, so the game adds none.
    pub fn closed(&self, table: &str) -> bool {
        self.fixtures
            .iter()
            .any(|fixture| fixture.table == table && fixture.whole_table)
    }

    /// Every folder a fixture sits in under `fixtures/`, by the system that declares it.
    pub fn fixture_folders(&self) -> BTreeSet<String> {
        self.fixtures
            .iter()
            .map(|fixture| fixture_dir(fixture.folder))
            .collect()
    }

    pub fn version(&self) -> ContentVersion {
        self.version
            .get_or_init(|| super::version::compute(self))
            .clone()
    }

    pub fn shapes(&self, settings: &SchemaSettings) -> ContentShapes {
        let tables = self
            .rows
            .iter()
            .map(|row| {
                let fixtures = || {
                    self.fixtures
                        .iter()
                        .filter(|fixture| fixture.table == row.name())
                        .flat_map(|fixture| fixture.paths.iter())
                };
                TableShape {
                    table: row.name().to_owned(),
                    bundle: row.shape() == Shape::Bundle,
                    shape: row.schema(settings),
                    fixtures: fixtures().map(|(path, _)| path.clone()).collect(),
                    locks: fixtures()
                        .filter(|(_, locks)| !locks.is_empty())
                        .map(|(path, locks)| (path.clone(), locks.clone()))
                        .collect(),
                    closed: self
                        .fixtures
                        .iter()
                        .any(|fixture| fixture.table == row.name() && fixture.whole_table),
                }
            })
            .collect();
        ContentShapes {
            tables,
            settings: self.settings.as_ref().map(|loader| loader.shape(settings)),
        }
    }

    /// Whether `file` is one the registry reads references from: the game's settings, a content
    /// file's fields, or a part of a bundle that names content.
    pub fn names_content(&self, file: &str) -> bool {
        file == SETTINGS_FILE
            || self.loader_of_fields(file).is_some()
            || self.json_part_of(file).is_some()
    }

    fn json_part_of<'f>(&self, file: &'f str) -> Option<(&JsonPart, &'f str)> {
        let (bundle, part) = self.bundle_of(file)?;
        let table = table_of(bundle)?;
        self.json_parts
            .iter()
            .find(|json| json.table == table && json.name == part)
            .map(|json| (json, bundle))
    }

    /// The loader of the type whose fields `file` holds: a content file, or a bundle's config.
    fn loader_of_fields(&self, file: &str) -> Option<&dyn TableLoader> {
        let path = match self.bundle_of(file) {
            Some((bundle, CONFIG_FILE)) => bundle,
            Some(_) => return None,
            None if file.ends_with(".json") => file,
            None => return None,
        };
        let table = table_of(path)?;
        self.rows
            .iter()
            .find(|row| row.name() == table)
            .map(Box::as_ref)
    }

    pub fn references(&self, content: &Content, file: &str, text: &str) -> Vec<Reference> {
        let json_part = self.json_part_of(file);
        let parse: &dyn Fn(&serde_json::Value) -> ParseResult = if file == SETTINGS_FILE {
            let Some(loader) = self.settings.as_deref() else {
                return Vec::new();
            };
            &|value| loader.parse_value(value)
        } else if let Some((part, bundle)) = json_part {
            &move |value| (part.read)(content, bundle, value).map(|()| Box::new(()) as _)
        } else {
            let Some(loader) = self.loader_of_fields(file) else {
                return Vec::new();
            };
            &|value| loader.parse_value(value)
        };
        let Ok(value) = parse_json::<serde_json::Value>(text) else {
            return Vec::new();
        };
        let unread = ContentScope::of(content);
        let (_, scope) = with_content(unread.clone(), || parse(&value));
        let index = JsonIndex::of(text);
        let probe = |pointer: &str| reads_reference(&unread, &value, pointer, parse);
        placed(scope.leaves(), &index, |_| true, probe)
            .into_iter()
            .filter_map(|(leaf, found)| {
                let found = found?;
                Some(Reference {
                    kind: leaf.kind,
                    value: leaf.value.clone(),
                    known: leaf.problem.is_none(),
                    pointer: found.pointer.clone(),
                    key: found.key,
                    line: found.line,
                    column: found.column,
                })
            })
            .collect()
    }

    pub(super) fn settings_loader(&self) -> Option<&dyn SettingsLoader> {
        self.settings.as_deref()
    }

    pub(super) fn loader_named(&self, table: &str) -> Option<&dyn TableLoader> {
        self.rows
            .iter()
            .find(|loader| loader.name() == table)
            .map(Box::as_ref)
    }

    pub(super) fn loader_of<Row: ContentFile>(&self) -> Option<&dyn TableLoader> {
        self.rows
            .iter()
            .find(|loader| loader.row_type() == TypeId::of::<Row>())
            .map(Box::as_ref)
    }

    pub(super) fn row_loaders(&self) -> &[Box<dyn TableLoader>] {
        &self.rows
    }

    pub(super) fn folder_writers(&self) -> &[FolderWriter] {
        &self.writers
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub kind: LeafKind,
    pub value: String,
    pub known: bool,
    pub pointer: String,
    pub key: bool,
    pub line: usize,
    pub column: usize,
}

pub trait ContentFolder: Send + Sync {
    fn name(&self) -> String;
    fn entries(&self, dir: &str) -> Vec<FolderEntry>;
    fn read(&self, file: &str) -> io::Result<Vec<u8>>;

    /// The hash of `file` when the folder holds it as this build's loader already checked it
    /// elsewhere, so a load takes its hash and its media as they are rather than working them out
    /// again.
    fn checked(&self, _file: &str) -> Option<FileHash> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
    pub name: String,
    pub dir: bool,
}

pub struct FilesystemFolder(pub PathBuf);

impl ContentFolder for FilesystemFolder {
    fn name(&self) -> String {
        self.0.display().to_string()
    }

    fn entries(&self, dir: &str) -> Vec<FolderEntry> {
        let Ok(read) = std::fs::read_dir(self.0.join(dir)) else {
            return Vec::new();
        };
        read.filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_owned();
                let dir = std::fs::metadata(entry.path()).ok()?.is_dir();
                Some(FolderEntry { name, dir })
            })
            .collect()
    }

    fn read(&self, file: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.0.join(file))
    }
}

pub struct MemoryFolder {
    pub name: String,
    pub files: BTreeMap<String, Arc<[u8]>>,
}

impl ContentFolder for MemoryFolder {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn entries(&self, dir: &str) -> Vec<FolderEntry> {
        entries_in(dir, |from| {
            self.files
                .range::<str, _>((Bound::Included(from), Bound::Unbounded))
                .next()
                .map(|(path, _)| path.clone())
        })
    }

    fn read(&self, file: &str) -> io::Result<Vec<u8>> {
        self.files
            .get(file)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("missing {file}")))
    }
}

/// The entries of `dir` among sorted paths, `next_path` giving the first path at or after one:
/// it seeks from entry to entry, so a folder costs what it holds directly rather than what the
/// whole folder holds.
pub(crate) fn entries_in(
    dir: &str,
    next_path: impl Fn(&str) -> Option<String>,
) -> Vec<FolderEntry> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let mut entries = Vec::new();
    let mut from = prefix.clone();
    while let Some(path) = next_path(&from) {
        let Some(rest) = path.strip_prefix(&prefix) else {
            break;
        };
        match rest.split_once('/') {
            Some((name, _)) => {
                entries.push(FolderEntry {
                    name: name.to_owned(),
                    dir: true,
                });
                // Past every path inside it: '0' is the character after '/'.
                from = format!("{prefix}{name}0");
            }
            None => {
                entries.push(FolderEntry {
                    name: rest.to_owned(),
                    dir: false,
                });
                from = format!("{path}\0");
            }
        }
    }
    entries
}

pub trait Settings: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static {
    fn content_version(&self) -> &ContentVersion;
}

pub fn load(
    registry: &ContentRegistry,
    folder: Arc<dyn ContentFolder>,
    build: &str,
    outcome: &str,
) -> Result<Content, LoadReport> {
    block_on(load_paced(registry, folder, build, outcome, &Pace::whole()))
}

/// Loads the game in `folder` at `pace`: whole, or a slice of every frame where the load must
/// leave the frames to whatever else draws them.
pub async fn load_paced(
    registry: &ContentRegistry,
    folder: Arc<dyn ContentFolder>,
    build: &str,
    outcome: &str,
    pace: &Pace,
) -> Result<Content, LoadReport> {
    let version = registry.version();
    let (listing, errors) = super::files::list_files(folder.as_ref(), &registry.file_types());
    let mut loading = Loading {
        folder: folder.clone(),
        listing: Arc::new(listing),
        builder: ContentBuilder::default(),
        snapshot: None,
        errors,
        warnings: Vec::new(),
        indexes: HashMap::new(),
        failed: HashSet::new(),
        media: Vec::new(),
        pace: pace.clone(),
    };
    loading.parse(registry, None).await;
    loading.check_rows(registry).await;
    loading.run_hooks(registry, LoadStep::Rows).await;
    loading.hash_files().await;
    loading.run_hooks(registry, LoadStep::Files).await;
    loading.run_hooks(registry, LoadStep::Expand).await;
    loading.run_hooks(registry, LoadStep::Assemble).await;
    loading.run_hooks(registry, LoadStep::Across).await;
    loading.check_fixtures(registry);
    if loading.has_settings(registry) {
        loading.run_hooks(registry, LoadStep::Settings).await;
    }
    loading.check_version(registry, &version);
    loading.built(version.clone());
    if loading.errors.is_empty() {
        loading.run_hooks(registry, LoadStep::Compile).await;
    }
    if loading.errors.is_empty() {
        let mut warnings = std::mem::take(&mut loading.warnings);
        warnings.sort_by(|a, b| (&a.file, &a.problem.found).cmp(&(&b.file, &b.problem.found)));
        loading.built(LoadWarnings(warnings));
        Ok(loading.content())
    } else {
        Err(LoadReport::new(
            folder.name(),
            build,
            version.0,
            outcome,
            loading.errors,
        ))
    }
}

/// What a game that loaded costs or leaves unseen, by the file each is about: the loader's warnings,
/// the designer's to accept.
pub struct LoadWarnings(pub Vec<LoadError>);

pub(super) fn check_files(
    registry: &ContentRegistry,
    folder: Arc<dyn ContentFolder>,
    listing: Arc<Listing>,
    listing_errors: &[LoadError],
    files: &[String],
) -> CheckedFiles {
    let touches = |file: &str| {
        files.iter().any(|touched| {
            touched == file
                || file.starts_with(&format!("{touched}/"))
                || touched.starts_with(&format!("{file}/"))
        })
    };
    let contents: HashSet<String> = files
        .iter()
        .filter_map(|file| content_of(&listing, file))
        .collect();
    let parts: Vec<String> = files
        .iter()
        .filter(|file| super::is_media_file(file))
        .filter(|file| listing.bundle_holding(file).is_some())
        .cloned()
        .collect();
    let mut loading = Loading {
        folder,
        listing,
        builder: ContentBuilder::default(),
        snapshot: None,
        errors: listing_errors
            .iter()
            .filter(|error| touches(&error.file))
            .cloned()
            .collect(),
        warnings: Vec::new(),
        indexes: HashMap::new(),
        failed: HashSet::new(),
        media: Vec::new(),
        pace: Pace::whole(),
    };
    block_on(loading.parse(registry, Some(&contents)));
    let misread: HashSet<String> = loading
        .failed
        .iter()
        .map(|(_, path)| path.clone())
        .collect();
    block_on(loading.check_rows(registry));
    let named = loading.media.clone();
    for file in parts.into_iter().chain(named) {
        if let Ok(bytes) = loading.read(&file)
            && let Some(problem) = super::media::problem_of(&file, &bytes)
        {
            loading.file_error(file, ErrorPlace::WholeFile, problem);
        }
    }
    loading.snapshot = None;
    let read = loading
        .builder
        .into_rows()
        .into_iter()
        .filter(|read| !misread.contains(&read.path))
        .collect();
    CheckedFiles {
        errors: loading.errors,
        read,
    }
}

/// What checking files finds: every problem they have on their own, and every content file among
/// them that read whole, as `FolderReader::read` reads it.
pub struct CheckedFiles {
    pub errors: Vec<LoadError>,
    pub read: Vec<ReadRow>,
}

/// The content `file` is, or is a part of, when the listing holds it: a content file, a bundle or
/// the settings.
fn content_of(listing: &Listing, file: &str) -> Option<String> {
    if file == SETTINGS_FILE {
        return Some(file.to_owned());
    }
    let held = listing.bundle_holding(file).unwrap_or(file);
    listing
        .content
        .values()
        .any(|paths| {
            paths
                .binary_search_by(|path| path.as_bytes().cmp(held.as_bytes()))
                .is_ok()
        })
        .then(|| held.to_owned())
}

pub struct Loading {
    folder: Arc<dyn ContentFolder>,
    listing: Arc<Listing>,
    builder: ContentBuilder,
    snapshot: Option<Content>,
    errors: Vec<LoadError>,
    warnings: Vec<LoadError>,
    indexes: HashMap<String, Arc<JsonIndex>>,
    failed: HashSet<(&'static str, String)>,
    /// Every media file outside the bundles that content names, sorted.
    media: Vec<String>,
    pace: Pace,
}

impl Loading {
    /// Lets the frame go when the load's slice of it is used up: a hook breathes between the
    /// pieces of its work.
    pub async fn breathe(&self) {
        self.pace.breathe().await;
    }

    /// `work` on every item, at the load's pace: across every thread at once in a whole load,
    /// one after another with breaths between them in a paced one.
    pub async fn each<T, R>(&self, items: Vec<T>, work: impl Fn(T) -> R + Send + Sync) -> Vec<R>
    where
        T: Send,
        R: Send,
    {
        self.pace.each(items, work).await
    }

    pub fn content(&mut self) -> Content {
        self.snapshot
            .get_or_insert_with(|| self.builder.build())
            .clone()
    }

    pub fn built<T: Any + Send + Sync>(&mut self, value: T) {
        self.snapshot = None;
        self.builder.built(value);
    }

    pub fn error(&mut self, error: LoadError) {
        self.errors.push(error);
    }

    pub fn extend_errors(&mut self, errors: impl IntoIterator<Item = LoadError>) {
        self.errors.extend(errors);
    }

    pub fn file_error(&mut self, file: impl Into<String>, at: ErrorPlace, problem: ContentProblem) {
        self.errors.push(LoadError {
            file: file.into(),
            at,
            problem,
        });
    }

    pub fn json_error(&mut self, file: impl Into<String>, pointer: &str, problem: ContentProblem) {
        let file = file.into();
        let at = self.place(&file, pointer);
        self.file_error(file, at, problem);
    }

    /// Something the game loads with but should know it pays for, at `pointer` in `file`.
    pub fn json_warning(
        &mut self,
        file: impl Into<String>,
        pointer: &str,
        problem: ContentProblem,
    ) {
        let file = file.into();
        let at = self.place(&file, pointer);
        self.warnings.push(LoadError { file, at, problem });
    }

    /// A warning at `pointer` in the file holding the fields of `id`.
    pub fn row_warning<Row: ContentFile>(
        &mut self,
        id: ContentId<Row>,
        pointer: &str,
        problem: ContentProblem,
    ) {
        let file = self.row_file(id);
        self.json_warning(file, pointer, problem);
    }

    /// A problem at `pointer` in the file holding the fields of `id`.
    pub fn row_error<Row: ContentFile>(
        &mut self,
        id: ContentId<Row>,
        pointer: &str,
        problem: ContentProblem,
    ) {
        let file = self.row_file(id);
        self.json_error(file, pointer, problem);
    }

    /// A problem at `pointer` in the JSON part `part` of the bundle `id`.
    pub fn part_error<Row: ContentFile>(
        &mut self,
        id: ContentId<Row>,
        part: &str,
        pointer: &str,
        problem: ContentProblem,
    ) {
        let file = format!("{}/{part}", id.path(&self.content()));
        self.json_error(file, pointer, problem);
    }

    pub fn row_errors<Row: ContentFile>(&mut self, id: ContentId<Row>, check: RowCheck) {
        for error in check.into_errors() {
            self.row_error(id, &error.pointer, error.problem);
        }
    }

    /// The file holding the fields of `id`: the file itself, or its bundle's config.
    pub fn row_file<Row: ContentFile>(&mut self, id: ContentId<Row>) -> String {
        fields_file(id.path(&self.content()))
    }

    pub fn fail<Row: ContentFile>(&mut self, id: ContentId<Row>) {
        let path = id.path(&self.content()).to_owned();
        self.failed.insert((Row::TABLE, path));
    }

    pub fn rows<'c, Row: ContentFile>(
        &self,
        content: &'c Content,
    ) -> Vec<(ContentId<Row>, &'c Row)> {
        content
            .table::<Row>()
            .iter()
            .filter(|(id, _)| {
                !self
                    .failed
                    .contains(&(Row::TABLE, id.path(content).to_owned()))
            })
            .collect()
    }

    pub fn folder(&self) -> Arc<dyn ContentFolder> {
        self.folder.clone()
    }

    pub fn read(&self, file: &str) -> io::Result<Vec<u8>> {
        self.folder.read(file)
    }

    pub fn read_text(&self, file: &str) -> io::Result<String> {
        let bytes = self.read(file)?;
        String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"))
    }

    /// Every media file outside the bundles that content names, by its path from the game folder:
    /// the only ones the game reads.
    pub fn media(&self) -> &[String] {
        &self.media
    }

    /// Every media file kept as a part of a bundle, by its path from the game folder.
    pub fn media_parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = self
            .listing
            .parts
            .iter()
            .flat_map(|(bundle, parts)| {
                parts
                    .iter()
                    .filter(|part| super::is_media_file(part))
                    .map(move |part| format!("{bundle}/{part}"))
            })
            .collect();
        parts.sort_unstable();
        parts
    }

    /// Every part of the bundle at `path`, by its path inside the bundle.
    pub fn parts_of(&self, path: &str) -> &[String] {
        self.listing.parts.get(path).map_or(&[], Vec::as_slice)
    }

    pub fn place(&mut self, file: &str, pointer: &str) -> ErrorPlace {
        match self.index(file).and_then(|index| index.position(pointer)) {
            Some((line, column)) => ErrorPlace::Json {
                line,
                column,
                pointer: pointer.to_owned(),
            },
            None => ErrorPlace::WholeFile,
        }
    }

    fn index(&mut self, file: &str) -> Option<Arc<JsonIndex>> {
        if let Some(index) = self.indexes.get(file) {
            return Some(index.clone());
        }
        let text = self.read_text(file).ok()?;
        let index = Arc::new(JsonIndex::of(&text));
        self.indexes.insert(file.to_owned(), index.clone());
        Some(index)
    }

    /// Parses every content file, or only the content in `only` and the settings when it holds
    /// them.
    async fn parse(&mut self, registry: &ContentRegistry, only: Option<&HashSet<String>>) {
        let scope = listing_scope(registry, &self.listing);
        self.builder.built(self.listing.files());

        let jobs: Vec<(usize, String)> = registry
            .row_loaders()
            .iter()
            .enumerate()
            .flat_map(|(index, loader)| {
                self.listing
                    .paths_of(loader.name())
                    .into_iter()
                    .filter(|path| only.is_none_or(|only| only.contains(path)))
                    .map(move |path| (index, path))
            })
            .collect();
        let root = self.folder.clone();
        let listing = self.listing.clone();
        let parsed: Vec<(usize, String, ParsedFile)> = self
            .each(jobs, |(index, path)| {
                let loader = registry.row_loaders()[index].as_ref();
                let parsed = parse_file(registry, loader, &root, &listing, &scope, &path);
                (index, path, parsed)
            })
            .await;

        let mut rows: Vec<Vec<(String, Box<dyn Any + Send + Sync>)>> =
            registry.row_loaders().iter().map(|_| Vec::new()).collect();
        let mut media: BTreeSet<String> = BTreeSet::new();
        for (loader, path, parsed) in parsed {
            media.extend(parsed.media);
            if let Some(index) = parsed.index {
                self.indexes.insert(fields_file(&path), index);
            }
            self.indexes.extend(parsed.parts);
            if !parsed.errors.is_empty() {
                let table = registry.row_loaders()[loader].name();
                self.failed.insert((table, path.clone()));
            }
            self.errors.extend(parsed.errors);
            if let Some(row) = parsed.row {
                rows[loader].push((path, row));
            }
        }
        for (loader, rows) in registry.row_loaders().iter().zip(rows) {
            loader.finish(
                self.listing.paths_of(loader.name()),
                rows,
                &mut self.builder,
            );
        }
        self.snapshot = None;

        if only.is_some_and(|only| !only.contains(SETTINGS_FILE)) {
            self.media = media.into_iter().collect();
            return;
        }
        let settings = parse_settings(registry, root.as_ref(), &scope);
        media.extend(settings.media);
        self.media = media.into_iter().collect();
        self.errors.extend(settings.errors);
        if let Some(index) = settings.index {
            self.indexes.insert(SETTINGS_FILE.to_owned(), index);
        }
        match settings.row {
            Some(settings) => {
                if let Some(loader) = registry.settings_loader() {
                    loader.store(settings, &mut self.builder);
                }
            }
            None if !self.listing.settings => self.file_error(
                SETTINGS_FILE,
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "missing file",
                    "no such file",
                    format!("the game's settings at {SETTINGS_FILE}"),
                ),
            ),
            None => {}
        }
        self.snapshot = None;
    }

    async fn check_rows(&mut self, registry: &ContentRegistry) {
        let content = self.content();
        let mut errors: Vec<(&'static str, String, RowCheck)> = Vec::new();
        for loader in registry.row_loaders() {
            loader.check_rows(&content, &mut errors);
            self.breathe().await;
        }
        for (table, path, check) in errors {
            let file = fields_file(&path);
            self.failed.insert((table, path));
            for error in check.into_errors() {
                self.json_error(file.clone(), &error.pointer, error.problem);
            }
        }
    }

    async fn hash_files(&mut self) {
        let mut files: Vec<String> = Vec::new();
        if self.listing.settings {
            files.push(SETTINGS_FILE.to_owned());
        }
        for paths in self.listing.content.values() {
            for path in paths {
                match self.listing.parts.get(path) {
                    Some(parts) => files.extend(parts.iter().map(|part| format!("{path}/{part}"))),
                    None => files.push(path.clone()),
                }
            }
        }
        files.extend(self.media.iter().cloned());
        let root = self.folder.clone();
        let hashed: Vec<(String, io::Result<FileHash>)> = self
            .each(files, |file| {
                let hashed = match root.checked(&file) {
                    Some(checked) => Ok(checked),
                    None => root.read(&file).map(|bytes| FileHash::of(&bytes)),
                };
                (file, hashed)
            })
            .await;
        let mut hashes = BTreeMap::new();
        for (file, hashed) in hashed {
            match hashed {
                Ok(hashed) => {
                    hashes.insert(file, hashed);
                }
                Err(error) => self.file_error(
                    file,
                    ErrorPlace::WholeFile,
                    ContentProblem::new("unreadable file", error, "a readable file"),
                ),
            }
        }
        self.built(FileHashes(hashes));
    }

    fn has_settings(&mut self, registry: &ContentRegistry) -> bool {
        registry
            .settings_loader()
            .is_some_and(|loader| loader.built(&self.content()))
    }

    async fn run_hooks(&mut self, registry: &ContentRegistry, step: LoadStep) {
        for (_, hook) in registry.hooks.iter().filter(|(at, _)| *at == step) {
            hook(self).await;
        }
    }

    fn check_fixtures(&mut self, registry: &ContentRegistry) {
        for fixture in &registry.fixtures {
            (fixture.check)(self, registry.fixtures_required);
        }
    }

    fn check_version(&mut self, registry: &ContentRegistry, version: &ContentVersion) {
        let Some(loader) = registry.settings_loader() else {
            return;
        };
        let content = self.content();
        let Some(found) = loader.version(&content) else {
            return;
        };
        if found != version {
            self.json_error(
                SETTINGS_FILE,
                "/content_version",
                ContentProblem::new(
                    VERSION_RULE,
                    format!("{:?}", found.0),
                    format!("{:?}", version.0),
                ),
            );
        }
    }
}

/// What a file held, as every file of a game is told apart: the hex of its BLAKE3 hash, and its
/// size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHash {
    pub hash: String,
    pub size: u64,
}

impl FileHash {
    pub fn of(bytes: &[u8]) -> FileHash {
        FileHash {
            hash: blake3::hash(bytes).to_hex().to_string(),
            size: bytes.len() as u64,
        }
    }
}

pub struct FileHashes(pub BTreeMap<String, FileHash>);

pub(super) struct ParsedFile {
    pub(super) row: Option<Box<dyn Any + Send + Sync>>,
    index: Option<Arc<JsonIndex>>,
    parts: Vec<(String, Arc<JsonIndex>)>,
    pub(super) errors: Vec<LoadError>,
    /// Every media file the file names, as far as it names files there are.
    media: Vec<String>,
}

pub struct ParseError {
    pub pointer: String,
    pub inner: serde_json::Error,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {}: {}", self.pointer, self.inner)
    }
}

impl From<serde_path_to_error::Error<serde_json::Error>> for ParseError {
    fn from(error: serde_path_to_error::Error<serde_json::Error>) -> ParseError {
        ParseError {
            pointer: pointer_of(error.path()),
            inner: error.into_inner(),
        }
    }
}
type ParseResult = Result<Box<dyn Any + Send + Sync>, ParseError>;

/// A file read and parsed as JSON, before any type reads it.
struct ReadFile {
    file: String,
    value: Option<serde_json::Value>,
    index: Option<Arc<JsonIndex>>,
    errors: Vec<LoadError>,
}

/// The scope of every content and media path `listing` holds, which the files it lists are read
/// in.
pub(super) fn listing_scope(registry: &ContentRegistry, listing: &Listing) -> ContentScope {
    let mut scope = ContentScope::new();
    for loader in registry.row_loaders() {
        scope.table_by_type(
            loader.row_type(),
            loader.name(),
            listing.paths_of(loader.name()),
        );
    }
    scope.media(listing.media.clone());
    scope.bundles(registry.bundle_tables());
    scope
}

/// The content file at `path` read and parsed by `loader`, with the parts of its bundle.
pub(super) fn parse_file(
    registry: &ContentRegistry,
    loader: &dyn TableLoader,
    root: &Arc<dyn ContentFolder>,
    listing: &Listing,
    scope: &ContentScope,
    path: &str,
) -> ParsedFile {
    let file = read_file(root.as_ref(), &fields_file(path));
    let mut parsed = parse_read(file, scope, &registry.list_kinds, &|value| {
        loader.parse_value(value)
    });
    if let Some(row) = parsed.row.as_mut() {
        let parts = listing.parts.get(path).map_or(&[][..], Vec::as_slice);
        let mut reader = Parts::new(path, parts, root, scope);
        loader.read_parts(row.as_mut(), &mut reader);
        let read = reader.finish();
        parsed.errors.extend(read.errors);
        parsed.parts.extend(read.indexes);
        parsed.media.extend(read.media);
    }
    parsed
}

/// The game's settings read and parsed by the registry's settings type.
pub(super) fn parse_settings(
    registry: &ContentRegistry,
    root: &dyn ContentFolder,
    scope: &ContentScope,
) -> ParsedFile {
    parse_read(
        read_file(root, SETTINGS_FILE),
        scope,
        &registry.list_kinds,
        &|value| {
            registry
                .settings_loader()
                .map(|loader| loader.parse_value(value))
                .unwrap_or_else(|| Ok(Box::new(())))
        },
    )
}

fn read_file(root: &dyn ContentFolder, file: &str) -> ReadFile {
    let mut read = ReadFile {
        file: file.to_owned(),
        value: None,
        index: None,
        errors: Vec::new(),
    };
    let text = match root.read(file) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                read.errors.push(whole_file(
                    file,
                    "unreadable file",
                    "not UTF-8",
                    "a UTF-8 JSON file",
                ));
                return read;
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => return read,
        Err(error) => {
            read.errors.push(whole_file(
                file,
                "unreadable file",
                error,
                "a readable file",
            ));
            return read;
        }
    };
    let index = Arc::new(JsonIndex::of(&text));
    match parse_json(&text) {
        Ok(value) => read.value = Some(value),
        Err(error) => read.errors.push(shape_error(file, &error, &index, &[])),
    }
    read.index = Some(index);
    read
}

fn parse_read(
    read: ReadFile,
    scope: &ContentScope,
    list_kinds: &[ListKind],
    parse: &dyn Fn(&serde_json::Value) -> ParseResult,
) -> ParsedFile {
    let mut parsed = ParsedFile {
        row: None,
        index: read.index,
        parts: Vec::new(),
        errors: read.errors,
        media: Vec::new(),
    };
    let (Some(value), Some(index)) = (read.value, parsed.index.clone()) else {
        return parsed;
    };
    let (result, read_scope) = with_content(scope.clone(), || parse(&value));
    match result {
        Ok(row) => {
            parsed.row = Some(row);
            parsed.media = named_media(read_scope.leaves());
            let probe = |pointer: &str| reads_reference(scope, &value, pointer, parse);
            parsed.errors.extend(reference_errors(
                &read.file,
                read_scope.leaves(),
                &index,
                probe,
            ));
        }
        Err(error) => parsed
            .errors
            .push(shape_error(&read.file, &error, &index, list_kinds)),
    }
    parsed
}

/// Every media file `leaves` name that is there.
pub(super) fn named_media(leaves: &[Leaf]) -> Vec<String> {
    leaves
        .iter()
        .filter(|leaf| leaf.kind == LeafKind::Asset && leaf.problem.is_none())
        .map(|leaf| leaf.value.clone())
        .collect()
}

fn whole_file(
    file: &str,
    rule: &'static str,
    found: impl Display,
    expected: impl Display,
) -> LoadError {
    LoadError {
        file: file.to_owned(),
        at: ErrorPlace::WholeFile,
        problem: ContentProblem::new(rule, found, expected),
    }
}

pub fn reference_errors(
    file: &str,
    leaves: &[Leaf],
    index: &JsonIndex,
    probe: impl Fn(&str) -> bool,
) -> Vec<LoadError> {
    placed(leaves, index, |leaf| leaf.problem.is_some(), probe)
        .into_iter()
        .filter_map(|(leaf, found)| {
            Some(LoadError {
                file: file.to_owned(),
                at: found.map_or(ErrorPlace::WholeFile, |value| ErrorPlace::Json {
                    line: value.line,
                    column: value.column,
                    pointer: value.pointer.clone(),
                }),
                problem: leaf.reported()?,
            })
        })
        .collect()
}

/// Where each leaf is written: an occurrence of its value. The parse may meet the leaves in another
/// order than the file has them (flattened fields are read in declaration order), so leaves are
/// matched to occurrences per value. When a value is written in more places than it is read as a
/// reference (a script node named like a quest), `probe` tells which places are references. Only
/// the leaves `needed` picks are placed.
fn placed<'a>(
    leaves: &'a [Leaf],
    index: &'a JsonIndex,
    needed: impl Fn(&Leaf) -> bool,
    probe: impl Fn(&str) -> bool,
) -> Vec<(&'a Leaf, Option<&'a IndexedValue>)> {
    if !leaves.iter().any(&needed) {
        return leaves.iter().map(|leaf| (leaf, None)).collect();
    }
    let mut occurrences: HashMap<&'a str, Vec<&'a IndexedValue>> = HashMap::new();
    for value in index.strings() {
        if let Some(string) = value.string.as_deref() {
            occurrences.entry(string).or_default().push(value);
        }
    }
    let mut by_value: HashMap<&'a str, Vec<usize>> = HashMap::new();
    for (at, leaf) in leaves.iter().enumerate() {
        by_value.entry(leaf.value.as_str()).or_default().push(at);
    }
    let mut found: Vec<Option<&'a IndexedValue>> = vec![None; leaves.len()];
    for (value, read) in by_value {
        if !read.iter().any(|&at| needed(&leaves[at])) {
            continue;
        }
        let written = occurrences.remove(value).unwrap_or_default();
        let places = if written.len() > read.len() {
            let references: Vec<&IndexedValue> = written
                .iter()
                .copied()
                .filter(|place| probe(&place.pointer))
                .collect();
            if references.is_empty() {
                written
            } else {
                references
            }
        } else {
            written
        };
        for (at, place) in read.into_iter().zip(places) {
            found[at] = Some(place);
        }
    }
    leaves.iter().zip(found).collect()
}

/// Whether `value`, parsed by `parse`, reads the string at `pointer` as a reference: a stand-in
/// written there comes back as a leaf.
fn reads_reference(
    scope: &ContentScope,
    value: &serde_json::Value,
    pointer: &str,
    parse: &dyn Fn(&serde_json::Value) -> ParseResult,
) -> bool {
    const PROBE: &str = "\u{1}probe";
    let mut probed = value.clone();
    let Some(slot) = probed.pointer_mut(pointer) else {
        return false;
    };
    *slot = serde_json::Value::String(PROBE.to_owned());
    let (_, scope) = with_content(scope.clone(), || parse(&probed));
    scope.leaves().iter().any(|leaf| leaf.value == PROBE)
}

pub fn pointer_of(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let mut pointer = String::new();
    for segment in path.iter() {
        match segment {
            Segment::Seq { index } => {
                pointer.push('/');
                pointer.push_str(&index.to_string());
            }
            Segment::Map { key } => {
                pointer.push('/');
                pointer.push_str(&escape(key));
            }
            Segment::Enum { .. } | Segment::Unknown => {}
        }
    }
    pointer
}

pub(super) fn shape_error(
    file: &str,
    error: &ParseError,
    index: &JsonIndex,
    list_kinds: &[ListKind],
) -> LoadError {
    use serde_json::error::Category;
    let inner = &error.inner;
    let pointer = error.pointer.clone();
    let message = message_of(inner);
    if matches!(
        inner.classify(),
        Category::Syntax | Category::Eof | Category::Io
    ) {
        return LoadError {
            file: file.to_owned(),
            at: ErrorPlace::Json {
                line: inner.line().max(1),
                column: inner.column().max(1),
                pointer: pointer.clone(),
            },
            problem: ContentProblem::new(
                "invalid json",
                message,
                "one JSON object: no comments, no trailing commas",
            ),
        };
    }
    let (line, column) = index
        .position(&pointer)
        .unwrap_or((inner.line().max(1), inner.column().max(1)));
    let at = ErrorPlace::Json {
        line,
        column,
        pointer: pointer.clone(),
    };
    LoadError {
        file: file.to_owned(),
        at,
        problem: classify_shape(&message, &pointer, list_kinds),
    }
}

/// What is wrong with a value that didn't parse as its type.
pub fn shape_problem(error: &ParseError) -> ContentProblem {
    classify_shape(&message_of(&error.inner), &error.pointer, &[])
}

fn message_of(error: &serde_json::Error) -> String {
    let message = error.to_string();
    message
        .split(" at line ")
        .next()
        .unwrap_or(&message)
        .to_owned()
}

fn classify_shape(message: &str, pointer: &str, list_kinds: &[ListKind]) -> ContentProblem {
    if let Some(rest) = message.strip_prefix("missing field `") {
        let field = rest.trim_end_matches('`');
        return ContentProblem::new(
            "missing field",
            format!("no field {field}"),
            format!("{} {field} field", article(field)),
        );
    }
    if let Some(rest) = message.strip_prefix("duplicate field `") {
        let field = rest.trim_end_matches('`');
        return ContentProblem::new(
            "duplicate field",
            format!("{field} twice"),
            format!("{field} once"),
        );
    }
    if let Some(rest) = message.strip_prefix("unknown field `") {
        let (field, options) = split_options(rest);
        let nearest = nearest(&options, field);
        return ContentProblem::new(
            "unknown field",
            format!("{field:?}"),
            expected_options("a field", &options),
        )
        .near(nearest.map(|f| format!("{f:?}")));
    }
    if let Some(rest) = message.strip_prefix("unknown variant `") {
        let (variant, options) = split_options(rest);
        let nearest = nearest(&options, variant);
        let list = pointer
            .rsplit('/')
            .nth(1)
            .and_then(|parent| list_kinds.iter().find(|kind| kind.field == parent));
        return ContentProblem::new(
            list.map_or("unknown variant", |kind| kind.rule),
            format!("{variant:?}"),
            expected_options(list.map_or("a variant", |kind| kind.expected), &options),
        )
        .near(nearest.map(|v| format!("{v:?}")));
    }
    if let Some(rest) = message
        .strip_prefix("invalid type: ")
        .or_else(|| message.strip_prefix("invalid value: "))
        .or_else(|| message.strip_prefix("invalid length "))
    {
        let (found, expected) = rest
            .split_once(", expected ")
            .unwrap_or((rest, "another value"));
        return ContentProblem::new("wrong type", found, expected);
    }
    if let Some(rest) = message.strip_prefix("data did not match any variant of untagged enum ") {
        return ContentProblem::new(
            "wrong type",
            "a value of another shape",
            format!("a {rest} value"),
        );
    }
    ContentProblem::new("wrong type", message, "a value of the field's type")
}

fn split_options(rest: &str) -> (&str, Vec<String>) {
    let (name, tail) = rest.split_once('`').unwrap_or((rest, ""));
    let options = tail
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    (name, options)
}

fn expected_options(what: &str, options: &[String]) -> String {
    if options.is_empty() {
        format!("{what} the type has")
    } else if options.len() <= 12 {
        format!("one of {}", options.join(", "))
    } else {
        format!("{what} the type has ({} of them)", options.len())
    }
}

pub(super) trait TableLoader: Send + Sync {
    fn name(&self) -> &'static str;
    fn row_type(&self) -> TypeId;
    fn shape(&self) -> Shape;
    fn parts(&self) -> &'static [PartSpec];
    fn written_parts(&self) -> &'static [PartSpec];
    fn schema(&self, settings: &SchemaSettings) -> serde_json::Value;
    fn picked_from_table(&self) -> bool;
    fn parse_value(&self, value: &serde_json::Value) -> ParseResult;
    fn read_parts(&self, row: &mut (dyn Any + Send + Sync), parts: &mut Parts);
    fn finish(
        &self,
        keys: Vec<String>,
        rows: Vec<(String, Box<dyn Any + Send + Sync>)>,
        builder: &mut ContentBuilder,
    );
    fn check_rows(&self, content: &Content, errors: &mut Vec<(&'static str, String, RowCheck)>);
    fn write_rows(&self, content: &Content, writer: &mut FileWriter) -> io::Result<()>;
    fn write_row(&self, row: &dyn Any, path: &str, writer: &mut FileWriter) -> io::Result<()>;
    fn fresh_row(&self, near: &Nearby) -> Result<AnyRow, String>;
}

fn shape_of<T: JsonSchema>(settings: &SchemaSettings) -> serde_json::Value {
    settings
        .clone()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

struct RowLoader<Row> {
    parts: Option<BundleParts<Row>>,
    marker: PhantomData<fn() -> Row>,
}

struct BundleParts<Row> {
    specs: &'static [PartSpec],
    written: &'static [PartSpec],
    read: fn(&mut Row, &mut Parts),
    write: fn(&Row, &str, &mut FileWriter) -> io::Result<()>,
}

impl<Row: ContentFile> TableLoader for RowLoader<Row> {
    fn name(&self) -> &'static str {
        Row::TABLE
    }

    fn row_type(&self) -> TypeId {
        TypeId::of::<Row>()
    }

    fn shape(&self) -> Shape {
        Row::SHAPE
    }

    fn parts(&self) -> &'static [PartSpec] {
        self.parts.as_ref().map_or(&[], |parts| parts.specs)
    }

    fn written_parts(&self) -> &'static [PartSpec] {
        self.parts.as_ref().map_or(&[], |parts| parts.written)
    }

    fn schema(&self, settings: &SchemaSettings) -> serde_json::Value {
        shape_of::<Row>(settings)
    }

    fn picked_from_table(&self) -> bool {
        Row::PICKED_FROM_TABLE
    }

    fn parse_value(&self, value: &serde_json::Value) -> ParseResult {
        parse_value::<Row>(value).map(|row| {
            row.read_held();
            Box::new(row) as Box<dyn Any + Send + Sync>
        })
    }

    fn read_parts(&self, row: &mut (dyn Any + Send + Sync), parts: &mut Parts) {
        if let Some(bundle) = &self.parts {
            let row = row
                .downcast_mut::<Row>()
                .expect("a file parsed by its own loader");
            (bundle.read)(row, parts);
        }
    }

    fn finish(
        &self,
        paths: Vec<String>,
        rows: Vec<(String, Box<dyn Any + Send + Sync>)>,
        builder: &mut ContentBuilder,
    ) {
        let mut table = Table::<Row>::of_paths(paths);
        for (path, row) in rows {
            let row = *row
                .downcast::<Row>()
                .expect("a file parsed by its own loader");
            table.insert(&path, row);
        }
        builder.insert_table(table);
    }

    fn check_rows(&self, content: &Content, errors: &mut Vec<(&'static str, String, RowCheck)>) {
        for (id, row) in content.table::<Row>().iter() {
            let mut check = RowCheck::default();
            row.check(&mut check);
            if !check.is_empty() {
                errors.push((Row::TABLE, id.path(content).to_owned(), check));
            }
        }
    }

    fn write_rows(&self, content: &Content, writer: &mut FileWriter) -> io::Result<()> {
        for (id, row) in content.table::<Row>().iter() {
            self.write_row(row, id.path(content), writer)?;
        }
        Ok(())
    }

    fn write_row(&self, row: &dyn Any, path: &str, writer: &mut FileWriter) -> io::Result<()> {
        let row = row
            .downcast_ref::<Row>()
            .expect("a file written by its own loader");
        writer.json(&fields_file(path), row)?;
        if let Some(bundle) = &self.parts {
            (bundle.write)(row, path, writer)?;
        }
        Ok(())
    }

    fn fresh_row(&self, near: &Nearby) -> Result<AnyRow, String> {
        Ok(Box::new(Row::fresh(near)?))
    }
}

fn no_modules(table: &str) -> String {
    format!("{} {table} files hold no modules", article(table))
}

// Tracking the path of every value read makes each read about half as slow again, so a read is
// tracked only when made again to find where it failed.
pub fn parse_value<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, ParseError> {
    let met = super::id::leaves_met();
    T::deserialize(value).or_else(|_| {
        super::id::forget_leaves_after(met);
        Ok(serde_path_to_error::deserialize(value)?)
    })
}

pub fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, ParseError> {
    let met = super::id::leaves_met();
    serde_json::from_str(text).or_else(|_| {
        super::id::forget_leaves_after(met);
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value: T = serde_path_to_error::deserialize(&mut deserializer)?;
        deserializer.end().map_err(|inner| ParseError {
            pointer: String::new(),
            inner,
        })?;
        Ok(value)
    })
}

pub(super) trait SettingsLoader: Send + Sync {
    fn parse_value(&self, value: &serde_json::Value) -> ParseResult;
    fn shape(&self, settings: &SchemaSettings) -> serde_json::Value;
    fn store(&self, settings: Box<dyn Any + Send + Sync>, builder: &mut ContentBuilder);
    fn built(&self, content: &Content) -> bool;
    fn version<'a>(&self, content: &'a Content) -> Option<&'a ContentVersion>;
    fn write(&self, content: &Content, writer: &mut FileWriter) -> io::Result<()>;
    fn write_value(&self, settings: &dyn Any, writer: &mut FileWriter) -> io::Result<()>;
}

struct SettingsOf<G>(PhantomData<fn() -> G>);

impl<G: Settings> SettingsLoader for SettingsOf<G> {
    fn shape(&self, settings: &SchemaSettings) -> serde_json::Value {
        shape_of::<G>(settings)
    }

    fn parse_value(&self, value: &serde_json::Value) -> ParseResult {
        parse_value::<G>(value).map(|settings| Box::new(settings) as Box<dyn Any + Send + Sync>)
    }

    fn store(&self, settings: Box<dyn Any + Send + Sync>, builder: &mut ContentBuilder) {
        let settings = *settings
            .downcast::<G>()
            .expect("settings parsed by their own loader");
        builder.built(settings);
    }

    fn built(&self, content: &Content) -> bool {
        content.has_built::<G>()
    }

    fn version<'a>(&self, content: &'a Content) -> Option<&'a ContentVersion> {
        self.built(content)
            .then(|| content.built::<G>().content_version())
    }

    fn write(&self, content: &Content, writer: &mut FileWriter) -> io::Result<()> {
        if self.built(content) {
            self.write_value(content.built::<G>(), writer)?;
        }
        Ok(())
    }

    fn write_value(&self, settings: &dyn Any, writer: &mut FileWriter) -> io::Result<()> {
        let settings = settings
            .downcast_ref::<G>()
            .expect("settings written by their own loader");
        writer.json(SETTINGS_FILE, settings)
    }
}

async fn module_rules<Row: ModuleSet>(loading: &mut Loading) {
    let content = loading.content();
    let mut errors: Vec<(ContentId<Row>, usize, String)> = Vec::new();
    with_content(ContentScope::of(&content), || {
        for (id, row) in content.table::<Row>().iter() {
            let mut seen = Vec::new();
            for (index, module) in row.modules().iter().enumerate() {
                if Row::rule(module) != ModuleRule::AtMostOne {
                    continue;
                }
                let kind = std::mem::discriminant(module);
                if seen.contains(&kind) {
                    errors.push((id, index, TypeTag::of(module).unwrap_or_default()));
                } else {
                    seen.push(kind);
                }
            }
        }
    });
    for (id, index, tag) in errors {
        loading.row_error(
            id,
            &format!("/{MODULES}/{index}"),
            ContentProblem::new(
                "too many",
                format!("a second {tag} module"),
                format!(
                    "at most one {tag} module on {} {} row",
                    article(Row::TABLE),
                    Row::TABLE
                ),
            ),
        );
    }
}

pub const KEY_RULE: &str =
    "1 to 64 of ASCII letters, digits, _ and -, not starting with -, not a Windows device name";

pub fn valid_key(key: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    (1..=64).contains(&key.len())
        && !key.starts_with('-')
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !DEVICES
            .iter()
            .any(|device| device.eq_ignore_ascii_case(key))
}

pub struct FileWriter {
    into: WrittenInto,
    written: Vec<String>,
    kept: usize,
}

enum WrittenInto {
    Folder(PathBuf),
    Memory(BTreeMap<String, Vec<u8>>),
}

impl FileWriter {
    pub fn new(root: &Path) -> FileWriter {
        FileWriter {
            into: WrittenInto::Folder(root.to_path_buf()),
            written: Vec::new(),
            kept: 0,
        }
    }

    /// A writer that keeps what it writes, for [`FileWriter::into_files`].
    pub fn in_memory() -> FileWriter {
        FileWriter {
            into: WrittenInto::Memory(BTreeMap::new()),
            written: Vec::new(),
            kept: 0,
        }
    }

    /// Every file written in memory, by its path from the game folder.
    pub fn into_files(self) -> BTreeMap<String, Vec<u8>> {
        match self.into {
            WrittenInto::Memory(files) => files,
            WrittenInto::Folder(_) => BTreeMap::new(),
        }
    }

    pub fn json<T: Serialize>(&mut self, file: &str, value: &T) -> io::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        self.file(file, &bytes)
    }

    /// A file shared with another folder through a symlink is written where the link points, so the
    /// one copy changes rather than the link turning into a copy of its own.
    pub fn file(&mut self, file: &str, bytes: &[u8]) -> io::Result<()> {
        let root = match &mut self.into {
            WrittenInto::Folder(root) => root,
            WrittenInto::Memory(files) => {
                files.insert(file.to_owned(), bytes.to_vec());
                self.written.push(file.to_owned());
                return Ok(());
            }
        };
        let linked = root.join(file);
        let path = std::fs::canonicalize(&linked).unwrap_or(linked);
        if std::fs::read(&path).is_ok_and(|current| current == bytes) {
            self.kept += 1;
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &path)?;
        self.written.push(file.to_owned());
        Ok(())
    }

    pub fn written(&self) -> &[String] {
        &self.written
    }

    pub fn kept(&self) -> usize {
        self.kept
    }
}

pub fn write(registry: &ContentRegistry, content: Content, root: &Path) -> io::Result<FileWriter> {
    let mut writer = FileWriter::new(root);
    let result = with_content(ContentScope::of(&content), || {
        if let Some(settings) = registry.settings_loader() {
            settings.write(&content, &mut writer)?;
        }
        for loader in registry.row_loaders() {
            loader.write_rows(&content, &mut writer)?;
        }
        for folder in registry.folder_writers() {
            folder(&content, &mut writer)?;
        }
        Ok::<(), io::Error>(())
    })
    .0;
    result.map(|()| writer)
}

/// Every fixture of `F` is at its path, keeping what the engine reads from it, and a type whose
/// files are all fixtures holds no other file.
fn check_fixture<F: Fixture>(loading: &mut Loading, required: bool) {
    let content = loading.content();
    let table = F::Row::TABLE;
    let mut ids = HashMap::new();
    for fixture in F::VARIANTS {
        let path = fixture.path();
        let Some(id) = content.table::<F::Row>().id_of(&path) else {
            if !required {
                continue;
            }
            loading.file_error(
                path.clone(),
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "missing fixture",
                    format!("no {table} {} at {path}", fixture.name()),
                    format!("the {table} the engine requires at {path}"),
                ),
            );
            continue;
        };
        ids.insert(fixture.name(), id);
        if !required {
            continue;
        }
        let Ok(row) = serde_json::to_value(id.get(&content)) else {
            continue;
        };
        let mut check = RowCheck::default();
        for lock in fixture.locks() {
            lock.check(&row, &mut check);
        }
        loading.row_errors(id, check);
    }
    loading.built(FixtureIds::<F>(ids));
    if !required || !F::WHOLE_TABLE {
        return;
    }
    let fixed: Vec<String> = F::VARIANTS.iter().map(|fixture| fixture.path()).collect();
    for (id, _) in content.table::<F::Row>().iter() {
        let path = id.path(&content);
        if !fixed.iter().any(|fixed| fixed == path) {
            loading.file_error(
                path,
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "file the engine never reads",
                    path,
                    format!(
                        "only the {table} fixtures the engine requires, under {}/",
                        fixture_dir(F::FOLDER)
                    ),
                ),
            );
        }
    }
}
