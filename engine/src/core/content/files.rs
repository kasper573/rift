use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::core::assets::AssetKind;

use super::bundle::PartSpec;
use super::id::nearest;
use super::load::{ContentFolder, FolderEntry};
use super::report::{ContentProblem, ErrorPlace, LoadError, MISSING_FILE};

/// The game's own settings: the one content file kept at a path the engine names outright.
pub const SETTINGS_FILE: &str = "fixtures/game.config.json";

/// The folder fixtures declared under `folder` sit in: `fixtures/<folder>`, or `fixtures/` itself.
pub fn fixture_dir(folder: Option<&str>) -> String {
    match folder {
        Some(folder) => format!("{FIXTURES_DIR}/{folder}"),
        None => FIXTURES_DIR.to_owned(),
    }
}

/// Where the engine looks for the settings and for every fixture, each under the folder of the
/// system that declares it.
pub const FIXTURES_DIR: &str = "fixtures";

/// A bundle's own fields, beside the parts it keeps.
pub const CONFIG_FILE: &str = "config.json";

pub const NOTE_EXTENSION: &str = "md";

pub const NAME_RULE: &str = "1 to 64 of a to z, 0 to 9, _ and -, not starting with -, maybe \
                             numbered as \" (1)\", never a Windows device name";

/// How a content file of a type is kept: one `<name>.<type>.json`, or a bundle folder
/// `<name>.<type>/` of its `config.json` and the parts its type declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Json,
    Bundle,
}

/// What a file is, told from its name and where it sits alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameFile {
    Settings,
    Content {
        table: &'static str,
        shape: Shape,
    },
    Media(AssetKind),
    Note,
    /// A file the game never reads, such as a seller's readme or an archive it came in.
    Other,
}

/// A type the folder may hold content of: its table, how its files are kept and, for a bundle, the
/// parts it keeps.
#[derive(Clone, Copy, Debug)]
pub struct FileType {
    pub table: &'static str,
    pub shape: Shape,
    pub parts: &'static [PartSpec],
}

/// The path of every content file and media file of a folder, from the game folder: a bundle by
/// its folder, a media file outside every bundle by its file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameFiles {
    pub content: BTreeSet<String>,
    pub media: BTreeSet<String>,
}

/// Everything a walk of the folder found, sorted by kind.
#[derive(Default)]
pub struct Listing {
    pub settings: bool,
    /// Every content file's path, by table, sorted bytewise.
    pub content: BTreeMap<&'static str, Vec<String>>,
    /// Every part of every bundle, by the bundle's path, each path from inside the bundle.
    pub parts: HashMap<String, Vec<String>>,
    pub media: Vec<String>,
    pub notes: Vec<String>,
    pub other: Vec<String>,
}

impl Listing {
    pub fn files(&self) -> GameFiles {
        GameFiles {
            content: self.content.values().flatten().cloned().collect(),
            media: self.media.iter().cloned().collect(),
        }
    }

    pub fn paths_of(&self, table: &str) -> Vec<String> {
        self.content.get(table).cloned().unwrap_or_default()
    }

    /// The bundle the listing holds that `file` is a part of, when it is one.
    pub fn bundle_holding(&self, file: &str) -> Option<&str> {
        file.match_indices('/')
            .map(|(end, _)| &file[..end])
            .find_map(|folder| {
                self.parts
                    .get_key_value(folder)
                    .map(|(path, _)| path.as_str())
            })
    }
}

/// What `name`, the last part of `path`, says it is. A name that says nothing the game reads is
/// another file: it stays in the folder, and the game never looks at it.
pub fn classify(
    path: &str,
    dir: bool,
    types: &[FileType],
) -> Result<Option<GameFile>, ContentProblem> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.starts_with('.') {
        return Ok(None);
    }
    let (stem, rest) = name
        .split_once('.')
        .map_or((name, None), |(stem, rest)| (stem, Some(rest)));
    if dir {
        let bundle = rest.and_then(|table| {
            types
                .iter()
                .find(|ty| ty.table == table && ty.shape == Shape::Bundle)
        });
        return match bundle {
            Some(ty) => content_named(stem, name, ty, Shape::Bundle),
            None => Ok(None),
        };
    }
    if path == SETTINGS_FILE {
        return Ok(Some(GameFile::Settings));
    }
    if Some(name) == SETTINGS_FILE.rsplit('/').next() {
        return Err(misplaced_settings(name));
    }
    // Nothing names a note or fetches it by URL, so its name is free, as README.md is.
    if rest == Some(NOTE_EXTENSION) {
        return Ok(Some(GameFile::Note));
    }
    if let Some(table) = rest
        .and_then(|rest| rest.strip_suffix(".json"))
        .filter(|table| !table.is_empty() && !table.contains('.'))
    {
        return match types.iter().find(|ty| ty.table == table) {
            Some(ty) if ty.shape == Shape::Json => content_named(stem, name, ty, Shape::Json),
            Some(ty) => Err(ContentProblem::new(
                "bundle kept as one file",
                format!("file {name}"),
                format!(
                    "a folder {stem}.{}/ holding {CONFIG_FILE}: {} files are bundles",
                    ty.table, ty.table
                ),
            )),
            None => Err(unknown_type(name, table, types, Shape::Json)),
        };
    }
    match AssetKind::of(name) {
        Some(_) if name.contains('#') => Err(ContentProblem::new(
            "invalid name",
            format!("{name:?}"),
            "a media file named without #, which a path to it reads as where its name ends",
        )),
        Some(kind) => Ok(Some(GameFile::Media(kind))),
        None => Ok(Some(GameFile::Other)),
    }
}

