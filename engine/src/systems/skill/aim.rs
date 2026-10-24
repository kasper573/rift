use bevy::prelude::*;
use ui::egui;
use ui::egui::Stroke;
use ui::ink;

use super::{CastAim, Rank, SkillId, Targeting, UseSkillRequest, casting};
use crate::core::content::{Content, Fixture};
use crate::core::geometry::{Footprint, Shape};
use crate::core::math::Pos;
use crate::core::time::Seconds;
use crate::core::units::{Meters, TilePos};
use crate::core::vfx::render::ShaderOverlay;
use crate::core::vfx::{ParamValue, VfxParams, VfxSite};
use crate::core::{gui, render};
use crate::systems::actor::{self, Actor};
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interface::{CursorShape, InterfaceSound};
use crate::systems::player::session;
use crate::systems::vfx::{VfxShaderFixture, fixture};
use crate::systems::{movement, notification};

/// The skill a press armed, waiting for the click that aims it, and the surface drawn on the ground
/// where an area skill would land.
#[derive(Resource, Default)]
pub struct Aiming {
    armed: Option<Aimed>,
    surface: Option<Entity>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aimed {
    pub skill: SkillId,
    pub rank: Rank,
}

pub fn arm(world: &mut World, skill: SkillId, rank: Rank) {
    world.resource_mut::<Aiming>().armed = Some(Aimed { skill, rank });
    InterfaceSound::UiMove.play(world);
}

/// Quick cast: the skill is cast at what the pointer rests on, with no arming in between. Whether
/// anything fitting was there to cast at.
pub fn cast_at_pointer(world: &mut World, skill: SkillId) -> bool {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let aim = render::cursor_tile(world).and_then(|tile| aim_at(world, tile, def.target));
    match aim {
        Some(aim) => {
            world.write_message(UseSkillRequest { skill, aim });
            true
        }
        None => {
            notification::local_error(
                world,
                format!("{}: nothing under the pointer to cast at", def.name),
            );
            false
        }
    }
}

pub fn disarm(world: &mut World) -> bool {
    world.resource_mut::<Aiming>().armed.take().is_some()
}

pub fn aiming(world: &World) -> Option<Aimed> {
    world.resource::<Aiming>().armed
}

pub fn forget(world: &mut World) {
    disarm(world);
    clear_surface(world);
}

/// An armed skill forgets itself once the character stops knowing it.
pub fn settle(world: &mut World) {
    let Some(aimed) = aiming(world) else {
        return;
    };
    let known =
        session::me(world).is_some_and(|me| super::rank_of(world, me, aimed.skill) > Rank::NONE);
    if !known {
        disarm(world);
    }
}

/// Keeps the ground the armed skill's area would cover painted through the game's area shader,
/// following the pointer, and takes it away when no area is aimed.
pub fn surface(world: &mut World) {
    let Some((footprint, hostile)) = aiming(world).and_then(|aimed| aimed_area(world, aimed))
    else {
        clear_surface(world);
        return;
    };
    let content = world.resource::<Content>().clone();
    let uptime = Seconds(world.resource::<Time>().elapsed_secs());
    let shown = world
        .resource::<Aiming>()
        .surface
        .filter(|entity| world.get_entity(*entity).is_ok());
    let since = shown
        .and_then(|entity| world.get::<ShaderOverlay>(entity))
        .map_or(uptime, |overlay| overlay.since);
    let overlay = ShaderOverlay {
        shader: VfxShaderFixture::SkillArea.id(&content),
        site: VfxSite::Area(footprint),
        params: VfxParams(vec![(
            fixture::HOSTILE.to_owned(),
            ParamValue::Bool(hostile),
        )]),
        since,
    };
    match shown {
        Some(entity) => {
            world.entity_mut(entity).insert(overlay);
        }
        None => {
            let entity = world.spawn(overlay).id();
            world.resource_mut::<Aiming>().surface = Some(entity);
        }
    }
}

/// Takes the click while a skill is armed: an actor the skill fits, a point for an area, and the
/// bare ground for a single-target skill, which puts the skill away.
pub struct AimGesture;

impl Gesture for AimGesture {
    fn claim(&self, world: &mut World, tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        let aimed = aiming(world)?;
        if !session::is_alive(world) {
            return None;
        }
        let tile = tile?;
        let targeting = aimed.skill.get(world.resource::<Content>()).target;
        if targeting == Targeting::Caster {
            return None;
        }
        Some(match aim_at(world, tile, targeting) {
            Some(CastAim::Actor(target)) => ClickClaim::Aim(target),
            Some(CastAim::Point(point)) => ClickClaim::AimAt(point),
            Some(CastAim::None) | None => ClickClaim::AimAt(tile.snap()),
        })
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if !start {
            return;
        }
        let Some(aimed) = aiming(world) else {
            return;
        };
        let content = world.resource::<Content>().clone();
        let aim = match claim {
            Some(ClickClaim::Aim(target)) => CastAim::Actor(target),
            Some(ClickClaim::AimAt(point)) if aimed.skill.get(&content).target.aimed_at_point() => {
                CastAim::Point(point)
            }
            _ => {
                disarm(world);
                return;
            }
        };
        disarm(world);
        world.write_message(UseSkillRequest {
            skill: aimed.skill,
            aim,
        });
    }

