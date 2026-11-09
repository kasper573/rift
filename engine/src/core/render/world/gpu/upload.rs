//! How much of the world's data the GPU is sent a frame: whatever writes it spends one budget, a
//! few megabytes a frame, so much arriving at once costs many frames a little rather than one frame
//! a lot; behind a cover, where a long frame shows nothing, far more.
//!
//! Bevy's own assets, the meshes and images, are left out of the budget: bevy 0.20 never draws a
//! mesh whose upload its per-frame limit put off past the frame that first asked for its pipeline,
//! so a body's part or a held item would stay missing for good.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};

pub struct UploadPlugin;

impl Plugin for UploadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameUploads>()
            .add_systems(First, begin_frame);
    }
}

/// What is left of the frame's budget for writing the world's data to the GPU.
#[derive(SystemParam)]
pub struct UploadBudget<'w> {
    uploads: ResMut<'w, FrameUploads>,
    transition: Option<Res<'w, ActiveTransition>>,
}

impl UploadBudget<'_> {
    /// Spends `bytes`, if what is left holds them: what is larger than the whole budget goes
    /// alone in a frame, the first thing spent.
    pub fn spend(&mut self, bytes: usize) -> bool {
        let covered = self.transition.as_deref().is_some_and(hides_world);
        self.uploads.spend(bytes, per_frame(covered))
    }
}

/// Spends `bytes` of the frame's budget for writing the world's data to the GPU, if what is left
/// holds them, as [`UploadBudget::spend`] does for a system of its own.
pub fn spend(world: &mut World, bytes: usize) -> bool {
    let covered = world
        .get_resource::<ActiveTransition>()
        .is_some_and(hides_world);
    world
        .get_resource_mut::<FrameUploads>()
        .is_none_or(|mut uploads| uploads.spend(bytes, per_frame(covered)))
}

/// The most bytes written to the GPU for the world in a frame: what is larger than what is left goes
/// in a frame of its own.
const PER_FRAME: usize = 4 << 20;
/// The same while a cover hides the world, so what stands behind it stands in a few frames: as
/// much as a frame still writes within its time, as the cover's own frames keep the frame
/// budget too.
const PER_COVERED_FRAME: usize = 16 << 20;

/// The bytes spent this frame on writing the world's data to the GPU.
#[derive(Resource, Default)]
pub struct FrameUploads {
    spent: usize,
}

impl FrameUploads {
    fn spend(&mut self, bytes: usize, budget: usize) -> bool {
        if self.spent > 0 && self.spent + bytes > budget {
            return false;
        }
        self.spent += bytes;
        true
    }
}

fn begin_frame(mut uploads: ResMut<FrameUploads>) {
    uploads.spent = 0;
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
