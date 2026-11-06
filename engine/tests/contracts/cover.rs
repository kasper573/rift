use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER, shore_distances};
use engine::core::math::Pos;
use engine::core::render::world::cover::{
    CopyModel, Mark, Patch, Piece, Place, SHORE_REACH, Spread, copy_meshes, merged_copies, seen_at,
    strew_places,
};
use engine::core::render::world::lod::GRAIN_PIXELS;
use engine::core::units::Meters;
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

fn marks(mesh: &Mesh) -> Vec<Mark> {
    match mesh.attribute(Mesh::ATTRIBUTE_UV_1).expect("marks") {
        VertexAttributeValues::Float32x2(marks) => marks.iter().map(|uv| Mark::of(*uv)).collect(),
        _ => panic!("marks are pairs"),
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

    let packed = 1.5 * GRAIN_PIXELS / extent;
    let apart = packed / spread.per_square_meter.sqrt();
    assert!(
        spread.told_apart(extent, packed * 2.0) && !spread.told_apart(extent, packed),
        "copies wide enough to tell apart but packed closer than the grain aren't told apart"
    );
    let thinned = strew_places(&patch(&heights, &full), &spread.grained(extent, packed), 3);
    let covered = |places: &[Place]| -> f32 {
        places
            .iter()
            .map(|place| place.transform.scale.x.powi(2))
            .sum()
    };
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
