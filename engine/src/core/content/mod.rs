pub mod bundle;
pub mod check;
pub mod files;
pub mod fixture;
pub mod fresh;
pub mod gltf;
pub mod id;
pub mod key;
pub mod load;
pub mod locate;
pub mod media;
pub mod reader;
pub mod report;
pub mod serial;
pub mod store;
pub mod sync;
pub mod table;
pub mod version;

pub use bundle::{Bundle, PartSpec, Parts};
pub use check::RowCheck;
pub use files::{
    CONFIG_FILE, FIXTURES_DIR, GameFile, GameFiles, NOTE_EXTENSION, SETTINGS_FILE, Shape,
    bundle_of, content_path, fields_file, name_of, numbered, parent_of, table_of, valid_name,
};
pub use fixture::Fixture;
pub use fresh::Nearby;
pub use id::{ContentId, ContentScope, rescoped, with_content};
pub use key::key_of;
pub use load::{
    CheckedFiles, ContentFolder, ContentRegistry, FileHashes, FileWriter, FilesystemFolder,
    FolderEntry, JsonPart, LoadStep, LoadWarnings, Loading, MemoryFolder, Reference, Settings,
    load, pointer_of, valid_key, write,
};
pub use reader::{FolderReader, Unreadable, WrittenFiles};
pub use report::{ContentProblem, ErrorPlace, LoadError, LoadReport, MISSING_FILE};
pub use table::{
    AnyTable, Content, ContentBuilder, ContentFile, ContentNames, MODULES, ModuleRule, ModuleSet,
    NamedContent, ReadRow, Table,
};
pub use version::ContentVersion;

/// Whether `file` is media by its extension, fetched and cached on its own rather than read with the
/// content files.
pub fn is_media_file(file: &str) -> bool {
    crate::core::assets::AssetKind::of(file).is_some()
}

pub fn named<Row: ContentFile>(
    world: &bevy_ecs::world::World,
    path: &str,
) -> Result<ContentId<Row>, String> {
    world
        .resource::<Content>()
        .by_path::<Row>(path)
        .ok_or_else(|| format!("`{path}` is not a known {}", Row::TABLE))
}