fn content_named(
    stem: &str,
    name: &str,
    ty: &FileType,
    shape: Shape,
) -> Result<Option<GameFile>, ContentProblem> {
    if !valid_name(stem) {
        return Err(ContentProblem::new(
            "invalid name",
            format!("{name:?}"),
            NAME_RULE,
        ));
    }
    Ok(Some(GameFile::Content {
        table: ty.table,
        shape,
    }))
}

fn misplaced_settings(name: &str) -> ContentProblem {
    ContentProblem::new(
        "unknown file",
        format!("file {name}"),
        format!("the game's settings at {SETTINGS_FILE}"),
    )
}

fn unknown_type(name: &str, table: &str, types: &[FileType], shape: Shape) -> ContentProblem {
    let known: Vec<String> = types
        .iter()
        .filter(|ty| ty.shape == shape)
        .map(|ty| ty.table.to_owned())
        .collect();
    ContentProblem::new(
        "unknown type",
        format!("{name}: {table:?}"),
        "a type the game has content of",
    )
    .near(nearest(&known, table).map(|near| format!("{near:?}")))
}

/// Whether `name`, a file's name before its first dot or a folder's whole name, is one a disk and
/// a URL both keep as it is.
pub fn valid_name(name: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let base = numbered(name).map_or(name, |(base, _)| base);
    (1..=64).contains(&name.len())
        && !base.is_empty()
        && !base.starts_with('-')
        && base
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !DEVICES.contains(&base)
}

/// `name` split into what it numbers and its number, when it ends in " (n)".
pub fn numbered(name: &str) -> Option<(&str, u32)> {
    let (base, number) = name.strip_suffix(')')?.rsplit_once(" (")?;
    let valid = !number.starts_with('0') && number.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| number.parse().ok().map(|number| (base, number)))?
}

/// The name a content file or media file goes by: its file name before the first dot.
pub fn name_of(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.split_once('.').map_or(file, |(name, _)| name)
}

/// The folder holding `path`, empty at the top of the game folder.
pub fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// The file a content file's own fields are read from: the file itself, or a bundle's config.
pub fn fields_file(path: &str) -> String {
    if path.ends_with(".json") {
        path.to_owned()
    } else {
        format!("{path}/{CONFIG_FILE}")
    }
}

/// The path of the content file of `table` named `name` in `folder`.
pub fn content_path(folder: &str, name: &str, table: &str, shape: Shape) -> String {
    let file = match shape {
        Shape::Json => format!("{name}.{table}.json"),
        Shape::Bundle => format!("{name}.{table}"),
    };
    match folder {
        "" => file,
        folder => format!("{folder}/{file}"),
    }
}

/// The type a content path names in its file name.
pub fn table_of(path: &str) -> Option<&str> {
    let file = path.rsplit('/').next()?;
    let rest = file.split_once('.')?.1;
    Some(rest.strip_suffix(".json").unwrap_or(rest))
}

/// The bundle `file` sits in, and its path inside the bundle, when it is a part of one: the first
/// folder on its path named as a bundle of a type among `bundles`, the bundle types' tables.
pub fn bundle_of<'f>(file: &'f str, bundles: &[&str]) -> Option<(&'f str, &'f str)> {
    let mut at = 0;
    for segment in file.split('/') {
        let end = at + segment.len();
        if end == file.len() {
            return None;
        }
        if segment
            .split_once('.')
            .is_some_and(|(_, table)| bundles.contains(&table))
        {
            return Some((&file[..end], &file[end + 1..]));
        }
        at = end + 1;
    }
    None
}

/// The table of every type among `types` kept as a bundle.
pub fn bundle_tables(types: &[FileType]) -> Vec<&'static str> {
    types
        .iter()
        .filter(|ty| ty.shape == Shape::Bundle)
        .map(|ty| ty.table)
        .collect()
}

