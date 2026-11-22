use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, SeqAccess, Visitor};
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
        deserializer.deserialize_seq(Entries(PhantomData))
    }
}

/// The entries of a saved list read one by one, each set aside or kept as it reads: an entry
/// that doesn't read fails as the element it is, so the failure says which entry it was.
struct Entries<T>(PhantomData<T>);

impl<'de, T: DeserializeOwned> Visitor<'de> for Entries<T> {
    type Value = SavedList<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut entries: A) -> Result<SavedList<T>, A::Error> {
        let mut list = SavedList::default();
        while let Some(entry) = entries.next_element_seed(Entry(PhantomData))? {
            match entry {
                Ok(read) => list.here.push(read),
                Err(aside) => list.set_aside.push(aside),
            }
        }
        Ok(list)
    }
}

/// One entry of a saved list: read, or set aside for the files it names that the game no longer
/// has.
struct Entry<T>(PhantomData<T>);

impl<'de, T: DeserializeOwned> DeserializeSeed<'de> for Entry<T> {
    type Value = Result<T, SetAside>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        let entry = serde_json::Value::deserialize(deserializer)?;
        let met = id::references_met();
        let read = T::deserialize(&entry).map_err(de::Error::custom)?;
        let missing = id::forget_missing_since(met);
        if missing.is_empty() {
            Ok(Ok(read))
        } else {
            Ok(Err(SetAside { entry, missing }))
        }
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
