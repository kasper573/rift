use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use super::id::ContentId;
use bevy_ecs::prelude::Resource;

pub trait ContentRow: Send + Sync + 'static {
    const TABLE: &'static str;
}

pub trait ModuleSet: ContentRow {
    type Module;
    fn modules(&self) -> &[Self::Module];
    fn rule(module: &Self::Module) -> ModuleRule;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleRule {
    AtMostOne,
    AnyNumber,
}

pub struct Table<Row> {
    keys: Vec<String>,
    rows: Vec<Row>,
}

impl<Row: ContentRow> Table<Row> {
    pub fn new(mut rows: Vec<(String, Row)>) -> Table<Row> {
        rows.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
        for pair in rows.windows(2) {
            if pair[0].0.eq_ignore_ascii_case(&pair[1].0) {
                panic!(
                    "{} has two rows keyed {:?} and {:?}",
                    Row::TABLE,
                    pair[0].0,
                    pair[1].0
                );
            }
        }
        let (keys, rows) = rows.into_iter().unzip();
        Table { keys, rows }
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = ContentId<Row>> + Clone + '_ {
        (0..self.rows.len() as u32).map(ContentId::new)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ContentId<Row>, &Row)> {
        self.ids().zip(&self.rows)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, id: ContentId<Row>) -> &Row {
        &self.rows[id.index()]
    }

    pub fn key(&self, id: ContentId<Row>) -> &str {
        &self.keys[id.index()]
    }

    pub fn id_of(&self, key: &str) -> Option<ContentId<Row>> {
        self.keys
            .binary_search_by(|candidate| candidate.as_bytes().cmp(key.as_bytes()))
            .ok()
            .or_else(|| {
                self.keys
                    .iter()
                    .position(|candidate| candidate.eq_ignore_ascii_case(key))
            })
            .map(|index| ContentId::new(index as u32))
    }
}

pub(super) trait AnyTable: Any + Send + Sync {
    fn name(&self) -> &'static str;
    fn keys(&self) -> &[String];
    fn as_any(&self) -> &dyn Any;
}

impl<Row: ContentRow> AnyTable for Table<Row> {
    fn name(&self) -> &'static str {
        Row::TABLE
    }

    fn keys(&self) -> &[String] {
        &self.keys
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Resource, Clone)]
pub struct Content(Arc<HashMap<TypeId, Box<dyn AnyTable>>>);

impl Content {
    pub fn table<Row: ContentRow>(&self) -> &Table<Row> {
        self.0
            .get(&TypeId::of::<Row>())
            .unwrap_or_else(|| panic!("table {} was never registered", Row::TABLE))
            .as_any()
            .downcast_ref()
            .expect("a table is stored under its row type")
    }

    pub fn ids<Row: ContentRow>(
        &self,
    ) -> impl ExactSizeIterator<Item = ContentId<Row>> + Clone + '_ {
        self.table::<Row>().ids()
    }

    pub fn by_key<Row: ContentRow>(&self, key: &str) -> Option<ContentId<Row>> {
        self.table::<Row>().id_of(key)
    }

    pub(super) fn tables(&self) -> impl Iterator<Item = (TypeId, &dyn AnyTable)> {
        self.0.iter().map(|(id, table)| (*id, table.as_ref()))
    }
}

#[derive(Default)]
pub struct ContentBuilder {
    tables: HashMap<TypeId, Box<dyn AnyTable>>,
}

impl ContentBuilder {
    pub fn table<Row: ContentRow>(&mut self, rows: Vec<(String, Row)>) -> &mut ContentBuilder {
        self.tables
            .insert(TypeId::of::<Row>(), Box::new(Table::new(rows)));
        self
    }

    pub fn build(self) -> Content {
        Content(Arc::new(self.tables))
    }
}