/// Walks the whole folder, sorting every file by what its name says, and reports every file and
/// folder that says nothing it may hold, every case clash, and every bundle's missing or stray
/// parts.
pub fn list_files(folder: &dyn ContentFolder, types: &[FileType]) -> (Listing, Vec<LoadError>) {
    let mut listing = Listing::default();
    let mut errors = Vec::new();
    walk(folder, "", types, &mut listing, &mut errors);
    for paths in listing.content.values_mut() {
        paths.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    }
    listing.media.sort_unstable();
    listing.notes.sort_unstable();
    listing.other.sort_unstable();
    (listing, errors)
}

fn walk(
    folder: &dyn ContentFolder,
    dir: &str,
    types: &[FileType],
    listing: &mut Listing,
    errors: &mut Vec<LoadError>,
) {
    let entries = entries(folder, dir);
    clashes(dir, &entries, errors);
    for entry in entries {
        let path = join(dir, &entry.name);
        match classify(&path, entry.dir, types) {
            Ok(None) if entry.dir => walk(folder, &path, types, listing, errors),
            Ok(None) => {}
            Ok(Some(GameFile::Settings)) => listing.settings = true,
            Ok(Some(GameFile::Note)) => listing.notes.push(path),
            Ok(Some(GameFile::Other)) => listing.other.push(path),
            Ok(Some(GameFile::Media(_))) => listing.media.push(path),
            Ok(Some(GameFile::Content { table, shape })) => {
                if shape == Shape::Bundle {
                    let declared = types
                        .iter()
                        .find(|ty| ty.table == table)
                        .map_or(&[][..], |ty| ty.parts);
                    let parts = bundle_parts(folder, &path, declared, errors);
                    listing.parts.insert(path.clone(), parts);
                }
                listing.content.entry(table).or_default().push(path);
            }
            Err(problem) => errors.push(LoadError {
                file: path,
                at: ErrorPlace::WholeFile,
                problem,
            }),
        }
    }
}

/// Every file of the bundle at `path`, from inside it, reporting a missing config and every file
/// its type doesn't declare.
fn bundle_parts(
    folder: &dyn ContentFolder,
    path: &str,
    declared: &[PartSpec],
    errors: &mut Vec<LoadError>,
) -> Vec<String> {
    let mut files = Vec::new();
    every_file(folder, path, "", &mut files, errors);
    if !files.iter().any(|file| file == CONFIG_FILE) {
        errors.push(LoadError {
            file: path.to_owned(),
            at: ErrorPlace::WholeFile,
            problem: ContentProblem::new(
                MISSING_FILE,
                format!("no {CONFIG_FILE} in {path}/"),
                format!("{CONFIG_FILE} holding the bundle's fields"),
            ),
        });
    }
    files.retain(|file| {
        let known = file == CONFIG_FILE || declared.iter().any(|spec| spec.matches(file));
        if !known {
            let expected: Vec<String> = std::iter::once(CONFIG_FILE.to_owned())
                .chain(declared.iter().map(PartSpec::describe))
                .collect();
            errors.push(LoadError {
                file: format!("{path}/{file}"),
                at: ErrorPlace::WholeFile,
                problem: ContentProblem::new(
                    "unknown part",
                    format!("file {file}"),
                    format!("only {} in the bundle", expected.join(", ")),
                ),
            });
        }
        known
    });
    files
}

fn every_file(
    folder: &dyn ContentFolder,
    bundle: &str,
    inner: &str,
    out: &mut Vec<String>,
    errors: &mut Vec<LoadError>,
) {
    let entries = entries(folder, &join(bundle, inner));
    clashes(&join(bundle, inner), &entries, errors);
    for entry in entries
        .into_iter()
        .filter(|entry| !entry.name.starts_with('.'))
    {
        let path = join(inner, &entry.name);
        if entry.dir {
            every_file(folder, bundle, &path, out, errors);
        } else {
            out.push(path);
        }
    }
}

fn clashes(dir: &str, entries: &[FolderEntry], errors: &mut Vec<LoadError>) {
    for (index, entry) in entries.iter().enumerate() {
        if let Some(first) = entries[..index]
            .iter()
            .find(|other| other.name.eq_ignore_ascii_case(&entry.name))
        {
            errors.push(LoadError {
                file: join(dir, &entry.name),
                at: ErrorPlace::WholeFile,
                problem: ContentProblem::new(
                    "case clash",
                    format!("{} and {}", join(dir, &first.name), join(dir, &entry.name)),
                    "names unique in their folder ignoring case",
                ),
            });
        }
    }
}

fn entries(folder: &dyn ContentFolder, dir: &str) -> Vec<FolderEntry> {
    let mut entries: Vec<FolderEntry> = folder
        .entries(dir)
        .into_iter()
        .filter(|entry| !entry.name.starts_with('.'))
        .collect();
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    entries
}

fn join(dir: &str, name: &str) -> String {
    match (dir, name) {
        ("", name) => name.to_owned(),
        (dir, "") => dir.to_owned(),
        (dir, name) => format!("{dir}/{name}"),
    }
}
