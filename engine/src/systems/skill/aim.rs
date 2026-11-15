use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui;
use ui::egui::Stroke;
use ui::egui::emath::TSTransform;
use ui::face::{self, Face, TextRole};
use ui::{dialog, dusk, ink};

use super::{CastAim, Rank, SkillId, Targeting, UseSkillRequest, casting};
use crate::core::content::{Content, Fixture};
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::{Pos, PosSchema};
use crate::core::render::Pointed;
use crate::core::text::{self, StyledLine, TextLook};
use crate::core::time::{self};
use crate::core::units::Meters;
use crate::core::vfx::render::ShaderOverlay;
use crate::core::vfx::{ParamValue, VfxParams, VfxSite};
use crate::core::{gui, render};
use crate::systems::actor::{self, Actor};
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::{CursorShape, InterfaceSound};
use crate::systems::mount::seat::MountRequest;
use crate::systems::mount::{self, MountId, Travels};
use crate::systems::player::session;
use crate::systems::vfx::{VfxShaderFixture, fixture};
use crate::systems::{movement, notification};

/// What a press armed, waiting for the click that aims it, and the surface drawn on the ground
/// where an area skill would land.
#[derive(Resource, Default)]
pub struct Aiming {
    armed: Option<Aimed>,
    surface: Option<Entity>,
}

/// What waits on a click to aim it: a skill, or the ground to step off a mount for.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Aimed {
    Skill { skill: SkillId, rank: Rank },
    Landing,
}

impl Aimed {
    pub fn skill(self) -> Option<SkillId> {
        match self {
            Aimed::Skill { skill, .. } => Some(skill),
            Aimed::Landing => None,
        }
    }

    fn targeting(self, content: &Content) -> Targeting {
        match self {
            Aimed::Skill { skill, .. } => skill.get(content).target,
            Aimed::Landing => Targeting::Spot,
        }
    }
}

/// The spot an armed aim would take where the pointer points, and what the spot has against it,
/// in the game's words.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SpotAim {
    pub aimed: Aimed,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    pub refused: Option<String>,
}

pub fn arm(world: &mut World, skill: SkillId, rank: Rank) {
    world.resource_mut::<Aiming>().armed = Some(Aimed::Skill { skill, rank });
    InterfaceSound::UiMove.play(world);
}

/// Arms the aim for the ground to step off what the character rides for.
pub fn arm_landing(world: &mut World) {
    world.resource_mut::<Aiming>().armed = Some(Aimed::Landing);
    InterfaceSound::UiMove.play(world);
}

/// The spot the armed aim would take under the pointer, while what is armed takes a spot.
pub fn spot_aim(world: &mut World) -> Option<SpotAim> {
    let aimed = aiming(world)?;
    let me = session::me(world)?;
    if aimed.targeting(world.resource::<Content>()) != Targeting::Spot {
        return None;
    }
    let at = render::pointed(world)?.surface?;
    Some(SpotAim {
        aimed,
        at,
        refused: spot_refusal(world, me, aimed, at),
    })
}

