//! The one stage every preview in the editor plays on, so a preview shows what the game shows: the
//! game's starting area behind, actors drawn with the game's own frames, anchor and row depths,
//! vfx through the game's presenters. A scene is either posed by the
//! designer, or taken live from a server world running the game's own mechanics.

mod images;
pub mod live;
mod mirror;

use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::geometry::{PlacedShape, Radians};
use engine::core::math::{Pos, Rgba};
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::core::vfx::{VfxLayer, VfxParams};
use engine::systems::GameSettings;
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::actor::{Action, ModelId};
use engine::systems::skill::SkillId;
use engine::systems::status::StatusEffectId;
use ui::egui::{Color32, Pos2, Ui, Vec2 as EVec2};

use self::live::LiveWorld;
use self::mirror::{ActorPose, Stage};
use crate::app::Editor;
use crate::thumbs;

pub const SIZE: [u32; 2] = [384, 288];
pub const ZOOM: f32 = 2.0;
pub const NARROWEST: f32 = 240.0;
pub use crate::views::navigation::PX_PER_METER;
const SPREAD_CANDIDATES: usize = 4096;
const SPREAD_ROUNDS: usize = 12;

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

    fn render(&mut self, world: &mut World, key: &str, frame: &mirror::Frame) {
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

#[derive(Clone, PartialEq)]
pub struct PosedActor {
    pub model: ModelId,
    pub action: Action,
    pub facing: Radians,
    pub tint: Rgba,
    pub at: Pos<Meters>,
    pub plate: Option<PlateLabel>,
    /// Held at this moment of the action, or animated.
    pub held: Option<Seconds>,
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
pub struct Shown {
    pub rect: egui::Rect,
    /// Screen points per world pixel.
    pub px: f32,
    pub center: Pos<Meters>,
    /// What a live scenario reports: its cast count, or why a cast refuses.
    pub report: Option<String>,
}

impl Shown {
    pub fn screen(&self, world: Vec2) -> Pos2 {
        let offset = world - to_screen(self.center);
        self.rect.center() + EVec2::new(offset.x * self.px, -offset.y * self.px)
    }
}

struct Request {
    key: String,
    scene: Scene,
}

impl PosedActor {
    pub fn standing(model: ModelId, at: Pos<Meters>) -> PosedActor {
        PosedActor {
            model,
            action: Action::Idle,
            facing: Radians::SOUTH,
            tint: Rgba::WHITE,
            at,
            plate: None,
            held: None,
        }
    }
}

/// Where the stage stands: the game's initial spawn, which every scene is laid out around.
pub fn center(content: &Content) -> Pos<Meters> {
    content.built::<GameSettings>().initial_spawn.at
}

/// The width every view gives its preview out of `room`: the stage's own width, narrowing only as
/// far as the stage stays readable, so previews look alike across views.
pub fn preview_width(room: f32) -> f32 {
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
) -> Option<Shown> {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return None;
    };
    let spawn = content.built::<GameSettings>().initial_spawn;
    let fit = (ui.available_width() / SIZE[0] as f32).min(1.0);
    let size = EVec2::new(SIZE[0] as f32, SIZE[1] as f32) * fit;
    let px = ZOOM * fit;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    if ground {
        let area = spawn.area.path(&content).to_owned();
        crate::thumbs::areas::draw_area(editor, &painter, &area, rect, spawn.at, px);
    }
    let shown = Shown {
        rect,
        px,
        center: spawn.at,
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
pub fn to_screen(at: Pos<Meters>) -> Vec2 {
    Vec2::new(at.x * PX_PER_METER, -at.y * PX_PER_METER)
}

/// `count` points spread evenly over the footprint, the same every time: a sunflower spiral over
/// the footprint's reach, kept where it falls inside, gives the candidates; picks start at even
/// strides through them and relax to the middle of the candidates nearest each, so they end up
/// evenly apart and well inside whatever the shape is.
pub fn spread(footprint: &PlacedShape, count: u32) -> Vec<Pos<Meters>> {
    let reach = footprint.reach().0.max(0.1);
    let golden = PI * (3.0 - 5f32.sqrt());
    let inside: Vec<Pos<Meters>> = (0..SPREAD_CANDIDATES)
        .map(|index| {
            let radius = reach * ((index as f32 + 0.5) / SPREAD_CANDIDATES as f32).sqrt();
            let angle = index as f32 * golden;
            Pos::new(
                footprint.at.x + radius * angle.cos(),
                footprint.at.y + radius * angle.sin(),
            )
        })
        .filter(|point| footprint.contains(*point))
        .collect();
    if inside.is_empty() {
        return vec![footprint.at; count as usize];
    }
    let mut picks: Vec<usize> = (0..count)
        .map(|k| {
            let stride = (k as f32 + 0.5) * inside.len() as f32 / count as f32;
            (stride as usize).min(inside.len() - 1)
        })
        .collect();
    let nearest = |to: Pos<Meters>, among: &[usize]| {
        among
            .iter()
            .copied()
            .min_by(|a, b| {
                (inside[*a] - to)
                    .length()
                    .total_cmp(&(inside[*b] - to).length())
            })
            .unwrap_or(0)
    };
    let all: Vec<usize> = (0..inside.len()).collect();
    for _ in 0..SPREAD_ROUNDS {
        let mut sums = vec![(Vec2::ZERO, 0usize); picks.len()];
        for point in &inside {
            let owner = picks
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (inside[*a.1] - *point)
                        .length()
                        .total_cmp(&(inside[*b.1] - *point).length())
                })
                .map_or(0, |(owner, _)| owner);
            sums[owner].0 += Vec2::new(point.x, point.y);
            sums[owner].1 += 1;
        }
        for (pick, (sum, members)) in picks.iter_mut().zip(sums) {
            if members > 0 {
                let mean = sum / members as f32;
                *pick = nearest(Pos::new(mean.x, mean.y), &all);
            }
        }
    }
    picks.into_iter().map(|pick| inside[pick]).collect()
}
fn posed_frame<'a>(posed: &Posed, content: &'a Content) -> mirror::Frame<'a> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let actors = posed
        .actors
        .iter()
        .map(|actor| ActorPose {
            model: actor.model,
            at: actor.at,
            plate: actor.plate.clone(),
        })
        .collect();
    let mut not_yet = Vec::new();
    if posed.vfx.is_some() {
        not_yet.push("vfx");
    }
    mirror::Frame {
        content,
        center: spawn.at,
        actors,
        not_yet,
    }
}

fn live_frame<'a>(live: &mut LiveWorld, content: &'a Content) -> mirror::Frame<'a> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let actors = live
        .actors()
        .into_iter()
        .map(|shown| ActorPose {
            model: shown.actor.model,
            at: shown.at,
            plate: shown.plate,
        })
        .collect();
    let not_yet = if live.vfx().is_empty() {
        Vec::new()
    } else {
        vec!["vfx"]
    };
    mirror::Frame {
        content,
        center: spawn.at,
        actors,
        not_yet,
    }
}
