use std::any::{Any, TypeId};
use std::sync::{Arc, OnceLock};

use bevy_ecs::prelude::Resource;
use bevy_platform::collections::HashMap;
use bevy_platform::hash::NoOpHash;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::check::RowCheck;
use super::files::{GameFiles, Shape};
use super::fresh::Nearby;
use super::id::{ContentId, ScopePaths};

/// A type of content file: what its files hold, named `<name>.<TABLE>.json`, or kept as bundles
/// `<name>.<TABLE>/` when the type implements `Bundle`.
pub trait ContentFile: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static {
    const TABLE: &'static str;
    /// `Shape::Bundle` exactly when the type implements `Bundle`.
    const SHAPE: Shape = Shape::Json;
    /// Whether every file is in use by being there, offered to pick from rather than named by
    /// another file.
    const PICKED_FROM_TABLE: bool = false;

    /// What a file of this type holds when a designer makes one at the place `near` tells: the
    /// least that loads, naming the nearest files and media wherever it must name some. Says why
    /// when the game holds nothing it must name.
    fn fresh(near: &Nearby) -> Result<Self, String>;

    fn check(&self, _cx: &mut RowCheck) {}

    /// Reads, in the scope the file was just parsed in, what it holds as plain JSON for another
    /// file to read later, so the files named there count as this file's references.
    fn read_held(&self) {}
}

/// A type of content file that carries a name: the one a player reads it by, or for content no
/// player sees, the one a designer knows it by.
pub trait NamedContent: ContentFile {
    fn shown_name(&self) -> &str;
}

/// Where a description of content finds the names of the files it names.
pub trait ContentNames {
    fn name_of<Row: NamedContent>(&self, id: ContentId<Row>) -> String;
}

impl ContentNames for Content {
    fn name_of<Row: NamedContent>(&self, id: ContentId<Row>) -> String {
        id.get(self).shown_name().to_owned()
    }
}

/// The field a file of a `ModuleSet` lists its modules in.
pub const MODULES: &str = "modules";

pub trait ModuleSet: ContentFile {
    type Module: Serialize + DeserializeOwned;
    fn modules(&self) -> &[Self::Module];
    fn modules_mut(&mut self) -> &mut Vec<Self::Module>;
    fn rule(module: &Self::Module) -> ModuleRule;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleRule {
    AtMostOne,
    AnyNumber,
}

/// The path of every file of one type, sorted bytewise, and the files that parsed. A path without a
/// file failed to load; it keeps its index so references to it still resolve while the loader
/// collects errors.
pub struct Table<Row> {
    paths: Vec<String>,
    rows: Vec<Option<Row>>,
}

impl<Row: ContentFile> Table<Row> {
    pub fn new(rows: Vec<(String, Row)>) -> Table<Row> {
        let paths = rows.iter().map(|(path, _)| path.clone()).collect();
        let mut table = Table::of_paths(paths);
        for (path, row) in rows {
            table.insert(&path, row);
        }
        table
    }

    pub fn of_paths(mut paths: Vec<String>) -> Table<Row> {
        paths.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        paths.dedup();
        let rows = paths.iter().map(|_| None).collect();
        Table { paths, rows }
    }

    pub fn insert(&mut self, path: &str, row: Row) {
        let index = self
            .paths
            .binary_search_by(|candidate| candidate.as_bytes().cmp(path.as_bytes()))
            .unwrap_or_else(|_| panic!("{} has no file {path:?}", Row::TABLE));
        self.rows[index] = Some(row);
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = ContentId<Row>> + Clone + '_ {
        (0..self.rows.len() as u32).map(ContentId::new)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ContentId<Row>, &Row)> {
        self.ids()
            .zip(&self.rows)
            .filter_map(|(id, row)| row.as_ref().map(|row| (id, row)))
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().flatten()
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, id: ContentId<Row>) -> &Row {
        self.get(id)
            .unwrap_or_else(|| panic!("{} failed to load", self.path(id)))
    }

    pub fn get(&self, id: ContentId<Row>) -> Option<&Row> {
        self.rows.get(id.index()).and_then(Option::as_ref)
    }

    pub fn path(&self, id: ContentId<Row>) -> &str {
        &self.paths[id.index()]
    }

    pub fn id_of(&self, path: &str) -> Option<ContentId<Row>> {
        self.paths
            .binary_search_by(|candidate| candidate.as_bytes().cmp(path.as_bytes()))
            .ok()
            .map(|index| ContentId::new(index as u32))
    }
}

pub trait AnyTable: Any + Send + Sync {
    fn name(&self) -> &'static str;
    fn paths(&self) -> &[String];
    /// The raw id of the file at `path`, for code that names a table only by its name.
    fn raw_of(&self, path: &str) -> Option<u32>;
    fn as_any(&self) -> &dyn Any;
    /// Every row the table holds, by its path and with its type, when nothing else holds the
    /// table.
    fn into_rows(self: Arc<Self>) -> Vec<ReadRow>;
}

/// A content file read whole, as `FolderReader::read` reads it: its path, its type and its row.
pub struct ReadRow {
    pub path: String,
    pub row_type: TypeId,
    pub row: Arc<dyn Any + Send + Sync>,
}

impl<Row: ContentFile> AnyTable for Table<Row> {
    fn name(&self) -> &'static str {
        Row::TABLE
    }

