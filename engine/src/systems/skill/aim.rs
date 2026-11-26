use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::egui;
use ui::egui::Stroke;
use ui::egui::emath::TSTransform;
use ui::face::{self, Face, TextRole};
use ui::{dialog, dusk, ink};

use super::{CastAim, Rank, SkillId, Targeting, UseSkillRequest, casting};
use crate::core::content::fixture::FixtureLock;
use crate::core::content::{Content, Fixture, Loading, Nearby};
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::{Pos, PosSchema};
use crate::core::render::Pointed;
use crate::core::text::LineText;
use crate::core::text::{self, StyledLine, TextLook};
use crate::core::time::{self};
use crate::core::units::{Meters, Radians};
use crate::core::vfx::{
    ParamValue, SiteKind, VfxDef, VfxInstance, VfxParam, VfxParams, VfxPart, VfxSite,
    check_fixtures_played,
};
use crate::core::{gui, render};
use crate::systems::actor::{self, Actor};
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::escape::{self, Dismissal};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::{CursorShape, InterfaceSound};
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::mount::seat::MountRequest;
use crate::systems::mount::{self, MountId, Travels};
use crate::systems::player::session;
use crate::systems::{movement, notification};

/// The parameters the engine fills in the vfx it shows while a skill is aimed: whether the skill
/// hurts whom it lands on, and whether it is swung from where the character stands.
pub const HOSTILE: &str = "hostile";
pub const MELEE: &str = "melee";

/// The vfx every game provides for a skill being aimed: the ground its area would land on, the
/// ring of how far it reaches round the character, and the mark on each actor its area would
/// land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum AimVfx {
    SkillArea,
    SkillReach,
    SkillTarget,
}

impl Fixture for AimVfx {
    type Row = VfxDef;
    const FOLDER: Option<&'static str> = Some("vfx");

    fn locks(self) -> Vec<FixtureLock> {
        VfxDef::fixture_locks(&self.params())
    }

    fn fresh(self, near: &Nearby) -> Result<VfxDef, String> {
        VfxDef::fixture_fresh(near, self.params())
    }
}

/// Refuses a layer of an aim's vfx that the site the engine shows it on cannot carry.
pub fn check_vfx(loading: &mut Loading) {
    check_fixtures_played::<AimVfx>(loading, |fixture| match fixture {
        AimVfx::SkillArea | AimVfx::SkillReach => &[SiteKind::Area],
        AimVfx::SkillTarget => &[SiteKind::Actor],
    });
}

impl AimVfx {
    fn params(self) -> Vec<VfxParam> {
        let flag = |name: &str| VfxParam {
            name: name.to_owned(),
            default: ParamValue::Bool(false),
        };
        match self {
            AimVfx::SkillArea => vec![flag(HOSTILE), flag(MELEE)],
            AimVfx::SkillReach => Vec::new(),
            AimVfx::SkillTarget => vec![flag(HOSTILE)],
        }
    }
}

/// What a press armed, waiting for the click that aims it, and the vfx shown for it: the ground
/// an area skill would land on, the ring of its reach, and the mark on each actor it would land on.
#[derive(Resource, Default)]
pub struct Aiming {
    armed: Option<Aimed>,
    area: Option<Entity>,
    reach: Option<Entity>,
    targets: Vec<(Entity, Entity)>,
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
            Aimed::Landing => Targeting::Spot { range: None },
        }
    }
}

/// The spot an armed aim would take where the pointer points, how far around the character it
/// reaches, and what the spot has against it, in the game's words.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SpotAim {
    pub aimed: Aimed,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    pub reach: Meters,
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
    if !matches!(
        aimed.targeting(world.resource::<Content>()),
        Targeting::Spot { .. }
    ) {
        return None;
    }
    let at = render::pointed(world)?.surface?;
    Some(SpotAim {
        aimed,
        at,
        reach: reach_of(world, me, aimed)?,
        refused: spot_refusal(world, me, aimed, at)
            .map(|why| why.plain_words(world.resource::<Content>())),
    })
}

