use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::scene::EntityScene;
use bevy::ui::Val2;
use bevy::window::PrimaryWindow;
use ui::tokens::typography;

use super::{DebugMode, outline as outline_rect};
use crate::core::assets::AssetService;
use crate::core::audio::playback::Listener;
use crate::core::audio::soundscape::{SoundscapeShape, SoundscapeZone};
use crate::core::math::{Offset, Rect};
use crate::core::render::cursor_tile;
use crate::core::render::screen::ToScreen;
use crate::core::tiling::Tiles;
use crate::systems::area::{self, AreaTag};
use crate::systems::player::session::Viewpoint;

const SAMPLES_PER_TILE: f32 = 4.0;
const GRADIENT_OPACITY: f32 = 0.2;
const GRADIENT_Z: f32 = 99.0;
const CURSOR_GAP: f32 = 24.0;

#[derive(Resource, Default)]
pub(super) struct ShownSoundscape(Option<&'static [SoundscapeZone]>);

#[derive(Component, Default, Clone)]
pub(super) struct ZoneOverlay;

#[derive(Component, Default, Clone)]
pub(super) struct ZoneReadout;

#[derive(Component, Default, Clone)]
pub(super) struct ZoneLabel(usize);

#[derive(Component, Default, Clone)]
pub(super) struct ZoneHearing(usize);

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    mode: Res<DebugMode>,
    viewpoint: Res<Viewpoint>,
    areas: Query<&AreaTag>,
    service: Res<AssetService>,
    mut shown: ResMut<ShownSoundscape>,
    overlays: Query<Entity, With<ZoneOverlay>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let wanted = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .filter(|_| *mode == DebugMode::Soundscapes)
        .map(|tag| {
            service
                .resolve(tag.area.get().map, area::build_area)
                .soundscape
                .as_slice()
        });
    if shown.0.map(<[_]>::as_ptr) == wanted.map(<[_]>::as_ptr) {
        return;
    }
    for overlay in &overlays {
        commands.entity(overlay).despawn();
    }
    shown.0 = wanted;
    let Some(zones) = wanted else {
        return;
    };
    for (index, zone) in zones.iter().enumerate() {
        let audible = audible_bounds(zone);
        commands.spawn((
            ZoneOverlay,
            Sprite {
                image: images.add(gradient(zone, audible)),
                color: zone_color(index).with_alpha(GRADIENT_OPACITY),
                custom_size: Some(audible.size.to_screen()),
                ..default()
            },
            Transform::from_translation(audible.center().to_screen().extend(GRADIENT_Z)),
        ));
    }
    commands.spawn_scene(readout(zones));
}

