//! The one stage every preview in the editor plays on, so a preview shows what the game shows: the
//! game's starting area behind, actors drawn with the game's own frames, anchor and row depths,
//! items as they lie dropped, vfx through the game's presenters. A scene is either posed by the
//! designer, or taken live from a server world running the game's own mechanics.

pub mod live;
mod mirror;

use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::geometry::{Footprint, Radians};
use engine::core::math::{Direction, Offset, Pos, Rgba};
use engine::core::render::ToScreen;
use engine::core::time::{PlaybackRate, Seconds};
use engine::core::units::Meters;
use engine::core::vfx::render::SiteTexture;
use engine::core::vfx::{VfxLayer, VfxParams};
use engine::systems::GameSettings;
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::actor::{Action, ModelId};
use engine::systems::item::ItemId;
use engine::systems::skill::SkillId;
use engine::systems::status::StatusEffectId;
use ui::egui::{Color32, Pos2, Ui, Vec2 as EVec2};

use self::live::LiveWorld;
use self::mirror::{ActorPose, Site, Source, Stage, VfxPose};
use crate::app::Editor;
use crate::project::ProjectFiles;
use crate::thumbs;

use crate::views::area::scene::{AreaSceneRequest, AreaSceneSpec, AreaScenes};

pub const SIZE: [u32; 2] = [384, 288];
pub const ZOOM: f32 = 2.0;
pub const NARROWEST: f32 = 240.0;
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
}

/// What a window asks to see this frame.
#[derive(Clone, PartialEq)]
pub enum Scene {
    Posed(Posed),
    Live(Scenario),
}

/// A scene arranged by the designer: actors standing as told, items on the ground, one vfx played
/// on a site of it.
#[derive(Clone, PartialEq, Default)]
pub struct Posed {
    pub actors: Vec<PosedActor>,
    pub items: Vec<ItemId>,
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
    /// Shader text as it is being typed, by shader key, played in place of the file's.
    pub drafts: Vec<(String, String)>,
}

#[derive(Clone, PartialEq)]
pub enum PosedSite {
    Actor(usize),
    Area(Footprint),
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
        let offset = world - self.center.to_screen();
        self.rect.center() + EVec2::new(offset.x * self.px, -offset.y * self.px)
    }
}

struct Request {
    key: String,
    scene: Scene,
    ground: bool,
    now: f64,
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

/// The way a sheet's strip of `direction` faces.
pub fn facing_of(direction: Direction) -> Radians {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let (x, y) = match direction {
        Direction::E => (1.0, 0.0),
        Direction::SE => (D, D),
        Direction::S => (0.0, 1.0),
        Direction::SW => (-D, D),
        Direction::W => (-1.0, 0.0),
        Direction::NW => (-D, -D),
        Direction::N => (0.0, -1.0),
        Direction::NE => (D, -D),
    };
    Radians::of(Offset::new(x, y))
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
    if ground {
        let ppp = ui.ctx().pixels_per_point();
        let generation = editor
            .project
            .as_ref()
            .map_or(0, |project| project.generation);
        let scene_key = ground_key(key);
        editor.views.area.requests.push(AreaSceneRequest {
            key: scene_key.clone(),
            spec: AreaSceneSpec::area(spawn.area.path(&content)),
            width: (rect.width() * ppp) as u32,
            height: (rect.height() * ppp) as u32,
            center: spawn.at,
            zoom: px * ppp,
            generation,
            live: true,
        });
        painter.rect_filled(rect, 4.0, theme::CANVAS);
        if let Some(scene) = editor.views.area.scenes.by_path.get(&scene_key) {
            painter.image(scene.texture, rect, scene.uv(), Color32::WHITE);
        }
    } else {
        painter.rect_filled(rect, 4.0, theme::CANVAS);
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
        ground,
        now: ui.input(|input| input.time),
    });
    Some(shown)
}

/// Renders every stage asked for this frame and lets go of the rest.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let requests = std::mem::take(&mut editor.views.stage.requests);
    let Editor { views, project, .. } = editor;
    let stages = &mut views.stage;
    let loaded = project.as_ref().and_then(|project| {
        Some((
            &project.files,
            project.content.as_ref()?,
            project.generation,
        ))
    });
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
    let Some((files, content, generation)) = loaded else {
        return;
    };
    let real = world.resource::<Time<Real>>().delta();
    for request in requests {
        let behind = behind(&views.area.scenes, &request);
        let mut frame = match &request.scene {
            Scene::Posed(posed) => posed_frame(posed, content, files, &request),
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
                live_frame(live, content, files, &request)
            }
        };
        frame.behind = behind;
        stages
            .stage(world, &request.key)
            .render(world, &mut views.area.scenes, &frame);
    }
}

