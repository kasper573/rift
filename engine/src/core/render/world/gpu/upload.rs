//! How much of the world's meshes the GPU is sent a frame: a mesh made for the world is staged,
//! its handle given out at once, and the staged meshes are handed to bevy a few megabytes a frame,
//! so much arriving at once costs many frames a little rather than one frame a lot; behind a
//! cover, where a long frame shows nothing, far more. What makes meshes asks whether there is room
//! to stage more, so what waits stays a few frames' worth rather than all that is wanted at once.
//! What waits for its meshes can ask whether they are on the GPU yet, so it never stands in for
//! what it replaces too soon.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bevy::mesh::Mesh3d;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssets;
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};

pub struct UploadPlugin;

impl Plugin for UploadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StagedMeshes>()
            .add_systems(Last, send_meshes);
        let arrivals = Arrivals::default();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(arrivals.clone())
                .add_systems(Render, land.in_set(RenderSystems::PrepareResources));
            app.insert_resource(arrivals);
        }
    }
}

/// Stages `mesh` to be sent to the GPU as the frames' budget allows, giving out its handle at once:
/// what draws it shows once it is sent.
pub fn stage(world: &mut World, mesh: Mesh) -> Handle<Mesh> {
    let handle = world.resource::<Assets<Mesh>>().reserve_handle();
    let mut staged = world.resource_mut::<StagedMeshes>();
    staged.bytes += bytes_of(&mesh);
    staged.queue.push_back((handle.clone(), mesh));
    handle
}

/// Whether what is staged goes to the GPU within a few frames, so more meshes may be made: what
/// makes them waits while it doesn't, rather than holding all it makes in memory meanwhile.
pub fn has_room(world: &World) -> bool {
    let covered = world
        .get_resource::<ActiveTransition>()
        .is_some_and(hides_world);
    world
        .get_resource::<StagedMeshes>()
        .is_none_or(|staged| staged.bytes < per_frame(covered) * FRAMES_AHEAD)
}

/// Whether every mesh `entity` and everything under it draws is on the GPU, so it shows whole.
pub fn sent(world: &World, entity: Entity) -> bool {
    let Some(arrivals) = world.get_resource::<Arrivals>() else {
        return true;
    };
    let mut waiting = arrivals.0.lock().expect("the arrivals lock");
    descendants(world, entity)
        .into_iter()
        .filter_map(|part| world.get::<Mesh3d>(part))
        .filter(|mesh| !waiting.arrived(mesh.id()))
        .count()
        == 0
}

/// Whether `mesh` is on the GPU.
pub fn mesh_sent(world: &World, mesh: &Handle<Mesh>) -> bool {
    world.get_resource::<Arrivals>().is_none_or(|arrivals| {
        arrivals
            .0
            .lock()
            .expect("the arrivals lock")
            .arrived(mesh.id())
    })
}

/// The most bytes of staged meshes handed to bevy in a frame: a mesh larger than what is left goes
/// in a frame of its own.
const PER_FRAME: usize = 4 << 20;
/// The same while a cover hides the world, so what stands behind it stands in a few frames.
const PER_COVERED_FRAME: usize = 64 << 20;

/// How many frames' worth of meshes may wait staged.
const FRAMES_AHEAD: usize = 4;

/// The meshes staged for the GPU, oldest first, each with the handle given out for it, and the
/// bytes they take.
#[derive(Resource, Default)]
struct StagedMeshes {
    queue: VecDeque<(Handle<Mesh>, Mesh)>,
    bytes: usize,
}

/// Hands the staged meshes to bevy, oldest first, as many as the frame's budget allows. A mesh
/// whose every holder went before it was sent is dropped unsent.
fn send_meshes(
    mut staged: ResMut<StagedMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    transition: Option<Res<ActiveTransition>>,
) {
    if staged.queue.is_empty() {
        return;
    }
    let budget = per_frame(transition.is_some_and(|transition| hides_world(&transition)));
    let mut left = budget;
    while let Some((_, next)) = staged.queue.front() {
        let bytes = bytes_of(next);
        if bytes > left && left < budget {
            break;
        }
        left = left.saturating_sub(bytes);
        staged.bytes = staged.bytes.saturating_sub(bytes);
        let Some((handle, mesh)) = staged.queue.pop_front() else {
            break;
        };
        if meshes.insert(handle.id(), mesh).is_err() {
            continue;
        }
    }
}

fn per_frame(covered: bool) -> usize {
    if covered {
        PER_COVERED_FRAME
    } else {
        PER_FRAME
    }
}

fn hides_world(transition: &ActiveTransition) -> bool {
    transition.phase() == ScreenTransitionPhase::Hold
}

/// The bytes `mesh` takes on the GPU.
fn bytes_of(mesh: &Mesh) -> usize {
    mesh.get_vertex_buffer_size() + mesh.get_index_buffer_bytes().map_or(0, <[u8]>::len)
}

/// The meshes the main world asked after since the render world last looked that the GPU didn't
/// hold, and those it holds.
#[derive(Resource, Clone, Default)]
struct Arrivals(Arc<Mutex<Waiting>>);

#[derive(Default)]
struct Waiting {
    awaited: HashSet<AssetId<Mesh>>,
    arrived: HashSet<AssetId<Mesh>>,
}

impl Waiting {
    /// Whether `mesh` is on the GPU, as the render world last found; awaited from now on if not.
    fn arrived(&mut self, mesh: AssetId<Mesh>) -> bool {
        if self.arrived.contains(&mesh) {
            return true;
        }
        self.awaited.insert(mesh);
        false
    }
}

/// Takes in every awaited mesh the GPU now holds, and lets go of those it holds no longer. What
/// is still awaited is forgotten, as whoever waits on it asks again, and a mesh dropped before it
/// was sent is never asked after.
fn land(arrivals: Res<Arrivals>, meshes: Res<RenderAssets<RenderMesh>>) {
    let mut waiting = arrivals.0.lock().expect("the arrivals lock");
    waiting.arrived.retain(|&mesh| meshes.get(mesh).is_some());
    let landed: Vec<AssetId<Mesh>> = waiting
        .awaited
        .drain()
        .filter(|&mesh| meshes.get(mesh).is_some())
        .collect();
    waiting.arrived.extend(landed);
}

fn descendants(world: &World, entity: Entity) -> Vec<Entity> {
    let mut all = vec![entity];
    let mut at = 0;
    while let Some(&next) = all.get(at) {
        all.extend(world.get::<Children>(next).into_iter().flatten());
        at += 1;
    }
    all
}
