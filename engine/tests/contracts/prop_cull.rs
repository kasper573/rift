use std::collections::HashMap;

use bevy::camera::primitives::{Aabb, Frustum, Sphere};
use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::render_resource::BufferUsages;
use bevy::render::renderer::initialize_renderer;
use bevy::render::settings::{Backends, RenderResources, WgpuSettings};
use bevy::shape::ViewFrustum;
use engine::core::math::Rng;
use engine::core::render::world::gpu::arena::{
    GpuChunk, InstanceArena, InstanceChange, InstanceRecord, LevelRange, PartRanges, PlacedModel,
};
use engine::core::render::world::gpu::cull::{
    CullKernel, CullView, DrawTable, GpuDrawTable, ViewCull, seen_chunks,
};
use engine::core::render::world::gpu::slab::Slab;

const PROPS: u32 = 8;
/// The props past the last with a model, whose instances the arena holds but nothing draws.
const MODELLED: u32 = 6;
const CHUNKS: u32 = 48;
const FIELD: f32 = 1600.0;
const PIXELS_AT_A_METER: f32 = 935.0;
const GRAIN_PIXELS: f32 = 2.0;
const STRAY_PIXELS: f32 = 0.5;
const NEAR: f32 = 0.5;

/// The GPU draws a scenery's props as a reference culler on the CPU would: every instance whose
/// model's sphere reaches into the view's frustum, those smaller than the grain thinned by their
/// seeds and grown to make up for the rest, each part at the coarsest level straying by no more
/// than half a pixel, listed in that level's command once for each part, and nothing of a prop
/// whose model isn't on the GPU.
#[test]
fn the_gpu_draws_each_prop_the_view_sees_at_the_level_its_size_asks_for() {
    let seed = testkit::drawn_seed();
    let mut rng = Rng::new(seed);
    let RenderResources(device, queue, ..) = bevy::tasks::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    let models: Vec<PlacedModel> = (0..MODELLED).map(|_| model(&mut rng)).collect();
    let mut arena = InstanceArena::default();
    let mut slab = Slab::new("instances", BufferUsages::STORAGE);
    let mut chunks = HashMap::new();
    let mut records = Vec::new();
    for chunk in 0..CHUNKS {
        let placed: Vec<InstanceRecord> = (0..rng.rand_range(0..300))
            .map(|_| instance(&mut rng, chunk))
            .collect();
        let change = arena.insert(chunk, &placed, model_id);
        if let Some(InstanceChange::Placed {
            offset,
            bytes,
            instances,
            capacity,
            contents,
            ..
        }) = change
        {
            slab.fit(&device, &queue, capacity);
            slab.write(&queue, offset, &bytes);
            let first = (offset / InstanceRecord::BYTES) as usize;
            records.resize(records.len().max(first + placed.len()), None);
            for (at, record) in placed.iter().enumerate() {
                records[first + at] = Some(*record);
            }
            chunks.insert(
                chunk,
                GpuChunk {
                    offset,
                    instances,
                    contents,
                },
            );
        }
    }
    let table = DrawTable::of(chunks.values().flat_map(|chunk| {
        chunk
            .contents
            .props
            .iter()
            .filter(|(prop, _, _)| *prop < MODELLED)
            .map(|&(prop, _, instances)| (prop, &models[prop as usize], instances))
    }));
    let kernel = CullKernel::new(&device);
    let gpu_table = GpuDrawTable::new(&device, &queue, table.clone()).expect("a table to draw");
    let (frustum, view) = camera(&mut rng);
    let seen = seen_chunks(&chunks, &table, &frustum, &view);
    let mut culled = ViewCull::new(&device, &gpu_table);
    let instances = slab.buffer().expect("the instances written");
    culled.update(&device, &queue, &kernel, &gpu_table, instances, seen, view);
    let mut encoder = device.create_command_encoder(&default());
    culled.encode(&kernel, &gpu_table, &mut encoder);
    queue.submit([encoder.finish()]);
    let read = culled.read(&device, &queue);

    let mut drawn: HashMap<u32, Vec<(u32, f32)>> = HashMap::new();
    for (command, (&count, start)) in read.instances.iter().zip(&table.commands).enumerate() {
        let region = start.first_drawn as usize..(start.first_drawn + count) as usize;
        for &(record, scale) in &read.listed[region] {
            drawn
                .entry(record)
                .or_default()
                .push((command as u32, scale));
        }
    }
    let mut expected_drawn = 0;
    let mut judged = 0;
    for (index, record) in records.iter().enumerate() {
        let Some(record) = record else {
            continue;
        };
        let index = index as u32;
        let mut got = drawn.remove(&index).unwrap_or_default();
        got.sort_by_key(|(command, _)| *command);
        let Some(expected) = reference(record, &models, &table, &frustum) else {
            continue;
        };
        judged += 1;
        expected_drawn += u32::from(!expected.is_empty());
        let commands = |listed: &[(u32, f32)]| -> Vec<u32> {
            listed.iter().map(|(command, _)| *command).collect()
        };
        assert_eq!(
            commands(&got),
            commands(&expected),
            "instance {index} ({record:?}) is drawn by the commands the reference draws it by \
             (test seed {seed})"
        );
        for ((_, got), (_, expected)) in got.iter().zip(&expected) {
            assert!(
                (got - expected).abs() <= expected * 1e-4,
                "instance {index} is drawn at scale {expected}, not {got} (test seed {seed})"
            );
        }
    }
    assert!(
        drawn.is_empty(),
        "nothing but the arena's instances is drawn (test seed {seed})"
    );
    let ambiguous = records.iter().flatten().count() as u32 - judged;
    assert!(
        read.drawn.abs_diff(expected_drawn) <= ambiguous,
        "the kernel counts the {expected_drawn} instances it draws, not {} (test seed {seed})",
        read.drawn
    );
    assert!(
        expected_drawn > 100 && judged as usize > records.iter().flatten().count() * 99 / 100,
        "the scene puts the kernel to the test: {expected_drawn} drawn, {judged} judged \
         (test seed {seed})"
    );
}

