use std::collections::HashMap;

use bevy::camera::primitives::{Aabb, Frustum, Sphere};
use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::render_resource::{Buffer, BufferInitDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, initialize_renderer};
use bevy::render::settings::{Backends, RenderResources, WgpuSettings};
use bevy::shape::ViewFrustum;
use engine::core::math::Rng;
use engine::core::render::world::gpu::arena::{
    ChunkPlace, GpuChunk, InstanceArena, InstanceChange, InstanceRecord, LevelRange, PartRanges,
    PlacedModel,
};
use engine::core::render::world::gpu::cull::{
    BAND_PIXELS, CullKernel, CullView, DrawTable, DrawnProp, GpuDrawTable, IMPOSTOR_PIXELS,
    ListedInstance, ViewCull, seen_chunks,
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
/// How far a fade moves in the frame culled.
const STEP: f32 = 0.25;

/// The GPU draws a scenery's props as a reference culler on the CPU would: every instance whose
/// model's sphere reaches into the view's frustum, those smaller than the grain thinned by their
/// seeds and grown to make up for the rest, each part at the coarsest level straying by no more
/// than half a pixel, listed in that level's command once for each part, and nothing of a prop
/// whose model isn't on the GPU. An instance of a prop that fades, whose box the way from the eye
/// to the camera's subject passes through or the eye stands in, is drawn a step cleared; every
/// other one whole.
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
        let change = arena.insert(chunk, &placed, model_id, ChunkPlace::Near { index: chunk });
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
            .map(|&(prop, _, instances)| {
                (
                    prop,
                    DrawnProp {
                        model: model_id(prop),
                        placed: &models[prop as usize],
                        instances,
                        dressed: true,
                        fades: fades(prop),
                    },
                )
            })
    }));
    let kernel = CullKernel::new(&device);
    let gpu_table = GpuDrawTable::new(&device, &queue, table.clone()).expect("a table to draw");
    let hider = records
        .iter()
        .flatten()
        .filter(|record| record.prop < MODELLED && fades(record.prop))
        .map(|record| (record, &models[record.prop as usize]))
        .filter(|&(record, model)| hides_level_across_the_field(record, model))
        .map(|(record, model)| middle_of(record, model))
        .min_by(|a, b| {
            a.distance(frustum_eye())
                .total_cmp(&b.distance(frustum_eye()))
        })
        .expect("an instance to hide the camera's subject");
    let subject = frustum_eye() + (hider - frustum_eye()) * 2.0;
    let (frustum, view) = camera(subject);
    let seen = seen_chunks(&chunks, &table, &frustum, &view);
    let mut culled = ViewCull::new(&device, &gpu_table);
    let instances = slab.buffer().expect("the instances written");
    let mut shown = Slab::new("instances shown", BufferUsages::STORAGE);
    shown.fit(&device, &queue, records.len() as u64 * 4);
    shown.write(&queue, 0, &1.0f32.to_le_bytes().repeat(records.len()));
    culled.update(
        &device,
        &queue,
        &kernel,
        &gpu_table,
        instances,
        shown.buffer().expect("the shares shown written"),
        &no_chunk_standing(&device),
        seen,
        view.clearing(subject, STEP),
    );
    let mut encoder = device.create_command_encoder(&default());
    culled.encode(&kernel, &gpu_table, &mut encoder);
    queue.submit([encoder.finish()]);
    let read = culled.read(&device, &queue);

    let mut drawn: HashMap<u32, Vec<(u32, ListedInstance)>> = HashMap::new();
    for (command, region) in read.commands.iter().enumerate() {
        for listed in &read.listed[region.start as usize..region.end as usize] {
            drawn
                .entry(listed.record)
                .or_default()
                .push((command as u32, *listed));
        }
    }
    let mut cleared = 0;
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
        let Some((expected_shown, around_eye)) = faded(record, &models, frustum_eye(), subject)
        else {
            continue;
        };
        judged += 1;
        expected_drawn += u32::from(!expected.is_empty());
        let commands = |listed: &[(u32, ListedInstance)]| -> Vec<u32> {
            listed.iter().map(|(command, _)| *command).collect()
        };
        let commands_expected = |listed: &[(u32, f32)]| -> Vec<u32> {
            listed.iter().map(|(command, _)| *command).collect()
        };
        assert_eq!(
            commands(&got),
            commands_expected(&expected),
            "instance {index} ({record:?}) is drawn by the commands the reference draws it by \
             (test seed {seed})"
        );
        for ((_, got), (_, expected)) in got.iter().zip(&expected) {
            assert!(
                (got.scale - expected).abs() <= expected * 1e-4,
                "instance {index} is drawn at scale {expected}, not {} (test seed {seed})",
                got.scale
            );
            assert_eq!(
                (got.shown, got.around_eye, got.faded_in),
                (expected_shown, around_eye, 1.0),
                "instance {index} ({record:?}) shows as far as the reference says, by its model \
                 whole without an impostor (test seed {seed})"
            );
        }
        cleared += u32::from(!expected.is_empty() && expected_shown < 1.0);
    }
    assert!(
        drawn.is_empty(),
        "nothing but the arena's instances is drawn (test seed {seed})"
    );
    assert_eq!(
        read.missing, 0,
        "every instance is drawn as its size asks, by its model (test seed {seed})"
    );
    let ambiguous = records.iter().flatten().count() as u32 - judged;
    assert!(
        read.drawn.abs_diff(expected_drawn) <= ambiguous,
        "the kernel counts the {expected_drawn} instances it draws, not {} (test seed {seed})",
        read.drawn
    );
    assert!(
        expected_drawn > 100
            && cleared > 0
            && judged as usize > records.iter().flatten().count() * 99 / 100,
        "the scene puts the kernel to the test: {expected_drawn} drawn, {cleared} cleared, \
         {judged} judged (test seed {seed})"
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

/// The middle of the box of `record`'s model, where the instance stands.
fn middle_of(record: &InstanceRecord, model: &PlacedModel) -> Vec3 {
    Vec3::from_array(record.position)
        + Quat::from_rotation_y(record.yaw) * (Vec3::from(model.bounds.center) * record.scale)
}

/// Whether the instances of `prop` clear away where they hide the camera's subject.
fn fades(prop: u32) -> bool {
    prop.is_multiple_of(2)
}

/// How far the reference shows `record` after one frame of clearing from all of it shown, and
/// whether the eye stands inside it: none where the way from the eye to `subject` passes too near
/// the edge of its box for the GPU's rounding to be sure to judge it alike.
fn faded(
    record: &InstanceRecord,
    models: &[PlacedModel],
    eye: Vec3,
    subject: Vec3,
) -> Option<(f32, bool)> {
    let Some(model) = models
        .get(record.prop as usize)
        .filter(|_| fades(record.prop))
    else {
        return Some((1.0, false));
    };
    let local_from_world = Transform::from_translation(Vec3::from_array(record.position))
        .with_rotation(Quat::from_rotation_y(record.yaw))
        .with_scale(Vec3::splat(record.scale))
        .to_matrix()
        .inverse();
    let (from, to) = (
        local_from_world.transform_point3(eye),
        local_from_world.transform_point3(subject),
    );
    let middle = Vec3::from(model.bounds.center);
    let half = Vec3::from(model.bounds.half_extents);
    let judged = |grown: f32| {
        let (low, high) = (middle - half - grown, middle + half + grown);
        let inside = from.cmpge(low).all() && from.cmple(high).all();
        (inside, inside || segment_crosses(from, to, low, high))
    };
    let margin = 1e-3 * (1.0 + half.max_element());
    let (inside, crossed) = judged(0.0);
    if judged(margin) != judged(-margin) {
        return None;
    }
    Some((if crossed { 1.0 - STEP } else { 1.0 }, inside))
}

/// Whether the segment from `a` to `b` passes through the box from `low` to `high`.
fn segment_crosses(a: Vec3, b: Vec3, low: Vec3, high: Vec3) -> bool {
    let way = b - a;
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        if way[axis].abs() < 1e-6 {
            if a[axis] < low[axis] || a[axis] > high[axis] {
                return false;
            }
            continue;
        }
        let (t1, t2) = (
            (low[axis] - a[axis]) / way[axis],
            (high[axis] - a[axis]) / way[axis],
        );
        enter = enter.max(t1.min(t2));
        exit = exit.min(t1.max(t2));
        if enter > exit {
            return false;
        }
    }
    true
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

/// A camera at [`frustum_eye`] looking at `target` over the field, and how the kernel culls
/// through it.
fn camera(target: Vec3) -> (Frustum, CullView) {
    let eye = frustum_eye();
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

/// Whether `record` stands where a camera at [`frustum_eye`] looking through it looks out level
/// over most of the field, taking in many of its instances, and is drawn there whole and never
/// thinned away, so that whatever the seed it is shown and cleared.
fn hides_level_across_the_field(record: &InstanceRecord, model: &PlacedModel) -> bool {
    let eye = frustum_eye();
    let way = middle_of(record, model) - eye;
    let level = way.with_y(0.0);
    let across_the_field = (Vec3::ZERO - eye).with_y(0.0).normalize();
    let pixels_across = 2.0
        * Vec3::from(model.bounds.half_extents).max_element()
        * record.scale
        * PIXELS_AT_A_METER
        / way.length();
    level.length() > 200.0
        && level.normalize().dot(across_the_field) > 0.9
        && pixels_across > 2.0 * GRAIN_PIXELS
}

fn frustum_eye() -> Vec3 {
    Vec3::new(-FIELD / 3.0, 30.0, -FIELD / 3.0)
}

fn model_id(prop: u32) -> AssetId<Gltf> {
    AssetId::Uuid {
        uuid: bevy::asset::uuid::Uuid::from_u128(u128::from(prop) + 1),
    }
}

/// The buffer telling the kernel which near chunks of the ground stand: none, for a scene with no
/// far field.
fn no_chunk_standing(device: &RenderDevice) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("no chunk standing"),
        contents: &[0; 4],
        usage: BufferUsages::STORAGE,
    })
}