pub(super) fn read_out_hovered(world: &mut World) {
    let Some(zones) = world.resource::<ShownSoundscape>().0 else {
        return;
    };
    let hovered_tile = cursor_tile(world);
    let hovered: Vec<bool> = zones
        .iter()
        .map(|zone| hovered_tile.is_some_and(|at| zone.heard_from(at).is_some()))
        .collect();
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .and_then(Window::cursor_position)
        .filter(|_| hovered.contains(&true));
    let mut readouts = world.query_filtered::<(&mut Node, &mut Visibility), With<ZoneReadout>>();
    for (mut node, mut visibility) in readouts.iter_mut(world) {
        let shown = match cursor {
            Some(cursor) => {
                let left = Val::Px(cursor.x.round());
                let top = Val::Px((cursor.y + CURSOR_GAP).round());
                if node.left != left || node.top != top {
                    node.left = left;
                    node.top = top;
                }
                Visibility::Inherited
            }
            None => Visibility::Hidden,
        };
        if *visibility != shown {
            *visibility = shown;
        }
    }
    let mut labels = world.query::<(&ZoneLabel, &mut Node)>();
    for (label, mut node) in labels.iter_mut(world) {
        let display = if hovered[label.0] {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let listener = world.resource::<Listener>().0;
    let mut hearings = world.query::<(&ZoneHearing, &mut Text)>();
    for (hearing, mut text) in hearings.iter_mut(world) {
        let reading = match listener.and_then(|at| zones[hearing.0].heard_from(at)) {
            Some(heard) => format!("you hear it at {:.0}%", heard.proximity * 100.0),
            None => "out of your earshot".to_owned(),
        };
        if text.0 != reading {
            text.0 = reading;
        }
    }
}

pub(super) fn hide(
    mut shown: ResMut<ShownSoundscape>,
    overlays: Query<Entity, With<ZoneOverlay>>,
    mut commands: Commands,
) {
    shown.0 = None;
    for overlay in &overlays {
        commands.entity(overlay).despawn();
    }
}

pub(super) fn outline(gizmos: &mut Gizmos, zones: &[SoundscapeZone]) {
    for (index, zone) in zones.iter().enumerate() {
        let color = zone_color(index);
        match zone.shape {
            SoundscapeShape::Rect(bounds) => outline_rect(gizmos, bounds, color),
            SoundscapeShape::Ellipse(bounds) => {
                gizmos.ellipse_2d(
                    Isometry2d::from_translation(bounds.center().to_screen()),
                    bounds.size.to_screen() / 2.0,
                    color,
                );
            }
        }
    }
}

// Golden-angle hues keep every zone's color apart from the zones listed next to it.
fn zone_color(index: usize) -> Color {
    Color::hsl((index as f32 * 137.508) % 360.0, 0.8, 0.6)
}

fn audible_bounds(zone: &SoundscapeZone) -> Rect<Tiles> {
    let reach = zone.reach().0;
    zone.shape.bounds().inflate(reach, reach)
}

fn gradient(zone: &SoundscapeZone, audible: Rect<Tiles>) -> Image {
    let width = (audible.width() * SAMPLES_PER_TILE).ceil().max(1.0) as u32;
    let height = (audible.height() * SAMPLES_PER_TILE).ceil().max(1.0) as u32;
    let data = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let at = audible.origin
                + Offset::new(
                    (x as f32 + 0.5) / SAMPLES_PER_TILE,
                    (y as f32 + 0.5) / SAMPLES_PER_TILE,
                );
            let loudness = zone.heard_from(at).map_or(0.0, |heard| heard.proximity);
            [255, 255, 255, (loudness * 255.0).round() as u8]
        })
        .collect();
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

fn readout(zones: &[SoundscapeZone]) -> impl Scene + use<> {
    let labels: Vec<Box<dyn Scene>> = zones
        .iter()
        .enumerate()
        .map(|(index, zone)| -> Box<dyn Scene> { Box::new(label(index, zone)) })
        .collect();
    bsn! {
        ZoneOverlay
        ZoneReadout
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            padding: {UiRect::all(Val::Px(8.0))},
        }
        UiTransform { translation: {Val2::percent(-50.0, 0.0)} }
        BackgroundColor({Color::BLACK.with_alpha(0.75)})
        GlobalZIndex({ui::tokens::layer::ANCHORED})
        Pickable::IGNORE
        Children [ {labels} ]
    }
}

fn label(index: usize, zone: &SoundscapeZone) -> impl Scene + use<> {
    let name = (!zone.name.is_empty()).then(|| {
        EntityScene(ui::styled_text(
            zone.name.clone(),
            zone_color(index),
            typography::LABEL,
        ))
    });
    let channels = zone
        .channels
        .iter()
        .map(|(channel, layer)| {
            format!(
                "channel {}: {} ({:?}, volume {}, pan {})",
                channel.0,
                layer.src,
                channel.category(),
                layer.volume,
                layer.pan
            )
        })
        .chain([format!("heard {} tiles past its edge", zone.reach().0)])
        .collect::<Vec<_>>()
        .join("\n");
    bsn! {
        ZoneLabel({index})
        Node { flex_direction: FlexDirection::Column, display: Display::None }
        Pickable::IGNORE
        Children [
            {name},
            {EntityScene(ui::styled_text(channels, Color::WHITE, typography::CAPTION))},
            (
                {ui::styled_text("", Color::WHITE.with_alpha(0.7), typography::CAPTION)}
                ZoneHearing({index})
            ),
        ]
    }
}
