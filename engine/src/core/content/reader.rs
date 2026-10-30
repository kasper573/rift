use std::any::Any;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use super::files::{Listing, list_files, table_of};
use super::fresh::Nearby;
use super::id::{ContentId, ContentScope, with_content};
use super::load::{
    CheckedFiles, ContentFolder, ContentRegistry, FileWriter, Settings, TableLoader, check_files,
    listing_scope, parse_file, parse_settings, parse_value,
};
use super::report::{ContentProblem, ErrorPlace, LoadError, MISSING_FILE};
use super::table::ContentFile;
use serde::de::DeserializeOwned;

/// A game folder read one content file at a time, each exactly as the loader reads it, against
/// every path the folder holds: how a tool that edits a file reads it, and writes it back as the
/// loader would read it again.
pub struct FolderReader {
    registry: &'static ContentRegistry,
    folder: Arc<dyn ContentFolder>,
    listed: Arc<Listed>,
}

/// What the folder's paths say it holds: worked out once for every reader of the same paths.
struct Listed {
    listing: Arc<Listing>,
    errors: Vec<LoadError>,
    scope: ContentScope,
}

/// What writing a content file makes of the folder: the files written, by their path from the
/// game folder, and the parts of its bundle no longer written.
#[derive(Debug, Default)]
pub struct WrittenFiles {
    pub files: BTreeMap<String, Vec<u8>>,
    pub removed: Vec<String>,
}

/// A file that can't be read as its type: its errors, as loading the folder would report them.
pub type Unreadable = Vec<LoadError>;

impl FolderReader {
    pub fn new(registry: &'static ContentRegistry, folder: Arc<dyn ContentFolder>) -> FolderReader {
        let (listing, errors) = list_files(folder.as_ref(), &registry.file_types());
        let scope = listing_scope(registry, &listing);
        FolderReader {
            registry,
            folder,
            listed: Arc::new(Listed {
                listing: Arc::new(listing),
                errors,
                scope,
            }),
        }
    }

    /// What reads `folder`, which holds the very paths this reader's folder holds with other bytes,
    /// without listing it again: listing a whole folder costs far more than an edit to a file's
    /// fields does.
    pub fn rereading(&self, folder: Arc<dyn ContentFolder>) -> FolderReader {
        FolderReader {
            registry: self.registry,
            folder,
            listed: self.listed.clone(),
        }
    }

    /// The problems the files at `files` have on their own, as a load reports them: what their
    /// names say, the shape and references of the content each belongs to, that content's parts
    /// and the rules of its type, and whether the media they name and the bundle parts among them
    /// decode. Only what the rest of the game
    /// says of them is left to a whole load, so whatever writes content can ask before it writes.
    pub fn check(&self, files: &[String]) -> CheckedFiles {
        check_files(
            self.registry,
            self.folder.clone(),
            self.listed.listing.clone(),
            &self.listed.errors,
            files,
        )
    }

    /// The content file at `path`, its fields and the parts of its bundle, when it reads whole:
    /// parsed, with every reference naming a file the folder holds.
    pub fn read<Row: ContentFile>(&self, path: &str) -> Result<Row, Unreadable> {
        self.read_in(path, &self.listed.scope)
    }

    /// The content file at `path` read as `read` reads it, its references resolved among the
    /// paths of `scope` instead: a file read to run against content loaded apart from this folder,
    /// which may hold other paths.
    pub fn read_in<Row: ContentFile>(
        &self,
        path: &str,
        scope: &ContentScope,
    ) -> Result<Row, Unreadable> {
        let Some(loader) = self.registry.loader_of::<Row>() else {
            return Err(vec![unknown_type(path, Row::TABLE)]);
        };
        if self.listed.scope.id_of::<Row>(path).is_none() || scope.id_of::<Row>(path).is_none() {
            return Err(vec![missing(path, Row::TABLE)]);
        }
        let parsed = parse_file(
            self.registry,
            loader,
            &self.folder,
            &self.listed.listing,
            scope,
            path,
        );
        whole(parsed.row, parsed.errors, path)
    }

    /// `json` read as a `T`, its references resolved among the folder's paths: the fields of a file
    /// about to be made, or a value kept apart from the files.
    pub fn from_json<T: DeserializeOwned>(&self, json: &serde_json::Value) -> Result<T, String> {
        self.within(|| parse_value::<T>(json).map_err(|error| error.to_string()))
    }

