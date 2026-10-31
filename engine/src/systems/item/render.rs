//! Items as they lie dropped: each as the prop it looks like, hopping from whoever dropped it to
//! where it lands, glowing on the ground under it and counted over it when there is more than one.

use bevy::prelude::*;
use ui::egui;
use ui::motion;

use crate::core::audio::playback::{PlaySfx, SfxId, SfxPlace};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::decal::{DecalLook, GroundDecal};
use crate::core::render::{ground_height, window_point, world_point};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::item::{DroppedItem, ItemConsumed, ItemsDropped};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::screen::ClientScreen;
use ui::face::TextRole;

const DROP_STAGGER: Seconds = Seconds(0.06);
const DROP_DURATION: Seconds = Seconds(0.5);
const DROP_HOP: Meters = Meters(0.5);
/// How fast a dropped item turns on the spot, so it catches the eye.
const SPIN: f32 = 0.8;
const GLOW: Meters = Meters(0.9);
const GLOW_COLOR: Color = Color::srgba(1.0, 0.9, 0.5, 0.55);
const COUNT_HEIGHT: Meters = Meters(0.7);

pub struct ItemRenderPlugin;

impl Plugin for ItemRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(dress_drop)
            .add_systems(
                Update,
                (
                    use_sounds,
                    (start_drops, place_drops, animate_drops, tidy_glows)
                        .chain()
                        .in_set(WorldViewSystems),
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                count_drops
                    .in_set(gui::Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

#[derive(Component)]
struct DropAnim {
    from: Pos<Meters>,
    to: Pos<Meters>,
    delay: Seconds,
    elapsed: Seconds,
    drop_sfx: SfxId,
}

/// The glow on the ground under a dropped item, kept beside the drop so the drop's own transform
/// leaves it lying flat.
#[derive(Component)]
struct DropGlow(Entity);

fn use_sounds(
    content: Res<Content>,
    mut consumed: MessageReader<ItemConsumed>,
    positions: Query<&RenderPosition>,
    mut play: MessageWriter<PlaySfx>,
) {
    for consumed in consumed.read() {
        let Some(id) = consumed.item.get(&content).sfx.on_use else {
            continue;
        };
        let Ok(position) = positions.get(consumed.actor) else {
            continue;
        };
        play.write(PlaySfx {
            id,
            place: SfxPlace::World(position.0),
        });
    }
}

fn dress_drop(
    add: On<Add<DroppedItem>>,
    drops: Query<(&DroppedItem, &Position)>,
    content: Res<Content>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok((dropped, position)) = drops.get(add.entity) else {
        return;
    };
    let look = dropped.item.get(&content).look.get(&content);
    let scene = crate::core::assets::model_scene(&assets, look.model.path());
    commands.entity(add.entity).insert((
        WorldAssetRoot(scene),
        Transform::default(),
        Visibility::default(),
    ));
    commands.spawn((
        DropGlow(add.entity),
        GroundDecal {
            at: position.pos,
            size: GLOW,
            yaw: default(),
            look: DecalLook::Glow { color: GLOW_COLOR },
        },
    ));
}

fn start_drops(
    mut dropped: MessageReader<ItemsDropped>,
    content: Res<Content>,
    drops: Query<(&DroppedItem, &Position)>,
    mut commands: Commands,
) {
    for dropped in dropped.read() {
        for (index, &entity) in dropped.items.iter().enumerate() {
            let Ok((item, position)) = drops.get(entity) else {
                continue;
            };
            commands.entity(entity).try_insert(DropAnim {
                from: dropped.from,
                to: position.pos,
                delay: DROP_STAGGER * index as f32,
                elapsed: Seconds(0.0),
                drop_sfx: item.item.get(&content).sfx.drop,
            });
        }
    }
}

fn place_drops(world: &mut World) {
    let spin = world.resource::<Time>().elapsed_secs() * SPIN;
    let drops: Vec<(Entity, Pos<Meters>)> = world
        .query_filtered::<(Entity, &Position), (With<DroppedItem>, Without<DropAnim>)>()
        .iter(world)
        .map(|(entity, position)| (entity, position.pos))
        .collect();
    for (entity, at) in drops {
        let ground = ground_height(world, at);
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = world_point(at, ground);
            transform.rotation = Quat::from_rotation_y(spin);
        }
    }
    let glows: Vec<(Entity, Entity)> = world
        .query::<(Entity, &DropGlow)>()
        .iter(world)
        .map(|(glow, of)| (glow, of.0))
        .collect();
    for (glow, of) in glows {
        let Some(at) = world.get::<Position>(of).map(|position| position.pos) else {
            continue;
        };
        if let Some(mut decal) = world.get_mut::<GroundDecal>(glow)
            && decal.at != at
        {
            decal.at = at;
        }
    }
}

fn animate_drops(world: &mut World) {
    let dt = Seconds(world.resource::<Time>().delta_secs());
    let flying: Vec<Entity> = world
        .query_filtered::<Entity, With<DropAnim>>()
        .iter(world)
        .collect();
    for entity in flying {
        let Some(mut anim) = world.get_mut::<DropAnim>(entity) else {
            continue;
        };
        anim.elapsed += dt;
        let t = (anim.elapsed - anim.delay)
            .ratio(DROP_DURATION)
            .clamp(0.0, 1.0);
        let landed = anim.elapsed - anim.delay >= DROP_DURATION;
        let (from, to, sfx) = (anim.from, anim.to, anim.drop_sfx);
        let at = from.lerp(to, motion::ease_out(t));
        let hop = DROP_HOP.0 * (std::f32::consts::PI * t).sin();
        let ground = ground_height(world, at);
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = world_point(at, ground + Meters(hop));
        }
        if landed {
            world.write_message(PlaySfx {
                id: sfx,
                place: SfxPlace::World(to),
            });
            world.entity_mut(entity).remove::<DropAnim>();
        }
    }
}

fn tidy_glows(world: &mut World) {
    let gone: Vec<Entity> = world
        .query::<(Entity, &DropGlow)>()
        .iter(world)
        .filter(|(_, glow)| {
            !world
                .get_entity(glow.0)
                .is_ok_and(|item| item.contains::<DroppedItem>())
        })
        .map(|(entity, _)| entity)
        .collect();
    for entity in gone {
        world.despawn(entity);
    }
}

/// Letters how many of an item lie in a pile over it.
fn count_drops(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let piles: Vec<(Entity, Pos<Meters>, u32)> = world
        .query::<(Entity, &Position, &DroppedItem)>()
        .iter(world)
        .filter(|(_, _, dropped)| dropped.count > 1)
        .map(|(entity, position, dropped)| (entity, position.pos, dropped.count))
        .collect();
    for (entity, at, count) in piles {
        let Some(over) = window_point(world, at, COUNT_HEIGHT) else {
            continue;
        };
        gui::world_area(world, ("drop count", entity))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(egui::pos2(over.x, over.y))
            .show(&gui, |ui| {
                ui.label(
                    egui::RichText::new(format!("×{count}"))
                        .font(TextRole::Name.font_id())
                        .color(egui::Color32::WHITE),
                );
            });
    }
}
