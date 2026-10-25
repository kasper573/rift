use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use engine::core::math::Pos;
use engine::core::render::world::cover::{
    BladeShape, Mark, Patch, Piece, Spread, blades, sheet, strew_places, strewn,
};
use engine::core::terrain::{Heightfield, SAMPLES_PER_METER, TerrainSize};
use engine::core::units::Meters;

const SIZE: TerrainSize = TerrainSize {
    width: 16,
    height: 16,
};
const BLADE: BladeShape = BladeShape {
    width: 0.06,
    height: 0.4,
};
const DARK: LinearRgba = LinearRgba::rgb(0.1, 0.4, 0.1);
const LIGHT: LinearRgba = LinearRgba::rgb(0.4, 0.8, 0.3);
const SPREAD: Spread = Spread {
    piece: 2,
    per_square_meter: 16.0,
    size: (0.7, 1.3),
    color: (DARK, LIGHT),
};

/// Ground rising a metre every four metres eastward.
fn slope() -> Heightfield {
    Heightfield::from_fn(SIZE, |column, _| {
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

fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|positions| positions.as_float3())
        .expect("positions")
        .to_vec()
}

fn colors(mesh: &Mesh) -> Vec<[f32; 4]> {
    match mesh.attribute(Mesh::ATTRIBUTE_COLOR).expect("colours") {
        VertexAttributeValues::Float32x4(colors) => colors.clone(),
        _ => panic!("colours are rgba"),
    }
}

/// Whether `color` lies on the way from `from` to `to`, channel by channel.
fn between(color: [f32; 4], from: LinearRgba, to: LinearRgba) -> bool {
    let (from, to) = (from.to_f32_array(), to.to_f32_array());
    (0..3).all(|channel| {
        let (low, high) = (
            from[channel].min(to[channel]),
            from[channel].max(to[channel]),
        );
        color[channel] >= low - 1e-4 && color[channel] <= high + 1e-4
    })
}

fn marks(mesh: &Mesh) -> Vec<Mark> {
    match mesh.attribute(Mesh::ATTRIBUTE_UV_1).expect("marks") {
        VertexAttributeValues::Float32x2(marks) => marks.iter().map(|uv| Mark::of(*uv)).collect(),
        _ => panic!("marks are pairs"),
    }
}

#[test]
fn blades_grow_as_thick_as_their_ground_is_painted_rooted_on_the_terrain_each_its_own_size_colour_and_seed()
 {
    let heights = slope();
    let full = |_: u32, _: u32| 1.0;
    let half = |_: u32, _: u32| 0.5;
    let none = |_: u32, _: u32| 0.0;
    let count = |share: &dyn Fn(u32, u32) -> f32, seed: u64| {
        blades(&patch(&heights, share), &BLADE, &SPREAD, seed)
            .map_or(0, |mesh| mesh.count_vertices() / 3)
    };
    let whole = (SIZE.width * SIZE.height) as f32 * SPREAD.per_square_meter;
    assert!(
        (count(&full, 7) as f32 - whole).abs() <= whole * 0.05,
        "full weight grew {} blades, not about {whole}",
        count(&full, 7)
    );
    assert!(
        (count(&half, 7) as f32 - whole / 2.0).abs() <= whole * 0.05,
        "half weight grew {} blades, not about {}",
        count(&half, 7),
        whole / 2.0
    );
    assert_eq!(count(&none, 7), 0, "unpainted ground grows nothing");
    let grown = |seed| blades(&patch(&heights, &full), &BLADE, &SPREAD, seed).expect("grass");
    assert_eq!(
        positions(&grown(7)),
        positions(&grown(7)),
        "the same seed grows the same"
    );
    assert_ne!(
        positions(&grown(7)),
        positions(&grown(8)),
        "another seed scatters anew"
    );

    let mesh = grown(7);
    let seeds: std::collections::BTreeSet<u32> = marks(&mesh)
        .iter()
        .map(|mark| (mark.seed * 1000.0) as u32)
        .collect();
    assert!(
        seeds.len() > 100,
        "blades carry seeds of their own, not {seeds:?}"
    );
    for blade in marks(&mesh).chunks(3) {
        assert!(
            blade
                .iter()
                .all(|mark| mark.seed == blade[0].seed && mark.piece == SPREAD.piece),
            "a blade's vertices share its seed and name their piece: {blade:?}"
        );
    }
    let mut shades = std::collections::BTreeSet::new();
    for blade in colors(&mesh).chunks(3) {
        assert!(
            blade
                .iter()
                .all(|color| *color == blade[0] && between(*color, DARK, LIGHT)),
            "a blade is one colour drawn between the piece's two: {blade:?}"
        );
        shades.insert((blade[0][1] * 100.0) as u32);
    }
    assert!(shades.len() > 10, "blades differ in colour: {shades:?}");
    for (position, mark) in positions(&mesh).iter().zip(marks(&mesh)) {
        let ground = heights.height_at(Pos::new(position[0], position[2])).0;
        assert!(mark.weight > 0.99, "a blade on full weight carries it");
        if mark.measure == 0.0 {
            assert!(
                (position[1] - ground).abs() < 0.02,
                "a root at {position:?} stands {ground} above the ground"
            );
        } else {
            let height = position[1] - ground;
            assert!(
                (BLADE.height * SPREAD.size.0..=BLADE.height * SPREAD.size.1).contains(&height),
                "a tip stands {height} m over the ground, outside the piece's sizes"
            );
        }
    }
}

#[test]
fn a_sheet_lies_on_the_terrain_or_flat_at_a_level_weighted_as_the_cells_around_each_point_are_painted()
 {
    let heights = slope();
    let west = |column: u32, _: u32| if column < 8 { 1.0 } else { 0.0 };
    let mesh = sheet(&patch(&heights, &west), None, DARK).expect("a sheet");
    assert!(
        colors(&mesh)
            .iter()
            .all(|color| *color == DARK.to_f32_array()),
        "a sheet is the ground's colour"
    );
    for (position, mark) in positions(&mesh).iter().zip(marks(&mesh)) {
        let at = Pos::new(position[0], position[2]);
        let ground = heights.height_at(at).0;
        assert!(
            (position[1] - ground - 0.02).abs() < 0.01,
            "{position:?} lies {} over the ground",
            position[1] - ground
        );
        let column = ((at.x + 0.5) * SAMPLES_PER_METER as f32).round() as u32;
        let wanted = match column {
            0..=7 => 1.0,
            8 => 0.5,
            _ => 0.0,
        };
        assert!(
            mark.measure == 0.0 && (mark.weight - wanted).abs() < 0.005,
            "the mark at column {column}: {mark:?}, not a weight of {wanted}"
        );
    }
    let level = 1.0;
    let pool = sheet(&patch(&heights, &west), Some(level), DARK).expect("a pool");
    for (position, mark) in positions(&pool).iter().zip(marks(&pool)) {
        assert_eq!(position[1], level, "a pool lies flat at its level");
        let ground = heights.height_at(Pos::new(position[0], position[2])).0;
        assert!(
            (mark.measure - (level - ground).max(0.0)).abs() < 1e-4,
            "{position:?} over ground at {ground} is marked {} m deep",
            mark.measure
        );
    }
    let none = |_: u32, _: u32| 0.0;
    assert!(
        sheet(&patch(&heights, &none), None, DARK).is_none(),
        "nothing painted, no sheet"
    );
}

#[test]
fn strewn_copies_of_a_prop_stand_where_the_paint_puts_them_marked_how_far_up_they_reach() {
    let heights = slope();
    let full = |_: u32, _: u32| 1.0;
    let spread = Spread {
        piece: 1,
        per_square_meter: 1.0,
        size: (1.0, 1.0),
        color: (DARK, LIGHT),
    };
    let places = strew_places(&patch(&heights, &full), &spread, 3);
    let whole = (SIZE.width * SIZE.height) as f32;
    assert!(
        (places.len() as f32 - whole).abs() <= whole * 0.1,
        "{} copies, not about {whole}",
        places.len()
    );
    let stem = Piece {
        mesh: Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 1.0, 0.0]],
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.5, 1.0, 1.0]; 3]),
        transform: Transform::from_scale(Vec3::splat(0.5)),
    };
    let mesh = strewn(&[stem], &places).expect("copies");
    assert_eq!(mesh.count_vertices(), places.len() * 3);
    for (copy_colors, place) in colors(&mesh).chunks(3).zip(&places) {
        let tint = place.tint.to_f32_array();
        assert!(
            copy_colors
                .iter()
                .all(|color| (color[1] - 0.5 * tint[1]).abs() < 1e-5 && color[0] == tint[0]),
            "a copy's colours are its prop's tinted by its place: {copy_colors:?} by {tint:?}"
        );
        assert!(between(tint, DARK, LIGHT));
    }
    for ((copy, copy_marks), place) in positions(&mesh)
        .chunks(3)
        .zip(marks(&mesh).chunks(3))
        .zip(&places)
    {
        let root = place.transform.translation;
        assert!(
            (Vec3::from_array(copy[2]) - root).length() < 0.6,
            "a copy's top at {:?} is far from its root at {root:?}",
            copy[2]
        );
        assert_eq!(place.weight, 1.0);
        assert!(
            copy_marks
                .iter()
                .all(|mark| (mark.seed - place.seed).abs() < 0.004
                    && mark.weight > 0.99
                    && mark.piece == 1),
            "a copy carries its place's seed and weight: {copy_marks:?} for {place:?}"
        );
    }
    for (mark, position) in marks(&mesh).iter().zip(positions(&mesh)) {
        assert!(
            (mark.measure
                - (position[1] - heights.height_at(Pos::new(position[0], position[2])).0) / 0.5)
                .abs()
                < 0.3,
            "a point {} m up is marked {} of the way",
            position[1],
            mark.measure
        );
    }
}
