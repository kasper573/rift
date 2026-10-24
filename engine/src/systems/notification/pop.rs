use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};
use ui::face::Face;
use ui::raster::raster;

use super::{Notification, NotificationKind, NotificationSent};
use crate::core::content::{Content, Fixture};
use crate::core::math::{Pos, Rng};
use crate::core::render::camera::tile_to_window;
use crate::core::render::transition::WorldViewSystems;
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::core::vfx::render::{
    ParamsUniform, Run, SiteFrame, SiteTexture, compiled_shader, hide, place_in_depth,
    present_shader, screen_layer, shader_piece,
};
use crate::core::vfx::{VfxParams, VfxShaderId};
use crate::systems::actor::Hitbox;
use crate::systems::movement::RenderPosition;
use crate::systems::screen::ClientScreen;
use crate::systems::text::LineText;
use crate::systems::vfx::fixture::VfxShaderFixture;
use crate::systems::visibility;

const LIFE: Seconds = Seconds(1.0);
const FACE: Face = Face::SmallCapsBold;
const SIZE: f32 = 35.0;
const PAD: u32 = 6;
const SPREAD: f32 = 0.6;
const SPEED: (f32, f32) = (210.0, 270.0);
const FALL: f32 = 600.0;
const SCATTER: f32 = 10.0;
const PUNCH: f32 = 0.6;
const PUNCH_SECS: f32 = 0.16;
const BODY_SHARE: f32 = 0.35;
const Z: f32 = 3.0;
const TEXTS_KEPT: usize = 256;

/// Bursts `text` out of `actor` for everyone who sees it, drawn by `shader` with `params`, or by
/// the game's own pop shader when no shader is given. A pop is feedback of the moment: it is not
/// kept in anyone's history.
pub fn pop(
    world: &mut World,
    actor: Entity,
    text: impl Into<String>,
    shader: Option<VfxShaderId>,
    params: VfxParams,
) {
    let sent = NotificationSent {
        notification: Notification::new(
            NotificationKind::Pop { shader, params },
            LineText::plain(text.into()),
        ),
        actor: Some(actor),
    };
    for client in visibility::seen_by(world, actor) {
        visibility::send_to(world, client, sent.clone());
    }
}

/// A pop drawn by the game's own shader for the kind of moment it marks.
pub fn pop_as(world: &mut World, actor: Entity, text: impl Into<String>, look: VfxShaderFixture) {
    let shader = look.id(world.resource::<Content>());
    pop(world, actor, text, Some(shader), VfxParams::default());
}

pub struct PopPlugin;

impl Plugin for PopPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pops>()
            .init_resource::<PopTexts>()
            .add_systems(
                PostUpdate,
                burst
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn show(world: &mut World, sent: NotificationSent) {
    let NotificationKind::Pop { shader, params } = sent.notification.kind else {
        return;
    };
    let Some(actor) = sent.actor.filter(|actor| world.get_entity(*actor).is_ok()) else {
        return;
    };
    let shader = shader.unwrap_or_else(|| VfxShaderFixture::Pop.id(world.resource::<Content>()));
    let text = sent.notification.text.plain_words();
    let texture = text_texture(world, &text);
    let piece = shader_piece(world);
    let born = uptime(world);
    let mut pops = world.resource_mut::<Pops>();
    let turn = (pops.rng.rand_float() * 2.0 - 1.0) * SPREAD;
    let speed = SPEED.0 + pops.rng.rand_float() * (SPEED.1 - SPEED.0);
    let scatter = Vec2::new(pops.rng.rand_float(), pops.rng.rand_float()) * 2.0 - Vec2::ONE;
    pops.live.push(LivePop {
        piece,
        actor,
        texture,
        shader,
        params,
        born,
        launch: Vec2::new(turn.sin(), turn.cos()) * speed,
        scatter: scatter * SCATTER,
    });
}

/// A pop in the air: the actor it bursts out of and its words.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShownPop {
    pub actor: Entity,
    pub text: String,
}

pub fn shown(world: &World) -> Vec<ShownPop> {
    world
        .resource::<Pops>()
        .live
        .iter()
        .map(|pop| ShownPop {
            actor: pop.actor,
            text: pop.texture.text.clone(),
        })
        .collect()
}

#[derive(Resource)]
struct Pops {
    live: Vec<LivePop>,
    rng: Rng,
}

