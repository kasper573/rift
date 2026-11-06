//! What the GPU keeps of the world: blocks of bytes in a buffer that grows, the prop models laid in
//! such buffers, and the few slots of an atlas the tiles near the views take turns to hold.

use std::ops::Range;

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use engine::core::math::Rng;
use engine::core::render::world::cover::Piece;
use engine::core::render::world::gpu::arena::{
    INDEX_BYTES, ModelSlabs, ModelVertex, PartRanges, PreparedLevel, PreparedModel, PreparedPart,
};
use engine::core::render::world::gpu::residency::{Admitted, Residency};
use engine::core::render::world::gpu::slab::{SlabBlock, SlabSpace};

#[test]
fn a_slabs_blocks_keep_their_place_never_overlap_and_give_their_room_again_once_freed() {
    let mut rng = Rng::new(testkit::drawn_seed());
    let mut space = SlabSpace::new(16);
    let mut live: Vec<(SlabBlock, Range<u64>)> = Vec::new();
    let mut freed = Vec::new();
    for _ in 0..2000 {
        if live.is_empty() || rng.rand_range(0..3) > 0 {
            let bytes = u64::from(rng.rand_range(1..5000));
            let block = space.alloc(bytes);
            let range = space.range(block).expect("a block just given out");
            assert_eq!(range.end - range.start, bytes, "the bytes it was asked for");
            assert_eq!(range.start % 16, 0, "starting on a unit");
            live.push((block, range));
        } else {
            let (block, _) = live.swap_remove(rng.rand_range(0..live.len() as u32) as usize);
            assert!(space.free(block));
            freed.push(block);
        }
        for (block, range) in &live {
            assert_eq!(
                space.range(*block).as_ref(),
                Some(range),
                "where it was given out"
            );
            assert!(range.end <= space.capacity());
        }
        let mut ranges: Vec<&Range<u64>> = live.iter().map(|(_, range)| range).collect();
        ranges.sort_by_key(|range| range.start);
        for pair in ranges.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
    }
    for (block, _) in live.drain(..) {
        space.free(block);
    }
    for block in freed {
        assert_eq!(space.range(block), None, "a freed block names nothing");
        assert!(!space.free(block), "nor frees anything again");
    }
    let capacity = space.capacity();
    let whole = space.alloc(capacity);
    assert_eq!(
        (space.range(whole), space.capacity()),
        (Some(0..capacity), capacity),
        "all the room freed is one block again"
    );
}

#[test]
fn an_atlas_gives_its_slots_to_the_tiles_wanted_last_and_never_takes_one_wanted_this_frame() {
    let mut atlas: Residency<u32, u32> = Residency::new(3);
    for tile in [1, 2, 3] {
        assert_eq!(
            atlas.admit(tile, 0),
            Some(Admitted {
                slot: tile - 1,
                evicted: None
            }),
            "free slots first"
        );
    }
    assert_eq!(atlas.admit(4, 0), None, "every slot was wanted this frame");

    atlas.next_frame();
    atlas.want(1);
    atlas.want(3);
    assert_eq!(
        atlas.admit(4, 0),
        Some(Admitted {
            slot: 1,
            evicted: Some(2)
        }),
        "the one not wanted since"
    );
    assert_eq!(atlas.slot(2), None);

    atlas.next_frame();
    atlas.want(3);
    atlas.want(4);
    atlas.next_frame();
    atlas.want(4);
    assert_eq!(
        atlas.admit(2, 0).map(|admitted| admitted.evicted),
        Some(Some(1)),
        "the one wanted longest ago"
    );

    assert!(atlas.is_current(4, &0));
    assert!(!atlas.is_current(4, &1), "its data changed");
    assert_eq!(
        atlas.admit(4, 1),
        Some(Admitted {
            slot: 1,
            evicted: None
        }),
        "filled again in its own slot"
    );
    assert!(atlas.is_current(4, &1));
    assert_eq!(atlas.want(4), Some((1, &1)));
}

