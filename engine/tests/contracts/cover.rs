use bevy::prelude::*;
use engine::core::ground::{
    GroundSize, Heightfield, SAMPLES_PER_METER, WaterFalls, shore_distances,
};
use engine::core::math::Pos;
use engine::core::render::world::cover::{Patch, Place, Spread, strew_places};
use engine::core::render::world::lod::GRAIN_PIXELS;
use engine::core::render::world::water::{SHORE_BACK, SHORE_REACH};
use engine::core::units::{GROUND_ORIGIN, Meters};
use testkit::HEIGHTS;

const SIZE: GroundSize = GroundSize {
    width: 16,
    height: 16,
};
/// How near an even fall a river read from stepped levels runs along its way, as a share of the
/// even fall, and how much of the even fall it may run across its banks.
const EVENLY: std::ops::Range<f32> = 0.6..1.4;
const ACROSS: f32 = 0.2;
/// How many times as long as the straight way a way stepped from point to point, straight on
/// and cornerwise, is at its longest.
const STEPPED: f32 = 1.083;
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
fn a_river_whose_level_drops_in_steps_runs_evenly_downstream_round_its_bends() {
    let spacing = Meters(1.0 / SAMPLES_PER_METER as f32);
    let side = 200;
    // A channel five metres wide that runs east and then turns south, dropping eight
    // millimetres every ten metres of its way.
    let river: Vec<Option<f32>> = (0..side * side)
        .map(|at| {
            let (column, row) = (at % side, at / side);
            let along = if (40..50).contains(&row) && column < 160 {
                column
            } else if (150..160).contains(&column) && row >= 40 {
                155 + row - 45
            } else {
                return None;
            };
            Some(1.0 - 0.008 * (along / 20) as f32)
        })
        .collect();
    let falls = WaterFalls::read(&river, side, spacing);
    let even = 0.008 / 10.0;
    for (column, row, way) in [(80, 45, Vec2::X), (155, 120, Vec2::Y)] {
        let fall = falls.at(column, row);
        assert!(
            EVENLY.contains(&(fall.dot(way) / even)),
            "at {column}, {row} the river falls {fall} and not {even} toward {way}"
        );
        assert!(
            fall.perp_dot(way).abs() < even * ACROSS,
            "at {column}, {row} the river falls {fall} across its banks"
        );
    }
    assert_eq!(falls.at(20, 100), Vec2::ZERO, "dry ground far from it");

    let pond: Vec<Option<f32>> = (0..side * side)
        .map(|at| (at % side > 30).then_some(1.0))
        .collect();
    let still = WaterFalls::read(&pond, side, spacing);
    assert!(
        (0..side * side).all(|at| still.at(at % side, at / side).length() < 1e-6),
        "still water falls no way"
    );
}

#[test]
fn water_tells_how_far_it_lies_from_the_waterline_where_the_ground_comes_through_it() {
    let spacing = Meters(1.0 / SAMPLES_PER_METER as f32);
    let (columns, rows) = (80, 4);
    // A bank three times as steep under the water as over it: the waterline lies three
    // quarters of the way from the last wet point to the first dry one.
    let lake: Vec<f32> = (0..columns * rows)
        .map(|at| if at % columns < 60 { 0.3 } else { -0.1 })
        .collect();
    for (at, distance) in shore_distances(&lake, columns, spacing, SHORE_REACH, SHORE_BACK)
        .iter()
        .enumerate()
    {
        let column = at % columns;
        let out = ((59.75 - column as f32) * spacing.0).clamp(-SHORE_BACK.0, SHORE_REACH.0);
        assert!(
            (distance.0 - out).abs() < 1e-4,
            "column {column} lies {out} m out, not {}",
            distance.0
        );
    }
    let side = 41;
    let rock: Vec<f32> = (0..side * side)
        .map(|at| if at == side * side / 2 { -0.5 } else { 0.5 })
        .collect();
    for (at, distance) in shore_distances(&rock, side, spacing, SHORE_REACH, SHORE_BACK)
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != side * side / 2)
    {
        let (x, y) = ((at % side) as f32 - 20.0, (at / side) as f32 - 20.0);
        let straight = x.hypot(y) * spacing.0;
        let (nearest, farthest) = (
            straight - spacing.0 * std::f32::consts::FRAC_1_SQRT_2,
            (straight - spacing.0 * 0.5) * STEPPED,
        );
        assert!(
            distance.0 >= nearest - 1e-4 && distance.0 <= farthest + 1e-4,
            "{x}, {y} from a rock lies {straight} m from its middle, not {} from its edge",
            distance.0
        );
    }
    let open = shore_distances(&[1.0; 64], 8, spacing, SHORE_REACH, SHORE_BACK);
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
