use bevy::prelude::*;
use bevy::render::render_resource::{BufferDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue, initialize_renderer};
use bevy::render::settings::{Backends, RenderResources, WgpuSettings};
use engine::core::ground::{GroundSize, Heightfield, PaintMap, SAMPLES_PER_METER};
use engine::core::math::Pos;
use engine::core::render::world::gpu::grass::{
    Blade, BladeShape, GrassCell, GrassGround, GrassKernel, GrassPiece,
};
use engine::core::units::{GROUND_ORIGIN, Meters};
use testkit::HEIGHTS;

/// Wider than a cell and not a whole number of them, so a cell runs past the ground's edge.
const SIZE: GroundSize = GroundSize {
    width: 48,
    height: 40,
};
const GRASS: u8 = 3;
const PER_SQUARE_METER: f32 = 16.0;
const SIZES: (f32, f32) = (0.7, 1.3);

/// A cover's blades grow on the GPU from the ground they stand on alone: only where their terrain
/// paints it, about as thickly as the cover asks, each rooted on the ground and lit as it slopes
/// there, at its own size, tint and seed. The same ground grows the same blades again and another
/// seed scatters them anew, and the main world reserves room for exactly the blades the kernel
/// lays out, a cell running off the ground's edge too.
#[test]
fn a_covers_blades_grow_on_the_gpu_where_its_terrain_paints_rooted_on_the_ground() {
    let seed = testkit::drawn_seed();
    let RenderResources(device, queue, ..) = bevy::tasks::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    let heights = Heightfield::from_fn(SIZE, HEIGHTS, |column, row| {
        let (x, y) = (
            column as f32 / SAMPLES_PER_METER as f32,
            row as f32 / SAMPLES_PER_METER as f32,
        );
        Meters(x / 4.0 + (y / 3.0).sin())
    });
    let painted = |column: u32, row: u32| column < 70 && !(row / 8 + column / 8).is_multiple_of(3);
    let paint = PaintMap::from_fn(
        SIZE,
        |column, row| {
            if painted(column, row) { GRASS } else { 0 }
        },
    );
    let kernel = GrassKernel::new(&device);
    let cells: Vec<GrassCell> = (0..2)
        .flat_map(|row| (0..2).map(move |column| GrassCell { column, row }))
        .collect();
    let grounds: Vec<GrassGround> = cells
        .iter()
        .map(|&cell| GrassGround::of(cell, &heights, &paint))
        .collect();

    let grown = grow(&device, &queue, &kernel, &grounds, &piece(seed));
    let (columns, rows) = SIZE.paint_pixels();
    let painted_pixels = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .filter(|&(column, row)| painted(column, row))
        .count() as f32;
    let pixel = 1.0 / SAMPLES_PER_METER as f32;
    let wanted = painted_pixels * pixel * pixel * PER_SQUARE_METER;
    assert!(
        (grown.blades.len() as f32 - wanted).abs() <= wanted * 0.05,
        "the painted ground grew {} blades, not about {wanted}",
        grown.blades.len()
    );
    assert!(
        grown
            .unfilled
            .iter()
            .all(|blade| *blade == Blade::default()),
        "the kernel lays out exactly the blades the main world made room for"
    );
    for blade in &grown.blades {
        let (column, row) = (
            ((blade.root.x - GROUND_ORIGIN.x) / pixel).floor() as u32,
            ((blade.root.z - GROUND_ORIGIN.y) / pixel).floor() as u32,
        );
        assert!(
            column < columns && row < rows && painted(column, row),
            "a blade at {:?} grows where its terrain doesn't paint",
            blade.root
        );
        let at = Pos::new(blade.root.x, blade.root.z);
        let ground = heights.height_at(at).0;
        assert!(
            (blade.root.y - ground).abs() < 1e-3,
            "a blade's root at {:?} stands off the ground at {ground}",
            blade.root
        );
        let slope = ground_normal(&heights, at);
        assert!(
            blade.normal.distance(slope) < 1e-3,
            "a blade is lit by {:?}, not by the ground's slope {slope:?}",
            blade.normal
        );
        assert!(
            (SIZES.0..=SIZES.1).contains(&blade.size)
                && (0.0..1.0).contains(&blade.tint)
                && (0.0..1.0).contains(&blade.seed)
                && blade.weight == 1.0,
            "a blade's own draws stand within their ranges: {blade:?}"
        );
    }
    let distinct = |of: fn(&Blade) -> f32| {
        grown
            .blades
            .iter()
            .map(|blade| (of(blade) * 1000.0) as u32)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    assert!(
        distinct(|blade| blade.seed) > 900
            && distinct(|blade| blade.tint) > 900
            && distinct(|blade| blade.size) > 500
            && distinct(|blade| blade.yaw) > 900,
        "blades differ from one another in seed, tint, size and turn"
    );

    let again = grow(&device, &queue, &kernel, &grounds, &piece(seed));
    assert_eq!(again.blades, grown.blades, "the same ground grows the same");
    let scattered = grow(
        &device,
        &queue,
        &kernel,
        &grounds,
        &piece(seed.wrapping_add(1)),
    );
    assert_ne!(
        scattered
            .blades
            .iter()
            .map(|blade| blade.root)
            .collect::<Vec<_>>(),
        grown
            .blades
            .iter()
            .map(|blade| blade.root)
            .collect::<Vec<_>>(),
        "another seed scatters anew"
    );
}

fn piece(seed: u64) -> GrassPiece {
    GrassPiece {
        terrain: GRASS,
        piece: 2,
        seed,
        per_square_meter: PER_SQUARE_METER,
        size: SIZES,
        color: (
            LinearRgba::rgb(0.1, 0.4, 0.1),
            LinearRgba::rgb(0.4, 0.8, 0.3),
        ),
        shape: BladeShape {
            width: 0.06,
            height: 0.4,
        },
        material: Handle::default(),
    }
}

/// What the kernel laid out of `piece` over `grounds`, each cell's blades after the last's: the
/// blades the main world made room for, and what lies past them.
struct Grown {
    blades: Vec<Blade>,
    unfilled: Vec<Blade>,
}

/// Room past the blades made room for, which the kernel must leave as it is.
const PAST: u64 = 64;

fn grow(
    device: &RenderDevice,
    queue: &RenderQueue,
    kernel: &GrassKernel,
    grounds: &[GrassGround],
    piece: &GrassPiece,
) -> Grown {
    let mut first = 0;
    let mut laid = Vec::new();
    for ground in grounds {
        let (rows, blades) = ground.rows(piece);
        laid.push((ground, rows, first));
        first += blades;
    }
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("grown blades"),
        size: (u64::from(first) + PAST) * Blade::BYTES,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let cells: Vec<(&GrassGround, &[u32], u32)> = laid
        .iter()
        .map(|(ground, rows, first)| (*ground, rows.as_slice(), *first))
        .collect();
    kernel.grow(device, queue, piece, &cells, &buffer);
    let mut blades = GrassKernel::read(device, queue, &buffer);
    let unfilled = blades.split_off(first as usize);
    Grown { blades, unfilled }
}

/// The ground's normal at `at`, from its heights half a sample either way.
fn ground_normal(heights: &Heightfield, at: Pos<Meters>) -> Vec3 {
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let east = heights.height_at(Pos::new(at.x + step, at.y)).0
        - heights.height_at(Pos::new(at.x - step, at.y)).0;
    let south = heights.height_at(Pos::new(at.x, at.y + step)).0
        - heights.height_at(Pos::new(at.x, at.y - step)).0;
    Vec3::new(-east, 2.0 * step, -south).normalize()
}
