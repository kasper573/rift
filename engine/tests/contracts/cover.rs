use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER};
use engine::core::math::Pos;
use engine::core::render::world::cover::{
    BladeShape, CopyModel, Mark, Patch, Piece, Spread, blades, copy_meshes, merged_copies, seen_at,
    sheet, strew_places,
};
use engine::core::render::world::lod::GRAIN_PIXELS;
use engine::core::units::Meters;
use testkit::HEIGHTS;

const SIZE: GroundSize = GroundSize {
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

/// The one mesh `meshes` holds, as a patch this small makes.
fn one(meshes: Vec<Mesh>) -> Mesh {
    let [mesh] = <[Mesh; 1]>::try_from(meshes).expect("one mesh");
    mesh
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
    let grow = |share: &dyn Fn(u32, u32) -> f32, spread: &Spread, seed: u64| {
        let places = strew_places(&patch(&heights, share), spread, seed);
        blades(&heights, &places, &BLADE, spread)
    };
    let count_of = |share: &dyn Fn(u32, u32) -> f32, spread: &Spread, seed: u64| {
        grow(share, spread, seed)
            .iter()
            .map(|mesh| mesh.count_vertices() / 3)
            .sum::<usize>()
    };
    let count = |share: &dyn Fn(u32, u32) -> f32, seed: u64| count_of(share, &SPREAD, seed);
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
    let dense = Spread {
        per_square_meter: 400.0,
        ..SPREAD
    };
    let meshes = grow(&full, &dense, 7);
    assert!(
        meshes.len() > 1
            && meshes
                .iter()
                .all(|mesh| mesh.count_vertices() <= u16::MAX as usize),
        "a dense cover comes as several meshes of a bounded size"
    );
    let dense_whole = whole * dense.per_square_meter / SPREAD.per_square_meter;
    assert!(
        (count_of(&full, &dense, 7) as f32 - dense_whole).abs() <= dense_whole * 0.05,
        "a dense cover grows every blade over its meshes"
    );
    let grown = |seed| one(grow(&full, &SPREAD, seed));
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
    let mesh = one(sheet(&patch(&heights, &west), None, DARK, 1));
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
    let pool = one(sheet(&patch(&heights, &west), Some(level), DARK, 1));
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
        sheet(&patch(&heights, &none), None, DARK, 1).is_empty(),
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
        material: None,
    };
    let (_, mesh) = copy_meshes(&[stem]).pop().expect("a mesh to copy");
    assert!(
        colors(&mesh)
            .iter()
            .all(|color| *color == [1.0, 0.5, 1.0, 1.0]),
        "a copy keeps its prop's colours, for its tint to fall on"
    );
    for place in &places {
        let root = place.transform.translation;
        assert!(
            (root.y - heights.height_at(Pos::new(root.x, root.z)).0).abs() < 1e-4,
            "a copy at {root:?} stands on the ground"
        );
        assert_eq!(place.weight, 1.0);
        assert!((0.0..=1.0).contains(&place.tint) && (0.0..=1.0).contains(&place.seed));
        for (mark, position) in marks(&mesh).iter().zip(positions(&mesh)) {
            let point = place.transform.transform_point(Vec3::from_array(position));
            assert!(
                (mark.measure - (point.y - root.y) / 0.5).abs() < 1e-4,
                "a point {} m up its copy is marked {} of the way",
                point.y - root.y,
                mark.measure
            );
        }
    }
    let seeds: std::collections::HashSet<u32> =
        places.iter().map(|place| place.seed.to_bits()).collect();
    assert!(
        seeds.len() > places.len() / 2,
        "each copy has a seed of its own"
    );
}

#[test]
fn a_crowd_seen_from_afar_keeps_all_it_tells_apart_as_it_is_and_grows_none_past_the_grain() {
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
    let near = GRAIN_PIXELS / extent;
    assert_eq!(
        seen_at(&places, extent, near),
        places,
        "copies the view tells apart stay, each as it is"
    );
    let far = near / 4.0;
    let kept = seen_at(&places, extent, far);
    let share = kept.len() as f32 / places.len() as f32;
    assert!(
        (share - 1.0 / 16.0).abs() < 0.03,
        "a quarter the size, a sixteenth stay to cover as much: {share}"
    );
    for copy in &kept {
        let across = extent * copy.transform.scale.x * far;
        assert!(
            (across - GRAIN_PIXELS).abs() < 1e-3,
            "a kept copy grows to the grain and no further: {across} px"
        );
    }
    let farther = seen_at(&places, extent, far / 2.0);
    assert!(
        farther.iter().all(|copy| kept
            .iter()
            .any(|nearer| nearer.transform.translation == copy.transform.translation)),
        "a farther view keeps fewer of the same copies"
    );

    let tuft: Vec<[f32; 3]> = (0..16)
        .flat_map(|blade| {
            let x = blade as f32 * 0.1;
            [[x, 0.0, 0.0], [x + 0.02, 0.0, 0.0], [x + 0.01, 0.05, 0.0]]
        })
        .collect();
    let count = tuft.len();
    let tuft = Piece {
        mesh: Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, tuft)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; count]),
        transform: Transform::IDENTITY,
        material: None,
    };
    let model = CopyModel::of(&[tuft]);
    let one = [places[0]];
    let blades_at = |pixels_per_meter: f32| -> Vec<f32> {
        merged_copies(&one, &model, &spread, pixels_per_meter)
            .iter()
            .flat_map(|(_, mesh)| {
                positions(mesh)
                    .chunks(3)
                    .map(|blade| {
                        let low = blade[0][0].min(blade[1][0]).min(blade[2][0]);
                        let high = blade[0][0].max(blade[1][0]).max(blade[2][0]);
                        let top = blade.iter().map(|at| at[1]).fold(f32::MIN, f32::max);
                        let root = blade.iter().map(|at| at[1]).fold(f32::MAX, f32::min);
                        (high - low).max(top - root)
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let seen = GRAIN_PIXELS / 0.05;
    let close = blades_at(seen);
    assert!(
        close.len() == 16 && close.iter().all(|across| (across - 0.05).abs() < 1e-4),
        "every blade the view tells apart stands at its own size: {close:?}"
    );
    for pixels_per_meter in [seen / 2.0, seen / 8.0, seen / 32.0] {
        let blades = blades_at(pixels_per_meter);
        assert!(
            !blades.is_empty() && blades.len() < 16,
            "blades too small to tell apart thin: {} at {pixels_per_meter}",
            blades.len()
        );
        assert!(
            blades
                .iter()
                .all(|across| across * pixels_per_meter <= GRAIN_PIXELS + 1e-3),
            "no blade grows past the grain: {blades:?} at {pixels_per_meter}"
        );
    }
}