    fn paths(&self) -> &[String] {
        &self.paths
    }

    fn raw_of(&self, path: &str) -> Option<u32> {
        self.id_of(path).map(ContentId::raw)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_rows(self: Arc<Self>) -> Vec<ReadRow> {
        let Ok(table) = Arc::try_unwrap(self) else {
            return Vec::new();
        };
        table
            .paths
            .into_iter()
            .zip(table.rows)
            .filter_map(|(path, row)| {
                Some(ReadRow {
                    path,
                    row_type: TypeId::of::<Row>(),
                    row: Arc::new(row?),
                })
            })
            .collect()
    }
}

type Tables = HashMap<TypeId, Arc<dyn AnyTable>, NoOpHash>;
type Built = HashMap<TypeId, Arc<dyn Any + Send + Sync>, NoOpHash>;

struct Loaded {
    tables: Tables,
    built: Built,
    /// Every table's paths, gathered once for every scope that resolves against this content.
    scope: OnceLock<Arc<ScopePaths>>,
}

/// Every table of one content folder plus what the loader built from them (areas, models, compiled
/// scripts).
#[derive(Resource, Clone)]
pub struct Content(Arc<Loaded>);

impl Content {
    pub fn table<Row: ContentFile>(&self) -> &Table<Row> {
        self.0
            .tables
            .get(&TypeId::of::<Row>())
            .unwrap_or_else(|| panic!("table {} was never registered", Row::TABLE))
            .as_any()
            .downcast_ref()
            .expect("a table is stored under its row type")
    }

    pub fn ids<Row: ContentFile>(
        &self,
    ) -> impl ExactSizeIterator<Item = ContentId<Row>> + Clone + 'static {
        (0..self.table::<Row>().len() as u32).map(ContentId::new)
    }

    pub fn by_path<Row: ContentFile>(&self, path: &str) -> Option<ContentId<Row>> {
        self.table::<Row>().id_of(path)
    }

    /// What was built into this content as `T`, if it was: a build that failed leaves none.
    pub fn try_built<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.0
            .built
            .get(&TypeId::of::<T>())
            .and_then(|built| built.downcast_ref())
    }

    pub fn built<T: Any + Send + Sync>(&self) -> &T {
        self.try_built().unwrap_or_else(|| {
            panic!(
                "{} was never built into this content",
                std::any::type_name::<T>()
            )
        })
    }

    /// The path of every content file and media file. Content put together without the loader
    /// has none.
    pub fn files(&self) -> &GameFiles {
        static NONE: std::sync::LazyLock<GameFiles> = std::sync::LazyLock::new(GameFiles::default);
        self.0
            .built
            .get(&TypeId::of::<GameFiles>())
            .and_then(|built| built.downcast_ref())
            .unwrap_or(&NONE)
    }

    pub fn has_built<T: Any + Send + Sync>(&self) -> bool {
        self.0.built.contains_key(&TypeId::of::<T>())
    }

    pub fn table_named(&self, name: &str) -> Option<&dyn AnyTable> {
        self.0
            .tables
            .values()
            .find(|table| table.name() == name)
            .map(|table| table.as_ref())
    }

    pub(super) fn scope_paths(&self) -> Arc<ScopePaths> {
        self.0
            .scope
            .get_or_init(|| {
                Arc::new(
                    self.tables()
                        .map(|(id, table)| (id, (table.name(), table.paths().to_vec())))
                        .collect(),
                )
            })
            .clone()
    }

    pub(super) fn tables(&self) -> impl Iterator<Item = (TypeId, &dyn AnyTable)> {
        self.0
            .tables
            .iter()
            .map(|(id, table)| (*id, table.as_ref()))
    }
}

#[derive(Default)]
pub struct ContentBuilder {
    tables: Tables,
    built: Built,
}

impl ContentBuilder {
    pub fn insert_table<Row: ContentFile>(&mut self, table: Table<Row>) -> &mut ContentBuilder {
        self.tables.insert(TypeId::of::<Row>(), Arc::new(table));
        self
    }

    pub fn built<T: Any + Send + Sync>(&mut self, value: T) {
        self.built.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Every row of every table, as `AnyTable::into_rows` gives them.
    pub fn into_rows(self) -> Vec<ReadRow> {
        self.tables
            .into_values()
            .flat_map(AnyTable::into_rows)
            .collect()
    }

    pub fn build(&self) -> Content {
        Content(Arc::new(Loaded {
            tables: self.tables.clone(),
            built: self.built.clone(),
            scope: OnceLock::new(),
        }))
    }
}
