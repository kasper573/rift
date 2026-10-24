use std::sync::Arc;

use serde::de::DeserializeOwned;

use super::check::RowCheck;
use super::files::{CONFIG_FILE, valid_name};
use super::gltf::GltfInfo;
use super::id::{ContentScope, with_content};
use super::load::{ContentFolder, FileWriter, parse_json, reference_errors, shape_error};
use super::locate::JsonIndex;
use super::report::{ContentProblem, ErrorPlace, LoadError};
use super::table::ContentFile;
use crate::core::assets::FileSize;

/// A type whose files are bundles: a folder of its `config.json` and the parts it declares, read and
/// written with its fields. A part is never named from outside the bundle, so the bundle moves,
/// renames and copies as one.
pub trait Bundle: ContentFile {
    const PARTS: &'static [PartSpec];
    /// The parts `write_parts` writes: one it leaves unwritten is no part of the bundle any more.
    const WRITTEN: &'static [PartSpec] = &[];

    fn read_parts(&mut self, parts: &mut Parts);

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> std::io::Result<()>;
}

/// A part a bundle type keeps beside its config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartSpec {
    /// One file, `<name>.<ext>` with one of `exts`: `image.png`, `clip.wav`.
    File {
        name: &'static str,
        exts: &'static [&'static str],
    },
    /// A folder of files each named by its key: `busts/<expression>.png`.
    Keyed {
        folder: &'static str,
        exts: &'static [&'static str],
    },
}

impl PartSpec {
    pub fn matches(&self, part: &str) -> bool {
        match *self {
            PartSpec::File { name, exts } => part
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|ext| exts.contains(&ext)),
            PartSpec::Keyed { folder, exts } => part
                .strip_prefix(folder)
                .and_then(|rest| rest.strip_prefix('/'))
                .and_then(|file| file.rsplit_once('.'))
                .is_some_and(|(key, ext)| valid_name(key) && exts.contains(&ext)),
        }
    }

    /// The extensions a file of the part may have.
    pub fn exts(&self) -> &'static [&'static str] {
        match *self {
            PartSpec::File { exts, .. } | PartSpec::Keyed { exts, .. } => exts,
        }
    }

    /// The path in its bundle of the part with the extension `ext`, the file `key` of a keyed
    /// folder.
    pub fn named(&self, key: &str, ext: &str) -> String {
        match self {
            PartSpec::File { name, .. } => format!("{name}.{ext}"),
            PartSpec::Keyed { folder, .. } => format!("{folder}/{key}.{ext}"),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            PartSpec::File { name, exts } => format!("{name}.{}", exts.join("|")),
            PartSpec::Keyed { folder, exts } => format!("{folder}/<name>.{}", exts.join("|")),
        }
    }
}

/// The parts of one bundle as the loader found them, for its type to read with its fields.
pub struct Parts<'a> {
    bundle: &'a str,
    files: &'a [String],
    folder: &'a Arc<dyn ContentFolder>,
    scope: &'a ContentScope,
    errors: Vec<LoadError>,
    indexes: Vec<(String, Arc<JsonIndex>)>,
}

