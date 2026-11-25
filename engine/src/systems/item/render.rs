//! Items as they lie dropped: each as the prop it looks like and never smaller than the game's
//! least size, hopping from whoever dropped it to where it lands. One the player may take lies on
//! a disc in its rarity's colour and plays what its rarity plays, as far as the game's budget
//! goes, and every one pulses before it is gone.

use std::collections::HashSet;
use std::f32::consts::TAU;

use bevy::prelude::*;
use ui::motion;

use crate::core::audio::playback::{PlaySfx, SfxId, SfxPlace};
use crate::core::content::Content;
use crate::core::content::gltf::MeshBounds;
use crate::core::content::media::GltfInfos;
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::decal::{DecalLook, GroundDecal};
use crate::core::render::{ground_height, world_point};
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::core::vfx::render::SiteExtent;
use crate::core::vfx::standing::StandingVfx;
use crate::core::vfx::{VfxInstance, VfxSite};
use crate::systems::GameSettings;
use crate::systems::area::scenery::ShowsProp;
use crate::systems::area::shown_weather::PartsCover;
use crate::systems::identity::Lingering;
use crate::systems::item::plates::Landed;
use crate::systems::item::{self, DroppedItem, ItemConsumed, ItemsDropped, Reservation};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session::MyCharacter;
use crate::systems::player::session::{self, MyClient};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;
use crate::systems::society::SocietyView;
use crate::systems::visibility::Faint;

const DROP_STAGGER: Seconds = Seconds(0.06);
const DROP_DURATION: Seconds = Seconds(0.5);
const DROP_HOP: Meters = Meters(0.5);
/// How fast a dropped item turns on the spot, so it catches the eye.
const SPIN: f32 = 0.8;
const DISC: Meters = Meters(1.1);
const DISC_OPACITY: f32 = 0.9;
const PULSES_PER_SEC: f32 = 2.0;
/// How much of itself an item about to be gone shows at the low of a pulse.
const PULSE_LOW: f32 = 0.35;

pub struct ItemRenderPlugin;

impl Plugin for ItemRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(dress_drop)
            .add_observer(leave_its_look)
            .add_systems(
                Update,
                (
                    use_sounds,
                    (
                        start_drops,
                        place_drops,
                        animate_drops,
                        mark_drops,
                        play_effects,
                    )
                        .chain()
                        .in_set(WorldViewSystems),
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .init_resource::<CalmEffects>();
    }
}

/// Whether the player has what rarities play calmed: each vfx held still, none bursting as an
/// item lands, and an item about to be gone fading steadily rather than pulsing.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalmEffects(pub bool);

/// An item in the air, on its way from whoever dropped it to where it lands.
#[derive(Component)]
pub(super) struct DropAnim {
    from: Pos<Meters>,
    to: Pos<Meters>,
    delay: Seconds,
    elapsed: Seconds,
    drop_sfx: SfxId,
    /// The sound of its rarity, for the one item of a drop that plays it.
    rarity_sfx: Option<SfxId>,
}

/// An item that came to lie this frame.
#[derive(Component)]
struct JustLanded;

/// The vfx of a rarity bursting once as its item lands, until `0`.
#[derive(Component)]
struct LandingBurst(time::UnixMillis);

/// The disc on the ground under a dropped item, kept beside the drop so the drop's own transform
/// leaves it lying flat.
#[derive(Component)]
struct DropDisc(Entity);

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
    drops: Query<&DroppedItem, Without<Lingering>>,
    content: Res<Content>,
    mut commands: Commands,
) {
    let Ok(dropped) = drops.get(add.entity) else {
        return;
    };
    let shown = dropped.item.get(&content).look;
    let look = shown.get(&content);
    let least = content.built::<GameSettings>().drops.least_size;
    let bounds = content
        .built::<GltfInfos>()
        .0
        .get(look.model.as_str())
        .map(|info| info.bounds);
    let scale = bounds.map_or(1.0, |bounds| enlarged(bounds, least));
    let extent = bounds.map_or(
        SiteExtent {
            footprint: Vec2::splat(least.0),
            height: least.0,
        },
        |bounds| {
            let size = (bounds.max - bounds.min) * scale;
            SiteExtent {
                footprint: Vec2::new(size.x, size.z),
                height: bounds.max.y * scale,
            }
        },
    );
    commands.entity(add.entity).insert((
        ShowsProp(shown),
        Transform::from_scale(Vec3::splat(scale)),
        Visibility::default(),
        extent,
        PartsCover {
            girth: extent.footprint.max_element() / 2.0,
            height: extent.height,
        },
    ));
}

/// A dropped item that is gone, taken or left too long, lingers where it lay while its look
/// fades. Its look is all that lingers: nothing lies there to be named, lit or taken.
pub fn leave_its_look(
    add: On<Add<DroppedItem>>,
    lingering: Query<(), With<Lingering>>,
    mut commands: Commands,
) {
    if lingering.contains(add.entity) {
        commands
            .entity(add.entity)
            .try_remove::<(DroppedItem, Reservation, StandingVfx)>();
    }
}

