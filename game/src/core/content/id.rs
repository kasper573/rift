use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use super::table::{Content, ContentRow};

/// In files a key, in memory and on the wire the index among the table's keys sorted bytewise, so
/// an id is valid only within the `Content` it was resolved against.
pub struct ContentId<Row>(u32, PhantomData<fn() -> Row>);

impl<Row> ContentId<Row> {
    pub const fn new(index: u32) -> ContentId<Row> {
        ContentId(index, PhantomData)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl<Row: ContentRow> ContentId<Row> {
    pub fn get(self, content: &Content) -> &Row {
        content.table::<Row>().row(self)
    }

    pub fn key(self, content: &Content) -> &str {
        content.table::<Row>().key(self)
    }
}

impl<Row> Clone for ContentId<Row> {
    fn clone(&self) -> ContentId<Row> {
        *self
    }
}

impl<Row> Copy for ContentId<Row> {}

impl<Row> PartialEq for ContentId<Row> {
    fn eq(&self, other: &ContentId<Row>) -> bool {
        self.0 == other.0
    }
}

impl<Row> Eq for ContentId<Row> {}

impl<Row> PartialOrd for ContentId<Row> {
    fn partial_cmp(&self, other: &ContentId<Row>) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Row> Ord for ContentId<Row> {
    fn cmp(&self, other: &ContentId<Row>) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<Row> Hash for ContentId<Row> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<Row: ContentRow> fmt::Debug for ContentId<Row> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match SCOPE.with(|scope| {
            scope
                .borrow()
                .as_ref()
                .and_then(|scope| scope.key::<Row>(self.0))
        }) {
            Some(key) => write!(f, "{}/{key}", Row::TABLE),
            None => write!(f, "{}#{}", Row::TABLE, self.0),
        }
    }
}

impl<Row: ContentRow> Serialize for ContentId<Row> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !serializer.is_human_readable() {
            return serializer.serialize_u32(self.0);
        }
        let key = SCOPE.with(|scope| {
            scope
                .borrow()
                .as_ref()
                .and_then(|scope| scope.key::<Row>(self.0))
        });
        match key {
            Some(key) => serializer.serialize_str(&key),
            None => Err(serde::ser::Error::custom(format!(
                "a {} id is written as its key only inside with_content",
                Row::TABLE
            ))),
        }
    }
}

impl<'de, Row: ContentRow> Deserialize<'de> for ContentId<Row> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ContentId<Row>, D::Error> {
        if !deserializer.is_human_readable() {
            return u32::deserialize(deserializer).map(ContentId::new);
        }
        let key = String::deserialize(deserializer)?;
        SCOPE.with(|scope| {
            let mut scope = scope.borrow_mut();
            let Some(scope) = scope.as_mut() else {
                return Err(de::Error::custom(format!(
                    "a {} key is read only inside with_content",
                    Row::TABLE
                )));
            };
            Ok(scope.resolve::<Row>(key))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownRow {
    pub table: &'static str,
    pub key: String,
    pub nearest: Option<String>,
}

/// Collects the unknown rows met while reading, so one parse reports every dangling reference
/// instead of stopping at the first.
pub struct ContentScope {
    keys: HashMap<TypeId, (&'static str, Vec<String>)>,
    unknown: Vec<UnknownRow>,
}

impl ContentScope {
    pub fn new() -> ContentScope {
        ContentScope {
            keys: HashMap::new(),
            unknown: Vec::new(),
        }
    }

    pub fn of(content: &Content) -> ContentScope {
        let mut scope = ContentScope::new();
        for (type_id, table) in content.tables() {
            scope
                .keys
                .insert(type_id, (table.name(), table.keys().to_vec()));
        }
        scope
    }

    pub fn table<Row: ContentRow>(&mut self, keys: Vec<String>) {
        self.keys.insert(TypeId::of::<Row>(), (Row::TABLE, keys));
    }

    pub fn unknown(&self) -> &[UnknownRow] {
        &self.unknown
    }

    fn key<Row: ContentRow>(&self, index: u32) -> Option<String> {
        self.keys
            .get(&TypeId::of::<Row>())?
            .1
            .get(index as usize)
            .cloned()
    }

    fn resolve<Row: ContentRow>(&mut self, key: String) -> ContentId<Row> {
        let keys = self.keys.get(&TypeId::of::<Row>()).map(|(_, keys)| keys);
        match keys.and_then(|keys| keys.binary_search(&key).ok()) {
            Some(index) => ContentId::new(index as u32),
            None => {
                let nearest = keys.and_then(|keys| nearest(keys, &key));
                self.unknown.push(UnknownRow {
                    table: Row::TABLE,
                    key,
                    nearest,
                });
                ContentId::new(u32::MAX)
            }
        }
    }
}

impl Default for ContentScope {
    fn default() -> ContentScope {
        ContentScope::new()
    }
}

thread_local! {
    static SCOPE: RefCell<Option<ContentScope>> = const { RefCell::new(None) };
}

pub fn with_content<T>(scope: ContentScope, f: impl FnOnce() -> T) -> (T, ContentScope) {
    let previous = SCOPE.with(|slot| slot.replace(Some(scope)));
    let result = f();
    let scope = SCOPE.with(|slot| slot.replace(previous));
    (result, scope.expect("the scope installed above"))
}

pub fn nearest<'a>(candidates: impl IntoIterator<Item = &'a String>, key: &str) -> Option<String> {
    let allowed = if key.len() > 12 { 3 } else { 2 };
    let mut best: Option<(usize, &String)> = None;
    for candidate in candidates {
        let distance = edit_distance(candidate, key);
        if distance <= allowed && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.clone())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}
