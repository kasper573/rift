//! Globes on the stage: a ball of glass holding a liquid, lit by the stage's lamp and seen
//! straight from its side, so its liquid's face is a level line. The interface shows the lit
//! picture where the globe sits.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::camera::SubCameraView;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPostUpdateSet;
use ui::egui::{self, Color32, Pos2, Rect};

use super::{
    DUE_AFTER_A_CHANGE, Lamp, Lens, Picture, PictureLook, Scene, on_stage, rendered, view_about,
};
use crate::core::card::CardHandling;

/// A globe as it is stood on the stage this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Globe {
    /// What tells the globe from every other on the stage.
    pub key: egui::Id,
    pub center: Pos2,
    /// How far from its middle its glass reaches.
    pub radius: f32,
    /// How much of it its liquid fills, from none of it to all.
    pub full: f32,
    pub liquid: Color32,
}

pub(super) struct GlobeStagePlugin;

impl Plugin for GlobeStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobeStage>().add_systems(
            PostUpdate,
            strike
                .after(EguiPostUpdateSet::EndPass)
                .before(TransformSystems::Propagate),
        );
    }
}

/// Stands a globe on the stage and paints its lit picture with `painter`. Its liquid rises and
/// falls to how full the globe is rather than jumping there.
///
/// The renderer takes a few frames to draw a globe it has not drawn before. Until it has,
/// nothing is painted: `false` then, for whoever shows the globe to paint what stands in for it.
pub fn show(world: &mut World, painter: &egui::Painter, globe: &Globe) -> bool {
    let gui = painter.ctx().clone();
    let handling = *world.resource::<CardHandling>();
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .map(|window| window.resolution.physical_size());
    let Ok(window) = window else {
        return false;
    };
    let screen = gui.content_rect();
    let pixels = gui.pixels_per_point();
    let frame = world.resource::<Time>().delta_secs();
    let across = 2.0 * globe.radius * pixels;
    let side = (across.ceil() as u32 + 2 * MARGIN).next_multiple_of(2);
    let (view, corner) = view_about(globe.center, side, pixels, window);
    let at = on_stage(globe.center, screen);
    let scene = Scene {
        eye: handling.eye * screen.height(),
        lamp: key_light(Lamp::over(world, &handling.lamp, screen), at),
        height: screen.height(),
    };

    let mut stood = world
        .resource_mut::<GlobeStage>()
        .globes
        .remove(&globe.key)
        .unwrap_or_default();
    stood.kept = true;
    let full = globe.full.clamp(0.0, 1.0);
    let level = match stood.shot {
        Some(shot) => {
            let nearer = shot.level + (full - shot.level) * (1.0 - (-GLIDE * frame).exp());
            if (full - nearer).abs() * across < 0.5 {
                full
            } else {
                nearer
            }
        }
        None => full,
    };
    let shot = Shot {
        view,
        scene,
        at,
        radius: globe.radius,
        level,
        liquid: globe.liquid,
    };
    let reshot = stood.shot != Some(shot);
    if reshot {
        let look = PictureLook {
            lens: Lens::StraightOn,
            smoothing: Msaa::Sample4,
            even: EVEN,
        };
        stood.picture.frame(world, &view, &scene, look);
        stood.stand(world, &shot);
        stood.shot = Some(shot);
    }
    let was_drawn = stood.drawn;
    stood.drawn = stood.drawn || rendered(world, &mut stood.at_hand);
    stood.due = if reshot || stood.drawn != was_drawn {
        DUE_AFTER_A_CHANGE
    } else {
        stood.due.saturating_sub(1)
    };
    stood.picture.take(world, stood.due > 0 || !stood.drawn);
    let lit = stood.picture.texture().filter(|_| stood.drawn);
    if let Some(texture) = lit {
        let lies = Rect::from_min_size(
            Pos2::new(corner.x, corner.y) / pixels,
            egui::Vec2::splat(side as f32 / pixels),
        );
        painter.add(egui::Shape::image(
            texture,
            lies,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        ));
    }
    if level != full {
        gui.request_repaint();
    }
    world
        .resource_mut::<GlobeStage>()
        .globes
        .insert(globe.key, stood);
    lit.is_some()
}

/// The globes on the stage, each by its key, and what every one of them is made of.
#[derive(Resource, Default)]
struct GlobeStage {
    globes: HashMap<egui::Id, Stood>,
    ball: Option<Handle<Mesh>>,
    glass: Option<Handle<StandardMaterial>>,
}

