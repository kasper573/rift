//! The vfx preview: a stage off screen that plays a row's layers on a site of the designer's
//! choosing (an actor, an area, a crowd in an area, or the screen) into an image the view shows.

use std::collections::{BTreeMap, HashMap};
use std::f32::consts::PI;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::prelude::*;
use bevy::shader::Shader;
use bevy::sprite_render::MeshMaterial2d;
use bevy_egui::EguiUserTextures;
use egui::{Color32, FontId, Pos2, Stroke, Ui, Vec2 as EVec2};
use engine::core::geometry::{Degrees, Footprint, Shape};
use engine::core::math::{FrameSize, Pos, Rgba};
use engine::core::render::{TILE, ToScreen};
use engine::core::tiling::Tiles;
use engine::core::time::Seconds;
use engine::core::vfx::render::{
    self as vfx_render, SITE_ACTOR, SITE_AREA, SITE_SCREEN, SiteUniform, VfxMaterial, quad_overscan,
};
use engine::core::vfx::{
    ParamFill, ParamValue, Play, ShaderError, SpriteAnchor, VfxDef, VfxParams, VfxShaderDef,
    shader_file,
};
use engine::systems::actor::Action;
use serde::Deserialize;
use serde_json::Value;
use strum::VariantArray;

use super::SHADER_TABLE;
use super::params::{ParamSpec, param_control};
use crate::app::Editor;
use crate::project::{ProjectFiles, row_file};
use crate::ui::{theme, widgets};
use crate::views::area::scene;
use crate::views::model;

pub const PREVIEW: [u32; 2] = [384, 288];
const ZOOM: f32 = 2.0;
const PREVIEW_LAYER: usize = 31;
const REST_BETWEEN_PLAYS: f32 = 0.5;
const SLIDER_SURROUND: f32 = 130.0;
const KEPT_SHADERS: usize = 64;
const BACKDROP_SCENE: &str = "vfx-preview";
const MOST_ACTORS: u32 = 100;
const SPREAD_CANDIDATES: usize = 4096;
const GROUND_DEPTH: f32 = 0.5;
const OVER_ACTORS_DEPTH: f32 = 3.0;

/// The vfx previews: what the open row asked to see this frame, and the stage that renders it.
pub struct VfxViews {
    pub target: Target,
    pub background: bool,
    started: Option<(String, f64)>,
    pub fills: HashMap<String, BTreeMap<String, ParamValue>>,
    request: Option<PreviewRequest>,
    overlays: Vec<Overlay>,
    problems: Vec<String>,
    stage: Option<Stage>,
}

impl Default for VfxViews {
    fn default() -> VfxViews {
        VfxViews {
            target: Target::default(),
            background: true,
            started: None,
            fills: HashMap::new(),
            request: None,
            overlays: Vec::new(),
            problems: Vec::new(),
            stage: None,
        }
    }
}

/// The site a preview plays on. Actors stand on the backdrop playing an animation of the game's
/// models; a crowd spreads evenly over the area, cycling through the models in key order.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Actor {
        model: String,
        action: Action,
    },
    Area {
        shape: Shape,
        facing: Degrees,
    },
    Crowd {
        shape: Shape,
        facing: Degrees,
        count: u32,
        action: Action,
    },
    Screen,
}

impl Default for Target {
    fn default() -> Target {
        Target::Actor {
            model: String::new(),
            action: Action::Idle,
        }
    }
}

