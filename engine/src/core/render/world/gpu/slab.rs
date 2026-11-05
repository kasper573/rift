//! Room in one GPU buffer for blocks of bytes that come and go, as models' meshes do. A block keeps
//! its place for as long as it lives, so what refers to it by offset never has to move. Freed room
//! joins the free room beside it and is given out again, and when no room is left the buffer
//! doubles, keeping every block where it was.

use std::collections::BTreeSet;
use std::ops::Range;

use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue};

/// Where the blocks of one buffer lie: each a power of two of `unit` bytes, split off a larger
/// free one and joined back with its twin once both are free.
#[derive(Debug)]
pub struct SlabSpace {
    unit: u64,
    /// Units the space holds: a power of two, or none before the first block.
    capacity: u64,
    /// The free blocks of each order, by their first unit: a block of order `k` is `2^k` units.
    free: Vec<BTreeSet<u64>>,
    blocks: Vec<Block>,
    vacant: Vec<u32>,
}

/// A block a [`SlabSpace`] gave out, which names nothing once it is freed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlabBlock {
    index: u32,
    generation: u32,
}

impl SlabSpace {
    /// An empty space giving out blocks in whole `unit`s of bytes, each block starting on one.
    pub fn new(unit: u64) -> SlabSpace {
        SlabSpace {
            unit: unit.max(1),
            capacity: 0,
            free: Vec::new(),
            blocks: Vec::new(),
            vacant: Vec::new(),
        }
    }

    /// A block of at least `bytes`, the space doubling until it has room.
    pub fn alloc(&mut self, bytes: u64) -> SlabBlock {
        let units = bytes.div_ceil(self.unit).max(1);
        let order = units.next_power_of_two().trailing_zeros() as usize;
        let (first, mut at) = loop {
            let found = (order..self.free.len())
                .find_map(|at| self.free[at].first().map(|&first| (first, at)));
            if let Some((first, at)) = found {
                self.free[at].remove(&first);
                break (first, at);
            }
            self.grow();
        };
        while at > order {
            at -= 1;
            self.free[at].insert(first + (1 << at));
        }
        let block = Block {
            first,
            order: order as u8,
            bytes,
            generation: 0,
            live: true,
        };
        match self.vacant.pop() {
            Some(index) => {
                let held = &mut self.blocks[index as usize];
                let generation = held.generation.wrapping_add(1);
                *held = Block {
                    generation,
                    ..block
                };
                SlabBlock { index, generation }
            }
            None => {
                self.blocks.push(block);
                SlabBlock {
                    index: self.blocks.len() as u32 - 1,
                    generation: 0,
                }
            }
        }
    }

    /// Frees `block`, its room given out again: false if it was freed already.
    pub fn free(&mut self, block: SlabBlock) -> bool {
        let Some(held) = self.live_mut(block) else {
            return false;
        };
        held.live = false;
        let (first, order) = (held.first, held.order as usize);
        self.vacant.push(block.index);
        self.release(first, order);
        true
    }

    /// The bytes of the buffer `block` holds, as many as it was asked for: none once it is freed.
    pub fn range(&self, block: SlabBlock) -> Option<Range<u64>> {
        let held = self.blocks.get(block.index as usize)?;
        (held.live && held.generation == block.generation).then(|| {
            let start = held.first * self.unit;
            start..start + held.bytes
        })
    }

    /// The bytes the buffer must hold for every block to fit.
    pub fn capacity(&self) -> u64 {
        self.capacity * self.unit
    }

    fn live_mut(&mut self, block: SlabBlock) -> Option<&mut Block> {
        self.blocks
            .get_mut(block.index as usize)
            .filter(|held| held.live && held.generation == block.generation)
    }

    /// Doubles the space, its new half free.
    fn grow(&mut self) {
        if self.capacity == 0 {
            self.capacity = 1;
            self.free.push(BTreeSet::from([0]));
            return;
        }
        let half = self.capacity;
        let order = half.trailing_zeros() as usize;
        self.capacity *= 2;
        self.free.push(BTreeSet::new());
        self.release(half, order);
    }

    /// Frees the block of `order` at `first`, joined with its twin for as long as that is free.
    fn release(&mut self, mut first: u64, mut order: usize) {
        let top = self.capacity.trailing_zeros() as usize;
        while order < top {
            let twin = first ^ (1 << order);
            if !self.free[order].remove(&twin) {
                break;
            }
            first = first.min(twin);
            order += 1;
        }
        self.free[order].insert(first);
    }
}

/// A GPU buffer holding blocks of bytes where a [`SlabSpace`] places them, grown by copying into
/// one twice the size, so every block stays at its offset.
pub struct Slab {
    label: &'static str,
    usage: BufferUsages,
    space: SlabSpace,
    buffer: Option<Buffer>,
}

impl Slab {
    pub fn new(label: &'static str, usage: BufferUsages, unit: u64) -> Slab {
        Slab {
            label,
            usage: usage | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            space: SlabSpace::new(unit),
            buffer: None,
        }
    }

    /// Writes `bytes` into a block of their own, the buffer grown first if they don't fit.
    pub fn insert(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        bytes: &[u8],
    ) -> SlabBlock {
        let block = self.space.alloc(bytes.len() as u64);
        self.fit(device, queue);
        if let (Some(buffer), Some(range)) = (&self.buffer, self.space.range(block)) {
            queue.write_buffer(buffer, range.start, bytes);
        }
        block
    }

    /// Frees `block`'s bytes for others.
    pub fn remove(&mut self, block: SlabBlock) {
        self.space.free(block);
    }

    pub fn range(&self, block: SlabBlock) -> Option<Range<u64>> {
        self.space.range(block)
    }

    /// The buffer, none before the first block: a new one each time it grows.
    pub fn buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    /// Grows the buffer to the space's capacity, copying what it held to the same offsets.
    fn fit(&mut self, device: &RenderDevice, queue: &RenderQueue) {
        let size = self.space.capacity();
        if self
            .buffer
            .as_ref()
            .is_some_and(|buffer| buffer.size() >= size)
        {
            return;
        }
        let grown = device.create_buffer(&BufferDescriptor {
            label: Some(self.label),
            size,
            usage: self.usage,
            mapped_at_creation: false,
        });
        if let Some(old) = self.buffer.take() {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&old, 0, &grown, 0, old.size());
            queue.submit([encoder.finish()]);
        }
        self.buffer = Some(grown);
    }
}

#[derive(Debug)]
struct Block {
    first: u64,
    order: u8,
    bytes: u64,
    generation: u32,
    live: bool,
}
