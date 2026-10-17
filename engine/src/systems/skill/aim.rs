use bevy::prelude::*;
use design_tokens::{TextRole, color};
use egui::{Color32, FontId, Pos2, Stroke, Vec2};

use super::{Rank, SkillId, Targeting, UseSkillRequest, using};
use crate::core::content::{Content, Fixture};
use crate::core::geometry::{Footprint, Shape, facing_from};
use crate::core::gui;
use crate::core::math::Pos;
use crate::core::render;
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::Seconds;
use crate::core::vfx::render::ShaderOverlay;
use crate::core::vfx::{ParamValue, VfxParams, VfxSite};
use crate::systems::actor::{self, Actor};
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interface::{CursorShape, InterfaceSound};
use crate::systems::movement;
use crate::systems::notification;
use crate::systems::player::session;
use crate::systems::skill::Aim;
use crate::systems::vfx::VfxShaderFixture;

const OUTLINE_STEPS: usize = 40;
const HOSTILE_PARAM: &str = "hostile";

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
    let aim = render::cursor_tile(world).and_then(|tile| match def.target {
        Targeting::SingleAlly | Targeting::SingleEnemy => {
            actor_at(world, tile, def.target).map(Aim::Actor)
        }
        Targeting::AreaAlly | Targeting::AreaEnemy => Some(Aim::Point(tile)),
        Targeting::Caster => None,
    });
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
        params: VfxParams(vec![(HOSTILE_PARAM.to_owned(), ParamValue::Bool(hostile))]),
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
    fn claim(&self, world: &mut World, tile: Option<Pos<Tiles>>) -> Option<ClickClaim> {
        let aimed = aiming(world)?;
        if !session::is_alive(world) {
            return None;
        }
        let tile = tile?;
        let content = world.resource::<Content>().clone();
        let targeting = aimed.skill.get(&content).target;
        match targeting {
            Targeting::SingleAlly | Targeting::SingleEnemy => Some(
                actor_at(world, tile, targeting)
                    .map_or(ClickClaim::AimAt(tile.snap()), ClickClaim::Aim),
            ),
            Targeting::AreaAlly | Targeting::AreaEnemy => Some(ClickClaim::AimAt(tile)),
            Targeting::Caster => None,
        }
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
            Some(ClickClaim::Aim(target)) => Aim::Actor(target),
            Some(ClickClaim::AimAt(point)) if aimed.skill.get(&content).target.aimed_at_point() => {
                Aim::Point(point)
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

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        let aimed = aiming(world)?;
        let hostile = aimed
            .skill
            .get(world.resource::<Content>())
            .target
            .hostile();
        Some(Cursor::Shaped(match claim {
            ClickClaim::Aim(_) if hostile => CursorShape::Attack,
            ClickClaim::Aim(_) => CursorShape::Use,
            _ => CursorShape::Default,
        }))
    }
}

/// The armed skill's range around the character and, for a skill aimed at one target, a line
/// saying what a click does. The area itself lies on the ground, kept by `surface`, and says on
/// its own what it takes.
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
    let content = world.resource::<Content>().clone();
    let def = aimed.skill.get(&content);
    let Some(from) = movement::position(world, me) else {
        return;
    };
    let area = gui::world_area(world, "aim preview");
    let range = super::range_of(world, me, aimed.skill, aimed.rank);
    let mut lines: Vec<(Vec<Pos2>, Color32, f32)> = Vec::new();
    let reach = Footprint {
        at: from,
        facing: facing_from(from, from),
        shape: Shape::Circle { radius: range },
    };
    if let Some(ring) = project(world, &reach.outline(OUTLINE_STEPS)) {
        lines.push((ring, color::WARN.gamma_multiply(0.35), 1.0));
    }
    let label = def
        .target
        .aimed_at_actor()
        .then(|| format!("{}: click a target · Esc cancels", def.name));
    let screen = gui.content_rect();
    area.fixed_pos(screen.min).show(&gui, |ui| {
        let painter = ui.painter();
        for (points, color, width) in lines {
            painter.add(egui::Shape::closed_line(points, Stroke::new(width, color)));
        }
        let Some(label) = label else {
            return;
        };
        let at = gui
            .pointer_latest_pos()
            .map_or(screen.center(), |at| at + Vec2::new(16.0, 18.0));
        let font = FontId::proportional(TextRole::Small.size());
        let galley = painter.layout_no_wrap(label, font, color::TEXT);
        let rect = egui::Rect::from_min_size(at, galley.size() + Vec2::new(10.0, 6.0));
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(180));
        painter.galley(rect.min + Vec2::new(5.0, 3.0), galley, color::TEXT);
    });
}

fn clear_surface(world: &mut World) {
    if let Some(entity) = world.resource_mut::<Aiming>().surface.take()
        && world.get_entity(entity).is_ok()
    {
        world.despawn(entity);
    }
}

/// Where the armed skill's area lies: at the pointer for a circle or a rect, from the character for
/// a cone or a line, facing the pointer either way. With whether the skill harms what it takes.
fn aimed_area(world: &mut World, aimed: Aimed) -> Option<(Footprint, bool)> {
    let me = session::me(world)?;
    let content = world.resource::<Content>().clone();
    let def = aimed.skill.get(&content);
    let shape = def.area_at(aimed.rank)?;
    let from = movement::position(world, me)?;
    let point = render::cursor_tile(world)?;
    let at = match shape {
        Shape::Cone { .. } | Shape::Line { .. } => from,
        Shape::Circle { .. } | Shape::Rect { .. } => point,
    };
    let footprint = Footprint {
        at,
        facing: facing_from(from, point),
        shape,
    };
    Some((footprint, def.target.hostile()))
}

fn actor_at(world: &mut World, point: Pos<Tiles>, targeting: Targeting) -> Option<Entity> {
    let me = session::me(world)?;
    actor::frontmost_at(world, point, None, |world, entity| {
        world.get::<Actor>(entity).is_some() && using::fits(world, me, entity, targeting)
    })
}

fn project(world: &mut World, points: &[Pos<Tiles>]) -> Option<Vec<Pos2>> {
    points
        .iter()
        .map(|point| render::tile_to_window(world, *point).map(|at| Pos2::new(at.x, at.y)))
        .collect()
}
