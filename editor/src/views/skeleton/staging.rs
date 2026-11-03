use super::{Chosen, PosedClip, grips};
use crate::app::Editor;
use crate::project::GltfCache;
use crate::project::ProjectFiles;
use crate::views::area::camera::AreaCanvas;
use crate::views::area::viewport::StageClip;
use crate::views::model::{self};
use bevy::math::{Mat4, Vec3};
use engine::core::content::gltf::{self, ClipKeys, NodeTree, Rest, TreeNode};
use engine::core::math::Pos;
use engine::core::render::camera::ground_pos;
use engine::core::render::world::rig_clip;
use engine::core::units::Meters;
use engine::systems::actor::ModelDef;
use engine::systems::actor::skeleton::SkeletonDef;
use std::sync::Arc;
use ui::egui::{self, Pos2, Rect, Response, Stroke, Ui, Vec2};
use ui::{glyph, theme};

/// A stage drawn this frame: where, the canvas to draw over it through, and every joint of our rig
/// where it shows on the screen.
pub struct Staged {
    pub rect: Rect,
    pub response: Response,
    pub canvas: AreaCanvas,
    pub joints: Vec<(String, Pos2)>,
    lines: Vec<(Pos2, Pos2)>,
    /// Every node of the rig as it stands, in the rig's own frame.
    worlds: Vec<(String, Mat4)>,
    /// How the rig's frame stands in the scene.
    placed: Mat4,
    /// Whether this frame's click picked the hitbox or a grip frame, so it picks no joint.
    claimed: bool,
}

impl Staged {
    /// Draws the rig's joints over the stage, `lit` ones marked, and gives the joint clicked.
    pub fn pick(&self, ui: &Ui, lit: &[&str]) -> Option<String> {
        let painter = ui.painter_at(self.rect);
        for (from, to) in &self.lines {
            painter.line_segment([*from, *to], Stroke::new(1.5, theme::FAINT));
        }
        let hovered = self.response.hover_pos().and_then(|at| self.nearest(at));
        for (name, at) in &self.joints {
            let lit = lit.contains(&name.as_str());
            let colour = if lit { theme::ACCENT } else { theme::TEXT };
            painter.circle_filled(*at, if lit { 4.5 } else { 3.0 }, colour);
            if hovered.as_deref() == Some(name.as_str()) {
                painter.circle_stroke(*at, 7.0, Stroke::new(1.5, theme::ACCENT));
                glyph::haloed(
                    &painter,
                    *at + Vec2::new(9.0, -9.0),
                    egui::Align2::LEFT_BOTTOM,
                    name,
                    egui::FontId::monospace(12.0),
                    egui::Color32::WHITE,
                    glyph::Halo {
                        color: egui::Color32::BLACK,
                        width: 1.0,
                    },
                );
            }
        }
        if self.response.clicked() && !self.claimed {
            return self
                .response
                .interact_pointer_pos()
                .and_then(|at| self.nearest(at));
        }
        None
    }

    /// Draws the rig as its joints and the lines between them, in `colour`.
    pub fn draw(&self, ui: &Ui, colour: egui::Color32) {
        let painter = ui.painter_at(self.rect);
        for (from, to) in &self.lines {
            painter.line_segment([*from, *to], Stroke::new(2.0, colour));
        }
        for (_, at) in &self.joints {
            painter.circle_filled(*at, 2.5, colour);
        }
    }

    /// How the joint `name` stands, in the rig's frame.
    pub fn joint(&self, name: &str) -> Option<Mat4> {
        self.worlds
            .iter()
            .find(|(joint, _)| joint == name)
            .map(|(_, world)| *world)
    }

    /// Where `at`, a point in the rig's frame, shows on the screen.
    pub fn project(&self, at: Vec3) -> Pos2 {
        project(&self.canvas, self.placed.transform_point3(at))
    }

    /// The ground the rig stands on.
    pub fn ground(&self) -> Pos<Meters> {
        ground_pos(self.placed.w_axis.truncate())
    }

    fn nearest(&self, at: Pos2) -> Option<String> {
        self.joints
            .iter()
            .map(|(name, joint)| (name, joint.distance(at)))
            .filter(|(_, distance)| *distance < PICK_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name.clone())
    }
}

/// How near the pointer, in points, takes a joint.
pub(crate) const PICK_REACH: f32 = 10.0;