    /// `value` as JSON, its ids written as the paths they stand for among the folder's.
    pub fn to_json<T: serde::Serialize>(&self, value: &T) -> Result<serde_json::Value, String> {
        with_content(self.listed.scope.clone(), || super::serial::json_of(value))
            .0
            .map_err(|error| error.to_string())
    }

    /// The modules of `row`, a file of `table` as JSON, each as its type writes it.
    pub fn modules_in(
        &self,
        table: &str,
        row: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.within(|| self.registry.modules_of_json(table, row))
    }

    /// `row`, a file of `table` as JSON, holding `modules` in place of its own: each read as its
    /// type, so a module that doesn't read is refused.
    pub fn with_modules_in(
        &self,
        table: &str,
        row: &serde_json::Value,
        modules: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        self.within(|| self.registry.with_modules_json(table, row, modules))
    }

    /// The game's settings, when they read whole.
    pub fn settings<G: Settings>(&self) -> Result<G, Unreadable> {
        let parsed = parse_settings(self.registry, self.folder.as_ref(), &self.listed.scope);
        whole(parsed.row, parsed.errors, super::SETTINGS_FILE)
    }

    /// Every file `row` is kept in as the content file at `path`, by its path from the game
    /// folder: its fields, and the parts its type writes. The parts of the type the folder holds
    /// for it that it no longer writes are in `removed`.
    pub fn write<Row: ContentFile>(&self, path: &str, row: &Row) -> io::Result<WrittenFiles> {
        let loader = self.registry.loader_of::<Row>().ok_or_else(|| {
            io::Error::other(format!("no {} files in this game's registry", Row::TABLE))
        })?;
        let files = self.write_with(|writer| loader.write_row(row, path, writer))?;
        let removed = self.parts_left(loader, path, &files);
        Ok(WrittenFiles { files, removed })
    }

    /// The files of the content file at `path` once every path `renames` names as a key is
    /// written as its value: the file read as its type reads it, each of those paths still read
    /// where it was, and written back as the game writes it.
    pub fn rewritten(
        &self,
        path: &str,
        renames: &BTreeMap<String, String>,
    ) -> Result<WrittenFiles, String> {
        let loader = table_of(path)
            .and_then(|table| self.registry.loader_named(table))
            .ok_or_else(|| format!("{path} is no content file this game registers"))?;
        let scope = self.listed.scope.renaming(renames);
        let parsed = parse_file(
            self.registry,
            loader,
            &self.folder,
            &self.listed.listing,
            &scope,
            path,
        );
        let row = read_whole(parsed.row, parsed.errors, path).map_err(|errors| told(&errors))?;
        let files = write_in(&scope, |writer| {
            loader.write_row(row.as_ref(), path, writer)
        })
        .map_err(|error| error.to_string())?;
        let removed = self.parts_left(loader, path, &files);
        Ok(WrittenFiles { files, removed })
    }

    /// The game's settings file once every path `renames` names as a key is written as its
    /// value, as `rewritten` does a content file.
    pub fn rewritten_settings(
        &self,
        renames: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let loader = self
            .registry
            .settings_loader()
            .ok_or("no settings in this game's registry")?;
        let scope = self.listed.scope.renaming(renames);
        let parsed = parse_settings(self.registry, self.folder.as_ref(), &scope);
        let settings = read_whole(parsed.row, parsed.errors, super::SETTINGS_FILE)
            .map_err(|errors| told(&errors))?;
        write_in(&scope, |writer| {
            loader.write_value(settings.as_ref(), writer)
        })
        .map_err(|error| error.to_string())
    }

    pub fn write_settings<G: Settings>(
        &self,
        settings: &G,
    ) -> io::Result<BTreeMap<String, Vec<u8>>> {
        let loader = self
            .registry
            .settings_loader()
            .ok_or_else(|| io::Error::other("no settings in this game's registry"))?;
        self.write_with(|writer| loader.write_value(settings, writer))
    }

