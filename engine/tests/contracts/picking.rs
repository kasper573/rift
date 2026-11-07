use std::collections::HashMap;

use bevy::camera::primitives::Frustum;
use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages, CommandEncoder, CompareFunction,
    DepthStencilState, Extent3d, IndexFormat, LoadOp, MapMode, MultisampleState, Operations,
    Origin3d, PipelineCompilationOptions, PollType, PrimitiveState, RawRenderPipelineDescriptor,
    RawVertexBufferLayout, RawVertexState, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    ShaderModuleDescriptor, ShaderSource, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView, TextureViewDescriptor, VertexAttribute,
    VertexFormat, VertexStepMode,
};
use bevy::render::renderer::{RenderDevice, RenderQueue, initialize_renderer};
use bevy::render::settings::{Backends, RenderResources, WgpuSettings};
use bevy::shape::ViewFrustum;
use engine::core::render::world::gpu::arena::{
    ChunkPlace, GpuChunk, InstanceArena, InstanceChange, InstanceRecord, ModelSlabs, PreparedModel,
};
use engine::core::render::world::gpu::cull::{
    CullKernel, CullView, DrawTable, DrawnProp, GpuDrawTable, ViewCull, seen_chunks,
};
use engine::core::render::world::gpu::pick::{
    self, PICK_DEPTH, PICK_IDS, PickDraw, PickPass, PickTargets,
};
use engine::core::render::world::gpu::slab::Slab;
use engine::core::render::world::model::Piece;

const SIDE: u32 = 256;
const FOV: f32 = std::f32::consts::FRAC_PI_3;
const SLOT: u8 = 3;

