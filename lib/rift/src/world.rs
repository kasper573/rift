use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Mutex, OnceLock};

use crate::codec::{Tag, Wire, tag};

// Populated automatically when a world first creates a column for a type, so the migratable set
// is always exactly what the game uses; `Cluster` rebuilds whole entities from it.
type Reinsert = fn(&mut World, Entity, &mut &[u8]);

fn registry() -> &'static Mutex<HashMap<Tag, Reinsert>> {
    static REGISTRY: OnceLock<Mutex<HashMap<Tag, Reinsert>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
fn register_migratable<T: Wire + 'static>() {
    registry()
        .lock()
        .unwrap()
        .entry(tag::<T>())
        .or_insert(reinsert::<T>);
}
fn reinsert<T: Wire + 'static>(world: &mut World, entity: Entity, bytes: &mut &[u8]) {
    if let Some(value) = T::decode(bytes) {
        world.insert(entity, value);
    }
}

#[derive(crate::Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Entity(pub u32);

#[derive(crate::Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ClientId(pub u32);

// Default SipHash is cryptographic and far too slow for keys touched on every component access.
#[derive(Default)]
pub struct FastHasher(u64);
impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.0 = (self.0 ^ u64::from(value)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}
pub type Fast = BuildHasherDefault<FastHasher>;
pub type Map<K, V> = HashMap<K, V, Fast>;
pub type Set<T> = HashSet<T, Fast>;
pub type OwnerFn = fn(&World, Entity) -> Option<ClientId>;

// Column-major storage: the server stores live typed values, the client raw snapshot bytes.
// `Send` so a boxed column — and thus a whole `World` — can move between threads for parallel
// shard ticking.
trait Column: Send {
    fn remove(&mut self, entity: Entity) -> bool;
    fn contains(&self, entity: Entity) -> bool;
    fn entities(&self) -> Vec<Entity>;
    fn encode(&self, entity: Entity, out: &mut Vec<u8>) -> bool;
}

// A sparse set; ids are never recycled, so there is no stale-index hazard.
const EMPTY: u32 = u32::MAX;
struct Typed<T> {
    slot: Vec<u32>,
    values: Vec<T>,
    owners: Vec<Entity>,
}
impl<T> Default for Typed<T> {
    fn default() -> Self {
        Self {
            slot: Vec::new(),
            values: Vec::new(),
            owners: Vec::new(),
        }
    }
}
impl<T> Typed<T> {
    fn get(&self, entity: Entity) -> Option<&T> {
        match *self.slot.get(entity.0 as usize)? {
            EMPTY => None,
            index => Some(&self.values[index as usize]),
        }
    }
    fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        match *self.slot.get(entity.0 as usize)? {
            EMPTY => None,
            index => Some(&mut self.values[index as usize]),
        }
    }
    fn insert(&mut self, entity: Entity, value: T) {
        let key = entity.0 as usize;
        if key >= self.slot.len() {
            self.slot.resize(key + 1, EMPTY);
        }
        match self.slot[key] {
            EMPTY => {
                self.slot[key] = self.values.len() as u32;
                self.values.push(value);
                self.owners.push(entity);
            }
            index => self.values[index as usize] = value,
        }
    }
    fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.owners.iter().copied().zip(&self.values)
    }
    fn take(&mut self, entity: Entity) -> Option<T> {
        let key = entity.0 as usize;
        let index = *self.slot.get(key)?;
        if index == EMPTY {
            return None;
        }
        let value = self.values.swap_remove(index as usize);
        self.owners.swap_remove(index as usize);
        self.slot[key] = EMPTY;
        if let Some(&moved) = self.owners.get(index as usize) {
            self.slot[moved.0 as usize] = index;
        }
        Some(value)
    }
}
impl<T: Wire + 'static> Column for Typed<T> {
    fn remove(&mut self, entity: Entity) -> bool {
        self.take(entity).is_some()
    }
    fn contains(&self, entity: Entity) -> bool {
        matches!(self.slot.get(entity.0 as usize), Some(&index) if index != EMPTY)
    }
    fn entities(&self) -> Vec<Entity> {
        self.owners.clone()
    }
    fn encode(&self, entity: Entity, out: &mut Vec<u8>) -> bool {
        match self.get(entity) {
            Some(value) => {
                value.encode(out);
                true
            }
            None => false,
        }
    }
}