impl Default for Pops {
    fn default() -> Pops {
        Pops {
            live: Vec::new(),
            rng: Rng::from_entropy(),
        }
    }
}

struct LivePop {
    piece: Entity,
    actor: Entity,
    texture: TextTexture,
    shader: VfxShaderId,
    params: VfxParams,
    born: Seconds,
    launch: Vec2,
    scatter: Vec2,
}

#[derive(Clone)]
struct TextTexture {
    text: String,
    image: Handle<Image>,
    size: Vec2,
}

#[derive(Resource, Default)]
struct PopTexts(HashMap<String, TextTexture>);

fn burst(world: &mut World) {
    let now = uptime(world);
    let live = std::mem::take(&mut world.resource_mut::<Pops>().live);
    let window = window_size(world);
    let layer = screen_layer();
    let mut kept = Vec::with_capacity(live.len());
    for pop in live {
        let age = Seconds(now.0 - pop.born.0);
        if age.0 >= LIFE.0 || world.get_entity(pop.actor).is_err() {
            world.despawn(pop.piece);
            continue;
        }
        let drawn = window
            .zip(origin(world, pop.actor))
            .zip(compiled_shader(world, pop.shader));
        let Some(((window, origin), compiled)) = drawn else {
            hide(world, pop.piece);
            kept.push(pop);
            continue;
        };
        let t = age.0;
        let flight = Vec2::new(pop.launch.x * t, -(pop.launch.y * t - 0.5 * FALL * t * t));
        let punch = 1.0 + PUNCH * (1.0 - (t / PUNCH_SECS).clamp(0.0, 1.0)).powi(2);
        let size = pop.texture.size * punch;
        let at = origin + pop.scatter + flight;
        let corner = (at - size / 2.0).round();
        let center = Vec2::new(
            corner.x + size.x / 2.0 - window.x / 2.0,
            window.y / 2.0 - (corner.y + size.y / 2.0),
        );
        let frame = SiteFrame::text(
            center,
            size,
            SiteTexture {
                image: pop.texture.image.clone(),
                rect: Rect::from_corners(Vec2::ZERO, Vec2::ONE),
                pixel: Vec2::ONE / pop.texture.size,
            },
        );
        let content = world.resource::<Content>().clone();
        let packed = ParamsUniform::pack(pop.shader.get(&content), |name| pop.params.get(name));
        let run = Run {
            progress: t / LIFE.0,
            time: age,
        };
        present_shader(world, pop.piece, &frame, compiled, packed, run);
        place_in_depth(world, pop.piece, &frame, Z, &layer);
        kept.push(pop);
    }
    world.resource_mut::<Pops>().live = kept;
}

fn forget(world: &mut World) {
    let live = std::mem::take(&mut world.resource_mut::<Pops>().live);
    for pop in live {
        world.despawn(pop.piece);
    }
}

/// Where a pop bursts from, in window pixels: low on the actor's body, toward its feet.
fn origin(world: &mut World, actor: Entity) -> Option<Vec2> {
    let at = world.get::<RenderPosition>(actor)?.0;
    let height = world.get::<Hitbox>(actor)?.0.height.0;
    let low = Pos::<Meters>::new(at.x, at.y + 0.5 - height * BODY_SHARE);
    tile_to_window(world, low)
}

fn text_texture(world: &mut World, text: &str) -> TextTexture {
    if let Some(known) = world.resource::<PopTexts>().0.get(text) {
        return known.clone();
    }
    let drawn = raster(FACE, SIZE, text, PAD);
    let pixels = drawn
        .coverage
        .iter()
        .flat_map(|&cover| [255, 255, 255, cover])
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: drawn.width.max(1),
            height: drawn.height.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    let texture = TextTexture {
        text: text.to_owned(),
        image: world.resource_mut::<Assets<Image>>().add(image),
        size: Vec2::new(drawn.width as f32, drawn.height as f32),
    };
    let mut texts = world.resource_mut::<PopTexts>();
    if texts.0.len() >= TEXTS_KEPT {
        texts.0.clear();
    }
    texts.0.insert(text.to_owned(), texture.clone());
    texture
}

fn window_size(world: &mut World) -> Option<Vec2> {
    let window = world
        .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>()
        .single(world)
        .ok()?;
    Some(Vec2::new(
        window.resolution.width(),
        window.resolution.height(),
    ))
}

fn uptime(world: &World) -> Seconds {
    Seconds(world.resource::<Time>().elapsed_secs())
}