    fn cursor(&self, world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        aiming(world).map(|_| Cursor::Shaped(CursorShape::Target))
    }
}

/// The armed skill's range around the character. The area itself lies on the ground, kept by
/// `surface`, and says on its own what it takes.
pub fn draw_aim(world: &mut World) {
    let Some(aimed) = aiming(world) else {
        return;
    };
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(me) = session::me(world) else {
        return;
    };
    let Some(from) = movement::position(world, me) else {
        return;
    };
    let range = super::range_of(world, me, aimed.skill, aimed.rank);
    let reach = Footprint::cast(Shape::Circle { radius: range }, from, from);
    let Some(ring) = reach
        .outline()
        .iter()
        .map(|point| render::tile_to_window(world, *point).map(|at| egui::Pos2::new(at.x, at.y)))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let screen = gui.content_rect();
    let area = gui::world_area(world, "aim preview");
    area.fixed_pos(screen.min).show(&gui, |ui| {
        ui.painter().add(egui::Shape::closed_line(
            ring,
            Stroke::new(1.0, ink::wash(ink::GOLD2, 0.45)),
        ));
    });
}

/// What a skill of the targeting cast at a tile lands on: the fitting actor there, the point
/// itself for an area, or nothing fitting.
fn aim_at(world: &mut World, tile: Pos<Meters>, targeting: Targeting) -> Option<CastAim> {
    match targeting {
        Targeting::SingleAlly | Targeting::SingleEnemy => {
            let me = session::me(world)?;
            actor::target_at(world, tile, None, |world, entity| {
                world.get::<Actor>(entity).is_some() && casting::fits(world, me, entity, targeting)
            })
            .map(CastAim::Actor)
        }
        Targeting::AreaAlly | Targeting::AreaEnemy => Some(CastAim::Point(tile)),
        Targeting::Caster => None,
    }
}

fn clear_surface(world: &mut World) {
    if let Some(entity) = world.resource_mut::<Aiming>().surface.take()
        && world.get_entity(entity).is_ok()
    {
        world.despawn(entity);
    }
}

/// Where the armed skill's area lies, with whether the skill harms what it takes.
fn aimed_area(world: &mut World, aimed: Aimed) -> Option<(Footprint, bool)> {
    let me = session::me(world)?;
    let content = world.resource::<Content>().clone();
    let def = aimed.skill.get(&content);
    let shape = def.area_at(aimed.rank)?;
    let from = movement::position(world, me)?;
    let point = render::cursor_tile(world)?;
    Some((Footprint::cast(shape, from, point), def.target.hostile()))
}