/// A globe on the stage: its picture, its glass and its liquid.
#[derive(Default)]
struct Stood {
    kept: bool,
    /// How many frames the renderer had drawn when the globe was first stood, and whether it
    /// has drawn the globe since.
    at_hand: Option<u64>,
    drawn: bool,
    /// What the globe's picture was last taken of, and how many more passes it is taken anew: a
    /// globe that stands as it stood, as full as it was, keeps the picture it has.
    shot: Option<Shot>,
    due: u8,
    picture: Picture,
    parts: Option<Parts>,
}

struct Parts {
    glass: Entity,
    liquid: Entity,
    filling: Handle<Mesh>,
    colour: Handle<StandardMaterial>,
}

/// Everything a globe's picture is taken of: the part of the window it covers, the scene with
/// its lamp, where the globe stands in it and how big, and how high its liquid stands.
#[derive(Clone, Copy, PartialEq)]
struct Shot {
    view: SubCameraView,
    scene: Scene,
    at: Vec3,
    radius: f32,
    level: f32,
    liquid: Color32,
}

impl Stood {
    /// Stands the globe's glass and liquid where `shot` has them, the liquid as high as it
    /// stands there.
    fn stand(&mut self, world: &mut World, shot: &Shot) {
        let layer = self.picture.layer(world);
        let glass_at = Transform::from_translation(shot.at).with_scale(Vec3::splat(shot.radius));
        let liquid_at =
            Transform::from_translation(shot.at).with_scale(Vec3::splat(shot.radius * LIQUID));
        let filling = filling(shot.level);
        let colour = liquid(shot.liquid);
        let was = self.shot;
        match &self.parts {
            Some(parts) => {
                if was.is_none_or(|was| was.level != shot.level) {
                    world
                        .resource_mut::<Assets<Mesh>>()
                        .insert(parts.filling.id(), filling)
                        .ok();
                }
                if was.is_none_or(|was| was.liquid != shot.liquid) {
                    world
                        .resource_mut::<Assets<StandardMaterial>>()
                        .insert(parts.colour.id(), colour)
                        .ok();
                }
                world
                    .entity_mut(parts.glass)
                    .insert((glass_at, GlobalTransform::from(glass_at)));
                world
                    .entity_mut(parts.liquid)
                    .insert((liquid_at, GlobalTransform::from(liquid_at)));
            }
            None => {
                let (ball, glass) = made_of(world);
                let filling = world.resource_mut::<Assets<Mesh>>().add(filling);
                let colour = world.resource_mut::<Assets<StandardMaterial>>().add(colour);
                let part = (
                    RenderLayers::layer(layer),
                    NoFrustumCulling,
                    NotShadowCaster,
                );
                let glass = world
                    .spawn((
                        Mesh3d(ball),
                        MeshMaterial3d(glass),
                        glass_at,
                        GlobalTransform::from(glass_at),
                        part.clone(),
                    ))
                    .id();
                let liquid = world
                    .spawn((
                        Mesh3d(filling.clone()),
                        MeshMaterial3d(colour.clone()),
                        liquid_at,
                        GlobalTransform::from(liquid_at),
                        part,
                    ))
                    .id();
                self.parts = Some(Parts {
                    glass,
                    liquid,
                    filling,
                    colour,
                });
            }
        }
    }
}

/// The ball every globe's glass is, and the glass it is of, made the first time one is stood.
fn made_of(world: &mut World) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    let made = world.resource::<GlobeStage>();
    if let (Some(ball), Some(glass)) = (&made.ball, &made.glass) {
        return (ball.clone(), glass.clone());
    }
    let ball = world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(1.0).mesh().uv(AROUND as u32, 2 * RINGS as u32));
    // Clear glass adds nothing of its own but what it mirrors: the lamp's glint and its sheen.
    let glass = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::NONE,
            alpha_mode: AlphaMode::Premultiplied,
            perceptual_roughness: GLASS_ROUGH,
            reflectance: GLASS_SHEEN,
            clearcoat: 1.0,
            clearcoat_perceptual_roughness: GLASS_COAT_ROUGH,
            fog_enabled: false,
            ..default()
        });
    let mut made = world.resource_mut::<GlobeStage>();
    made.ball = Some(ball.clone());
    made.glass = Some(glass.clone());
    (ball, glass)
}

