//! The stage scenes play on: the game's starting area, drawn as the game draws it, with the
//! actors of a scene standing on it and their plates over them. A scene is either posed by the
//! designer, or taken live from a server world running the game's own mechanics.

pub mod live;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::geometry::PlacedShape;
use engine::core::math::Pos;
use engine::core::render::camera::world_point;
use engine::core::units::{Meters, Seconds};
use engine::core::vfx::{VfxLayer, VfxParams};
use engine::systems::GameSettings;
use engine::systems::actor::ModelId;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::area::Pictured;
use engine::systems::skill::SkillId;
use engine::systems::status::StatusEffectId;
use ui::egui::{self, Ui, Vec2 as EVec2};
use ui::theme;

use self::live::LiveWorld;
use crate::app::Editor;
use crate::views::area::camera::{AreaCanvas, OrbitCamera};
use crate::views::area::viewport::StandingActor;
use crate::views::stage::{self as view_stage, StagePicture};

/// The widest a stage is drawn, in points, and how tall it is to that.
const SIZE: [f32; 2] = [384.0, 288.0];
const NARROWEST: f32 = 240.0;
/// How far from its middle the stage takes in.
const REACH: Meters = Meters(6.0);

#[derive(Default)]
pub struct Stages {
    live: HashMap<String, LiveWorld>,
    shown: HashSet<String>,
    picture: StagePicture,
}

/// What a window asks to see this frame.
#[derive(Clone, PartialEq)]
pub enum Scene {
    Posed(PosedScene),
    Live(Scenario),
}

/// A scene arranged by the designer: actors standing as told, one vfx played
/// on a site of it.
#[derive(Clone, PartialEq, Default)]
pub struct PosedScene {
    pub actors: Vec<PosedActor>,
    pub vfx: Option<PosedVfx>,
}

/// An actor as the stage places it: where it stands and the plate over its head.
#[derive(Clone, PartialEq)]
pub struct PosedActor {
    pub model: ModelId,
    pub at: Pos<Meters>,
    pub plate: Option<PlateLabel>,
}

impl PosedActor {
    pub fn standing(model: ModelId, at: Pos<Meters>) -> PosedActor {
        PosedActor {
            model,
            at,
            plate: None,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct PosedVfx {
    pub layers: Vec<VfxLayer>,
    pub on: PosedSite,
    /// Layers each actor standing inside an area site also plays, as the game plays a vfx's
    /// `on_actors_inside` on whoever an area catches.
    pub inside: Option<Vec<VfxLayer>>,
    pub params: VfxParams,
    pub elapsed: Seconds,
}

#[derive(Clone, PartialEq)]
pub enum PosedSite {
    Actor(usize),
    Area(PlacedShape),
    Screen,
}

/// A scene the game's mechanics play out in a live world.
#[derive(Clone, PartialEq)]
pub enum Scenario {
    Status {
        model: ModelId,
        effect: StatusEffectId,
        applied: bool,
    },
    Skill {
        skill: SkillId,
    },
}

/// Where a stage was drawn, for a window to draw over it.
pub struct DrawnStage {
    pub canvas: AreaCanvas,
    /// What a live scenario reports: its cast count, or why a cast refuses.
    pub report: Option<String>,
}

/// Where the stage stands: the game's initial spawn, which every scene is laid out around.
pub fn center(content: &Content) -> Pos<Meters> {
    content.built::<GameSettings>().initial_spawn.at
}

/// The width every view gives its preview out of `room`: the stage's own width, narrowing only as
/// far as the stage stays readable, so previews look alike across views.
pub(crate) fn preview_width(room: f32) -> f32 {
    room.clamp(NARROWEST, SIZE[0])
}

/// Draws the stage for `key` as wide as the window allows, playing `scene` on it.
pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, scene: Scene) -> Option<DrawnStage> {
    let Some((content, generation)) = editor
        .project
        .as_ref()
        .and_then(|project| Some((project.content.clone()?, project.generation)))
    else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return None;
    };
    let stages = &mut editor.views.stage;
    stages.shown.insert(key.to_owned());
    let (actors, has_vfx, report) = match &scene {
        Scene::Posed(posed) => (posed.actors.clone(), posed.vfx.is_some(), None),
        Scene::Live(scenario) => {
            let live = match stages.live.get_mut(key) {
                Some(live) if live.generation == generation && !live.stale => live,
                _ => match LiveWorld::build(&content, scenario, generation) {
                    Ok(live) => stages
                        .live
                        .entry(key.to_owned())
                        .insert_entry(live)
                        .into_mut(),
                    Err(problem) => {
                        ui.label(theme::warn(problem));
                        return None;
                    }
                },
            };
            live.reconcile(scenario);
            live.step(Duration::from_secs_f32(ui.input(|i| i.stable_dt)));
            (
                live.actors(),
                !live.vfx().is_empty(),
                Some(live.report.clone()),
            )
        }
    };
    let spawn = &content.built::<GameSettings>().initial_spawn;
    let picture = stages
        .picture
        .of(&content, generation, Pictured::Area(spawn.area));
    let width = ui.available_width().min(SIZE[0]);
    let (rect, _) = ui.allocate_exact_size(
        EVec2::new(width, width * SIZE[1] / SIZE[0]),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let Some(picture) = picture else {
        ui.label(theme::warn("the starting area has no terrain to stand on"));
        return None;
    };
    let view_key = format!("stage/{key}");
    view_stage::show_picture(editor, ui, &painter, rect, &view_key);
    let camera = OrbitCamera {
        target: spawn.at,
        ..view_stage::framing(REACH.0)
    };
    let canvas = AreaCanvas::new(rect, camera, picture.1.area.clone());
    let cast = actors
        .iter()
        .map(|actor| standing(&content, &canvas, actor))
        .collect();
    for actor in &actors {
        if let Some(label) = &actor.plate {
            let height = ActorModel::get(&content, actor.model).hitbox().height;
            plate::draw_label(ui, canvas.lifted(actor.at, height), label);
        }
    }
    if has_vfx {
        ui.label(theme::warn("vfx don't play on the stage yet"));
    }
    ui.ctx().request_repaint();
    view_stage::request(editor, ui, &view_key, rect, camera, picture, cast);
    Some(DrawnStage { canvas, report })
}

/// Lets go of the live worlds of the stages no window showed since the last call.
pub fn sync(editor: &mut Editor) {
    let stages = &mut editor.views.stage;
    let shown = std::mem::take(&mut stages.shown);
    stages.live.retain(|key, _| shown.contains(key));
}

fn standing(content: &Content, canvas: &AreaCanvas, actor: &PosedActor) -> StandingActor {
    let model = ActorModel::get(content, actor.model);
    let ground = canvas.area().heights.height_at(actor.at);
    StandingActor {
        body: model.body(),
        clip: Some(view_stage::model_idle_clip(model)),
        transform: Transform::from_translation(world_point(actor.at, ground)),
        held: None,
        crossed: Vec::new(),
        props: Vec::new(),
        layers: None,
    }
}
