//! The grid every area's ground is cut into, the same for every area and every game: what a client
//! is told of, what sleeps and what is worked on in parallel are all found chunk by chunk.

use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;

use crate::core::math::{Pos, Rect, Size};
use crate::core::units::Meters;

/// A power of two, so chunks nest evenly in every coarser square laid on the ground from its
/// origin, and about the reach of a client's sight, so what one sees spans a few chunks a side.
pub const CHUNK_SIDE: Meters = Meters(32.0);

/// One square of the grid, counted in chunks from the ground's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chunk {
    pub column: i32,
    pub row: i32,
}

impl Chunk {
    pub fn at(at: Pos<Meters>) -> Chunk {
        Chunk {
            column: (at.x / CHUNK_SIDE.0).floor() as i32,
            row: (at.y / CHUNK_SIDE.0).floor() as i32,
        }
    }

    pub fn rect(self) -> Rect<Meters> {
        Rect::new(
            Pos::new(
                self.column as f32 * CHUNK_SIDE.0,
                self.row as f32 * CHUNK_SIDE.0,
            ),
            Size::new(CHUNK_SIDE.0, CHUNK_SIDE.0),
        )
    }

    /// The chunks holding any ground within `reach` of `at`, and maybe a corner chunk that holds
    /// none.
    pub fn around(at: Pos<Meters>, reach: Meters) -> impl Iterator<Item = Chunk> + use<> {
        let low = Chunk::at(Pos::new(at.x - reach.0, at.y - reach.0));
        let high = Chunk::at(Pos::new(at.x + reach.0, at.y + reach.0));
        (low.row..=high.row).flat_map(move |row| {
            (low.column..=high.column).map(move |column| Chunk { column, row })
        })
    }
}

/// The chunk an entity stands in. Whoever moves the entity keeps it current, and the world's
/// `ChunkIndex` follows it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable, on_insert = file, on_discard = unfile)]
pub struct InChunk(pub Chunk);

/// Every entity with an `InChunk`, by its chunk.
#[derive(Resource, Default)]
pub struct ChunkIndex(HashMap<Chunk, Vec<Entity>>);

impl ChunkIndex {
    pub fn in_chunk(&self, chunk: Chunk) -> &[Entity] {
        self.0.get(&chunk).map_or(&[], Vec::as_slice)
    }

    /// The entities standing in the chunks around `at`: every one within `reach` of it, and some
    /// a little farther.
    pub fn around(&self, at: Pos<Meters>, reach: Meters) -> impl Iterator<Item = Entity> + '_ {
        Chunk::around(at, reach).flat_map(|chunk| self.in_chunk(chunk).iter().copied())
    }

    /// The chunks someone stands in.
    pub fn occupied(&self) -> impl Iterator<Item = Chunk> + '_ {
        self.0.keys().copied()
    }
}

fn file(mut world: DeferredWorld, context: HookContext) {
    let chunk = world
        .get::<InChunk>(context.entity)
        .expect("the component being inserted")
        .0;
    index(&mut world)
        .0
        .entry(chunk)
        .or_default()
        .push(context.entity);
}

fn unfile(mut world: DeferredWorld, context: HookContext) {
    let chunk = world
        .get::<InChunk>(context.entity)
        .expect("the component being replaced")
        .0;
    let mut index = index(&mut world);
    let Some(entities) = index.0.get_mut(&chunk) else {
        return;
    };
    entities.retain(|&entity| entity != context.entity);
    if entities.is_empty() {
        index.0.remove(&chunk);
    }
}

fn index<'w>(world: &'w mut DeferredWorld) -> Mut<'w, ChunkIndex> {
    world
        .get_resource_mut::<ChunkIndex>()
        .expect("a world filing entities by chunk holds a ChunkIndex")
}
