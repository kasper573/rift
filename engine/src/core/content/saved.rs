use std::borrow::Cow;
use std::ops::{Deref, DerefMut};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};

use super::id;

/// A list that is saved and read again by a game whose files may have changed meanwhile. Read
/// inside a scope that sets aside, an entry naming a file the game no longer has is kept as it
/// was saved, apart from the entries the game still has, and is written back with them: it reads
/// again once the file is back. Read inside any other scope it is a plain list, and such an entry
/// is the dangling reference it would be anywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedList<T> {
    here: Vec<T>,
    set_aside: Vec<SetAside>,
}

/// An entry of a saved list kept as it was saved, apart from those the game has.
#[derive(Clone, Debug, PartialEq)]
pub struct SetAside {
    pub entry: serde_json::Value,
    /// The files it names that the game no longer has.
    pub missing: Vec<MissingFile>,
}

/// A file something saved names and the game no longer has: its type, and its path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingFile {
    pub table: &'static str,
    pub path: String,
}

impl<T> SavedList<T> {
    /// The entries set aside, as they were saved.
    pub fn set_aside(&self) -> &[SetAside] {
        &self.set_aside
    }

    /// The entries set aside, to put back those a list written from the game's own is without.
    pub fn set_aside_mut(&mut self) -> &mut Vec<SetAside> {
        &mut self.set_aside
    }

    /// Whether it holds nothing at all, set aside or not.
    pub fn is_empty(&self) -> bool {
        self.here.is_empty() && self.set_aside.is_empty()
    }

    /// Keeps `entry` as it would be saved, apart from the entries the game has, though it names
    /// no missing file.
    pub fn put_aside(&mut self, entry: serde_json::Value) {
        self.set_aside.push(SetAside {
            entry,
            missing: Vec::new(),
        });
    }
}

impl<T> Default for SavedList<T> {
    fn default() -> SavedList<T> {
        SavedList {
            here: Vec::new(),
            set_aside: Vec::new(),
        }
    }
}

impl<T> Deref for SavedList<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.here
    }
}

impl<T> DerefMut for SavedList<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.here
    }
}

impl<T> From<Vec<T>> for SavedList<T> {
    fn from(here: Vec<T>) -> SavedList<T> {
        SavedList {
            here,
            set_aside: Vec::new(),
        }
    }
}

impl<T> FromIterator<T> for SavedList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(entries: I) -> SavedList<T> {
        SavedList::from(entries.into_iter().collect::<Vec<T>>())
    }
}

impl<T: Serialize> Serialize for SavedList<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut entries = serializer.serialize_seq(Some(self.here.len() + self.set_aside.len()))?;
        for entry in &self.here {
            entries.serialize_element(entry)?;
        }
        for aside in &self.set_aside {
            entries.serialize_element(&aside.entry)?;
        }
        entries.end()
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for SavedList<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SavedList<T>, D::Error> {
        if !id::sets_aside() {
            return Vec::<T>::deserialize(deserializer).map(SavedList::from);
        }
        let mut list = SavedList::default();
        for entry in Vec::<serde_json::Value>::deserialize(deserializer)? {
            let met = id::references_met();
            let read = T::deserialize(&entry).map_err(de::Error::custom)?;
            let missing = id::forget_missing_since(met);
            if missing.is_empty() {
                list.here.push(read);
            } else {
                list.set_aside.push(SetAside { entry, missing });
            }
        }
        Ok(list)
    }
}

impl<T: JsonSchema> JsonSchema for SavedList<T> {
    fn schema_name() -> Cow<'static, str> {
        Vec::<T>::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        Vec::<T>::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Vec::<T>::json_schema(generator)
    }

    fn inline_schema() -> bool {
        Vec::<T>::inline_schema()
    }
}
