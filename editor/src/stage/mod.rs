//! The one stage every preview in the editor plays on: the game's starting area behind, and the
//! plates of the actors standing on it. A scene is either posed by the designer, or taken live from
//! a server world running the game's own mechanics.

pub mod live;
pub(crate) mod picture;

use std::collections::{HashMap, HashSet};
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::geometry::PlacedShape;
use engine::core::math::Pos;
use engine::core::units::{Meters, Seconds};
use engine::core::vfx::{VfxLayer, VfxParams};
use engine::systems::GameSettings;
use engine::systems::actor::ModelId;
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::skill::SkillId;
use engine::systems::status::StatusEffectId;
use ui::egui::{Color32, Pos2, Ui, Vec2 as EVec2};

use self::live::LiveWorld;
use self::picture::Stage;
use crate::app::Editor;
use crate::thumbs;

pub const SIZE: [u32; 2] = [384, 288];
pub(crate) const ZOOM: f32 = 2.0;
const NARROWEST: f32 = 240.0;
use crate::views::navigation::PX_PER_METER;

#[derive(Default)]
pub struct Stages {
    requests: Vec<Request>,
    stages: HashMap<String, Stage>,
    live: HashMap<String, LiveWorld>,
    indices: Vec<usize>,
}

impl Stages {
    fn stage(&mut self, world: &mut World, key: &str) -> &mut Stage {
        let indices = &mut self.indices;
        self.stages.entry(key.to_owned()).or_insert_with(|| {
            let index = (0..).find(|index| !indices.contains(index)).unwrap_or(0);
            indices.push(index);
            Stage::spawn(world, index)
        })
    }

    fn render(&mut self, world: &mut World, key: &str, frame: &picture::Frame) {
        self.stage(world, key);
        if let Some(stage) = self.stages.get_mut(key) {
            stage.render(world, frame);
        }
    }
}

/// What a window asks to see this frame.
#[derive(Clone, PartialEq)]
pub enum Scene {
    Posed(Posed),
    Live(Scenario),
}

/// A scene arranged by the designer: actors standing as told, one vfx played
/// on a site of it.
#[derive(Clone, PartialEq, Default)]
pub struct Posed {
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
    pub rect: egui::Rect,
    /// Screen points per world pixel.
    pub px: f32,
    pub center: Pos<Meters>,
    /// What a live scenario reports: its cast count, or why a cast refuses.
    pub report: Option<String>,
}

impl DrawnStage {
    pub fn screen(&self, world: Vec2) -> Pos2 {
        let offset = world - to_screen(self.center);
        self.rect.center() + EVec2::new(offset.x * self.px, -offset.y * self.px)
    }
}

struct Request {
    key: String,
    scene: Scene,
}

/// Where the stage stands: the game's initial spawn, which every scene is laid out around.
pub fn center(content: &Content) -> Pos<Meters> {
    content.built::<GameSettings>().initial_spawn.at
}

/// The width every view gives its preview out of `room`: the stage's own width, narrowing only as
/// far as the stage stays readable, so previews look alike across views.
pub(crate) fn preview_width(room: f32) -> f32 {
    room.clamp(NARROWEST, SIZE[0] as f32)
}

/// Draws the stage for `key` as wide as the window allows, with the game's starting area behind
/// when `ground` is on, and asks for the scene to be rendered for the next frame.
pub fn show(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    scene: Scene,
    ground: bool,
) -> Option<DrawnStage> {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return None;
    };
    let center = center(&content);
    let fit = (ui.available_width() / SIZE[0] as f32).min(1.0);
    let size = EVec2::new(SIZE[0] as f32, SIZE[1] as f32) * fit;
    let px = ZOOM * fit;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    if ground {
        let area = content
            .built::<GameSettings>()
            .initial_spawn
            .area
            .path(&content)
            .to_owned();
        crate::thumbs::areas::draw_area(editor, &painter, &area, rect, center, px);
    }
    let shown = DrawnStage {
        rect,
        px,
        center,
        report: editor
            .views
            .stage
            .live
            .get(key)
            .map(|live| live.report.clone()),
    };
    if let Some(stage) = editor.views.stage.stages.get(key) {
        painter.image(stage.texture, rect, thumbs::FULL_UV, Color32::WHITE);
        for (top, label) in &stage.plates {
            plate::draw_label(ui, shown.screen(*top), label);
        }
        for problem in &stage.problems {
            ui.label(theme::warn(problem));
        }
    }
    ui.ctx().request_repaint();
    editor.views.stage.requests.push(Request {
        key: key.to_owned(),
        scene,
    });
    Some(shown)
}

/// Renders every stage asked for this frame and lets go of the rest.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let requests = std::mem::take(&mut editor.views.stage.requests);
    let Editor { views, project, .. } = editor;
    let stages = &mut views.stage;
    let loaded = project
        .as_ref()
        .and_then(|project| Some((project.content.as_ref()?, project.generation)));
    let wanted: HashSet<&str> = requests
        .iter()
        .map(|request| request.key.as_str())
        .collect();
    let gone: Vec<String> = stages
        .stages
        .keys()
        .filter(|key| !wanted.contains(key.as_str()) || loaded.is_none())
        .cloned()
        .collect();
    for key in gone {
        if let Some(stage) = stages.stages.remove(&key) {
            stages.indices.retain(|index| *index != stage.index);
            stage.despawn(world);
        }
        stages.live.remove(&key);
    }
    let Some((content, generation)) = loaded else {
        return;
    };
    let real = world.resource::<Time<Real>>().delta();
    for request in requests {
        let frame = match &request.scene {
            Scene::Posed(posed) => posed_frame(posed, content),
            Scene::Live(scenario) => {
                let live = match stages.live.get_mut(&request.key) {
                    Some(live) if live.generation == generation && !live.stale => live,
                    _ => match LiveWorld::build(content, scenario, generation) {
                        Ok(live) => stages
                            .live
                            .entry(request.key.clone())
                            .insert_entry(live)
                            .into_mut(),
                        Err(problem) => {
                            stages.stage(world, &request.key).problems = vec![problem];
                            continue;
                        }
                    },
                };
                live.reconcile(scenario);
                live.step(real);
                live_frame(live, content)
            }
        };
        stages.render(world, &request.key, &frame);
    }
}

/// A ground point as the stage's flat camera sees it, in its pixels with y up.
pub(crate) fn to_screen(at: Pos<Meters>) -> Vec2 {
    Vec2::new(at.x * PX_PER_METER, -at.y * PX_PER_METER)
}

fn posed_frame<'a>(posed: &Posed, content: &'a Content) -> picture::Frame<'a> {
    picture::Frame {
        content,
        center: center(content),
        actors: posed.actors.clone(),
        has_vfx: posed.vfx.is_some(),
    }
}

fn live_frame<'a>(live: &mut LiveWorld, content: &'a Content) -> picture::Frame<'a> {
    picture::Frame {
        content,
        center: center(content),
        actors: live.actors(),
        has_vfx: !live.vfx().is_empty(),
    }
}