/// The hitbox of the model at `wearer` round the rig on `staged`, and the hands' grip frames on
/// it, as the view shows them: a click on either picks it for the inspector, and a click on neither
/// lets go of what was picked, unless it lands on a joint the page is picking. Gives whether the
/// pointer is dragging one, so the stage keeps still.
pub(crate) fn handles(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    def: &SkeletonDef,
    staged: &mut Staged,
    wearer: Option<&str>,
) -> bool {
    let toggles = &editor.views.toggles;
    let (hitbox_shown, grips_shown) = (toggles.hitbox, toggles.grips);
    let view = editor.views.skeleton.views.view_mut(key);
    if !hitbox_shown {
        view.hitbox.let_go();
    }
    view.chosen = view.chosen.filter(|chosen| match chosen {
        Chosen::Hitbox => hitbox_shown,
        Chosen::Grip(_) => grips_shown,
    });
    let chosen = view.chosen;
    let on_joint = view.picks_joints()
        && staged.response.clicked()
        && staged
            .response
            .interact_pointer_pos()
            .is_some_and(|at| staged.nearest(at).is_some());
    let hitbox = wearer.filter(|_| hitbox_shown).and_then(|path| {
        let model = editor.read::<ModelDef>(path)?.ok()?;
        Some((path.to_owned(), model.hitbox))
    });
    let view = editor.views.skeleton.views.view_mut(key);
    let touched = match &hitbox {
        Some((_, capsule)) => model::hitbox::overlay(
            ui,
            &staged.canvas,
            &staged.response,
            staged.ground(),
            *capsule,
            chosen == Some(Chosen::Hitbox),
            &mut view.hitbox,
        ),
        None => Default::default(),
    };
    let grip = match chosen {
        Some(Chosen::Grip(index)) => Some(index),
        _ => None,
    };
    let hands = view.grips.working(&def.hands);
    let gripped = grips::overlay(editor, ui, key, staged, &hands, grip);
    let view = editor.views.skeleton.views.view_mut(key);
    view.chosen = match (gripped.picked, touched.picked) {
        _ if on_joint => view.chosen,
        (Some(index), _) => {
            view.hand = index;
            Some(Chosen::Grip(index))
        }
        (None, true) => Some(Chosen::Hitbox),
        (None, false) if staged.response.clicked() && !gripped.claims_click => None,
        (None, false) => view.chosen,
    };
    staged.claimed =
        !on_joint && (gripped.picked.is_some() || touched.picked || gripped.claims_click);
    let busy = view.hitbox.dragging();
    if let (Some(sized), Some((path, _))) = (touched.sized, hitbox) {
        model::set_hitbox(editor, &path, sized);
    }
    busy
}

/// Where each joint of `rig` shows on the stage's picture, standing `pose` (at rest without one)
/// as `placed` stands its frame in the scene.
pub(crate) fn place(
    rect: Rect,
    response: Response,
    canvas: AreaCanvas,
    rig: &NodeTree,
    pose: Option<&[Rest]>,
    placed: Mat4,
) -> Staged {
    let worlds = posed_worlds(rig, pose);
    let screen = |world: &Mat4| project(&canvas, placed.transform_point3(world.w_axis.truncate()));
    let joints: Vec<(String, Pos2)> = rig
        .joints()
        .map(|joint| (rig.nodes[joint].name.clone(), screen(&worlds[joint])))
        .collect();
    let lines = rig
        .joints()
        .filter_map(|joint| {
            let parent = rig.nodes[joint]
                .parent
                .filter(|&parent| !rig.nodes[parent].mesh)?;
            Some((screen(&worlds[parent]), screen(&worlds[joint])))
        })
        .collect();
    let worlds = rig
        .nodes
        .iter()
        .zip(worlds)
        .map(|(node, world)| (node.name.clone(), world))
        .collect();
    Staged {
        rect,
        response,
        canvas,
        joints,
        lines,
        worlds,
        placed,
        claimed: false,
    }
}

/// Where `at`, a point in the scene, shows on `canvas`.
pub(crate) fn project(canvas: &AreaCanvas, at: Vec3) -> Pos2 {
    let ground = ground_pos(at);
    let lift = at.y - canvas.area().heights.height_at(ground).0;
    canvas.lifted(ground, Meters(lift))
}

/// Every node of `tree` in the scene, standing `pose`, or at rest without one.
pub(crate) fn posed_worlds(tree: &NodeTree, pose: Option<&[Rest]>) -> Vec<Mat4> {
    match pose {
        None => tree.worlds(),
        Some(pose) => NodeTree {
            nodes: tree
                .nodes
                .iter()
                .zip(pose)
                .map(|(node, rest)| TreeNode {
                    rest: *rest,
                    ..node.clone()
                })
                .collect(),
        }
        .worlds(),
    }
}

/// How `rig` stands `at` seconds into `clip`, as the game plays it there.
pub(crate) fn pose_of(
    editor: &mut Editor,
    rig: &Arc<NodeTree>,
    clip: &StageClip,
    at: f32,
) -> Option<Vec<Rest>> {
    let keys = rig_keys(editor, rig, clip)?;
    Some(keys.pose(rig, at))
}

/// The keys `clip` plays on `rig`, made once and kept while the rig stands.
pub(crate) fn rig_keys(
    editor: &mut Editor,
    rig: &Arc<NodeTree>,
    clip: &StageClip,
) -> Option<Arc<ClipKeys>> {
    let Editor { project, views, .. } = editor;
    let files = &project.as_ref()?.files;
    let gltfs = &mut views.gltfs;
    let views = &mut views.skeleton;
    let key = (
        Arc::as_ptr(rig) as usize,
        (clip.file.clone(), clip.animation, clip.pairs.clone()),
    );
    views
        .posed
        .retain(|_, posed| Arc::strong_count(&posed.rig) > 1);
    if let Some(posed) = views.posed.get(&key) {
        return posed.keys.clone();
    }
    if views.posed.len() > MAX_POSED {
        views.posed.clear();
    }
    let keys = made_keys(files, gltfs, rig, clip).map(Arc::new);
    views.posed.insert(
        key,
        PosedClip {
            rig: rig.clone(),
            keys: keys.clone(),
        },
    );
    keys
}

/// The most clips kept posed at once.
pub(crate) const MAX_POSED: usize = 64;

pub(crate) fn made_keys(
    files: &ProjectFiles,
    gltfs: &mut GltfCache,
    rig: &Arc<NodeTree>,
    clip: &StageClip,
) -> Option<ClipKeys> {
    let info = gltfs.info(files, &clip.file)?;
    let keys = gltf::clip_keys(&files.get(&clip.file)?, clip.animation).ok()?;
    let make = rig_clip::maker(rig.clone(), clip.pairs.clone());
    Some(make(&info.tree, &keys))
}