/// The commands the reference draws `record` by and the scale it draws it at, none where the
/// instance lies too near a judgement's edge for the GPU's rounding to be sure to judge it alike.
fn reference(
    record: &InstanceRecord,
    models: &[PlacedModel],
    table: &DrawTable,
    frustum: &Frustum,
) -> Option<Vec<(u32, f32)>> {
    let Some(model) = models.get(record.prop as usize) else {
        return Some(Vec::new());
    };
    let turn = Quat::from_rotation_y(record.yaw);
    let center =
        Vec3::from_array(record.position) + turn * (Vec3::from(model.bounds.center) * record.scale);
    let radius = model.bounds.half_extents.length() * record.scale;
    let reach = frustum
        .half_spaces
        .iter()
        .map(|half_space| half_space.normal_d().dot(center.extend(1.0)) + radius)
        .fold(f32::INFINITY, f32::min);
    let near = |a: f32, b: f32| (a - b).abs() <= 1e-3 * (1.0 + a.abs().max(b.abs()));
    if reach.abs() <= 1e-2 {
        return None;
    }
    debug_assert_eq!(
        frustum.intersects_sphere(
            &Sphere {
                center: center.into(),
                radius
            },
            true
        ),
        reach > 0.0
    );
    if reach <= 0.0 {
        return Some(Vec::new());
    }
    let eye = frustum_eye();
    let pixels_per_meter = PIXELS_AT_A_METER / center.distance(eye).max(NEAR);
    let mut scale = record.scale;
    let across = 2.0 * model.bounds.half_extents.max_element() * scale * pixels_per_meter;
    if near(across, GRAIN_PIXELS) {
        return None;
    }
    if across < GRAIN_PIXELS {
        let grown = GRAIN_PIXELS / across;
        let share = (record.seed >> 8) as f32 / 16_777_216.0;
        if near(share, 1.0 / (grown * grown)) {
            return None;
        }
        if share >= 1.0 / (grown * grown) {
            return Some(Vec::new());
        }
        scale *= grown;
    }
    let allowed = STRAY_PIXELS / (pixels_per_meter * scale);
    let prop = table.props[record.prop as usize];
    let mut commands = Vec::new();
    for part in &table.parts[prop.first_part as usize..(prop.first_part + prop.parts) as usize] {
        let levels =
            &table.levels[part.first_level as usize..(part.first_level + part.levels) as usize];
        if levels[1..].iter().any(|level| near(level.strays, allowed)) {
            return None;
        }
        let chosen = levels
            .iter()
            .rev()
            .find(|level| level.strays <= allowed)
            .unwrap_or(&levels[0]);
        commands.push((chosen.command, scale));
    }
    Some(commands)
}