    /// What `read` makes of JSON with its references resolved among the folder's paths, refused
    /// when one names no file the folder holds.
    fn within<T>(&self, read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let (read, scope) = with_content(self.listed.scope.clone(), read);
        let read = read?;
        match scope.leaves().iter().find_map(|leaf| leaf.reported()) {
            Some(problem) => Err(problem.to_string()),
            None => Ok(read),
        }
    }

    /// The paths every reference read here resolves among.
    pub fn scope(&self) -> &ContentScope {
        &self.listed.scope
    }

    pub fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> Option<&str> {
        self.listed.scope.path_of(id)
    }

    pub fn id_of<Row: ContentFile>(&self, path: &str) -> Option<ContentId<Row>> {
        self.listed.scope.id_of(path)
    }

    pub fn paths_of<Row: ContentFile>(&self) -> &[String] {
        self.listed.scope.paths_of::<Row>()
    }

    /// The pixels of the image at `path`, when the folder holds one that decodes.
    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        let bytes = self.folder.read(path).ok()?;
        super::media::image_size(path, None, &bytes)
    }

    /// The files a fresh content file of `table` at `path` is kept in, written as the game writes
    /// them.
    pub fn fresh(&self, table: &str, path: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let loader = self
            .registry
            .loader_named(table)
            .ok_or_else(|| format!("no {table} files in this game's registry"))?;
        let row = self
            .registry
            .fresh_row(loader, path, &Nearby::new(self, path))?;
        self.write_with(|writer| loader.write_row(row.as_ref(), path, writer))
            .map_err(|error| error.to_string())
    }

    pub(super) fn media(&self) -> &[String] {
        &self.listed.listing.media
    }

    fn write_with(
        &self,
        write: impl FnOnce(&mut FileWriter) -> io::Result<()>,
    ) -> io::Result<BTreeMap<String, Vec<u8>>> {
        write_in(&self.listed.scope, write)
    }

    /// The parts the folder holds for the content file at `path` that `files` no longer writes.
    fn parts_left(
        &self,
        loader: &dyn TableLoader,
        path: &str,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Vec<String> {
        let written = loader.written_parts();
        self.listed
            .listing
            .parts
            .get(path)
            .into_iter()
            .flatten()
            .filter(|part| written.iter().any(|spec| spec.matches(part)))
            .map(|part| format!("{path}/{part}"))
            .filter(|file| !files.contains_key(file))
            .collect()
    }
}

fn write_in(
    scope: &ContentScope,
    write: impl FnOnce(&mut FileWriter) -> io::Result<()>,
) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut writer = FileWriter::in_memory();
    with_content(scope.clone(), || write(&mut writer)).0?;
    Ok(writer.into_files())
}

fn whole<T: Any>(
    row: Option<Box<dyn Any + Send + Sync>>,
    errors: Vec<LoadError>,
    path: &str,
) -> Result<T, Unreadable> {
    Ok(*read_whole(row, errors, path)?
        .downcast::<T>()
        .expect("a file parsed by its own loader"))
}

fn read_whole(
    row: Option<Box<dyn Any + Send + Sync>>,
    errors: Vec<LoadError>,
    path: &str,
) -> Result<Box<dyn Any + Send + Sync>, Unreadable> {
    if !errors.is_empty() {
        return Err(errors);
    }
    row.ok_or_else(|| {
        vec![LoadError {
            file: path.to_owned(),
            at: ErrorPlace::WholeFile,
            problem: ContentProblem::new(MISSING_FILE, "no such file", "a file to read"),
        }]
    })
}

/// What keeps a file from reading, in a line: its first problem and where.
fn told(errors: &Unreadable) -> String {
    match errors.first() {
        Some(error) => format!("{} does not read: {}", error.file, error.problem),
        None => "it does not read".to_owned(),
    }
}

fn missing(path: &str, table: &str) -> LoadError {
    LoadError {
        file: path.to_owned(),
        at: ErrorPlace::WholeFile,
        problem: ContentProblem::new(MISSING_FILE, "no such file", format!("a {table} file")),
    }
}

fn unknown_type(path: &str, table: &str) -> LoadError {
    LoadError {
        file: path.to_owned(),
        at: ErrorPlace::WholeFile,
        problem: ContentProblem::new(
            "unknown type",
            format!("a {table} file"),
            "a type this game registers",
        ),
    }
}