fn liquid(colour: Color32) -> StandardMaterial {
    let [red, green, blue, _] = colour.to_array();
    StandardMaterial {
        base_color: Color::srgb_u8(red, green, blue),
        perceptual_roughness: LIQUID_ROUGH,
        reflectance: LIQUID_SHEEN,
        fog_enabled: false,
        ..default()
    }
}

/// A ball of radius one filled from its foot to `level` of its height: the part of the ball
/// under the liquid's face, and that face, flat and facing up.
fn filling(level: f32) -> Mesh {
    let top = (2.0 * level - 1.0).clamp(-1.0, 1.0);
    let highest = top.asin();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for ring in 0..=RINGS {
        let latitude = -PI / 2.0 + (highest + PI / 2.0) * ring as f32 / RINGS as f32;
        let (height, reach) = latitude.sin_cos();
        for step in 0..=AROUND {
            let (across, toward) = (TAU * step as f32 / AROUND as f32).sin_cos();
            let on_ball = [reach * across, height, reach * toward];
            positions.push(on_ball);
            normals.push(on_ball);
        }
    }
    let row = AROUND as u32 + 1;
    for ring in 0..RINGS as u32 {
        for step in 0..AROUND as u32 {
            let (below, above) = (ring * row + step, (ring + 1) * row + step);
            indices.extend([below, below + 1, above, above, below + 1, above + 1]);
        }
    }
    let middle = positions.len() as u32;
    let reach = highest.cos();
    positions.push([0.0, top, 0.0]);
    normals.push([0.0, 1.0, 0.0]);
    for step in 0..=AROUND {
        let (across, toward) = (TAU * step as f32 / AROUND as f32).sin_cos();
        positions.push([reach * across, top, reach * toward]);
        normals.push([0.0, 1.0, 0.0]);
    }
    for step in 0..AROUND as u32 {
        indices.extend([middle, middle + 1 + step, middle + 2 + step]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// The stage's lamp as a globe at `at` is lit by it: from where it hangs, in its colour, as
/// strongly as it shines, lighting the side of the globe turned to it to `KEY` of its colour
/// however far off the lamp hangs. A card takes the pool of light under the lamp as it falls; a
/// ball that took as little would show no roundness.
fn key_light(lamp: Lamp, at: Vec3) -> Lamp {
    let off = lamp.at.distance_squared(at);
    let under = lamp.at.z * lamp.at.z;
    Lamp {
        share: if under > 0.0 { KEY * off / under } else { 0.0 },
        ..lamp
    }
}

/// Takes every globe off the stage that was not stood on it this pass.
fn strike(world: &mut World) {
    let gone: Vec<Stood> = {
        let mut stage = world.resource_mut::<GlobeStage>();
        let struck: Vec<egui::Id> = stage
            .globes
            .iter_mut()
            .filter_map(|(key, stood)| (!std::mem::take(&mut stood.kept)).then_some(*key))
            .collect();
        struck
            .into_iter()
            .filter_map(|key| stage.globes.remove(&key))
            .collect()
    };
    for mut stood in gone {
        if let Some(parts) = stood.parts.take() {
            world.despawn(parts.glass);
            world.despawn(parts.liquid);
        }
        stood.picture.forget(world);
    }
}

/// How many pixels wider than its glass a globe's picture is on every side, for its smoothed
/// edge and for where the picture's whole pixels put its middle.
const MARGIN: u32 = 2;
/// How much of its glass's reach a globe's liquid fills: the rest is the glass's thickness.
const LIQUID: f32 = 0.94;
/// How quickly the liquid nears the height it is to stand at, per second.
const GLIDE: f32 = 6.0;
/// How much of a globe's light falls on it evenly, and how much of it is the lamp's on the side
/// turned to the lamp when the lamp shines its fullest.
const EVEN: f32 = 0.5;
const KEY: f32 = 0.8;
const GLASS_ROUGH: f32 = 0.3;
const GLASS_COAT_ROUGH: f32 = 0.06;
/// How much light glass and liquid reflect head-on, as Bevy's surfaces measure it.
const GLASS_SHEEN: f32 = 0.5;
const LIQUID_SHEEN: f32 = 0.3;
const LIQUID_ROUGH: f32 = 0.45;
/// How many steps round a globe's liquid is made of, and how many from its foot to its face.
const AROUND: usize = 48;
const RINGS: usize = 16;