struct Raw(Map<Entity, Vec<u8>>);
impl Column for Raw {
    fn remove(&mut self, entity: Entity) -> bool {
        self.0.remove(&entity).is_some()
    }
    fn contains(&self, entity: Entity) -> bool {
        self.0.contains_key(&entity)
    }
    fn entities(&self) -> Vec<Entity> {
        self.0.keys().copied().collect()
    }
    fn encode(&self, entity: Entity, out: &mut Vec<u8>) -> bool {
        match self.0.get(&entity) {
            Some(bytes) => {
                out.extend_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

enum Kind {
    Typed(TypeId),
    Raw,
}

struct Cell {
    kind: Kind,
    column: Box<dyn Column>,
}

impl Cell {
    // `Kind` records which type was boxed, so the downcast is checked without a vtable call.
    fn typed<T: 'static>(&self) -> Option<&Typed<T>> {
        match self.kind {
            Kind::Typed(id) if id == TypeId::of::<T>() => {
                // SAFETY: `Kind::Typed(TypeId::of::<T>())` is only set when the box holds a Typed<T>.
                Some(unsafe { &*(self.column.as_ref() as *const dyn Column as *const Typed<T>) })
            }
            _ => None,
        }
    }
    fn typed_mut<T: 'static>(&mut self) -> Option<&mut Typed<T>> {
        match self.kind {
            Kind::Typed(id) if id == TypeId::of::<T>() => {
                // SAFETY: see `typed`.
                Some(unsafe { &mut *(self.column.as_mut() as *mut dyn Column as *mut Typed<T>) })
            }
            _ => None,
        }
    }
    fn raw_mut(&mut self) -> Option<&mut Raw> {
        match self.kind {
            // SAFETY: `Kind::Raw` is only set when the box holds a Raw.
            Kind::Raw => {
                Some(unsafe { &mut *(self.column.as_mut() as *mut dyn Column as *mut Raw) })
            }
            Kind::Typed(_) => None,
        }
    }
    fn raw(&self) -> Option<&Raw> {
        match self.kind {
            // SAFETY: see `raw_mut`.
            Kind::Raw => {
                Some(unsafe { &*(self.column.as_ref() as *const dyn Column as *const Raw) })
            }
            Kind::Typed(_) => None,
        }
    }
}

#[derive(Default)]
pub struct World {
    next: u32,
    alive: HashSet<Entity, Fast>,
    columns: Map<Tag, Cell>,
    pub(crate) dirty: Vec<(Entity, Tag)>,
    pub(crate) gone: Vec<(Entity, Tag)>,
}

impl World {
    pub fn spawn(&mut self) -> Entity {
        let entity = Entity(self.next);
        self.next += 1;
        self.alive.insert(entity);
        entity
    }
    pub fn despawn(&mut self, entity: Entity) {
        if self.alive.remove(&entity) {
            for cell in self.columns.values_mut() {
                cell.column.remove(entity);
            }
            self.gone.push((entity, Tag(u64::MAX)));
        }
    }

    pub fn alive(&self, entity: Entity) -> bool {
        self.alive.contains(&entity)
    }

    pub fn entity_count(&self) -> usize {
        self.alive.len()
    }
    pub fn all_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive.iter().copied()
    }

    pub fn insert<T: Wire + 'static>(&mut self, entity: Entity, value: T) {
        if !self.alive.contains(&entity) {
            return;
        }
        let tag = tag::<T>();
        let cell = self.columns.entry(tag).or_insert_with(|| {
            register_migratable::<T>();
            Cell {
                kind: Kind::Typed(TypeId::of::<T>()),
                column: Box::new(Typed::<T>::default()),
            }
        });
        if let Some(typed) = cell.typed_mut::<T>() {
            typed.insert(entity, value);
            self.dirty.push((entity, tag));
        }
    }