pub fn preview(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    layers: Vec<Layer>,
    specs: &[ParamSpec],
    drafts: &[(String, String)],
) {
    let models = models_of(editor);
    let total = layers
        .iter()
        .map(|layer| layer.delay().0 + layer.duration().0)
        .fold(0.5_f32, f32::max);
    let now = ui.input(|input| input.time);
    let views = &mut editor.views.vfx;
    if let Target::Actor { model, .. } = &mut views.target
        && model.is_empty()
        && let Some(first) = models.first()
    {
        *model = first.clone();
    }
    ui.horizontal_wrapped(|ui| {
        for (label, on) in [
            ("actor", matches!(views.target, Target::Actor { .. })),
            ("area", matches!(views.target, Target::Area { .. })),
            (
                "area + actors",
                matches!(views.target, Target::Crowd { .. }),
            ),
            ("screen", views.target == Target::Screen),
        ] {
            if widgets::mode_chip(ui, label, on).clicked() && !on {
                views.target = match label {
                    "actor" => Target::Actor {
                        model: models.first().cloned().unwrap_or_default(),
                        action: Action::Idle,
                    },
                    "area" => Target::Area {
                        shape: Shape::Circle { radius: Tiles(1.5) },
                        facing: Degrees(0.0),
                    },
                    "area + actors" => Target::Crowd {
                        shape: Shape::Circle { radius: Tiles(2.5) },
                        facing: Degrees(0.0),
                        count: 5,
                        action: Action::Idle,
                    },
                    _ => Target::Screen,
                };
            }
        }
    });
    ui.horizontal_wrapped(|ui| match &mut views.target {
        Target::Actor { model, action } => {
            widgets::combo_box("preview-model")
                .width(160.0)
                .selected_text(theme::mono(model.as_str()).size(13.0))
                .show_ui(ui, |ui| {
                    for candidate in &models {
                        ui.selectable_value(model, candidate.clone(), candidate);
                    }
                });
            ui.end_row();
            action_combo(ui, action);
        }
        Target::Area { shape, facing } => {
            shape_fields(ui, shape);
            facing_field(ui, facing);
        }
        Target::Crowd {
            shape,
            facing,
            count,
            action,
        } => {
            shape_fields(ui, shape);
            facing_field(ui, facing);
            ui.end_row();
            ui.label(theme::faint("actors").size(12.0));
            ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_SURROUND).max(60.0);
            design_tokens::widgets::slider(ui, egui::Slider::new(count, 1..=MOST_ACTORS));
            ui.end_row();
            action_combo(ui, action);
        }
        Target::Screen => {}
    });
    let started = match &views.started {
        Some((playing, started)) if playing == key => *started,
        _ => {
            views.started = Some((key.to_owned(), now));
            now
        }
    };
    let elapsed = ((now - started) as f32)
        .rem_euclid(total + REST_BETWEEN_PLAYS)
        .min(total);
    ui.ctx().request_repaint();
    ui.horizontal(|ui| {
        ui.checkbox(&mut views.background, "background");
        ui.label(theme::faint(format!("{elapsed:.2} s of {total:.2}")).size(12.0));
    });
    let fills = views.fills.entry(key.to_owned()).or_default();
    for spec in specs {
        fills.entry(spec.name.clone()).or_insert(spec.default);
    }
    if !specs.is_empty() && ui.button("reset to defaults").clicked() {
        for spec in specs {
            fills.insert(spec.name.clone(), spec.default);
        }
    }
    ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_SURROUND).max(60.0);
    for spec in specs {
        ui.horizontal(|ui| {
            ui.label(theme::muted(&spec.name).monospace().size(12.5));
            let mut value = fills.get(&spec.name).copied().unwrap_or(spec.default);
            if param_control(ui, spec, &mut value) {
                fills.insert(spec.name.clone(), value);
            }
        });
    }
    let fills = fills.clone();
    let background = views.background;
    let fit = (ui.available_width() / PREVIEW[0] as f32).min(1.0);
    let size = EVec2::new(PREVIEW[0] as f32, PREVIEW[1] as f32) * fit;
    let zoom = ZOOM * fit;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if background {
        model::preview::backdrop(editor, ui.painter(), rect, BACKDROP_SCENE.to_owned(), zoom);
    } else {
        ui.painter().rect_filled(rect, 4.0, theme::CANVAS);
    }
    let views = &mut editor.views.vfx;
    if let Some(stage) = &views.stage {
        ui.painter().image(
            stage.texture,
            rect,
            egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let to_image =
        |at: Vec2| rect.min + EVec2::new(at.x * zoom + size.x / 2.0, -at.y * zoom + size.y / 2.0);
    if let Some(footprint) = views.target.footprint() {
        let outline: Vec<Pos2> = footprint
            .outline(32)
            .into_iter()
            .map(|point| to_image(point.to_screen()))
            .collect();
        if let (Some(first), true) = (outline.first().copied(), outline.len() > 1) {
            let mut closed = outline.clone();
            closed.push(first);
            ui.painter().add(egui::Shape::dashed_line(
                &closed,
                Stroke::new(1.0, theme::WARN),
                4.0,
                3.0,
            ));
        }
    }
    for overlay in &views.overlays {
        let at = to_image(overlay.at);
        let [r, g, b, _] = overlay.color.0.to_be_bytes();
        let ink = Color32::from_rgba_unmultiplied(r, g, b, (overlay.alpha * 255.0) as u8);
        ui.painter().text(
            at,
            egui::Align2::CENTER_BOTTOM,
            &overlay.text,
            FontId::proportional(overlay.size * zoom),
            ink,
        );
    }
    for problem in &views.problems {
        ui.label(theme::warn(problem));
    }
    let signature = format!(
        "{key}|{}|{:?}",
        serde_json::to_string(&layers_signature(&layers)).unwrap_or_default(),
        views.target
    );
    views.request = Some(PreviewRequest {
        layers,
        target: views.target.clone(),
        elapsed,
        now,
        fills,
        drafts: drafts.iter().cloned().collect(),
        signature,
    });
}

/// Renders the requested preview into its image, so the next frame's panel shows it.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let request = editor.views.vfx.request.take();
    let Some(request) = request else {
        if let Some(stage) = editor.views.vfx.stage.take() {
            stage.despawn(world);
        }
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let files = project.files.clone();
    let Editor { views, .. } = editor;
    let stage = views.vfx.stage.get_or_insert_with(|| Stage::spawn(world));
    if stage.signature != request.signature {
        stage.clear_pieces(world);
        stage.signature = request.signature.clone();
        stage.pieces = request
            .layers
            .iter()
            .map(|layer| stage.spawn_piece(world, layer, &mut views.area.scenes, &files))
            .collect();
        stage.actors = spawn_actors(world, &request.target, &mut views.area.scenes, &files);
    }
    for actor in &stage.actors {
        actor.animate(world, request.now);
    }
    let behind = views
        .area
        .scenes
        .by_key
        .get(BACKDROP_SCENE)
        .filter(|_| views.vfx.background)
        .map(|scene| (scene.image.clone(), scene.uv(), scene.alloc));
    let Some(frame) = site_frame(world, &request.target, stage.actors.first(), behind) else {
        return;
    };
    views.vfx.overlays.clear();
    views.vfx.problems.clear();
    let given = VfxParams(request.fills.iter().map(|(k, v)| (k.clone(), *v)).collect());
    for (index, layer) in request.layers.iter().enumerate() {
        let local = Seconds(request.elapsed - layer.delay().0);
        let active = local.0 >= 0.0 && local.0 <= layer.duration().0;
        if let Layer::Text {
            template,
            color,
            size,
            rise,
            duration,
            ..
        } = layer
        {
            if active {
                let progress = (local.0 / duration.0.max(0.001)).clamp(0.0, 1.0);
                let risen = frame.center.y + frame.size.y / 2.0 + rise.0 * TILE.0 * ease(progress);
                views.vfx.overlays.push(Overlay {
                    text: VfxDef::fill(template, &given),
                    color: *color,
                    size: *size,
                    at: Vec2::new(frame.center.x, risen),
                    alpha: 1.0 - ((progress - 0.6) / 0.4).clamp(0.0, 1.0),
                });
            }
            continue;
        }
        let Some(piece) = stage.pieces.get(index).copied().flatten() else {
            continue;
        };
        if !active {
            set_visible(world, piece, false);
            continue;
        }
        match layer {
            Layer::Sprite {
                frame: cell,
                frames,
                fps,
                anchor,
                scale,
                play,
                ..
            } => {
                let sheet = world
                    .get::<Sprite>(piece)
                    .and_then(|sprite| world.resource::<Assets<Image>>().get(&sprite.image))
                    .map(Image::size_f32);
                let Some(sheet) = sheet else {
                    set_visible(world, piece, false);
                    continue;
                };
                let cell = Vec2::new(cell.width as f32, cell.height as f32);
                let index = (local.0 * fps) as u32;
                let index = match play {
                    Play::Once if index >= *frames => {
                        set_visible(world, piece, false);
                        continue;
                    }
                    Play::Once => index,
                    Play::Loop { .. } => index % (*frames).max(1),
                };
                let columns = ((sheet.x / cell.x) as u32).max(1);
                let min = Vec2::new(
                    (index % columns) as f32 * cell.x,
                    (index / columns) as f32 * cell.y,
                );
                let drawn = cell * *scale;
                let at = match anchor {
                    SpriteAnchor::Center => frame.center,
                    SpriteAnchor::Feet => {
                        frame.center + Vec2::new(0.0, (drawn.y - frame.size.y) / 2.0)
                    }
                    SpriteAnchor::Head => {
                        frame.center + Vec2::new(0.0, (drawn.y + frame.size.y) / 2.0)
                    }
                };
                let mut entity = world.entity_mut(piece);
                if let Some(mut sprite) = entity.get_mut::<Sprite>() {
                    sprite.rect = Some(Rect::from_corners(min, min + cell));
                    sprite.custom_size = Some(drawn);
                }
                if let Some(mut transform) = entity.get_mut::<Transform>() {
                    transform.translation = Vec3::new(at.x, at.y, 2.0 + index as f32 * 0.01);
                }
                set_visible(world, piece, true);
            }
            Layer::Shader {
                shader,
                params,
                duration,
                ..
            } => {
                let shown = show_shader_piece(
                    world,
                    stage,
                    piece,
                    &frame,
                    shader,
                    params,
                    &given,
                    local,
                    *duration,
                    request.elapsed,
                    &request.drafts,
                    &files,
                    &mut views.vfx.problems,
                );
                set_visible(world, piece, shown);
                if shown
                    && frame.kind == SITE_ACTOR
                    && let Some(actor) = stage.actors.first()
                {
                    set_alpha(world, actor.entity, 0.0);
                }
            }
            Layer::Text { .. } => {}
        }
    }
    let overdrawn = frame.kind == SITE_ACTOR
        && request.layers.iter().enumerate().any(|(index, layer)| {
            matches!(layer, Layer::Shader { .. })
                && stage
                    .pieces
                    .get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|piece| {
                        world
                            .get::<Visibility>(piece)
                            .is_some_and(|visibility| *visibility == Visibility::Inherited)
                    })
        });
    if !overdrawn && let Some(actor) = stage.actors.first() {
        set_alpha(world, actor.entity, 1.0);
    }
}

