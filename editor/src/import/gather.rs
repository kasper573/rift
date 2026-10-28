//! The files picked to import, read off the disk as a game keeps them: a folder with everything
//! inside it, an archive unpacked into a folder of its name, and an FBX file converted to the glTF
//! binary a game reads models from.

use std::io::Read;
use std::path::Path;

use super::fbx::glb_of_fbx;

/// Every file found, by its path under the folder it is imported into, and every one that could
/// not be read or converted.
#[derive(Default)]
pub struct Gathered {
    pub files: Vec<Found>,
    pub failed: Vec<Failure>,
}

pub struct Found {
    pub path: String,
    pub bytes: Vec<u8>,
    pub came: Came,
}

/// How a found file came to be what is imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Came {
    AsIs,
    Unpacked { archive: String },
    Converted { from: String },
}

pub struct Failure {
    pub path: String,
    pub why: String,
}

/// The files `sources` hold, each folder and archive by its name, `reading` told the path of each
/// file before it is read.
pub fn gather(sources: &[impl AsRef<Path>], reading: &dyn Fn(&str)) -> Gathered {
    let mut gathered = Gathered::default();
    for source in sources {
        let source = source.as_ref();
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if source.is_dir() {
            walk(source, &name, &mut gathered, reading);
        } else {
            reading(&name);
            match std::fs::read(source) {
                Ok(bytes) => take(&mut gathered, name, bytes, Came::AsIs, reading),
                Err(error) => gathered.failed.push(Failure {
                    path: name,
                    why: error.to_string(),
                }),
            }
        }
    }
    gathered
}

fn walk(folder: &Path, path: &str, gathered: &mut Gathered, reading: &dyn Fn(&str)) {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) => {
            gathered.failed.push(Failure {
                path: path.to_owned(),
                why: error.to_string(),
            });
            return;
        }
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let inner = format!("{path}/{}", entry.file_name().to_string_lossy());
        let at = entry.path();
        if at.is_dir() {
            walk(&at, &inner, gathered, reading);
            continue;
        }
        reading(&inner);
        match std::fs::read(&at) {
            Ok(bytes) => take(gathered, inner, bytes, Came::AsIs, reading),
            Err(error) => gathered.failed.push(Failure {
                path: inner,
                why: error.to_string(),
            }),
        }
    }
}

/// Keeps the file `bytes` at `path`, unpacking it when it is an archive and converting it when it
/// is an FBX file.
fn take(gathered: &mut Gathered, path: String, bytes: Vec<u8>, came: Came, reading: &dyn Fn(&str)) {
    match extension(&path).as_deref() {
        Some("zip") => unpack(gathered, &path, &bytes, reading),
        Some("fbx") => match glb_of_fbx(&bytes) {
            Ok(glb) => gathered.files.push(Found {
                path: format!("{}.glb", &path[..path.len() - ".fbx".len()]),
                bytes: glb,
                came: Came::Converted { from: path },
            }),
            Err(why) => gathered.failed.push(Failure { path, why }),
        },
        _ => gathered.files.push(Found { path, bytes, came }),
    }
}

fn unpack(gathered: &mut Gathered, path: &str, bytes: &[u8], reading: &dyn Fn(&str)) {
    let folder = &path[..path.len() - ".zip".len()];
    let mut archive = match zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        Ok(archive) => archive,
        Err(error) => {
            gathered.failed.push(Failure {
                path: path.to_owned(),
                why: error.to_string(),
            });
            return;
        }
    };
    for index in 0..archive.len() {
        let mut file = match archive.by_index(index) {
            Ok(file) => file,
            Err(error) => {
                gathered.failed.push(Failure {
                    path: format!("{path} #{index}"),
                    why: error.to_string(),
                });
                continue;
            }
        };
        if file.is_dir() {
            continue;
        }
        let Some(inside) = file.enclosed_name() else {
            gathered.failed.push(Failure {
                path: format!("{path}: {}", String::from_utf8_lossy(file.name_raw())),
                why: "a path that leads out of the archive".to_owned(),
            });
            continue;
        };
        let inner = format!(
            "{folder}/{}",
            inside
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        );
        reading(&inner);
        let mut bytes = Vec::new();
        match file.read_to_end(&mut bytes) {
            Ok(_) => take(
                gathered,
                inner,
                bytes,
                Came::Unpacked {
                    archive: path.to_owned(),
                },
                reading,
            ),
            Err(error) => gathered.failed.push(Failure {
                path: inner,
                why: error.to_string(),
            }),
        }
    }
}

fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    name.rsplit_once('.').map(|(_, ext)| ext.to_lowercase())
}
