use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use engine::core::content::{
    FilesystemFolder, FolderReader, MemoryFolder, SETTINGS_FILE, table_of,
};
use engine::systems::{GameSettings, registry};
use include_dir::{Dir, include_dir};
use walkdir::WalkDir;

use crate::busy::Progress;
use crate::project::files::{content_holding, hidden};

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

pub struct GameTemplate {
    pub key: String,
    pub name: String,
    pub initial_spawn: String,
    /// The paths of the content files it holds, by type.
    pub rows: BTreeMap<String, Vec<String>>,
}

impl GameTemplate {
    pub(crate) fn rows_in(&self, table: &str) -> usize {
        self.rows.get(table).map_or(0, Vec::len)
    }
}

pub fn list() -> &'static [GameTemplate] {
    static LISTED: LazyLock<Vec<GameTemplate>> = LazyLock::new(|| {
        let mut templates: Vec<GameTemplate> = TEMPLATES.dirs().map(template_of).collect();
        templates.sort_by(|a, b| a.key.cmp(&b.key));
        templates
    });
    &LISTED
}

/// A new game named `name` at `folder`, made from the built-in template `template`, each file
/// written counted to `progress`.
pub fn new_game(
    template: &str,
    name: &str,
    folder: &Path,
    progress: &Progress,
) -> Result<(), String> {
    let dir = TEMPLATES
        .get_dir(template)
        .ok_or_else(|| format!("no built-in template {template}"))?;
    stage(folder, name, |staging| {
        let files = all_files(dir);
        progress.doing("Writing files", Some(files.len()));
        for file in files {
            progress.one_more();
            let relative = file
                .path()
                .strip_prefix(dir.path())
                .map_err(|e| e.to_string())?;
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&target, file.contents()).map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

/// A new game named `name` at `folder`, a copy of the game at `source`, which holds `files` files
/// when they are counted, each file copied counted to `progress`.
pub fn new_game_from(
    source: &Path,
    name: &str,
    folder: &Path,
    files: Option<usize>,
    progress: &Progress,
) -> Result<(), String> {
    if !source.join(SETTINGS_FILE).is_file() {
        return Err(format!("No {SETTINGS_FILE} in {}", source.display()));
    }
    progress.doing("Copying files", files);
    stage(folder, name, |staging| copy_dir(source, staging, progress))
}

fn stage(
    folder: &Path,
    name: &str,
    fill: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    ensure_empty(folder)?;
    let staging = staging_dir(folder);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let result = fill(&staging)
        .and_then(|()| write_name(&staging, name))
        .and_then(|()| move_into_place(&staging, folder));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn ensure_empty(folder: &Path) -> Result<(), String> {
    if folder.exists() {
        if !folder.is_dir() {
            return Err(format!("{} is not a folder", folder.display()));
        }
        let mut entries = std::fs::read_dir(folder).map_err(|error| error.to_string())?;
        if entries.next().is_some() {
            return Err(format!("{} is not empty", folder.display()));
        }
    }
    Ok(())
}

fn staging_dir(folder: &Path) -> PathBuf {
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "game".to_owned());
    folder
        .parent()
        .unwrap_or(folder)
        .join(format!(".{name}.new-game"))
}

fn move_into_place(staging: &Path, folder: &Path) -> Result<(), String> {
    if folder.exists() {
        std::fs::remove_dir(folder).map_err(|error| error.to_string())?;
    }
    std::fs::rename(staging, folder).map_err(|error| error.to_string())
}

fn write_name(folder: &Path, name: &str) -> Result<(), String> {
    let reader = FolderReader::new(registry(), Arc::new(FilesystemFolder(folder.to_owned())));
    let mut settings =
        reader
            .settings::<GameSettings>()
            .map_err(|errors| match errors.first() {
                Some(error) => format!("{SETTINGS_FILE} does not read: {}", error.problem),
                None => format!("{SETTINGS_FILE} does not read"),
            })?;
    settings.name = name.to_owned();
    let written = reader
        .write_settings(&settings)
        .map_err(|error| error.to_string())?;
    for (file, bytes) in written {
        std::fs::write(folder.join(file), bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path, progress: &Progress) -> Result<(), String> {
    let entries = WalkDir::new(from)
        .into_iter()
        .filter_entry(|entry| entry.file_name().to_str().is_none_or(|name| !hidden(name)));
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = to.join(
            entry
                .path()
                .strip_prefix(from)
                .expect("a walked entry lies under its root"),
        );
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(target).map_err(|error| error.to_string())?;
        } else {
            std::fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
            progress.one_more();
        }
    }
    Ok(())
}

fn template_of(dir: &Dir) -> GameTemplate {
    let key = dir.path().to_string_lossy().into_owned();
    let mut files = BTreeMap::new();
    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in all_files(dir) {
        let path = file.path().strip_prefix(dir.path()).unwrap_or(file.path());
        let Some(path) = path.to_str() else {
            continue;
        };
        files.insert(path.to_owned(), Arc::from(file.contents()));
        let Some(holder) = content_holding(path) else {
            continue;
        };
        if let Some(table) = table_of(holder) {
            let paths = rows.entry(table.to_owned()).or_default();
            if !paths.iter().any(|known| known == holder) {
                paths.push(holder.to_owned());
            }
        }
    }
    let folder = MemoryFolder {
        name: key.clone(),
        files,
    };
    let reader = FolderReader::new(registry(), Arc::new(folder));
    let settings = reader.settings::<GameSettings>().ok();
    GameTemplate {
        name: settings
            .as_ref()
            .map_or_else(|| key.clone(), |settings| settings.name.clone()),
        initial_spawn: settings.map_or_else(
            || "?".to_owned(),
            |settings| {
                let spawn = settings.initial_spawn;
                let area = reader.path_of(spawn.area).unwrap_or("?");
                format!("{area} · {}, {}", spawn.at.x, spawn.at.y)
            },
        ),
        key,
        rows,
    }
}

fn all_files<'a>(dir: &'a Dir<'a>) -> Vec<&'a include_dir::File<'a>> {
    let mut files: Vec<&include_dir::File> = dir.files().collect();
    for sub in dir.dirs() {
        files.extend(all_files(sub));
    }
    files
}