impl Target {
    fn footprint(&self) -> Option<Footprint> {
        match self {
            Target::Area { shape, facing } | Target::Crowd { shape, facing, .. } => {
                Some(Footprint {
                    at: Pos::new(0.0, 0.0),
                    facing: facing.radians(),
                    shape: *shape,
                })
            }
            Target::Actor { .. } | Target::Screen => None,
        }
    }
}

fn models_of(editor: &Editor) -> Vec<String> {
    let mut models: Vec<String> = editor
        .project
        .as_ref()
        .map(|project| project.files.keys_of("model").map(str::to_owned).collect())
        .unwrap_or_default();
    models.sort();
    models
}

fn action_combo(ui: &mut Ui, action: &mut Action) {
    widgets::combo_box("preview-action")
        .width(90.0)
        .selected_text(theme::mono(action.as_ref()).size(13.0))
        .show_ui(ui, |ui| {
            for candidate in Action::VARIANTS {
                ui.selectable_value(action, *candidate, candidate.as_ref());
            }
        });
}

fn facing_field(ui: &mut Ui, facing: &mut Degrees) {
    ui.add(
        egui::DragValue::new(&mut facing.0)
            .range(0.0..=360.0)
            .suffix("°"),
    );
}

fn shape_fields(ui: &mut Ui, shape: &mut Shape) {
    let kinds = ["circle", "rect", "cone", "line"];
    let current = match shape {
        Shape::Circle { .. } => "circle",
        Shape::Rect { .. } => "rect",
        Shape::Cone { .. } => "cone",
        Shape::Line { .. } => "line",
    };
    let mut picked = current;
    widgets::combo_box("preview-shape")
        .width(80.0)
        .selected_text(theme::mono(current).size(13.0))
        .show_ui(ui, |ui| {
            for kind in kinds {
                ui.selectable_value(&mut picked, kind, kind);
            }
        });
    if picked != current {
        *shape = match picked {
            "rect" => Shape::Rect {
                width: Tiles(3.0),
                height: Tiles(2.0),
            },
            "cone" => Shape::Cone {
                angle: Degrees(90.0),
                length: Tiles(2.5),
            },
            "line" => Shape::Line {
                length: Tiles(4.0),
                width: Tiles(1.0),
            },
            _ => Shape::Circle { radius: Tiles(1.5) },
        };
    }
    let tiles = |ui: &mut Ui, label: &str, value: &mut Tiles| {
        ui.label(theme::faint(label).size(12.0));
        ui.add(
            egui::DragValue::new(&mut value.0)
                .speed(0.1)
                .range(0.1..=12.0)
                .fixed_decimals(1),
        );
    };
    match shape {
        Shape::Circle { radius } => tiles(ui, "radius", radius),
        Shape::Rect { width, height } => {
            tiles(ui, "width", width);
            tiles(ui, "height", height);
        }
        Shape::Cone { angle, length } => {
            ui.label(theme::faint("angle").size(12.0));
            ui.add(
                egui::DragValue::new(&mut angle.0)
                    .range(5.0..=360.0)
                    .suffix("°"),
            );
            tiles(ui, "length", length);
        }
        Shape::Line { length, width } => {
            tiles(ui, "length", length);
            tiles(ui, "width", width);
        }
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layer {
    Sprite {
        sheet: String,
        frame: FrameSize,
        frames: u32,
        fps: f32,
        #[serde(default)]
        anchor: SpriteAnchor,
        #[serde(default = "one")]
        scale: f32,
        #[serde(default)]
        play: Play,
        #[serde(default)]
        delay: Seconds,
    },
    Text {
        template: String,
        color: Rgba,
        #[serde(default = "text_size")]
        size: f32,
        #[serde(default = "text_rise")]
        rise: Tiles,
        duration: Seconds,
        #[serde(default)]
        delay: Seconds,
    },
    Shader {
        shader: String,
        #[serde(default)]
        params: BTreeMap<String, ParamFill>,
        duration: Seconds,
        #[serde(default)]
        delay: Seconds,
    },
}

impl Layer {
    fn delay(&self) -> Seconds {
        match self {
            Layer::Sprite { delay, .. }
            | Layer::Text { delay, .. }
            | Layer::Shader { delay, .. } => *delay,
        }
    }

    fn duration(&self) -> Seconds {
        match self {
            Layer::Sprite {
                frames, fps, play, ..
            } => match play {
                Play::Once => Seconds(*frames as f32 / fps.max(0.001)),
                Play::Loop { seconds } => *seconds,
            },
            Layer::Text { duration, .. } | Layer::Shader { duration, .. } => *duration,
        }
    }
}

pub fn layers_of(row: &Value) -> Vec<Layer> {
    row["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|layer| serde_json::from_value(layer.clone()).ok())
        .collect()
}

struct PreviewRequest {
    layers: Vec<Layer>,
    target: Target,
    elapsed: f32,
    now: f64,
    fills: BTreeMap<String, ParamValue>,
    drafts: HashMap<String, String>,
    signature: String,
}

struct Overlay {
    text: String,
    color: Rgba,
    size: f32,
    at: Vec2,
    alpha: f32,
}

struct Stage {
    camera: Entity,
    image: Handle<Image>,
    texture: egui::TextureId,
    quad: Handle<Mesh>,
    blank: Handle<Image>,
    actors: Vec<StagedActor>,
    pieces: Vec<Option<Entity>>,
    shaders: HashMap<String, Result<Handle<Shader>, ShaderError>>,
    signature: String,
}

/// An actor on the stage: its sprite entity and the strip it plays, with its feet at `feet`.
struct StagedActor {
    entity: Entity,
    cell: Vec2,
    columns: u32,
    scale: f32,
    strip: Vec<(u32, u32)>,
    feet: Vec2,
}

struct SiteFrame {
    kind: u32,
    center: Vec2,
    size: Vec2,
    facing: f32,
    texture: Option<(Handle<Image>, Rect, Vec2)>,
    area: vfx_render::AreaShape,
}

impl Stage {
    fn spawn(world: &mut World) -> Stage {
        let (image, texture) = scene::render_target(world, PREVIEW);
        let camera = world
            .spawn((
                Camera2d,
                RenderTarget::Image(image.clone().into()),
                Camera {
                    order: -50,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                Projection::Orthographic(OrthographicProjection {
                    scale: 1.0 / ZOOM,
                    ..OrthographicProjection::default_2d()
                }),
                Transform::default(),
                RenderLayers::layer(PREVIEW_LAYER),
            ))
            .id();
        let quad = world
            .resource_mut::<Assets<Mesh>>()
            .add(Rectangle::new(1.0, 1.0));
        let blank = world.resource_mut::<Assets<Image>>().add(Image::new_fill(
            bevy::render::render_resource::Extent3d::default(),
            bevy::render::render_resource::TextureDimension::D2,
            &[0, 0, 0, 0],
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        ));
        Stage {
            camera,
            image,
            texture,
            quad,
            blank,
            actors: Vec::new(),
            pieces: Vec::new(),
            shaders: HashMap::new(),
            signature: String::new(),
        }
    }

    fn spawn_piece(
        &self,
        world: &mut World,
        layer: &Layer,
        scenes: &mut scene::AreaScenes,
        files: &ProjectFiles,
    ) -> Option<Entity> {
        match layer {
            Layer::Sprite { sheet, .. } => {
                let image = scene::image_handle(scenes, files, sheet, world);
                Some(
                    world
                        .spawn((
                            Sprite { image, ..default() },
                            Transform::default(),
                            Visibility::Hidden,
                            RenderLayers::layer(PREVIEW_LAYER),
                        ))
                        .id(),
                )
            }
            Layer::Shader { .. } => {
                let material = world
                    .resource_mut::<Assets<VfxMaterial>>()
                    .add(VfxMaterial {
                        image: self.blank.clone(),
                        site: SiteUniform::default(),
                        params: default(),
                        shader: Handle::default(),
                    });
                Some(
                    world
                        .spawn((
                            Mesh2d(self.quad.clone()),
                            MeshMaterial2d(material),
                            Transform::default(),
                            Visibility::Hidden,
                            RenderLayers::layer(PREVIEW_LAYER),
                        ))
                        .id(),
                )
            }
            Layer::Text { .. } => None,
        }
    }

    fn clear_pieces(&mut self, world: &mut World) {
        for piece in self.pieces.drain(..).flatten() {
            world.despawn(piece);
        }
        for actor in self.actors.drain(..) {
            world.despawn(actor.entity);
        }
    }

    fn despawn(mut self, world: &mut World) {
        self.clear_pieces(world);
        world.despawn(self.camera);
        world
            .resource_mut::<EguiUserTextures>()
            .remove_image(&self.image);
    }
}

impl StagedActor {
    fn animate(&self, world: &mut World, now: f64) {
        let total: u32 = self.strip.iter().map(|(_, ms)| ms).sum();
        let position = ((now * 1000.0) as u64 % u64::from(total.max(1))) as u32;
        let mut cursor = 0;
        let frame = self
            .strip
            .iter()
            .find(|(_, ms)| {
                cursor += ms;
                position < cursor
            })
            .or(self.strip.last())
            .map_or(0, |(frame, _)| *frame);
        let min = Vec2::new(
            (frame % self.columns) as f32 * self.cell.x,
            (frame / self.columns) as f32 * self.cell.y,
        );
        let drawn = self.cell * self.scale;
        let mut entity = world.entity_mut(self.entity);
        if let Some(mut sprite) = entity.get_mut::<Sprite>() {
            sprite.rect = Some(Rect::from_corners(min, min + self.cell));
            sprite.custom_size = Some(drawn);
        }
        if let Some(mut transform) = entity.get_mut::<Transform>() {
            transform.translation = Vec3::new(
                self.feet.x,
                self.feet.y + drawn.y / 2.0,
                1.0 - self.feet.y * 0.001,
            );
        }
    }
}

fn spawn_actors(
    world: &mut World,
    target: &Target,
    scenes: &mut scene::AreaScenes,
    files: &ProjectFiles,
) -> Vec<StagedActor> {
    let (placements, action): (Vec<(String, Vec2)>, Action) = match target {
        Target::Actor { model, action } => (vec![(model.clone(), Vec2::ZERO)], *action),
        Target::Crowd { count, action, .. } => {
            let mut models: Vec<String> = files.keys_of("model").map(str::to_owned).collect();
            models.sort();
            let Some(footprint) = target.footprint() else {
                return Vec::new();
            };
            if models.is_empty() {
                return Vec::new();
            }
            let placed = spread(&footprint, *count)
                .into_iter()
                .enumerate()
                .map(|(index, at)| (models[index % models.len()].clone(), at.to_screen()))
                .collect();
            (placed, *action)
        }
        Target::Area { .. } | Target::Screen => return Vec::new(),
    };
    placements
        .into_iter()
        .filter_map(|(model, feet)| spawn_actor(world, &model, action, feet, scenes, files))
        .collect()
}

fn spawn_actor(
    world: &mut World,
    model: &str,
    action: Action,
    feet: Vec2,
    scenes: &mut scene::AreaScenes,
    files: &ProjectFiles,
) -> Option<StagedActor> {
    let row = files.json(&row_file("model", model))?;
    let path = row["sprites"]["image"].as_str()?.to_owned();
    let image = scene::image_handle(scenes, files, &path, world);
    let sheet = world.resource::<Assets<Image>>().get(&image)?.size_f32();
    let cell = Vec2::new(
        row["sprites"]["frame_size"]["width"]
            .as_f64()
            .unwrap_or(16.0) as f32,
        row["sprites"]["frame_size"]["height"]
            .as_f64()
            .unwrap_or(16.0) as f32,
    );
    let actions = &row["sprites"]["actions"];
    let strip = [action.as_ref(), Action::Idle.as_ref()]
        .into_iter()
        .map(|name| &actions[name]["s"])
        .find(|strip| strip.is_array())?;
    let strip: Vec<(u32, u32)> = strip
        .as_array()?
        .iter()
        .filter_map(|frame| {
            Some((
                frame["frame"].as_u64()? as u32,
                frame["ms"].as_u64()? as u32,
            ))
        })
        .collect();
    let scale = row["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|module| module["type"] == "scale")
        .and_then(|module| module["factor"].as_f64())
        .unwrap_or(1.0) as f32;
    let entity = world
        .spawn((
            Sprite { image, ..default() },
            Transform::default(),
            RenderLayers::layer(PREVIEW_LAYER),
        ))
        .id();
    Some(StagedActor {
        entity,
        cell,
        columns: ((sheet.x / cell.x).floor() as u32).max(1),
        scale,
        strip,
        feet,
    })
}

/// `count` points spread evenly over the footprint, the same every time: a sunflower spiral over
/// the footprint's reach, kept where it falls inside, and sampled at even strides.
fn spread(footprint: &Footprint, count: u32) -> Vec<Pos<Tiles>> {
    let reach = footprint.reach().0.max(0.1);
    let golden = PI * (3.0 - 5f32.sqrt());
    let inside: Vec<Pos<Tiles>> = (0..SPREAD_CANDIDATES)
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
    (0..count)
        .map(|k| {
            let stride = (k as f32 + 0.5) * inside.len() as f32 / count as f32;
            inside[(stride as usize).min(inside.len() - 1)]
        })
        .collect()
}

// The screen site's image is the area drawn behind the preview, so a screen-wide shader has
// something to work on, as it has the frame in the game.
fn site_frame(
    world: &World,
    target: &Target,
    actor: Option<&StagedActor>,
    behind: Option<(Handle<Image>, egui::Rect, [u32; 2])>,
) -> Option<SiteFrame> {
    match target {
        Target::Actor { .. } => {
            let sprite = world.get::<Sprite>(actor?.entity)?;
            let image = world.resource::<Assets<Image>>().get(&sprite.image)?;
            let sheet = image.size_f32();
            let rect = sprite
                .rect
                .unwrap_or_else(|| Rect::from_corners(Vec2::ZERO, sheet));
            Some(SiteFrame {
                kind: SITE_ACTOR,
                center: Vec2::ZERO,
                size: rect.size(),
                facing: 0.0,
                texture: Some((
                    sprite.image.clone(),
                    Rect::from_corners(rect.min / sheet, rect.max / sheet),
                    Vec2::ONE / sheet,
                )),
                area: vfx_render::AreaShape::image(rect.size() / TILE.0),
            })
        }
        Target::Area { facing, .. } | Target::Crowd { facing, .. } => {
            let footprint = target.footprint()?;
            let (center, size, area) = vfx_render::area_quad(&footprint);
            Some(SiteFrame {
                kind: SITE_AREA,
                center,
                size,
                facing: -facing.radians().0,
                texture: None,
                area,
            })
        }
        Target::Screen => {
            let size = Vec2::new(PREVIEW[0] as f32, PREVIEW[1] as f32) / ZOOM;
            let texture = behind.map(|(image, uv, alloc)| {
                (
                    image,
                    Rect::from_corners(
                        Vec2::new(uv.min.x, uv.min.y),
                        Vec2::new(uv.max.x, uv.max.y),
                    ),
                    Vec2::ONE / Vec2::new(alloc[0] as f32, alloc[1] as f32),
                )
            });
            Some(SiteFrame {
                kind: SITE_SCREEN,
                center: Vec2::ZERO,
                size,
                facing: 0.0,
                texture,
                area: vfx_render::AreaShape::image(size / TILE.0),
            })
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one layer's whole state, read once"
)]
fn show_shader_piece(
    world: &mut World,
    stage: &mut Stage,
    piece: Entity,
    frame: &SiteFrame,
    shader: &str,
    fills: &BTreeMap<String, ParamFill>,
    given: &VfxParams,
    local: Seconds,
    duration: Seconds,
    elapsed: f32,
    drafts: &HashMap<String, String>,
    files: &ProjectFiles,
    problems: &mut Vec<String>,
) -> bool {
    let Some(row) = files.json(&row_file(SHADER_TABLE, shader)) else {
        problems.push(format!("no shader row {shader}"));
        return false;
    };
    let Ok(def) = serde_json::from_value::<VfxShaderDef>((*row).clone()) else {
        problems.push(format!("{SHADER_TABLE}/{shader} does not parse"));
        return false;
    };
    let source = drafts
        .get(shader)
        .cloned()
        .or_else(|| files.text(&shader_file(shader)))
        .unwrap_or_default();
    if stage.shaders.len() > KEPT_SHADERS {
        stage.shaders.clear();
    }
    let verdict = stage
        .shaders
        .entry(def.compose(&source))
        .or_insert_with_key(|composed| {
            def.check_source(&source)?;
            let path = format!(
                "vfx_preview/{shader}-{}.wgsl",
                crate::project::hash(composed.as_bytes())
            );
            Ok(world
                .resource_mut::<Assets<Shader>>()
                .add(Shader::from_wgsl(composed.clone(), path)))
        });
    let handle = match verdict {
        Ok(handle) => handle.clone(),
        Err(error) => {
            problems.push(format!("{shader}: {}", error.message));
            return false;
        }
    };
    let Some(material) = world
        .get::<MeshMaterial2d<VfxMaterial>>(piece)
        .map(|material| material.0.clone())
    else {
        return false;
    };
    let overscan = if frame.kind == SITE_SCREEN {
        Vec2::ONE
    } else {
        quad_overscan(frame.size)
    };
    let (image, rect, pixel) = frame.texture.clone().unwrap_or((
        stage.blank.clone(),
        Rect::from_corners(Vec2::ZERO, Vec2::ONE),
        Vec2::ONE,
    ));
    let site = SiteUniform {
        rect: Vec4::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y),
        size: frame.size,
        pixel,
        kind: frame.kind,
        facing: frame.facing,
        progress: (local.0 / duration.0.max(0.001)).clamp(0.0, 1.0),
        time: elapsed,
        overscan,
        shape: frame.area.shape,
        origin: frame.area.origin,
        extent: frame.area.extent,
        tiles: frame.area.tiles,
        ..SiteUniform::default()
    };
    let params = vfx_render::fill_params(&def, fills, given);
    if let Some(mut shown) = world
        .resource_mut::<Assets<VfxMaterial>>()
        .get_mut(&material)
    {
        shown.image = image;
        shown.site = site;
        shown.params = params;
        shown.shader = handle;
    }
    let depth = if frame.kind == SITE_AREA {
        GROUND_DEPTH
    } else {
        OVER_ACTORS_DEPTH
    };
    if let Some(mut transform) = world.get_mut::<Transform>(piece) {
        transform.translation = Vec3::new(frame.center.x, frame.center.y, depth);
        transform.scale = (frame.size * overscan).extend(1.0);
    }
    true
}

fn set_visible(world: &mut World, entity: Entity, shown: bool) {
    if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

fn set_alpha(world: &mut World, entity: Entity, alpha: f32) {
    if let Some(mut sprite) = world.get_mut::<Sprite>(entity)
        && sprite.color.alpha() != alpha
    {
        sprite.color.set_alpha(alpha);
    }
}

fn layers_signature(layers: &[Layer]) -> Vec<Value> {
    layers
        .iter()
        .map(|layer| serde_json::to_value(layer).unwrap_or(Value::Null))
        .collect()
}

fn ease(progress: f32) -> f32 {
    1.0 - (1.0 - progress) * (1.0 - progress)
}

fn one() -> f32 {
    1.0
}

fn text_size() -> f32 {
    14.0
}

fn text_rise() -> Tiles {
    Tiles(1.0)
}