/// A model of one to three parts, each of one to four levels straying more and more.
fn model(rng: &mut Rng) -> PlacedModel {
    let half = Vec3::new(
        0.1 + rng.rand_float() * 5.0,
        0.1 + rng.rand_float() * 8.0,
        0.1 + rng.rand_float() * 5.0,
    );
    let middle = Vec3::new(rng.rand_float() - 0.5, half.y, rng.rand_float() - 0.5);
    let mut first_index = 0;
    PlacedModel {
        bounds: Aabb {
            center: middle.into(),
            half_extents: half.into(),
        },
        parts: (0..1 + rng.rand_range(0..3))
            .map(|part| {
                let mut strays = 0.0;
                PartRanges {
                    material: None,
                    first_vertex: part * 1000,
                    vertices: 1000,
                    levels: (0..1 + rng.rand_range(0..4))
                        .map(|level| {
                            if level > 0 {
                                strays += 0.002 + rng.rand_float() * 0.05 * level as f32;
                            }
                            let indices = 3 * (300 >> level);
                            first_index += indices;
                            LevelRange {
                                first_index: first_index - indices,
                                indices,
                                strays,
                            }
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

fn instance(rng: &mut Rng, chunk: u32) -> InstanceRecord {
    let column = (chunk % 8) as f32;
    let row = (chunk / 8) as f32;
    let side = FIELD / 8.0;
    InstanceRecord {
        position: [
            (column + rng.rand_float()) * side - FIELD / 2.0,
            rng.rand_float() * 40.0 - 20.0,
            (row + rng.rand_float()) * side - FIELD / 2.0,
        ],
        yaw: (rng.rand_float() - 0.5) * std::f32::consts::TAU,
        scale: 0.5 + rng.rand_float() * 2.5,
        prop: rng.rand_range(0..PROPS),
        seed: rng.rand_range(0..u32::MAX),
        chunk,
    }
}

/// A camera at [`frustum_eye`] looking out over the field, and how the kernel culls through it.
fn camera(rng: &mut Rng) -> (Frustum, CullView) {
    let eye = frustum_eye();
    let target = Vec3::new(
        (rng.rand_float() - 0.5) * FIELD / 2.0,
        0.0,
        (rng.rand_float() - 0.5) * FIELD / 2.0,
    );
    let fov = 2.0 * (1.0 / (PIXELS_AT_A_METER / 540.0)).atan();
    let projection = PerspectiveProjection {
        fov,
        aspect_ratio: 16.0 / 9.0,
        near: NEAR,
        far: FIELD,
        ..default()
    }
    .get_clip_from_view();
    let world_from_view = Transform::from_translation(eye)
        .looking_at(target, Vec3::Y)
        .to_matrix();
    let backward = world_from_view.z_axis.truncate();
    let frustum = Frustum(ViewFrustum::from_clip_from_world_custom_far(
        &(projection * world_from_view.inverse()),
        &eye,
        &backward,
        FIELD,
    ));
    let view = CullView::new(&frustum, eye, PIXELS_AT_A_METER, false, false);
    (frustum, view)
}

fn frustum_eye() -> Vec3 {
    Vec3::new(-FIELD / 3.0, 30.0, -FIELD / 3.0)
}

fn model_id(prop: u32) -> AssetId<Gltf> {
    AssetId::Uuid {
        uuid: bevy::asset::uuid::Uuid::from_u128(u128::from(prop) + 1),
    }
}
