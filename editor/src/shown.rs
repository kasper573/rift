//! What a pass of the interface drew, for the frame log and the probe: the panels the window is
//! parted into, the trees with the rows they showed, the stage with what it holds where it shows,
//! and how long the trees and the stage's overlays took to draw. Kept in egui's data for the pass, begun anew before each.

use std::time::Duration;

use bevy::math::Mat4;
use ui::egui::{self, Context, Id, Pos2, Rect};

use crate::views::area::camera::OrbitCamera;

#[derive(Clone, Default)]
pub struct ShownPass {
    /// Whether a probe watches the editor, so the pass reports what only a probe reads.
    pub probed: bool,
    pub panels: Vec<(&'static str, Rect)>,
    pub trees: Vec<ShownTree>,
    pub stage: Option<ShownStage>,
    pub tree_time: Duration,
    pub overlay_time: Duration,
}

#[derive(Clone)]
pub struct ShownTree {
    pub name: &'static str,
    pub rect: Rect,
    pub rows: Vec<ShownRow>,
    pub selected: Vec<String>,
}

#[derive(Clone)]
pub struct ShownRow {
    pub id: String,
    pub rect: Rect,
    pub folder: bool,
    pub open: bool,
}

#[derive(Clone)]
pub struct ShownStage {
    pub rect: Rect,
    pub view_proj: Mat4,
    pub camera: OrbitCamera,
    pub farthest: f32,
    /// What the stage holds nearest the middle of its rect, each by its scene row, where it shows.
    pub marks: Vec<(String, Pos2)>,
    /// The scene row a click at the pointer picks.
    pub picked: Option<String>,
}

/// Begins a pass, reporting what only a probe reads when `probed`.
pub fn begin(ctx: &Context, probed: bool) {
    ctx.data_mut(|data| {
        data.insert_temp(
            id(),
            ShownPass {
                probed,
                ..ShownPass::default()
            },
        );
    });
}

/// What the pass begun last drew.
pub fn pass(ctx: &Context) -> ShownPass {
    ctx.data(|data| data.get_temp::<ShownPass>(id()))
        .unwrap_or_default()
}

pub fn probed(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<ShownPass>(id()))
        .is_some_and(|pass| pass.probed)
}

pub fn panel(ctx: &Context, name: &'static str, rect: Rect) {
    with_pass(ctx, |pass| pass.panels.push((name, rect)));
}

pub fn tree(ctx: &Context, tree: ShownTree, took: Duration) {
    with_pass(ctx, |pass| {
        pass.trees.push(tree);
        pass.tree_time += took;
    });
}

pub fn stage(ctx: &Context, stage: ShownStage) {
    with_pass(ctx, |pass| pass.stage = Some(stage));
}

pub fn overlays(ctx: &Context, took: Duration) {
    with_pass(ctx, |pass| pass.overlay_time += took);
}

fn with_pass(ctx: &Context, change: impl FnOnce(&mut ShownPass)) {
    ctx.data_mut(|data| change(data.get_temp_mut_or_default::<ShownPass>(id())));
}

fn id() -> Id {
    egui::Id::new("shown pass")
}
