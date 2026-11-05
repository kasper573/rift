//! What each frame costs where it is spent. The render world reports its own time and counts for
//! every frame it draws, by frame number; the GPU's time comes back through bevy's render
//! diagnostics a few frames later, each batch stamped with the number of the frame it measured, so
//! a frame's picture is whole once both have landed.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transparent3d};
use bevy::diagnostic::{DiagnosticsStore, FrameCount};
use bevy::mesh::Indices;
use bevy::pbr::Shadow;
use bevy::prelude::*;
use bevy::render::diagnostic::{DiagnosticsRecorder, RecordDiagnostics, RenderDiagnosticsPlugin};
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::{ExtractedAssets, prepare_assets};
use bevy::render::render_phase::{BinnedPhaseItem, ViewBinnedRenderPhases, ViewSortedRenderPhases};
use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferUsages};
use bevy::render::renderer::{
    PendingCommandBuffers, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::core::units::{Bytes, Millis};

/// Measures every frame the renderer draws, for whoever reads [`RenderReports`] and
/// [`gpu_reports`] on the main world.
pub struct FrameSamplingPlugin;

impl Plugin for FrameSamplingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RenderDiagnosticsPlugin)
            .init_resource::<RenderReports>()
            .init_resource::<GpuReportsRead>();
    }

    // The recorder the frame stamps go through exists only once the renderer does.
    fn finish(&self, app: &mut App) {
        let reports = app.world().resource::<RenderReports>().clone();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(reports)
            .init_resource::<RenderFrame>()
            .init_resource::<UploadedBytes>()
            .add_systems(RenderStartup, make_stamp)
            .add_systems(
                Render,
                (
                    begin_render.before(RenderSystems::ExtractCommands),
                    count_uploads
                        .in_set(RenderSystems::PrepareAssets)
                        .before(prepare_assets::<RenderMesh>)
                        .before(prepare_assets::<GpuImage>),
                    count_draws
                        .after(RenderSystems::PrepareResourcesFlush)
                        .before(RenderSystems::Render),
                    end_render.in_set(RenderSystems::PostCleanup),
                ),
            )
            .add_systems(RenderGraph, stamp_frame.in_set(RenderGraphSystems::Begin));
    }
}

/// Bytes the render world wrote to the GPU this frame on its own, beside the assets it prepared:
/// each writer adds what it wrote, and the frame's report takes it in.
#[derive(Resource, Default)]
pub struct UploadedBytes(pub u64);

/// What the render world measured of a frame, once it has drawn it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderReport {
    pub frame: u32,
    pub cpu_render_ms: Millis,
    pub draws: u32,
    pub instances: u32,
    pub upload_bytes: Bytes,
}

/// The render world's reports, in the order its frames finished, for the main world to take.
#[derive(Resource, Clone, Default)]
pub struct RenderReports(Arc<Mutex<Vec<RenderReport>>>);

impl RenderReports {
    pub fn take(&self) -> Vec<RenderReport> {
        std::mem::take(&mut *self.0.lock().expect("the render reports"))
    }
}

/// What the GPU measured of a frame: its passes' times, and their sum.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuReport {
    pub frame: u32,
    pub gpu_ms: Millis,
    pub passes: Vec<(String, Millis)>,
}

/// The GPU reports that have landed in the diagnostics since last asked.
pub fn gpu_reports(store: &DiagnosticsStore, read: &mut GpuReportsRead) -> Vec<GpuReport> {
    let Some(stamps) = store.get(&FRAME_STAMP_PATH) else {
        return Vec::new();
    };
    let batches: Vec<(Instant, u32)> = stamps
        .measurements()
        .filter(|stamp| read.0.is_none_or(|read| stamp.time > read))
        .map(|stamp| (stamp.time, stamp.value as u32))
        .collect();
    let Some((latest, _)) = batches.last() else {
        return Vec::new();
    };
    read.0 = Some(*latest);
    batches
        .into_iter()
        .map(|(time, frame)| {
            let passes: Vec<(String, Millis)> = store
                .iter()
                .filter(|diagnostic| {
                    let path = diagnostic.path().as_str();
                    path.starts_with("render/")
                        && path.ends_with("/elapsed_gpu")
                        && path.matches('/').count() == 2
                })
                .filter_map(|diagnostic| {
                    let pass = diagnostic.path().as_str();
                    let name = &pass["render/".len()..pass.len() - "/elapsed_gpu".len()];
                    let measured = diagnostic
                        .measurements()
                        .find(|measurement| measurement.time == time)?;
                    Some((name.to_owned(), Millis(measured.value as f32)))
                })
                .collect();
            GpuReport {
                frame,
                gpu_ms: Millis(passes.iter().map(|(_, ms)| ms.0).sum()),
                passes,
            }
        })
        .collect()
}

/// When the GPU reports were last read, so each batch is taken once.
#[derive(Resource, Default)]
pub struct GpuReportsRead(Option<Instant>);