/// `count` points spread evenly over the footprint, the same every time: a sunflower spiral over
/// the footprint's reach, kept where it falls inside, gives the candidates; picks start at even
/// strides through them and relax to the middle of the candidates nearest each, so they end up
/// evenly apart and well inside whatever the shape is.
pub fn spread(footprint: &Footprint, count: u32) -> Vec<Pos<Meters>> {
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

fn ground_key(key: &str) -> String {
    format!("{key}/ground")
}

fn behind(scenes: &AreaScenes, request: &Request) -> Option<SiteTexture> {
    if !request.ground {
        return None;
    }
    let scene = scenes.by_path.get(&ground_key(&request.key))?;
    let uv = scene.uv();
    Some(SiteTexture {
        image: scene.image.clone(),
        rect: Rect::from_corners(Vec2::new(uv.min.x, uv.min.y), Vec2::new(uv.max.x, uv.max.y)),
        pixel: Vec2::ONE / Vec2::new(scene.alloc[0] as f32, scene.alloc[1] as f32),
    })
}

fn posed_frame<'a>(
    posed: &Posed,
    content: &'a Content,
    files: &'a ProjectFiles,
    request: &Request,
) -> mirror::Frame<'a> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let actors = posed
        .actors
        .iter()
        .enumerate()
        .map(|(index, actor)| ActorPose {
            source: Source::Posed(index),
            model: actor.model,
            action: actor.action,
            facing: actor.facing,
            tint: actor.tint,
            attack_rate: PlaybackRate(1.0),
            at: actor.at,
            held: actor.held,
            plate: actor.plate.clone(),
        })
        .collect();
    let mut vfx = Vec::new();
    if let Some(posed_vfx) = &posed.vfx {
        vfx.push(VfxPose {
            owner: Source::Posed(0),
            layers: posed_vfx.layers.clone(),
            site: match &posed_vfx.on {
                PosedSite::Actor(index) => Site::Actor(Source::Posed(*index)),
                PosedSite::Area(footprint) => {
                    Site::World(engine::core::vfx::VfxSite::Area(*footprint))
                }
                PosedSite::Screen => Site::Screen,
            },
            heading: Radians::default(),
            elapsed: posed_vfx.elapsed,
            params: posed_vfx.params.clone(),
        });
        if let (PosedSite::Area(footprint), Some(inside)) = (&posed_vfx.on, &posed_vfx.inside) {
            for (index, actor) in posed.actors.iter().enumerate() {
                if footprint.contains(actor.at) {
                    vfx.push(VfxPose {
                        owner: Source::Posed(1 + index),
                        layers: inside.clone(),
                        site: Site::Actor(Source::Posed(index)),
                        heading: footprint.facing,
                        elapsed: posed_vfx.elapsed,
                        params: posed_vfx.params.clone(),
                    });
                }
            }
        }
    }
    mirror::Frame {
        content,
        files,
        area: spawn.area,
        center: spawn.at,
        now: request.now,
        actors,
        items: posed.items.clone(),
        vfx,
        drafts: posed
            .vfx
            .iter()
            .flat_map(|vfx| vfx.drafts.iter().cloned())
            .collect(),
        behind: None,
    }
}

fn live_frame<'a>(
    live: &mut LiveWorld,
    content: &'a Content,
    files: &'a ProjectFiles,
    request: &Request,
) -> mirror::Frame<'a> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let now = live.now();
    let actors = live
        .actors()
        .into_iter()
        .map(|shown| ActorPose {
            source: Source::Live(shown.entity),
            model: shown.actor.model,
            action: shown.actor.action,
            facing: shown.actor.facing,
            tint: shown.actor.color,
            attack_rate: shown.actor.attack_rate,
            at: shown.at,
            held: None,
            plate: shown.plate,
        })
        .collect();
    let vfx = live
        .vfx()
        .into_iter()
        .map(|(entity, instance)| VfxPose {
            owner: Source::Live(entity),
            layers: instance.vfx.get(content).layers.clone(),
            site: Site::World(instance.site.clone()),
            heading: instance.heading,
            elapsed: now.since(instance.started),
            params: instance.params.clone(),
        })
        .collect();
    mirror::Frame {
        content,
        files,
        area: spawn.area,
        center: spawn.at,
        now: request.now,
        actors,
        items: Vec::new(),
        vfx,
        drafts: HashMap::new(),
        behind: None,
    }
}
