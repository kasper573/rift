//! What the GPU keeps of the world: blocks of bytes in a buffer that grows, and the few slots of
//! an atlas the tiles near the views take turns to hold.

use std::ops::Range;

use engine::core::math::Rng;
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
