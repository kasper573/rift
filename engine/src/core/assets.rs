use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::{Arc, RwLock};

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, ErasedAssetReader, PathStream, Reader,
    VecReader,
};
use bevy::prelude::Resource;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum AssetKind {
    Image,
    Audio,
    Model,
}

impl AssetKind {
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            AssetKind::Image => &["png"],
            AssetKind::Audio => &["flac", "wav"],
            AssetKind::Model => &["glb"],
        }
    }

    pub fn of(path: &str) -> Option<AssetKind> {
        let (_, extension) = path.rsplit_once('.')?;
        AssetKind::VARIANTS.iter().copied().find(|kind| {
            kind.extensions()
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
    }
}

macro_rules! asset_ref {
    ($name:ident, $kind:ident) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(path: impl Into<String>) -> $name {
                $name(path.into())
            }

            pub fn path(&self) -> String {
                self.0.clone()
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                let mut schema = String::json_schema(generator);
                let kind: &'static str = AssetKind::$kind.into();
                schema.insert("x-asset".to_owned(), kind.into());
                schema
            }

            fn inline_schema() -> bool {
                true
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&crate::core::content::id::written_path(&self.0))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$name, D::Error> {
                let human = deserializer.is_human_readable();
                let path = String::deserialize(deserializer)?;
                if human {
                    crate::core::content::id::scope_asset(&path);
                }
                Ok($name(path))
            }
        }
    };
}

asset_ref!(ImageRef, Image);
asset_ref!(AudioRef, Audio);
asset_ref!(ModelRef, Model);

/// How much a file weighs to download.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileSize(pub usize);

impl FileSize {
    pub const fn kib(kib: usize) -> FileSize {
        FileSize(kib * 1024)
    }
}

impl Display for FileSize {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{} KiB", self.0.div_ceil(1024))
    }
}

#[derive(Resource, Clone, Default)]
pub struct SyncedFiles(Arc<RwLock<HashMap<String, Arc<[u8]>>>>);

impl SyncedFiles {
    pub fn replace(&self, files: HashMap<String, Arc<[u8]>>) {
        *self.0.write().expect("synced files lock") = files;
    }

    pub fn source(&self) -> AssetSourceBuilder {
        let files = self.clone();
        AssetSourceBuilder::new(move || Box::new(files.clone()) as Box<dyn ErasedAssetReader>)
    }

    /// The bytes of the file at `path` from the game folder, when it is there.
    pub fn bytes(&self, path: &str) -> Option<Arc<[u8]>> {
        let files = self.0.read().expect("synced files lock");
        files.get(path).cloned()
    }

    fn get(&self, path: &Path) -> Option<Arc<[u8]>> {
        self.bytes(path.to_str()?)
    }
}

impl AssetReader for SyncedFiles {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.get(path)
            .map(|bytes| VecReader::new(bytes.to_vec()))
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}
