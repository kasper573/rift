//! Loading is assertive — a malformed row or dangling reference panics with the offending file,
//! so broken content can never load.
//!
//! A content type implements [`Content`] to gain a typed index ([`Id`]) into its static table:
//! `id.get()` reads the row, [`Id::by_name`] resolves a string id, and [`Id::deserialize_named`]
//! is the `#[serde(deserialize_with = …)]` hook content tables use to reference rows by name. Each
//! content module still owns its own loader (a `OnceLock`), because loading differs — JSON via
//! [`load`], a directory of Tiled tilesets, or rows built from other rows.

use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::assets;

/// A content row addressable by a stable string id and laid out in a static, index-addressed
/// table. The index is the wire form of an [`Id`].
pub trait Content: Sized + 'static {
    fn table() -> &'static [Self];
    fn id(&self) -> &str;
}

/// A typed index into `T`'s [`Content::table`]. Serializes as its numeric index (the wire form
/// replication ships); resolve from a name with [`Id::by_name`] or the [`by_name`] serde hook.
pub struct Id<T> {
    index: u32,
    _content: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub const fn new(index: u32) -> Self {
        Id {
            index,
            _content: PhantomData,
        }
    }

    pub const fn index(self) -> usize {
        self.index as usize
    }
}

impl<T: Content> Id<T> {
    /// The row this id points at.
    pub fn get(self) -> &'static T {
        &T::table()[self.index as usize]
    }

    /// The id of the row with this string id, if any.
    pub fn by_name(name: &str) -> Option<Self> {
        T::table()
            .iter()
            .position(|row| row.id() == name)
            .map(|index| Id::new(index as u32))
    }

    /// The `#[serde(deserialize_with = "crate::table::Id::<…>::deserialize_named")]` hook:
    /// resolves a content reference written as its string id.
    pub fn deserialize_named<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::by_name(&name).ok_or_else(|| serde::de::Error::custom(format!("unknown id '{name}'")))
    }
}

pub fn load<T: DeserializeOwned>(file: &str) -> Vec<T> {
    let json = assets::text(file).unwrap_or_else(|| panic!("missing asset {file}"));
    serde_json::from_str(&json).unwrap_or_else(|error| panic!("{file}: {error}"))
}

pub fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, file: &str) {
    let mut seen: Vec<&str> = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            panic!("{file}: duplicate id '{id}'");
        }
        seen.push(id);
    }
}

// Derives by hand so the bounds never reach `T` — an `Id` is just a `u32`.
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<T> Eq for Id<T> {}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index.cmp(&other.index)
    }
}
impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}
impl<T> std::fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id<{}>({})", std::any::type_name::<T>(), self.index)
    }
}
impl<T> Default for Id<T> {
    fn default() -> Self {
        Id::new(0)
    }
}
impl<T> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.index.serialize(serializer)
    }
}
impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(Id::new)
    }
}
