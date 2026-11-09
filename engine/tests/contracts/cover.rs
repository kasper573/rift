use bevy::prelude::*;
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER, shore_distances};
use engine::core::math::Pos;
use engine::core::render::world::cover::{Patch, Place, SHORE_REACH, Spread, strew_places};
use engine::core::render::world::lod::GRAIN_PIXELS;
use engine::core::units::{GROUND_ORIGIN, Meters};
use testkit::HEIGHTS;

const SIZE: GroundSize = GroundSize {
    width: 16,
    height: 16,
};
const DARK: LinearRgba = LinearRgba::rgb(0.1, 0.4, 0.1);
const LIGHT: LinearRgba = LinearRgba::rgb(0.4, 0.8, 0.3);

/// Ground rising a metre every four metres eastward.
fn slope() -> Heightfield {
    Heightfield::from_fn(SIZE, HEIGHTS, |column, _| {
        Meters(column as f32 / SAMPLES_PER_METER as f32 / 4.0)
    })
}

fn patch<'a>(heights: &'a Heightfield, share: &'a dyn Fn(u32, u32) -> f32) -> Patch<'a> {
    let (columns, rows) = SIZE.paint_pixels();
    Patch {
        heights,
        first: (0, 0),
        columns,
        rows,
        share,
    }
}

#[test]
fn water_tells_how_far_it_lies_from_the_nearest_dry_ground_up_to_the_shore_reach() {
    let spacing = Meters(1.0 / SAMPLES_PER_METER as f32);
    let (columns, rows) = (80, 4);
    let lake: Vec<bool> = (0..columns * rows).map(|at| at % columns < 60).collect();
    for (at, distance) in shore_distances(&lake, columns, spacing, SHORE_REACH)
        .iter()
        .enumerate()
    {
        let column = at % columns;
        let ashore = (60usize.saturating_sub(column)) as f32 * spacing.0;
        assert!(
            (distance.0 - ashore.min(SHORE_REACH.0)).abs() < 1e-4,
            "column {column} lies {ashore} m out, not {}",
            distance.0
        );
    }
    let side = 41;
    let rock: Vec<bool> = (0..side * side).map(|at| at != side * side / 2).collect();
    for (at, distance) in shore_distances(&rock, side, spacing, SHORE_REACH)
        .iter()
        .enumerate()
    {
        let (x, y) = ((at % side) as f32 - 20.0, (at / side) as f32 - 20.0);
        let straight = x.hypot(y) * spacing.0;
        assert!(
            (distance.0 - straight).abs() <= straight * 0.083,
            "{x}, {y} from a rock lies {straight} m from it, not {}",
            distance.0
        );
    }
    let open = shore_distances(&[true; 64], 8, spacing, SHORE_REACH);
    assert!(
        open.iter().all(|distance| *distance == SHORE_REACH),
        "no dry ground in reach"
    );
}

#[test]
fn strewn_copies_of_a_prop_stand_on_the_ground_where_the_paint_puts_them() {
    let heights = slope();
    let full = |_: u32, _: u32| 1.0;
    let spread = Spread {
        piece: 1,
        per_square_meter: 1.0,
        size: (0.5, 2.0),
        color: (DARK, LIGHT),
    };
    let places = strew_places(&patch(&heights, &full), &spread, 3);
    let whole = (SIZE.width * SIZE.height) as f32;
    assert!(
        (places.len() as f32 - whole).abs() <= whole * 0.1,
        "{} copies, not about {whole}",
        places.len()
    );
    for place in &places {
        let root = place.at;
        assert!(
            (root.y - heights.height_at(Pos::new(root.x, root.z)).0).abs() < 1e-4,
            "a copy at {root:?} stands on the ground"
        );
        assert_eq!(place.weight, 1.0);
        assert!(
            (spread.size.0..=spread.size.1).contains(&place.scale)
                && (0.0..=1.0).contains(&place.tint)
                && (0.0..=1.0).contains(&place.seed)
                && (0.0..std::f32::consts::TAU).contains(&place.yaw.0),
            "a copy's own draws stand within their ranges: {place:?}"
        );
    }
    let seeds: std::collections::HashSet<u32> =
        places.iter().map(|place| place.seed.to_bits()).collect();
    assert!(
        seeds.len() > places.len() / 2,
        "each copy has a seed of its own"
    );
    assert_eq!(
        strew_places(&patch(&heights, &full), &spread, 3),
        places,
        "the same seed strews alike"
    );
    let half = |column: u32, _: u32| if column < 16 { 1.0 } else { 0.0 };
    let painted = strew_places(&patch(&heights, &half), &spread, 3);
    assert!(
        !painted.is_empty()
            && painted
                .iter()
                .all(|place| place.at.x < GROUND_ORIGIN.x + 8.0),
        "copies stand only where the paint puts them"
    );
}

#[test]
fn a_packed_crowd_seen_from_afar_thins_to_a_copy_a_grain_covering_as_much() {
    let heights = slope();
    let full = |_: u32, _: u32| 1.0;
    let spread = Spread {
        piece: 1,
        per_square_meter: 16.0,
        size: (1.0, 1.0),
        color: (DARK, LIGHT),
    };
    let places = strew_places(&patch(&heights, &full), &spread, 3);
    let extent = 0.5;
    let packed = 1.5 * GRAIN_PIXELS / extent;
    let apart = packed / spread.per_square_meter.sqrt();
    assert!(
        spread.told_apart(extent, packed * 2.0) && !spread.told_apart(extent, packed),
        "copies wide enough to tell apart but packed closer than the grain aren't told apart"
    );
    let thinned = strew_places(&patch(&heights, &full), &spread.grained(extent, packed), 3);
    let covered =
        |places: &[Place]| -> f32 { places.iter().map(|place| place.scale.powi(2)).sum() };
    let cover_share = covered(&thinned) / covered(&places);
    assert!(
        (cover_share - 1.0).abs() < 0.05,
        "the copies a packed crowd thins to cover as much: {cover_share}"
    );
    let grains = places.len() as f32 * (apart / GRAIN_PIXELS).powi(2);
    assert!(
        thinned.len() as f32 <= grains * 1.05,
        "a packed crowd thins to a copy a grain: {} copies over {grains} grains",
        thinned.len()
    );
}