/// Quick cast: the skill is cast at what the pointer rests on, with no arming in between. Whether
/// anything fitting was there to cast at.
pub fn cast_at_pointer(world: &mut World, skill: SkillId) -> bool {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let aim = render::pointed(world).and_then(|pointed| aim_at(world, &pointed, def.target));
    let refusal = match (aim, session::me(world)) {
        (Some(CastAim::Point(spot)), Some(me)) if matches!(def.target, Targeting::Spot { .. }) => {
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
            let said = messages::of(&content)
                .skill
                .nothing_to_aim_at(Mention::Skill(skill).line(&content));
            notification::local_error(world, said);
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
    let shown = std::mem::take(&mut *world.resource_mut::<Aiming>());
    for entity in shown
        .area
        .into_iter()
        .chain(shown.reach)
        .chain(shown.targets.into_iter().map(|(_, mark)| mark))
    {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
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

/// Shows what is armed while it is aimed: the ground an area skill would cover under the pointer,
/// the ring of how far it reaches round the character, and a mark on each actor the area would
/// land on, each through the game's own vfx; and takes them away as the aim ends.
pub fn surface(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let armed = aiming(world);
    let area = armed.and_then(|aimed| aimed_area(world, aimed));
    let me = session::me(world);
    let reach = armed
        .zip(me)
        .and_then(|(aimed, me)| Some((reach_of(world, me, aimed)?, movement::position(world, me)?)))
        .filter(|(radius, _)| radius.0 > 0.0)
        .map(|(radius, from)| {
            PlacedShape::centred(Shape::Circle { radius }, from, Radians::default())
        });
    let shown = world.resource::<Aiming>().area;
    let area_flags = area.as_ref().map(|aimed| {
        VfxParams(vec![
            (HOSTILE.to_owned(), ParamValue::Bool(aimed.hostile)),
            (MELEE.to_owned(), ParamValue::Bool(aimed.melee)),
        ])
    });
    let site = area.as_ref().map(|aimed| VfxSite::Area(aimed.footprint));
    let area_shown = show(
        world,
        shown,
        AimVfx::SkillArea.id(&content),
        site.zip(area_flags),
    );
    world.resource_mut::<Aiming>().area = area_shown;
    let shown = world.resource::<Aiming>().reach;
    let reach_shown = show(
        world,
        shown,
        AimVfx::SkillReach.id(&content),
        reach.map(|reach| (VfxSite::Area(reach), VfxParams::default())),
    );
    world.resource_mut::<Aiming>().reach = reach_shown;
    let caught: Vec<Entity> = match (&area, me) {
        (Some(aimed), Some(me)) => world
            .query_filtered::<Entity, With<Actor>>()
            .iter(world)
            .filter(|actor| *actor != me)
            .filter(|actor| {
                movement::position(world, *actor).is_some_and(|at| aimed.footprint.contains(at))
            })
            .collect(),
        _ => Vec::new(),
    };
    let was = std::mem::take(&mut world.resource_mut::<Aiming>().targets);
    let mut marks = Vec::new();
    for (actor, mark) in &was {
        if !caught.contains(actor)
            && let Ok(mark) = world.get_entity_mut(*mark)
        {
            mark.despawn();
        }
    }
    for actor in caught {
        let kept = was
            .iter()
            .find(|(marked, _)| *marked == actor)
            .map(|(_, mark)| *mark);
        let hostile = area.as_ref().is_some_and(|aimed| aimed.hostile);
        let flags = VfxParams(vec![(HOSTILE.to_owned(), ParamValue::Bool(hostile))]);
        let target = AimVfx::SkillTarget.id(&content);
        if let Some(mark) = show(world, kept, target, Some((VfxSite::Actor(actor), flags))) {
            marks.push((actor, mark));
        }
    }
    world.resource_mut::<Aiming>().targets = marks;
}

/// Keeps `vfx` playing on `site` with `params`, as `shown` already does, or takes it away when
/// there is no site: what the client shows of its own accord, which no server sends.
fn show(
    world: &mut World,
    shown: Option<Entity>,
    vfx: crate::core::vfx::VfxId,
    site: Option<(VfxSite, VfxParams)>,
) -> Option<Entity> {
    let shown = shown.filter(|entity| world.get_entity(*entity).is_ok());
    let Some((site, params)) = site else {
        if let Some(entity) = shown {
            world.despawn(entity);
        }
        return None;
    };
    let started = shown
        .and_then(|entity| world.get::<VfxInstance>(entity))
        .map(|instance| instance.started)
        .or_else(|| time::server_now(world))?;
    let instance = VfxInstance {
        vfx,
        site,
        part: VfxPart::Whole,
        params,
        dealer: None,
        heading: None,
        started,
        until: None,
    };
    match shown {
        Some(entity) => {
            if world.get::<VfxInstance>(entity) != Some(&instance) {
                world.entity_mut(entity).insert(instance);
            }
            Some(entity)
        }
        None => Some(world.spawn(instance).id()),
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
            (Targeting::Spot { .. }, CastAim::Point(spot), Some(me)) => {
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

/// A spot is marked where the pointer points, washed gold where it will do and red with what it
/// has against it where it will not, under a prompt that names what is being picked. The ring of
/// what is armed's reach and the ground an area would cover are vfx, kept by `surface`.
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
    let spot = spot_aim(world);
    let mark = spot.as_ref().and_then(|spot| {
        let shape = Shape::Circle {
            radius: SPOT_RADIUS,
        };
        let ring = outline(
            world,
            &PlacedShape::centred(shape, spot.at, Radians::default()),
        )?;
        let said = spot.refused.clone().or_else(|| good_spot(world, me, aimed));
        Some((ring, spot.refused.is_some(), said))
    });
    let screen = gui.content_rect();
    let area = gui::world_area(world, "aim preview");
    area.fixed_pos(screen.min).show(&gui, |ui| {
        let painter = ui.painter();
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
    if matches!(
        aimed.targeting(world.resource::<Content>()),
        Targeting::Spot { .. }
    ) {
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
    let words = &messages::of(&content).skill;
    let (asks, click, leaves) = match aimed {
        Aimed::Landing => (words.pick_shore(), words.step_off(), words.stay_aboard()),
        Aimed::Skill { skill, .. } if mount::left_by(world, me, skill) => {
            (words.pick_shore(), words.step_off(), words.stay_aboard())
        }
        Aimed::Skill { skill, .. } if mount::called_by(world, skill).is_some_and(floats) => {
            (words.pick_water(), words.launch(), words.put_away())
        }
        Aimed::Skill { .. } => (words.pick_spot(), words.use_there(), words.put_away()),
    };
    let puts_away = (escape::dismisses(world) == Some(Dismissal::Aim))
        .then_some((InputAction::Dismiss, leaves));
    let hints = StyledLine::new(map::hints(
        [(InputAction::Interact, click)]
            .into_iter()
            .chain(puts_away),
    ));
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
    let words = &messages::of(content).skill;
    match aimed {
        Aimed::Skill { skill, .. } if !mount::left_by(world, me, skill) => {
            let mount = mount::called_by(world, skill)?;
            Some(words.mount_here(&mount.get(content).name))
        }
        Aimed::Skill { .. } | Aimed::Landing => Some(words.step_off_here().to_owned()),
    }
}

/// How far from the character what is armed may be aimed: a skill's range, none for a skill used
/// where the character stands, or for a rider leaving their mount how far the mount lets them
/// step.
fn reach_of(world: &World, me: Entity, aimed: Aimed) -> Option<Meters> {
    match aimed {
        Aimed::Skill { skill, rank } if !mount::left_by(world, me, skill) => {
            super::range_of(world, me, skill, rank)
        }
        Aimed::Skill { .. } | Aimed::Landing => {
            Some(mount::reach_of(world, me).unwrap_or_default())
        }
    }
}

/// What the spot `at` has against being taken by what is armed, in the game's words.
fn spot_refusal(world: &World, me: Entity, aimed: Aimed, at: Pos<Meters>) -> Option<LineText> {
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
        Targeting::Single { .. } => {
            let me = session::me(world)?;
            actor::target_at(world, pointed, None, |world, entity| {
                world.get::<Actor>(entity).is_some() && casting::fits(world, me, entity, targeting)
            })
            .map(CastAim::Actor)
        }
        Targeting::RangedArea { .. } | Targeting::MeleeArea { .. } => {
            Some(CastAim::Point(pointed.ground?))
        }
        Targeting::Spot { .. } => Some(CastAim::Point(pointed.surface?)),
        Targeting::Caster => None,
    }
}

/// Where an armed area skill would land, whether it harms what it takes, and whether it is swung
/// from where the character stands.
struct AimedArea {
    footprint: PlacedShape,
    hostile: bool,
    melee: bool,
}

fn aimed_area(world: &mut World, aimed: Aimed) -> Option<AimedArea> {
    let me = session::me(world)?;
    let content = world.resource::<Content>().clone();
    let Aimed::Skill { skill, rank } = aimed else {
        return None;
    };
    let def = skill.get(&content);
    let shape = def.shape_at(rank)?;
    let from = movement::position(world, me)?;
    let point = render::cursor_ground(world)?;
    Some(AimedArea {
        footprint: def.target.place(shape, from, point),
        hostile: def.target.hostile(),
        melee: matches!(def.target, Targeting::MeleeArea { .. }),
    })
}