#[test]
fn a_models_parts_and_levels_lie_where_their_bytes_are_written_and_keep_their_place_apart_from_every_other_models()
 {
    let mut rng = Rng::new(testkit::drawn_seed());
    let mut slabs: ModelSlabs<u32> = ModelSlabs::default();
    let mut live: Vec<(u32, Vec<PartRanges>)> = Vec::new();
    for key in 0..300 {
        if live.is_empty() || rng.rand_range(0..3) > 0 {
            let model = model(&mut rng);
            let writes = slabs.insert(key, &model);
            let placed = slabs.get(&key).expect("a model just placed").parts.clone();
            for ((part, ranges), (vertices, indices)) in model
                .parts
                .iter()
                .zip(&placed)
                .zip(writes.vertices.iter().zip(&writes.indices))
            {
                assert_eq!(ranges.vertices as usize, part.vertices.len());
                assert_eq!(
                    (vertices.0, vertices.1.len() as u64),
                    (
                        u64::from(ranges.first_vertex) * ModelVertex::BYTES,
                        part.vertices.len() as u64 * ModelVertex::BYTES
                    ),
                    "the part's vertices are written where it lies"
                );
                let first = ranges.levels.first().expect("a part's full mesh");
                let written: Vec<u32> = indices
                    .1
                    .chunks(INDEX_BYTES as usize)
                    .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("an index")))
                    .collect();
                assert_eq!(indices.0, u64::from(first.first_index) * INDEX_BYTES);
                for (level, range) in part.levels.iter().zip(&ranges.levels) {
                    let at = (range.first_index - first.first_index) as usize;
                    assert_eq!(
                        &written[at..at + range.indices as usize],
                        level.indices.as_slice(),
                        "each level's indices are written where it lies"
                    );
                }
            }
            live.push((key, placed));
        } else {
            let (key, _) = live.swap_remove(rng.rand_range(0..live.len() as u32) as usize);
            assert!(slabs.remove(&key));
            assert!(slabs.get(&key).is_none(), "a model let go lies nowhere");
        }
        let (vertex_capacity, index_capacity) = slabs.capacity();
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (key, placed) in &live {
            assert_eq!(
                &slabs.get(key).expect("a live model").parts,
                placed,
                "a model keeps its place as others come and go"
            );
            for part in placed {
                let first = u64::from(part.first_vertex);
                vertices.push(first..first + u64::from(part.vertices));
                for level in &part.levels {
                    let first = u64::from(level.first_index);
                    indices.push(first..first + u64::from(level.indices));
                }
            }
        }
        for (ranges, capacity, unit) in [
            (&mut vertices, vertex_capacity, ModelVertex::BYTES),
            (&mut indices, index_capacity, INDEX_BYTES),
        ] {
            ranges.sort_by_key(|range| range.start);
            assert!(ranges.iter().all(|range| range.end * unit <= capacity));
            for pair in ranges.windows(2) {
                assert!(
                    pair[0].end <= pair[1].start,
                    "{:?} overlaps {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    let field = Plane3d::default()
        .mesh()
        .size(2.0, 2.0)
        .subdivisions(15)
        .build();
    let at = Transform::from_xyz(4.0, 1.0, -2.0).with_scale(Vec3::splat(2.0));
    let pieces = [
        Piece {
            mesh: field.clone(),
            transform: at,
            material: None,
        },
        Piece {
            mesh: field,
            transform: Transform::IDENTITY,
            material: Some(Handle::default()),
        },
    ];
    let prepared = PreparedModel::of(&pieces).expect("a field's model");
    assert_eq!(prepared.parts.len(), 2, "a part for each material");
    assert_eq!(
        prepared.bounds,
        Aabb::from_min_max(Vec3::new(-1.0, 0.0, -4.0), Vec3::new(6.0, 1.0, 1.0)),
        "the box both pieces fill where they stand"
    );
    for part in &prepared.parts {
        let triangles: Vec<usize> = part
            .levels
            .iter()
            .map(|level| level.indices.len())
            .collect();
        assert!(
            triangles.len() > 2,
            "a fine field simplifies: {triangles:?}"
        );
        assert!(
            triangles.windows(2).all(|pair| pair[1] < pair[0]),
            "each level coarser than the one before: {triangles:?}"
        );
        assert_eq!(part.levels[0].strays, 0.0, "the full mesh first");
        assert!(
            part.levels
                .iter()
                .flat_map(|level| &level.indices)
                .all(|&index| (index as usize) < part.vertices.len()),
            "every level over the part's own vertices"
        );
    }
}

/// A model of a few parts, each of some vertices and a few levels of indices over them.
fn model(rng: &mut Rng) -> PreparedModel {
    let parts = (0..rng.rand_range(1..4))
        .map(|_| {
            let vertices = rng.rand_range(3..300);
            PreparedPart {
                material: None,
                vertices: (0..vertices)
                    .map(|vertex| ModelVertex {
                        position: [vertex as f32, 0.0, 0.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 0.0],
                        color: [1.0; 4],
                    })
                    .collect(),
                levels: (0..rng.rand_range(1..5))
                    .map(|level| PreparedLevel {
                        indices: (0..rng.rand_range(1..200) * 3)
                            .map(|_| rng.rand_range(0..vertices))
                            .collect(),
                        strays: level as f32,
                    })
                    .collect(),
            }
        })
        .collect();
    PreparedModel {
        parts,
        bounds: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
    }
}