/// How many times its modelled size a model of `bounds` is drawn so that it is `least` across.
fn enlarged(bounds: MeshBounds, least: Meters) -> f32 {
    let across = (bounds.max - bounds.min).max_element();
    if across > 0.0 {
        (least.0 / across).max(1.0)
    } else {
        1.0
    }
}

/// Sends every item of a drop on its way, and picks the one that sounds its rarity as it lands:
/// of those the player may take, the one of the highest rank whose rarity has a sound.
fn start_drops(
    mut dropped: MessageReader<ItemsDropped>,
    content: Res<Content>,
    me: Res<MyClient>,
    mine: Res<MyCharacter>,
    parties: Query<&SocietyView>,
    drops: Query<(&DroppedItem, &Position, Option<&Reservation>)>,
    mut commands: Commands,
) {
    let party = mine
        .0
        .and_then(|me| parties.get(me).ok())
        .and_then(|view| view.party.as_ref());
    let taker = item::DropTaker {
        client: me.0,
        party: party.map_or_else(Vec::new, |party| party.clients().collect()),
    };
    for dropped in dropped.read() {
        let sounding = dropped
            .items
            .iter()
            .filter_map(|&entity| {
                let (item, _, reservation) = drops.get(entity).ok()?;
                let rarity = item.item.get(&content).rarity.get(&content);
                let sfx = rarity.dropped.sfx?;
                item::takeable(reservation, &taker).then_some((rarity.rank, entity, sfx))
            })
            .max_by_key(|(rank, ..)| *rank);
        for (index, &entity) in dropped.items.iter().enumerate() {
            let Ok((item, position, _)) = drops.get(entity) else {
                continue;
            };
            commands.entity(entity).try_insert(DropAnim {
                from: dropped.from,
                to: position.pos,
                delay: DROP_STAGGER * index as f32,
                elapsed: Seconds(0.0),
                drop_sfx: item.item.get(&content).sfx.drop,
                rarity_sfx: sounding
                    .filter(|(_, sounds, _)| *sounds == entity)
                    .map(|(.., sfx)| sfx),
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
        let (from, to) = (anim.from, anim.to);
        let sounds = [Some(anim.drop_sfx), anim.rarity_sfx];
        let at = from.lerp(to, motion::ease_out(t));
        let hop = DROP_HOP.0 * (std::f32::consts::PI * t).sin();
        let ground = ground_height(world, at);
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = world_point(at, ground + Meters(hop));
        }
        if landed {
            for id in sounds.into_iter().flatten() {
                world.write_message(PlaySfx {
                    id,
                    place: SfxPlace::World(to),
                });
            }
            let now = time::uptime(world);
            world
                .entity_mut(entity)
                .remove::<DropAnim>()
                .insert((Landed(now), JustLanded));
        }
    }
}

/// Marks every item the player may take with a disc in its rarity's colour once it lies, and
/// warns of every item about to be gone: it pulses, and its disc fades with the time it has left.
fn mark_drops(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let me = item::DropTaker::me(world);
    let fades_for = world.resource::<GameSettings>().drops.fades_for;
    let now = time::server_now(world);
    let calm = world.resource::<CalmEffects>().0;
    let discs: Vec<(Entity, Entity)> = world
        .query::<(Entity, &DropDisc)>()
        .iter(world)
        .map(|(disc, of)| (disc, of.0))
        .collect();
    let marked: HashSet<Entity> = discs.iter().map(|(_, of)| *of).collect();
    let lying: Vec<(Entity, DroppedItem, Pos<Meters>, bool, bool)> = world
        .query::<(
            Entity,
            &DroppedItem,
            &Position,
            Option<&Reservation>,
            Has<DropAnim>,
        )>()
        .iter(world)
        .map(|(entity, dropped, position, reservation, flying)| {
            let mine = item::takeable(reservation, &me);
            (entity, dropped.clone(), position.pos, mine, flying)
        })
        .collect();
    for (disc, of) in discs {
        let lies = lying.iter().find(|(entity, ..)| *entity == of);
        let Some((_, dropped, at, _, flying)) = lies else {
            world.despawn(disc);
            continue;
        };
        let shown = if *flying {
            0.0
        } else {
            DISC_OPACITY * left_share(now, dropped, fades_for)
        };
        let rarity = dropped.item.get(&content).rarity.get(&content);
        let look = DecalLook::Glow {
            color: rarity.gradient.from.color().with_alpha(shown),
        };
        if let Some(mut decal) = world.get_mut::<GroundDecal>(disc)
            && (decal.at != *at || decal.look != look)
        {
            decal.at = *at;
            decal.look = look;
        }
    }
    for (entity, dropped, at, mine, _) in &lying {
        if *mine && !marked.contains(entity) {
            world.spawn((
                DropDisc(*entity),
                GroundDecal {
                    at: *at,
                    size: DISC,
                    yaw: default(),
                    look: DecalLook::Glow { color: Color::NONE },
                },
            ));
        }
        let faint = if calm {
            Some(left_share(now, dropped, fades_for)).filter(|left| *left < 1.0)
        } else {
            pulse(now, dropped, fades_for)
        }
        .map(Faint);
        let mut drop = world.entity_mut(*entity);
        match faint {
            Some(faint) if drop.get::<Faint>() != Some(&faint) => {
                drop.insert(faint);
            }
            None if drop.contains::<Faint>() => {
                drop.remove::<Faint>();
            }
            _ => {}
        }
    }
}

/// How much of the time an item fades for it still has, 1 until that time begins.
fn left_share(now: Option<time::UnixMillis>, dropped: &DroppedItem, fades_for: Seconds) -> f32 {
    match now {
        Some(now) if fades_for.0 > 0.0 => dropped.until.since(now).ratio(fades_for).clamp(0.0, 1.0),
        _ => 1.0,
    }
}

/// How much of itself an item shows as it pulses in the time it fades for, and nothing to say
/// before then.
fn pulse(now: Option<time::UnixMillis>, dropped: &DroppedItem, fades_for: Seconds) -> Option<f32> {
    let left = dropped.until.since(now?);
    (left < fades_for).then(|| {
        let low = 0.5 - 0.5 * (TAU * PULSES_PER_SEC * left.0).cos();
        1.0 - (1.0 - PULSE_LOW) * low
    })
}

/// Plays what its rarity plays on every item the player may take, for as many as the game shows
/// at once: the highest ranks first, then the nearest. An item with a place plays its rarity's
/// vfx for as long as it lies and bursts once as it lands; one without waits for a place.
fn play_effects(world: &mut World) {
    let calm = world.resource::<UserSettings>().rarity_effects_calm();
    if world.resource::<CalmEffects>().0 != calm {
        world.insert_resource(CalmEffects(calm));
    }
    let content = world.resource::<Content>().clone();
    let me = item::DropTaker::me(world);
    let places = content.built::<GameSettings>().drops.effects_shown as usize;
    let eye = session::me(world).and_then(|me| world.get::<Position>(me).map(|at| at.pos));
    let mut asking: Vec<(Entity, DroppedItem, f32, bool)> = world
        .query_filtered::<(
            Entity,
            &DroppedItem,
            &Position,
            Option<&Reservation>,
            Has<JustLanded>,
        ), Without<DropAnim>>()
        .iter(world)
        .filter(|(.., reservation, _)| item::takeable(*reservation, &me))
        .map(|(entity, dropped, at, _, landed)| {
            let away = eye.map_or(0.0, |eye| eye.distance(at.pos).0);
            (entity, dropped.clone(), away, landed)
        })
        .filter(|(_, dropped, _, landed)| {
            let plays = &dropped.item.get(&content).rarity.get(&content).dropped;
            plays.vfx.is_some() || (*landed && plays.landing_vfx.is_some())
        })
        .collect();
    let rank = |dropped: &DroppedItem| dropped.item.get(&content).rarity.get(&content).rank;
    asking.sort_by(|a, b| rank(&b.1).cmp(&rank(&a.1)).then(a.2.total_cmp(&b.2)));
    asking.truncate(places);
    let lying: Vec<Entity> = world
        .query_filtered::<Entity, With<DroppedItem>>()
        .iter(world)
        .collect();
    let now = time::server_now(world);
    for entity in lying {
        let placed = asking.iter().find(|(asks, ..)| *asks == entity);
        let rarity = placed.map(|(_, dropped, ..)| dropped.item.get(&content).rarity.get(&content));
        let standing = rarity.and_then(|rarity| {
            Some(StandingVfx {
                vfx: vec![rarity.dropped.vfx?],
                params: rarity.vfx_params(),
                held: calm,
            })
        });
        let burst = placed
            .filter(|(.., landed)| *landed && !calm)
            .and_then(|_| rarity?.dropped.landing_vfx)
            .zip(now);
        if let Some((vfx, now)) = burst {
            let until = now.after(vfx.get(&content).lasts());
            world.spawn((
                VfxInstance {
                    vfx,
                    site: VfxSite::Object(entity),
                    params: rarity.map(|rarity| rarity.vfx_params()).unwrap_or_default(),
                    heading: default(),
                    started: now,
                    until: Some(until),
                },
                LandingBurst(until),
            ));
        }
        let mut drop = world.entity_mut(entity);
        drop.remove::<JustLanded>();
        match standing {
            Some(standing) if drop.get::<StandingVfx>() != Some(&standing) => {
                drop.insert(standing);
            }
            None if drop.contains::<StandingVfx>() => {
                drop.remove::<StandingVfx>();
            }
            _ => {}
        }
    }
    let spent: Vec<Entity> = world
        .query::<(Entity, &LandingBurst)>()
        .iter(world)
        .filter(|(_, burst)| now.is_none_or(|now| burst.0 <= now))
        .map(|(entity, _)| entity)
        .collect();
    for entity in spent {
        world.despawn(entity);
    }
}
