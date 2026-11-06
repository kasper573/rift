use bevy::color::LinearRgba;
use engine::core::ground::{GroundSize, Heightfield, PaintMap, SAMPLES_PER_METER};
use engine::core::math::{Pos, Rect, Rng, Size};
use engine::core::render::world::cover::{CHUNK_CELLS, Spread};
use engine::core::units::{MeterSize, Meters};
use engine::systems::area::prop::{PropDef, ScaleRange, Scatter};
use engine::systems::area::scenery::expand::{
    PaintChunk, SceneryChunk, StrewnCover, props_in, scattered_in, strewn_in,
};
use engine::systems::area::{self, AreaDef};
use serde_json::json;
use testkit::{Game, HEIGHTS, Map, path};

const GROUND: GroundSize = GroundSize {
    width: 160,
    height: 160,
};
const SCATTERS: usize = 100;
const PLACED: usize = 30;
const AREA_SCATTERS: usize = 20;

/// A chunk's scenery is worked out from the chunk alone: the props an area stands in a chunk, and
/// those a scatter strews there, come out the same, bit for bit and in order, as the whole area's
/// or the whole scatter's that stand there, every prop in one chunk; and a cover's copies in a chunk
/// come out the same whatever the ground around the chunk holds.
#[test]
fn a_chunks_scenery_is_worked_out_from_the_chunk_alone() {
    let seed = testkit::drawn_seed();
    let mut rng = Rng::new(seed);
    let ground = GROUND.bounds();
    let mut scene = Vec::new();
    for _ in 0..PLACED {
        let at = region(&mut rng, ground).center();
        scene.push(
            json!({ "type": "prop", "prop": path("prop", "rock"), "at": { "x": at.x, "y": at.y } }),
        );
    }
    for _ in 0..AREA_SCATTERS {
        let region = region(&mut rng, ground);
        scene.push(json!({
            "type": "scatter",
            "props": [path("prop", "pine"), path("prop", "rock")],
            "region": { "x": region.origin.x, "y": region.origin.y, "w": region.size.width, "h": region.size.height },
            "per_square_meter": ((0.1 + rng.rand_float() * 0.9) * 60.0 / region.area()).min(2.0),
            "seed": seed_of(&mut rng),
        }));
    }
    let content = Game::new()
        .prop("pine", json!({ "type": "none" }))
        .prop("rock", json!({ "type": "none" }))
        .area(
            "field",
            &Map::open(GROUND.width as usize, GROUND.height as usize),
            json!(scene),
        )
        .hero("field", 1.0, 1.0)
        .load();
    let id = content
        .by_path::<AreaDef>(&path("area", "field"))
        .expect("the field loads");
    let field = area::load(&content, id);
    let field_ground = field.size.bounds();
    let modules = || {
        area::placed(&content, id)
            .iter()
            .map(|placed| &placed.module)
    };
    let mut found = 0;
    for chunk in SceneryChunk::all(field_ground) {
        let held: Vec<_> = field
            .props
            .iter()
            .filter(|placed| chunk.holds(field_ground, placed.at))
            .cloned()
            .collect();
        let worked_out = props_in(modules(), field_ground, chunk);
        assert_eq!(
            worked_out, held,
            "the field stands the same props in chunk {chunk:?} (test seed {seed})"
        );
        found += worked_out.len();
    }
    assert_eq!(
        found,
        field.props.len(),
        "every prop of the field lies in one chunk (test seed {seed})"
    );

    let prop = |name| {
        content
            .by_path::<PropDef>(&path("prop", name))
            .expect("the prop loads")
    };
    let props = vec![prop("pine"), prop("rock")];
    for _ in 0..SCATTERS {
        let scatter = Scatter {
            props: props.clone(),
            region: region(&mut rng, ground),
            per_square_meter: rng.rand_float() * 0.2,
            seed: seed_of(&mut rng),
            scale: ScaleRange { min: 0.5, max: 2.0 },
        };
        let whole: Vec<_> = scatter.expand().collect();
        let mut found = 0;
        for chunk in SceneryChunk::all(ground) {
            let held: Vec<_> = whole
                .iter()
                .filter(|placed| chunk.holds(ground, placed.at))
                .cloned()
                .collect();
            let worked_out = scattered_in(&scatter, ground, chunk);
            assert_eq!(
                worked_out, held,
                "the scatter seeded {} strews the same in chunk {chunk:?} (test seed {seed})",
                scatter.seed
            );
            found += worked_out.len();
        }
        assert_eq!(
            found,
            whole.len(),
            "every prop of the scatter seeded {} lies in one chunk (test seed {seed})",
            scatter.seed
        );
    }
    let heights = Heightfield::from_fn(GROUND, HEIGHTS, |column, row| {
        Meters((column + row) as f32 / SAMPLES_PER_METER as f32 / 8.0)
    });
    let chunk = PaintChunk { column: 3, row: 2 };
    let (first, last) = (
        chunk.first_cell(),
        (
            chunk.first_cell().0 + CHUNK_CELLS,
            chunk.first_cell().1 + CHUNK_CELLS,
        ),
    );
    let inside = |column: u32, row: u32| {
        (first.0..last.0).contains(&column) && (first.1..last.1).contains(&row)
    };
    let checkered = |column: u32, row: u32| u8::from((column / 3 + row / 5).is_multiple_of(2));
    let paint = PaintMap::from_fn(GROUND, checkered);
    let elsewhere = PaintMap::from_fn(GROUND, |column, row| {
        if inside(column, row) {
            checkered(column, row)
        } else {
            1 - checkered(column, row)
        }
    });
    let spread = Spread {
        piece: 1,
        per_square_meter: 12.0,
        size: (0.5, 1.5),
        color: (LinearRgba::BLACK, LinearRgba::WHITE),
    };
    let cover = StrewnCover { slot: 1, seed: 9 };
    let strewn = strewn_in(&heights, &paint, cover, &spread, chunk);
    assert!(!strewn.is_empty(), "the chunk's painted cells strew copies");
    assert_eq!(
        strewn,
        strewn_in(&heights, &elsewhere, cover, &spread, chunk),
        "paint outside the chunk changes nothing in it"
    );
    let bounds = chunk.bounds();
    assert!(
        strewn.iter().all(|place| {
            let at = place.at;
            bounds.contains(Pos::new(at.x, at.z))
        }),
        "every copy stands in its chunk"
    );
    assert_ne!(
        strewn,
        strewn_in(
            &heights,
            &paint,
            StrewnCover { seed: 10, ..cover },
            &spread,
            chunk
        ),
        "another cover seed strews anew"
    );
}

/// A region of `ground` on its half-metre grid, so many of its edges lie on chunks' edges.
fn region(rng: &mut Rng, ground: Rect<Meters>) -> Rect<Meters> {
    let half_metres = |rng: &mut Rng, upto: u32| rng.rand_range(0..upto * 2) as f32 / 2.0;
    let origin = Pos::new(
        ground.origin.x + half_metres(rng, GROUND.width - 1),
        ground.origin.y + half_metres(rng, GROUND.height - 1),
    );
    let room = ground.max() - origin;
    Rect::new(
        origin,
        Size::new(
            (half_metres(rng, 120) + 0.5).min(room.x),
            (half_metres(rng, 120) + 0.5).min(room.y),
        ),
    )
}

fn seed_of(rng: &mut Rng) -> u64 {
    (u64::from(rng.rand_range(0..u32::MAX)) << 32) | u64::from(rng.rand_range(0..u32::MAX))
}