    pub fn get<T: Wire + Clone + 'static>(&self, entity: Entity) -> Option<T> {
        let cell = self.columns.get(&tag::<T>())?;
        if let Some(typed) = cell.typed::<T>() {
            typed.get(entity).cloned()
        } else if let Some(raw) = cell.raw() {
            raw.0
                .get(&entity)
                .and_then(|bytes| T::decode(&mut bytes.as_slice()))
        } else {
            None
        }
    }
    pub fn modify<T: Wire + 'static>(&mut self, entity: Entity, edit: impl FnOnce(&mut T)) {
        let tag = tag::<T>();
        let Some(cell) = self.columns.get_mut(&tag) else {
            return;
        };
        if let Some(typed) = cell.typed_mut::<T>() {
            if let Some(value) = typed.get_mut(entity) {
                edit(value);
                self.dirty.push((entity, tag));
            }
            return;
        }
        if let Some(raw) = cell.raw_mut() {
            let Some(mut value) = raw
                .0
                .get(&entity)
                .and_then(|b| T::decode(&mut b.as_slice()))
            else {
                return;
            };
            edit(&mut value);
            let mut bytes = Vec::new();
            value.encode(&mut bytes);
            raw.0.insert(entity, bytes);
            self.dirty.push((entity, tag));
        }
    }
    pub fn remove<T: Wire + 'static>(&mut self, entity: Entity) {
        let tag = tag::<T>();
        if let Some(cell) = self.columns.get_mut(&tag)
            && cell.column.remove(entity)
        {
            self.gone.push((entity, tag));
        }
    }

    /// Pairs with `insert` to edit a heap-backed component in place without reallocating.
    pub fn take<T: Wire + 'static>(&mut self, entity: Entity) -> Option<T> {
        let tag = tag::<T>();
        let value = self.columns.get_mut(&tag)?.typed_mut::<T>()?.take(entity);
        if value.is_some() {
            self.gone.push((entity, tag));
        }
        value
    }

    pub fn has<T: Wire + 'static>(&self, entity: Entity) -> bool {
        self.columns
            .get(&tag::<T>())
            .is_some_and(|cell| cell.column.contains(entity))
    }
    pub fn iter<T: Wire + Clone + 'static>(&self) -> Box<dyn Iterator<Item = (Entity, T)> + '_> {
        match self.columns.get(&tag::<T>()) {
            Some(cell) => {
                if let Some(typed) = cell.typed::<T>() {
                    Box::new(typed.iter().map(|(entity, value)| (entity, value.clone())))
                } else if let Some(raw) = cell.raw() {
                    Box::new(raw.0.iter().filter_map(|(&entity, bytes)| {
                        T::decode(&mut bytes.as_slice()).map(|value| (entity, value))
                    }))
                } else {
                    Box::new(std::iter::empty())
                }
            }
            None => Box::new(std::iter::empty()),
        }
    }

    pub fn ids<T: Wire + 'static>(&self) -> Vec<Entity> {
        self.columns
            .get(&tag::<T>())
            .map(|cell| cell.column.entities())
            .unwrap_or_default()
    }

    pub(crate) fn contains_tag(&self, entity: Entity, tag: Tag) -> bool {
        self.columns
            .get(&tag)
            .is_some_and(|cell| cell.column.contains(entity))
    }
    pub(crate) fn encode_tag(&self, entity: Entity, tag: Tag, out: &mut Vec<u8>) -> bool {
        self.columns
            .get(&tag)
            .is_some_and(|cell| cell.column.encode(entity, out))
    }
    pub(crate) fn components_of(&self, entity: Entity) -> Vec<Tag> {
        self.columns
            .iter()
            .filter(|(_, cell)| cell.column.contains(entity))
            .map(|(&tag, _)| tag)
            .collect()
    }

    pub(crate) fn extract(&self, entity: Entity) -> Vec<(Tag, Vec<u8>)> {
        let mut out = Vec::new();
        for tag in self.components_of(entity) {
            let mut bytes = Vec::new();
            if self.encode_tag(entity, tag, &mut bytes) {
                out.push((tag, bytes));
            }
        }
        out
    }
    pub(crate) fn reconstruct(&mut self, components: &[(Tag, Vec<u8>)]) -> Entity {
        let entity = self.spawn();
        for (tag, bytes) in components {
            // The fn may insert (and so re-lock to register a column); copy it out first.
            let reinsert = registry().lock().unwrap().get(tag).copied();
            if let Some(reinsert) = reinsert {
                reinsert(self, entity, &mut bytes.as_slice());
            }
        }
        entity
    }

    pub(crate) fn apply_raw(&mut self, entity: Entity, tag: Tag, bytes: Vec<u8>) {
        self.alive.insert(entity);
        let cell = self.columns.entry(tag).or_insert_with(|| Cell {
            kind: Kind::Raw,
            column: Box::new(Raw(Map::default())),
        });
        if let Some(raw) = cell.raw_mut() {
            raw.0.insert(entity, bytes);
        }
    }
    pub(crate) fn drop_entity(&mut self, entity: Entity) {
        self.alive.remove(&entity);
        for cell in self.columns.values_mut() {
            cell.column.remove(entity);
        }
    }
    pub(crate) fn drop_component(&mut self, entity: Entity, tag: Tag) {
        if let Some(cell) = self.columns.get_mut(&tag) {
            cell.column.remove(entity);
        }
    }
    pub(crate) fn clear(&mut self) {
        self.alive.clear();
        self.columns.clear();
    }
}