const FRAME_STAMP: &str = "frame_number";
const FRAME_STAMP_PATH: bevy::diagnostic::DiagnosticPath =
    bevy::diagnostic::DiagnosticPath::const_new("render/frame_number");

#[derive(Resource, Default)]
struct RenderFrame {
    started: Option<Instant>,
    uploads: u64,
    draws: u32,
    instances: u32,
}

/// The number of the frame being drawn, for the diagnostics to read back with its GPU times.
#[derive(Resource)]
struct FrameStamp(Buffer);

fn make_stamp(mut commands: Commands, device: Res<RenderDevice>) {
    commands.insert_resource(FrameStamp(device.create_buffer(&BufferDescriptor {
        label: Some("frame stamp"),
        size: size_of::<u32>() as u64,
        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })));
}

fn begin_render(mut frame: ResMut<RenderFrame>) {
    *frame = RenderFrame {
        started: Some(Instant::now()),
        ..default()
    };
}

fn count_uploads(
    mut frame: ResMut<RenderFrame>,
    meshes: Option<Res<ExtractedAssets<RenderMesh>>>,
    images: Option<Res<ExtractedAssets<GpuImage>>>,
) {
    let meshes: u64 = meshes
        .iter()
        .flat_map(|meshes| meshes.extracted.iter())
        .map(|(_, mesh)| {
            let indices = match mesh.indices() {
                Some(Indices::U16(indices)) => indices.len() * size_of::<u16>(),
                Some(Indices::U32(indices)) => indices.len() * size_of::<u32>(),
                None => 0,
            };
            (mesh.get_vertex_buffer_size() + indices) as u64
        })
        .sum();
    let images: u64 = images
        .iter()
        .flat_map(|images| images.extracted.iter())
        .map(|(_, image)| image.data.as_ref().map_or(0, |data| data.len() as u64))
        .sum();
    frame.uploads += meshes + images;
}

fn count_draws(
    mut frame: ResMut<RenderFrame>,
    opaque: Option<Res<ViewBinnedRenderPhases<Opaque3d>>>,
    alpha_mask: Option<Res<ViewBinnedRenderPhases<AlphaMask3d>>>,
    shadow: Option<Res<ViewBinnedRenderPhases<Shadow>>>,
    transparent: Option<Res<ViewSortedRenderPhases<Transparent3d>>>,
) {
    let mut counted = (0, 0);
    let mut add = |(draws, instances): (u32, u32)| {
        counted.0 += draws;
        counted.1 += instances;
    };
    if let Some(opaque) = opaque {
        add(binned(&opaque));
    }
    if let Some(alpha_mask) = alpha_mask {
        add(binned(&alpha_mask));
    }
    if let Some(shadow) = shadow {
        add(binned(&shadow));
    }
    if let Some(transparent) = transparent {
        let items: usize = transparent.0.values().map(|phase| phase.items.len()).sum();
        add((items as u32, items as u32));
    }
    frame.draws = counted.0;
    frame.instances = counted.1;
}

/// The draws and instances of every view's binned phase: a draw per bin of batchable meshes, and
/// one per mesh that batches with nothing.
fn binned<BPI: BinnedPhaseItem>(phases: &ViewBinnedRenderPhases<BPI>) -> (u32, u32) {
    let mut draws = 0;
    let mut instances = 0;
    for phase in phases.0.values() {
        draws += phase.batchable_meshes.len();
        instances += phase
            .batchable_meshes
            .values()
            .map(|bin| bin.entities().len())
            .sum::<usize>();
        let single: usize = phase
            .unbatchable_meshes
            .values()
            .map(|bin| bin.entities.len())
            .chain(phase.non_mesh_items.values().map(|bin| bin.entities.len()))
            .sum();
        draws += single;
        instances += single;
    }
    (draws as u32, instances as u32)
}

fn stamp_frame(
    recorder: Option<Res<DiagnosticsRecorder>>,
    stamp: Option<Res<FrameStamp>>,
    frame_count: Option<Res<FrameCount>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let (Some(recorder), Some(stamp), Some(frame_count)) = (recorder, stamp, frame_count) else {
        return;
    };
    queue.write_buffer(&stamp.0, 0, &frame_count.0.to_le_bytes());
    let mut encoder = device.create_command_encoder(&default());
    recorder.record_u32(&mut encoder, &stamp.0.slice(..), FRAME_STAMP);
    pending.push_encoder(encoder, "frame stamp");
}

fn end_render(
    mut frame: ResMut<RenderFrame>,
    mut written: ResMut<UploadedBytes>,
    frame_count: Option<Res<FrameCount>>,
    reports: Res<RenderReports>,
) {
    frame.uploads += std::mem::take(&mut written.0);
    let (Some(started), Some(frame_count)) = (frame.started.take(), frame_count) else {
        return;
    };
    reports
        .0
        .lock()
        .expect("the render reports")
        .push(RenderReport {
            frame: frame_count.0,
            cpu_render_ms: Millis(started.elapsed().as_secs_f32() * 1000.0),
            draws: frame.draws,
            instances: frame.instances,
            upload_bytes: Bytes(frame.uploads),
        });
}