/// Across the impostor band the GPU hands a prop from its model to its impostor: an instance
/// wider across than the band is drawn by its model's parts alone, one narrower than the band by
/// its impostor alone, and one within it by both, each faded in by its share of the band. A prop
/// the atlas holds no impostor of is drawn by its model however narrow; one whose model stands
/// undressed is drawn by its impostor, or by its box without one, and counted missing. An
/// instance of the far field is drawn by its impostor alone, and not at all where the near chunk
/// it belongs to stands.
#[test]
fn the_gpu_hands_a_prop_from_its_model_to_its_impostor_across_the_band() {
    let RenderResources(device, queue, ..) = bevy::tasks::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    let model = PlacedModel {
        bounds: Aabb {
            center: Vec3::new(0.0, 1.0, 0.0).into(),
            half_extents: Vec3::ONE.into(),
        },
        parts: vec![PartRanges {
            material: None,
            first_vertex: 0,
            vertices: 100,
            levels: vec![LevelRange {
                first_index: 0,
                indices: 300,
                strays: 0.0,
            }],
        }],
    };
    let (frustum, view) = camera(Vec3::ZERO);
    let eye = frustum_eye();
    let way = (Vec3::ZERO - eye).normalize();
    let across = |pixels: f32| 2.0 * PIXELS_AT_A_METER / pixels;
    let standing_at = |prop: u32, distance: f32, chunk: u32| InstanceRecord {
        position: (eye + way * distance - Vec3::Y).to_array(),
        yaw: 0.0,
        scale: 1.0,
        prop,
        seed: 0,
        chunk,
    };
    let near = [
        standing_at(
            DRESSED_WITH_IMPOSTOR,
            across(IMPOSTOR_PIXELS + BAND_PIXELS),
            NEAR_CHUNK,
        ),
        standing_at(DRESSED_WITH_IMPOSTOR, across(IMPOSTOR_PIXELS), NEAR_CHUNK),
        standing_at(
            DRESSED_WITH_IMPOSTOR,
            across(IMPOSTOR_PIXELS - BAND_PIXELS),
            NEAR_CHUNK,
        ),
        standing_at(
            DRESSED_WITHOUT_IMPOSTOR,
            across(IMPOSTOR_PIXELS - BAND_PIXELS),
            NEAR_CHUNK,
        ),
        standing_at(
            UNDRESSED_WITH_IMPOSTOR,
            across(IMPOSTOR_PIXELS + BAND_PIXELS),
            NEAR_CHUNK,
        ),
        standing_at(
            UNDRESSED_WITHOUT_IMPOSTOR,
            across(IMPOSTOR_PIXELS + BAND_PIXELS),
            NEAR_CHUNK,
        ),
    ];
    let far = [
        standing_at(
            DRESSED_WITH_IMPOSTOR,
            across(IMPOSTOR_PIXELS + BAND_PIXELS),
            NEAR_CHUNK,
        ),
        standing_at(
            DRESSED_WITH_IMPOSTOR,
            across(IMPOSTOR_PIXELS + BAND_PIXELS),
            ABSENT_CHUNK,
        ),
    ];
    let mut arena = InstanceArena::default();
    let mut slab = Slab::new("instances", BufferUsages::STORAGE);
    let mut chunks = HashMap::new();
    let mut firsts = HashMap::new();
    for (chunk, records, place) in [
        (0, &near[..], ChunkPlace::Near { index: NEAR_CHUNK }),
        (1, &far[..], ChunkPlace::Far),
    ] {
        let Some(InstanceChange::Placed {
            offset,
            bytes,
            instances,
            capacity,
            contents,
            ..
        }) = arena.insert(chunk, records, model_id, place)
        else {
            panic!("the chunk {chunk} placed");
        };
        slab.fit(&device, &queue, capacity);
        slab.write(&queue, offset, &bytes);
        firsts.insert(chunk, (offset / InstanceRecord::BYTES) as u32);
        chunks.insert(
            chunk,
            GpuChunk {
                offset,
                instances,
                contents,
            },
        );
    }
    let table = DrawTable::of(chunks.values().flat_map(|chunk| {
        chunk.contents.props.iter().map(|&(prop, _, instances)| {
            (
                prop,
                DrawnProp {
                    model: model_id(prop),
                    placed: &model,
                    instances,
                    dressed: prop < UNDRESSED_WITH_IMPOSTOR,
                    fades: false,
                },
            )
        })
    }));
    let kernel = CullKernel::new(&device);
    let mut gpu_table = GpuDrawTable::new(&device, &queue, table.clone()).expect("a table to draw");
    gpu_table.place_impostors(&device, &queue, 1, |model| {
        [DRESSED_WITH_IMPOSTOR, UNDRESSED_WITH_IMPOSTOR]
            .into_iter()
            .position(|prop| model_id(prop) == model)
            .map(|layer| layer as u32)
    });
    let seen = seen_chunks(&chunks, &table, &frustum, &view);
    let mut culled = ViewCull::new(&device, &gpu_table);
    let mut shown = Slab::new("instances shown", BufferUsages::STORAGE);
    let records = near.len() + far.len();
    shown.fit(&device, &queue, records as u64 * 4);
    shown.write(&queue, 0, &1.0f32.to_le_bytes().repeat(records));
    let standing = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("near chunks standing"),
        contents: &(1u32 << NEAR_CHUNK).to_le_bytes(),
        usage: BufferUsages::STORAGE,
    });
    culled.update(
        &device,
        &queue,
        &kernel,
        &gpu_table,
        slab.buffer().expect("the instances written"),
        shown.buffer().expect("the shares shown written"),
        &standing,
        seen,
        view,
    );
    let mut encoder = device.create_command_encoder(&default());
    culled.encode(&kernel, &gpu_table, &mut encoder);
    queue.submit([encoder.finish()]);
    let read = culled.read(&device, &queue);

    let drawn_by = |record: u32| -> Vec<DrawnBy> {
        let table = &table;
        let mut listed: Vec<DrawnBy> = read
            .commands
            .iter()
            .enumerate()
            .flat_map(|(command, region)| {
                read.listed[region.start as usize..region.end as usize]
                    .iter()
                    .filter(|listed| listed.record == record)
                    .map(move |listed| (Drawn::by(table, command as u32), listed.faded_in))
            })
            .collect();
        listed.sort_by_key(|(drawn, _)| *drawn);
        listed
    };
    let near_first = firsts[&0];
    let far_first = firsts[&1];
    let expected: [(&str, u32, Vec<DrawnBy>); 8] = [
        ("wider than the band", near_first, vec![(Drawn::Model, 1.0)]),
        (
            "within the band",
            near_first + 1,
            vec![(Drawn::Model, 0.5), (Drawn::Impostor, 0.5)],
        ),
        (
            "narrower than the band",
            near_first + 2,
            vec![(Drawn::Impostor, 1.0)],
        ),
        (
            "without an impostor",
            near_first + 3,
            vec![(Drawn::Model, 1.0)],
        ),
        ("undressed", near_first + 4, vec![(Drawn::Impostor, 1.0)]),
        (
            "undressed without an impostor",
            near_first + 5,
            vec![(Drawn::Box, 1.0)],
        ),
        ("far where the near chunk stands", far_first, vec![]),
        ("far", far_first + 1, vec![(Drawn::Impostor, 1.0)]),
    ];
    for (what, record, wanted) in expected {
        let got = drawn_by(record);
        let alike = got.len() == wanted.len()
            && got
                .iter()
                .zip(&wanted)
                .all(|((drawn, faded), (by, share))| drawn == by && (faded - share).abs() < 0.02);
        assert!(
            alike,
            "the instance {what} is drawn as {wanted:?}, not {got:?}"
        );
    }
    assert_eq!(
        read.missing, 2,
        "the undressed instances are counted missing"
    );
    assert_eq!(
        read.drawn, 7,
        "every instance but the far one whose near chunk stands is drawn"
    );
}

/// What a command draws an instance by, and how far the instance has faded into it.
type DrawnBy = (Drawn, f32);

/// What a command draws a prop by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Drawn {
    Model,
    Impostor,
    Box,
}

impl Drawn {
    fn by(table: &DrawTable, command: u32) -> Drawn {
        if table.impostors.contains(&command) {
            Drawn::Impostor
        } else if table.proxies.contains(&command) {
            Drawn::Box
        } else {
            Drawn::Model
        }
    }
}

const DRESSED_WITH_IMPOSTOR: u32 = 0;
const DRESSED_WITHOUT_IMPOSTOR: u32 = 1;
const UNDRESSED_WITH_IMPOSTOR: u32 = 2;
const UNDRESSED_WITHOUT_IMPOSTOR: u32 = 3;
/// The near chunk of the ground the band's instances stand in, which stands in the arena.
const NEAR_CHUNK: u32 = 7;
/// A near chunk of the ground that doesn't stand in the arena.
const ABSENT_CHUNK: u32 = 9;