/// Each pixel of a view's picture names the instance of a scenery's arena it shows: the instance
/// drawn nearest there, by its place among the arena's under the scenery's slot, wherever it is
/// the surface the view's own depth shows. Nothing is named where nothing of the arena stands,
/// where something the arena doesn't hold stands nearer, nor for an instance the view's cull
/// leaves out, and an id names back the slot and instance it was written for.
#[test]
fn a_views_picture_names_the_instance_it_shows_at_each_pixel() {
    let seed = testkit::drawn_seed();
    let RenderResources(device, queue, ..) = bevy::tasks::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    let block = PreparedModel::of(&[Piece {
        mesh: Cuboid::new(1.0, 1.0, 1.0).into(),
        transform: Transform::default(),
        material: None,
    }])
    .expect("a block with vertices");
    let mut models = ModelSlabs::default();
    let writes = models.insert(0u32, &block);
    let (vertex_bytes, index_bytes) = models.capacity();
    let mut vertices = Slab::new("vertices", BufferUsages::VERTEX);
    let mut indices = Slab::new("indices", BufferUsages::INDEX);
    vertices.fit(&device, &queue, vertex_bytes);
    indices.fit(&device, &queue, index_bytes);
    for (offset, bytes) in &writes.vertices {
        vertices.write(&queue, *offset, bytes);
    }
    for (offset, bytes) in &writes.indices {
        indices.write(&queue, *offset, bytes);
    }
    let placed = models.get(&0).expect("the block placed").clone();

    let turned = (seed % 628) as f32 / 100.0;
    let eye = Vec3::new(0.0, 1.5, 8.0);
    let open = record(Vec3::new(-1.5, 0.0, 0.0), turned, 1.0, 0);
    let behind = record(eye + (Vec3::from(open.position) - eye) * 1.5, 0.0, 0.6, 0);
    let covered = record(Vec3::new(1.5, 0.0, 0.0), turned, 1.0, 0);
    let left_out = record(Vec3::ZERO, 0.0, 0.8, 1);
    let records = [open, behind, covered, left_out];
    let cover = (Vec3::new(1.5, 0.0, 3.0), 2.0);

    let mut arena = InstanceArena::default();
    let mut slab = Slab::new("instances", BufferUsages::STORAGE);
    let mut chunks = HashMap::new();
    let mut firsts = HashMap::new();
    for chunk in [0, 1] {
        let held: Vec<InstanceRecord> = records
            .iter()
            .filter(|record| record.chunk == chunk)
            .copied()
            .collect();
        let Some(InstanceChange::Placed {
            offset,
            bytes,
            instances,
            capacity,
            contents,
            ..
        }) = arena.insert(chunk, &held, model_id, ChunkPlace::Near { index: chunk })
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
    let place_of = |record: &InstanceRecord| {
        let first = firsts[&record.chunk];
        let among = records
            .iter()
            .filter(|other| other.chunk == record.chunk)
            .position(|other| other == record)
            .expect("the record among its chunk's");
        first + among as u32
    };

    let table = DrawTable::of(
        chunks
            .values()
            .flat_map(|chunk| chunk.contents.props.iter())
            .map(|&(prop, _, instances)| {
                (
                    prop,
                    DrawnProp {
                        model: model_id(prop),
                        placed: &placed,
                        instances,
                        dressed: true,
                        fades: false,
                    },
                )
            }),
    );
    let kernel = CullKernel::new(&device);
    let gpu_table = GpuDrawTable::new(&device, &queue, table.clone()).expect("a table to draw");
    let (clip_from_world, frustum, view) = camera(eye);
    let mut seen = seen_chunks(&chunks, &table, &frustum, &view);
    seen.spans.retain(|chunk| chunk.x != firsts[&1]);
    let mut cull = ViewCull::new(&device, &gpu_table);
    let instances = slab.buffer().expect("the instances written");
    let mut shown = Slab::new("instances shown", BufferUsages::STORAGE);
    shown.fit(&device, &queue, records.len() as u64 * 4);
    shown.write(&queue, 0, &1.0f32.to_le_bytes().repeat(records.len()));
    cull.update(
        &device,
        &queue,
        &kernel,
        &gpu_table,
        instances,
        shown.buffer().expect("the shares shown written"),
        &no_chunk_standing(&device),
        seen,
        view,
    );

    let view_depth = target(&device, PICK_DEPTH, TextureUsages::TEXTURE_BINDING);
    let depth = target(&device, PICK_DEPTH, TextureUsages::empty());
    let ids = target(&device, PICK_IDS, TextureUsages::COPY_SRC);
    let mut encoder = device.create_command_encoder(&default());
    cull.encode(&kernel, &gpu_table, &mut encoder);
    let scene: Vec<(Vec3, f32, f32)> = records
        .iter()
        .map(|record| (Vec3::from(record.position), record.yaw, record.scale))
        .chain([(cover.0, 0.0, cover.1)])
        .collect();
    draw_depth(
        &device,
        &mut encoder,
        &view_depth.1,
        clip_from_world,
        &block,
        &scene,
    );
    PickPass::new(&device).encode(
        &device,
        &mut encoder,
        &PickTargets {
            ids: &ids.1,
            depth: &depth.1,
            view_depth: &view_depth.1,
        },
        &[PickDraw {
            slot: SLOT,
            clip_from_world,
            instances,
            vertices: vertices.buffer().expect("the vertices written"),
            indices: indices.buffer().expect("the indices written"),
            drawn: cull.drawn(),
            commands_buffer: cull.commands(),
            commands: table.commands.len() as u32,
        }],
    );
    let picture = read_ids(&device, &queue, encoder, &ids.0);

    let pixel_of = |at: Vec3| {
        let clip = clip_from_world * at.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        UVec2::new(
            ((ndc.x * 0.5 + 0.5) * SIDE as f32) as u32,
            ((0.5 - ndc.y * 0.5) * SIDE as f32) as u32,
        )
    };
    let id_at = |at: Vec3| {
        let pixel = pixel_of(at);
        picture[(pixel.y * SIDE + pixel.x) as usize]
    };
    let open_id = pick::encode(SLOT, place_of(&open));
    assert_eq!(id_at(Vec3::from(open.position)), open_id, "seed {seed}");
    assert_eq!(id_at(Vec3::from(covered.position)), 0, "seed {seed}");
    assert_eq!(id_at(Vec3::from(left_out.position)), 0, "seed {seed}");
    let named: Vec<u32> = picture.iter().copied().filter(|&id| id != 0).collect();
    assert!(
        named.len() > 300,
        "the open block fills a good part of the picture: {} pixels, seed {seed}",
        named.len()
    );
    assert!(
        named.iter().all(|&id| id == open_id),
        "only the open block shows, the one behind it and the covered one nowhere: seed {seed}"
    );
    assert!(
        !picture.contains(&pick::encode(SLOT, place_of(&left_out))),
        "seed {seed}"
    );
    for (slot, instance) in [(0, 0), (SLOT, place_of(&behind)), (255, (1 << 24) - 2)] {
        assert_eq!(
            pick::decode(pick::encode(slot, instance)),
            Some((slot, instance))
        );
    }
    assert_eq!(pick::decode(0), None);
}

fn record(at: Vec3, yaw: f32, scale: f32, chunk: u32) -> InstanceRecord {
    InstanceRecord {
        position: at.to_array(),
        yaw,
        scale,
        prop: 0,
        seed: 0,
        chunk,
    }
}

fn model_id(prop: u32) -> AssetId<Gltf> {
    AssetId::Uuid {
        uuid: bevy::asset::uuid::Uuid::from_u128(u128::from(prop) + 1),
    }
}

/// A square picture's camera at `eye` looking at the blocks' middle, and how the kernel culls
/// through it.
fn camera(eye: Vec3) -> (Mat4, Frustum, CullView) {
    let projection = PerspectiveProjection {
        fov: FOV,
        aspect_ratio: 1.0,
        near: 0.1,
        far: 100.0,
        ..default()
    }
    .get_clip_from_view();
    let world_from_view = Transform::from_translation(eye)
        .looking_at(Vec3::ZERO, Vec3::Y)
        .to_matrix();
    let clip_from_world = projection * world_from_view.inverse();
    let frustum = Frustum(ViewFrustum::from_clip_from_world_custom_far(
        &clip_from_world,
        &eye,
        &world_from_view.z_axis.truncate(),
        100.0,
    ));
    let pixels_at_a_meter = SIDE as f32 / 2.0 / (FOV / 2.0).tan();
    let view = CullView::new(&frustum, eye, pixels_at_a_meter, false, false);
    (clip_from_world, frustum, view)
}

fn target(
    device: &RenderDevice,
    format: TextureFormat,
    usage: TextureUsages,
) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("picking target"),
        size: Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: usage | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

/// Draws the depth a view's main pass would leave: each of `scene`'s blocks, placed, turned and
/// sized as an instance is, nearer greater.
fn draw_depth(
    device: &RenderDevice,
    encoder: &mut CommandEncoder,
    depth: &TextureView,
    clip_from_world: Mat4,
    block: &PreparedModel,
    scene: &[(Vec3, f32, f32)],
) {
    let part = &block.parts[0];
    let mut corners: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for &(at, yaw, scale) in scene {
        let first = (corners.len() / 4) as u32;
        for vertex in &part.vertices {
            let world = at + Quat::from_rotation_y(yaw) * (Vec3::from(vertex.position) * scale);
            corners.extend((clip_from_world * world.extend(1.0)).to_array());
        }
        indices.extend(part.levels[0].indices.iter().map(|index| first + index));
    }
    let corners = buffer(device, &corners_bytes(&corners), BufferUsages::VERTEX);
    let indices_buffer = buffer(
        device,
        &indices
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect::<Vec<u8>>(),
        BufferUsages::INDEX,
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("scene depth"),
        source: ShaderSource::Wgsl(
            "@vertex fn vertex(@location(0) clip: vec4<f32>) -> @builtin(position) vec4<f32> { return clip; }"
                .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("scene depth"),
        layout: None,
        vertex: RawVertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[Some(RawVertexBufferLayout {
                array_stride: 16,
                step_mode: VertexStepMode::Vertex,
                attributes: &[VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        primitive: PrimitiveState::default(),
        depth_stencil: Some(DepthStencilState {
            format: PICK_DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::GreaterEqual),
            stencil: default(),
            bias: default(),
        }),
        multisample: MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    });
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("scene depth"),
        color_attachments: &[],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(Operations {
                load: LoadOp::Clear(0.0),
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&pipeline);
    pass.set_vertex_buffer(0, Some((*corners).slice(..)));
    pass.set_index_buffer((*indices_buffer).slice(..), IndexFormat::Uint32);
    pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
}

fn corners_bytes(corners: &[f32]) -> Vec<u8> {
    corners
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn buffer(device: &RenderDevice, bytes: &[u8], usage: BufferUsages) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("picking scene"),
        contents: bytes,
        usage,
    })
}

/// Every id the picture holds, row by row, once the GPU has done what `encoder` asks.
fn read_ids(
    device: &RenderDevice,
    queue: &RenderQueue,
    mut encoder: CommandEncoder,
    ids: &Texture,
) -> Vec<u32> {
    let row_bytes = SIDE * 4;
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("picking readback"),
        size: u64::from(row_bytes * SIDE),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: ids,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &staging,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: None,
            },
        },
        Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(MapMode::Read, move |mapped| {
        let _ = sender.send(mapped);
    });
    device
        .poll(PollType::wait_indefinitely())
        .expect("the GPU finishing the readback");
    receiver
        .recv()
        .expect("the readback's answer")
        .expect("the readback mapping");
    let bytes = slice
        .get_mapped_range()
        .expect("the readback mapped")
        .to_vec();
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect()
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