impl<'a> Parts<'a> {
    pub fn new(
        bundle: &'a str,
        files: &'a [String],
        folder: &'a Arc<dyn ContentFolder>,
        scope: &'a ContentScope,
    ) -> Parts<'a> {
        Parts {
            bundle,
            files,
            folder,
            scope,
            errors: Vec::new(),
            indexes: Vec::new(),
        }
    }

    pub fn bundle(&self) -> &str {
        self.bundle
    }

    /// The path from the game folder of the part at `part` in the bundle.
    pub fn path(&self, part: &str) -> String {
        format!("{}/{part}", self.bundle)
    }

    /// The size in pixels of the image at `part`, when it decodes.
    pub fn image_size(&self, part: &str) -> Option<(u32, u32)> {
        let path = self.path(part);
        let bytes = self.folder.read(&path).ok()?;
        super::media::image_size(&path, &bytes)
    }

    /// What the model at `part` holds, when it reads.
    pub fn gltf_info(&self, part: &str) -> Option<GltfInfo> {
        let path = self.path(part);
        let bytes = self.folder.read(&path).ok()?;
        super::media::gltf_info(&path, &bytes)
    }

    /// How much the file at `part` weighs.
    pub fn weight(&self, part: &str) -> Option<FileSize> {
        let bytes = self.folder.read(&self.path(part)).ok()?;
        Some(FileSize(bytes.len()))
    }

    /// The problems `check` found in the bundle's config, each at its place there.
    pub fn config_errors(&mut self, check: RowCheck) {
        for error in check.into_errors() {
            self.json_error(CONFIG_FILE, &error.pointer, error.problem);
        }
    }

    pub fn has(&self, part: &str) -> bool {
        self.files.iter().any(|file| file == part)
    }

    /// The part a `PartSpec::File` names, as its path inside the bundle, when the bundle keeps it.
    pub fn file(&self, spec: PartSpec) -> Option<&'a str> {
        let files: &'a [String] = self.files;
        files
            .iter()
            .find(|file| *file != CONFIG_FILE && spec.matches(file))
            .map(String::as_str)
    }

    /// The part a `PartSpec::File` names, reporting it missing when the bundle has none.
    pub fn required(&mut self, spec: PartSpec) -> Option<&'a str> {
        let found = self.file(spec);
        if found.is_none() {
            self.error(
                "",
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "missing file",
                    format!("no {} in {}/", spec.describe(), self.bundle),
                    format!("{} in the bundle", spec.describe()),
                ),
            );
        }
        found
    }

    /// Every part of a keyed folder, by key, as paths inside the bundle.
    pub fn keyed(&self, spec: PartSpec) -> Vec<(&'a str, &'a str)> {
        let PartSpec::Keyed { folder, .. } = spec else {
            return Vec::new();
        };
        let files: &'a [String] = self.files;
        files
            .iter()
            .filter(|file| spec.matches(file))
            .filter_map(|file| {
                let name = file.strip_prefix(folder)?.strip_prefix('/')?;
                Some((name.rsplit_once('.')?.0, file.as_str()))
            })
            .collect()
    }

    pub fn text(&mut self, part: &str) -> Option<String> {
        let bytes = self.bytes(part)?;
        match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                self.error(
                    part,
                    ErrorPlace::WholeFile,
                    ContentProblem::new("unreadable file", "not UTF-8", "a readable UTF-8 file"),
                );
                None
            }
        }
    }

    pub fn bytes(&mut self, part: &str) -> Option<Vec<u8>> {
        match self.folder.read(&self.path(part)) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                self.error(
                    part,
                    ErrorPlace::WholeFile,
                    ContentProblem::new("unreadable file", error, "a readable file"),
                );
                None
            }
        }
    }

    /// Reads the JSON part at `part` in the content scope the bundle was read in, reporting its
    /// shape and every dangling reference in it at its own file. `held` reads, in that scope, what
    /// the part holds as plain JSON, so the references there count too.
    pub fn json<T: DeserializeOwned>(&mut self, part: &str, held: impl FnOnce(&T)) -> Option<T> {
        let text = self.text(part)?;
        let file = self.path(part);
        let index = Arc::new(JsonIndex::of(&text));
        self.indexes.push((file.clone(), index.clone()));
        let (parsed, scope) = with_content(self.scope.clone(), || {
            parse_json::<T>(&text).inspect(|value| held(value))
        });
        match parsed {
            Ok(value) => {
                self.errors
                    .extend(reference_errors(&file, scope.leaves(), &index, |_| true));
                Some(value)
            }
            Err(error) => {
                self.errors.push(shape_error(&file, &error, &index, &[]));
                None
            }
        }
    }

    pub fn error(&mut self, part: &str, at: ErrorPlace, problem: ContentProblem) {
        let file = if part.is_empty() {
            self.bundle.to_owned()
        } else {
            self.path(part)
        };
        self.errors.push(LoadError { file, at, problem });
    }

    /// A problem at `pointer` in the JSON part at `part`.
    pub fn json_error(&mut self, part: &str, pointer: &str, problem: ContentProblem) {
        let file = self.path(part);
        let index = match self.indexes.iter().find(|(known, _)| *known == file) {
            Some((_, index)) => Some(index.clone()),
            None => self
                .folder
                .read(&file)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|text| Arc::new(JsonIndex::of(&text))),
        };
        let at = match index.and_then(|index| index.position(pointer)) {
            Some((line, column)) => ErrorPlace::Json {
                line,
                column,
                pointer: pointer.to_owned(),
            },
            None => ErrorPlace::WholeFile,
        };
        self.errors.push(LoadError { file, at, problem });
    }

    pub fn errors(&mut self) -> &mut Vec<LoadError> {
        &mut self.errors
    }

    pub(super) fn finish(self) -> (Vec<LoadError>, Vec<(String, Arc<JsonIndex>)>) {
        (self.errors, self.indexes)
    }
}