/// Quick cast: the skill is cast at what the pointer rests on, with no arming in between. Whether
/// anything fitting was there to cast at.
pub fn cast_at_pointer(world: &mut World, skill: SkillId) -> bool {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let aim = render::pointed(world).and_then(|pointed| aim_at(world, &pointed, def.target));
    let refusal = match (aim, session::me(world)) {
        (Some(CastAim::Point(spot)), Some(me)) if def.target == Targeting::Spot => {
            casting::spot_refused(world, me, skill, spot)
        }
        _ => None,
    };
    if let Some(refusal) = refusal {
        notification::local_error(world, refusal);
        return false;
    }
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

/// What is armed forgets itself once the character could no longer do it: a skill they stopped
/// knowing, a landing with nothing to step off.
pub fn settle(world: &mut World) {
    let Some(aimed) = aiming(world) else {
        return;
    };
    let holds = session::me(world).is_some_and(|me| match aimed {
        Aimed::Skill { skill, .. } => super::rank_of(world, me, skill) > Rank::NONE,
        Aimed::Landing => mount::rides(world, me),
    });
    if !holds {
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
    let uptime = time::uptime(world);
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
pub struct AimClaimant;

impl Claimant for AimClaimant {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        let aimed = aiming(world)?;
        if !session::is_alive(world) {
            return None;
        }
        let pointed = pointed?;
        let targeting = aimed.targeting(world.resource::<Content>());
        if targeting == Targeting::Caster {
            return None;
        }
        Some(match aim_at(world, pointed, targeting) {
            Some(CastAim::Actor(target)) => ClickClaim::Aim(target),
            Some(CastAim::Point(point)) => ClickClaim::AimAt(point),
            Some(CastAim::None) | None => ClickClaim::AimAt(pointed.ground?),
        })
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if !start {
            return;
        }
        let Some(aimed) = aiming(world) else {
            return;
        };
        let targeting = aimed.targeting(world.resource::<Content>());
        let aim = match claim {
            Some(ClickClaim::Aim(target)) => CastAim::Actor(target),
            Some(ClickClaim::AimAt(point)) if targeting.aimed_at_point() => CastAim::Point(point),
            _ => {
                disarm(world);
                return;
            }
        };
        // A click on a spot that will not do says why and leaves the aim armed for a better one.
        let refusal = match (targeting, aim, session::me(world)) {
            (Targeting::Spot, CastAim::Point(spot), Some(me)) => {
                spot_refusal(world, me, aimed, spot)
            }
            _ => None,
        };
        if let Some(refusal) = refusal {
            notification::local_error(world, refusal);
            return;
        }
        disarm(world);
        match (aimed, aim) {
            (Aimed::Skill { skill, .. }, aim) => {
                world.write_message(UseSkillRequest { skill, aim });
            }
            (Aimed::Landing, CastAim::Point(at)) => {
                world.write_message(MountRequest::StepOff { at: Some(at) });
            }
            (Aimed::Landing, CastAim::Actor(_) | CastAim::None) => {}
        }
    }

    fn cursor(&self, world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        aiming(world).map(|_| Cursor::fixed(world, CursorShape::Target))
    }
}

/// How far what is armed reaches around the character, drawn as a ring. An area lies on the
/// ground, kept by `surface`, and says on its own what it takes; a spot is marked where the
/// pointer points, washed gold where it will do and red with what it has against it where it will
/// not, under a prompt that names what is being picked.
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
    let reach = PlacedShape::cast(
        Shape::Circle {
            radius: reach_of(world, me, aimed),
        },
        from,
        from,
    );
    let ring = outline(world, &reach);
    let spot = spot_aim(world);
    let mark = spot.as_ref().and_then(|spot| {
        let shape = Shape::Circle {
            radius: SPOT_RADIUS,
        };
        let ring = outline(world, &PlacedShape::cast(shape, spot.at, spot.at))?;
        let said = spot.refused.clone().or_else(|| good_spot(world, me, aimed));
        Some((ring, spot.refused.is_some(), said))
    });
    let screen = gui.content_rect();
    let area = gui::world_area(world, "aim preview");
    area.fixed_pos(screen.min).show(&gui, |ui| {
        let painter = ui.painter();
        if let Some(ring) = ring {
            painter.add(egui::Shape::closed_line(
                ring,
                Stroke::new(1.0, ink::wash(ink::GOLD2, 0.45)),
            ));
        }
        let Some((ring, refused, said)) = mark else {
            return;
        };
        let (ground, words) = if refused {
            (ink::VERMILION, dusk::DANGER)
        } else {
            (ink::GOLD2, ink::GOLD2)
        };
        let middle = egui::Rect::from_points(&ring);
        // The pointer is the one crosshair of an aim: the spot under it is only washed.
        painter.add(egui::Shape::convex_polygon(
            ring,
            ink::wash(ground, SPOT_WASH),
            Stroke::NONE,
        ));
        if let Some(said) = said {
            let galley =
                painter.layout_no_wrap(said, Face::BookBold.id(TextRole::Body.size()), words);
            let at = egui::Align2::CENTER_TOP
                .anchor_size(middle.center_bottom() + SPOT_WORDS_BELOW, galley.size());
            dusk::lettering(painter, at.min, galley, dusk::EDGED, TSTransform::IDENTITY);
        }
    });
    if aimed.targeting(world.resource::<Content>()) == Targeting::Spot {
        prompt(world, &gui, me, aimed);
    }
}

/// How wide a spot is marked.
const SPOT_RADIUS: Meters = Meters(0.6);
const SPOT_WASH: f32 = 0.45;
/// Under the spot and clear of the pointer resting on it.
const SPOT_WORDS_BELOW: egui::Vec2 = egui::Vec2::new(0.0, 14.0);
/// How far over the bottom of the window the prompt of an aim stands, clear of the bar.
const PROMPT_FROM_BOTTOM: f32 = 140.0;
const PROMPT_WIDTH: f32 = 380.0;

/// Names what a spot is being picked for, and what a click and stepping out of the aim do.
fn prompt(world: &mut World, gui: &egui::Context, me: Entity, aimed: Aimed) {
    let content = world.resource::<Content>().clone();
    let floats = |mount: MountId| matches!(mount.get(&content).travels, Travels::Floating { .. });
    let (asks, click, leaves) = match aimed {
        Aimed::Landing => ("Pick a spot on the shore", " step off", " stay aboard"),
        Aimed::Skill { skill, .. } if mount::left_by(world, me, skill) => {
            ("Pick a spot on the shore", " step off", " stay aboard")
        }
        Aimed::Skill { skill, .. } if mount::called_by(world, skill).is_some_and(floats) => {
            ("Pick a spot on the water", " launch", " put it away")
        }
        Aimed::Skill { .. } => ("Pick a spot", " use it there", " put it away"),
    };
    let hints = StyledLine::new(map::hints([
        (InputAction::Interact, click),
        (InputAction::Dismiss, leaves),
    ]));
    let area = gui::world_area(world, "aim prompt");
    area.anchor(
        egui::Align2::CENTER_BOTTOM,
        egui::Vec2::new(0.0, -PROMPT_FROM_BOTTOM),
    )
    .show(gui, |ui| {
        dialog::note(ui, PROMPT_WIDTH, |ui| {
            ui.label(face::body(asks).color(ink::INK));
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, &hints, look, None);
        });
    });
}

