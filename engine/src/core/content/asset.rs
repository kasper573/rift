use super::SchemaMark;
use std::borrow::Cow;
use std::fmt::{Display, Formatter};

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
    pub const fn extensions(self) -> &'static [&'static str] {
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
                SchemaMark::Asset.set(&mut schema, kind);
                schema
            }

            fn inline_schema() -> bool {
                true
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&super::id::written_path(&self.0))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$name, D::Error> {
                let human = deserializer.is_human_readable();
                let path = String::deserialize(deserializer)?;
                if human {
                    super::id::scope_asset(&path);
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