/// What a good spot is marked as: what would stand there.
fn good_spot(world: &World, me: Entity, aimed: Aimed) -> Option<String> {
    let content = world.resource::<Content>();
    match aimed {
        Aimed::Skill { skill, .. } if !mount::left_by(world, me, skill) => {
            let mount = mount::called_by(world, skill)?;
            Some(format!("{} here", mount.get(content).name))
        }
        Aimed::Skill { .. } | Aimed::Landing => Some("Step off here".to_owned()),
    }
}

/// How far from the character what is armed may be aimed: a skill's range, or for a rider
/// leaving their mount how far the mount lets them step.
fn reach_of(world: &World, me: Entity, aimed: Aimed) -> Meters {
    match aimed {
        Aimed::Skill { skill, rank } if !mount::left_by(world, me, skill) => {
            super::range_of(world, me, skill, rank)
        }
        Aimed::Skill { .. } | Aimed::Landing => mount::reach_of(world, me).unwrap_or_default(),
    }
}

/// What the spot `at` has against being taken by what is armed, in the game's words.
fn spot_refusal(world: &World, me: Entity, aimed: Aimed, at: Pos<Meters>) -> Option<String> {
    match aimed {
        Aimed::Skill { skill, .. } => casting::spot_refused(world, me, skill, at),
        Aimed::Landing => mount::landing(world, me, Some(at)).err(),
    }
}

/// The outline of `shape` as the window shows it, lying on the ground or the water over it: none
/// while any of it is out of the camera's sight.
fn outline(world: &mut World, shape: &PlacedShape) -> Option<Vec<egui::Pos2>> {
    let view = render::view_of(world)?;
    shape
        .outline()
        .iter()
        .map(|point| {
            let height = render::surface_height(world, *point);
            view.project(render::world_point(*point, height))
                .map(|at| egui::Pos2::new(at.x, at.y))
        })
        .collect()
}

/// What a skill of the targeting cast where the pointer points lands on: the fitting actor there,
/// the ground point for an area, the point on the ground or the water over it for a spot, or
/// nothing fitting.
fn aim_at(world: &mut World, pointed: &Pointed, targeting: Targeting) -> Option<CastAim> {
    match targeting {
        Targeting::SingleAlly | Targeting::SingleEnemy => {
            let me = session::me(world)?;
            actor::target_at(world, pointed, None, |world, entity| {
                world.get::<Actor>(entity).is_some() && casting::fits(world, me, entity, targeting)
            })
            .map(CastAim::Actor)
        }
        Targeting::AreaAlly | Targeting::AreaEnemy => Some(CastAim::Point(pointed.ground?)),
        Targeting::Spot => Some(CastAim::Point(pointed.surface?)),
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
fn aimed_area(world: &mut World, aimed: Aimed) -> Option<(PlacedShape, bool)> {
    let me = session::me(world)?;
    let content = world.resource::<Content>().clone();
    let Aimed::Skill { skill, rank } = aimed else {
        return None;
    };
    let def = skill.get(&content);
    let shape = def.area_at(rank)?;
    let from = movement::position(world, me)?;
    let point = render::cursor_ground(world)?;
    Some((PlacedShape::cast(shape, from, point), def.target.hostile()))
}
