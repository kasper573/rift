pub mod fixture;

pub use fixture::{VfxFixture, VfxShaderFixture};

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{AppRuleExt, Replicated};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::core::content::{Content, Fixture, Loading, RowCheck};
use crate::core::geometry::{PlacedShape, Radians, Shape, facing_from};
use crate::core::math::Pos;
use crate::core::script::Node;
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, Eval, Expr, FlowCx, NEXT, ScriptFault, ValueType,
};
use crate::core::time::{Seconds, WallClock};
use crate::core::units::Meters;
use crate::core::vfx::{
    ParamKind, ParamValue, SiteKind, VfxDef, VfxId, VfxInstance, VfxParams, VfxSite, WidgetName,
};
use crate::systems::WorldArea;
use crate::systems::area::{AreaObject, AreaTag};
use crate::systems::movement::{Position, position};
use crate::systems::player::PlayedBy;
use crate::systems::script::effect::RiftEffect;
use crate::systems::script::query::{Among, RiftQueryKind, TickRoster};
use crate::systems::script::{Condition, EffectCall, Reach, RiftScripting, RunnerKind, ScriptDef};
use crate::systems::skill::{self, SkillDef};
use crate::systems::status::StatusEffectDef;
use crate::systems::visibility::Presence;

/// How long an instance outlives its last layer, so a client sees the end before the entity goes.
const LINGER: Seconds = Seconds(0.25);

pub fn register(app: &mut App) {
    app.replicate::<VfxInstance>();
}

/// Every vfx plays on sites its layers' stages take: a script names the site it plays a vfx on, the
/// engine's fixtures and a status effect's vfx play on actors.
pub async fn check_sites(loading: &mut Loading) {
    let content = loading.content();
    let mut played: Vec<(VfxId, &[SiteKind], String)> = Vec::new();
    for (id, script) in loading.rows::<ScriptDef>(&content) {
        for (key, node) in &script.nodes {
            if let Node::Effect(RiftEffect::Vfx(play)) = node {
                let by = format!("{}, node {key}", id.path(&content));
                played.push((play.vfx, play.sites(), by));
            }
        }
    }
    for fixture in VfxFixture::VARIANTS {
        if let Some(vfx) = content.by_path::<VfxDef>(&fixture.path()) {
            let by = format!("the engine, as the {} fixture", fixture.name());
            played.push((vfx, fixture.sites(), by));
        }
    }
    for (id, status) in loading.rows::<StatusEffectDef>(&content) {
        let phases = [
            status.vfx.on_activate,
            status.vfx.during,
            status.vfx.on_expire,
        ];
        for vfx in phases.into_iter().flatten() {
            played.push((vfx, &[SiteKind::Actor], id.path(&content).to_owned()));
        }
    }
    for (vfx, sites, by) in played {
        let mut check = RowCheck::default();
        vfx.get(&content)
            .check_sites(&content, sites, &by, &mut check);
        loading.row_errors(vfx, check);
    }
}

/// How a vfx is dealt, beyond its site: `from` where the blow came, which heads a vfx on an actor
/// from whoever dealt it, and `struck`, the actors a cast hit, which an area's `on_actors_inside`
/// then reaches instead of everyone standing in it.
#[derive(Default)]
pub struct Delivery {
    pub from: Option<Pos<Meters>>,
    pub struck: Option<Vec<Entity>>,
}

pub fn show(world: &mut World, vfx: VfxId, site: VfxSite) -> Option<Entity> {
    spawn(world, vfx, site, VfxParams::default(), Delivery::default())
}

/// Plays a vfx on a site everyone nearby sees.
pub fn spawn(
    world: &mut World,
    vfx: VfxId,
    site: VfxSite,
    params: VfxParams,
    delivery: Delivery,
) -> Option<Entity> {
    let at = match &site {
        VfxSite::Actor(entity) | VfxSite::Object(entity) => position(world, *entity)?,
        VfxSite::Area(footprint) => footprint.at,
        VfxSite::Path(points) => *points.first()?,
        VfxSite::Screen | VfxSite::Widget(_) => return None,
    };
    let heading = match &site {
        VfxSite::Area(footprint) => footprint.facing,
        VfxSite::Path(points) => facing_from(*points.first()?, *points.last()?),
        _ => delivery
            .from
            .filter(|from| *from != at)
            .map_or(Radians::default(), |from| facing_from(from, at)),
    };
    if let VfxSite::Area(footprint) = &site
        && let Some(inner) = vfx.get(world.resource::<Content>()).on_actors_inside
    {
        for (actor, stood) in reached(world, footprint, delivery.struck.as_deref()) {
            spawn_at(
                world,
                inner,
                VfxSite::Actor(actor),
                params.clone(),
                stood,
                footprint.facing,
                None,
            );
        }
    }
    Some(spawn_at(world, vfx, site, params, at, heading, None))
}

/// The living actors of this world's area an area vfx reaches, each where it stands: the `struck`
/// ones standing inside `footprint`, or everyone standing in it.
fn reached(
    world: &World,
    footprint: &PlacedShape,
    struck: Option<&[Entity]>,
) -> Vec<(Entity, Pos<Meters>)> {
    let area = world.resource::<WorldArea>().0;
    let mut reached: Vec<(Entity, Pos<Meters>)> = world
        .get_resource::<TickRoster>()
        .into_iter()
        .flat_map(|roster| roster.near(Among::Anyone, footprint.at, footprint.reach()))
        .filter(|listed| listed.area == area && !listed.dead && footprint.contains(listed.at))
        .filter(|listed| struck.is_none_or(|struck| struck.contains(&listed.entity)))
        .map(|listed| (listed.entity, listed.at))
        .collect();
    reached.sort_unstable_by_key(|(entity, _)| *entity);
    reached
}

/// Plays a vfx on one player's screen or interface, shown to that player alone.
pub fn spawn_for(
    world: &mut World,
    player: Entity,
    vfx: VfxId,
    site: VfxSite,
    params: VfxParams,
) -> Option<Entity> {
    let client = world.get::<PlayedBy>(player)?.client;
    let at = position(world, player)?;
    Some(spawn_at(
        world,
        vfx,
        site,
        params,
        at,
        Radians::default(),
        Some(Presence::For(client)),
    ))
}

pub fn expire(world: &mut World, instances: &mut QueryState<(Entity, &VfxInstance)>) {
    let now = world.resource::<WallClock>().now;
    let done: Vec<Entity> = instances
        .iter(world)
        .filter(|(_, instance)| instance.until.after(LINGER) <= now)
        .map(|(entity, _)| entity)
        .collect();
    for entity in done {
        world.despawn(entity);
    }
}

/// Plays a vfx on whatever `on` names: each actor of a set, a point or an area; on a path from the
/// subject through the actors named, each nearest to the last; or, for a screen or widget site, on
/// the screen of each character named.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(extend("x-system" = "vfx"))]
pub struct PlayVfx {
    pub vfx: VfxId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub on: Option<Condition>,
    #[serde(default, skip_serializing_if = "VfxTarget::is_place")]
    pub site: VfxTarget,
    /// Keyed by the vfx's declared parameters, each given or left to the layers' own values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("x-params-of" = "vfx"))]
    pub params: BTreeMap<String, Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxTarget {
    #[default]
    Place,
    Path,
    Screen,
    Widget {
        name: String,
    },
}

impl PlayVfx {
    /// The kinds of site the node plays its vfx on: a place is the actors `on` names, or the area
    /// or point it names; without `on`, the subject.
    pub fn sites(&self) -> &'static [SiteKind] {
        match &self.site {
            VfxTarget::Place => match self.on.as_ref().map(value_type) {
                None | Some(Some(ValueType::Actor | ValueType::Actors)) => {
                    &[SiteKind::Actor, SiteKind::Object]
                }
                Some(Some(ValueType::Point | ValueType::Area)) => &[SiteKind::Area],
                Some(_) => &[SiteKind::Actor, SiteKind::Object, SiteKind::Area],
            },
            VfxTarget::Path => &[SiteKind::Path],
            VfxTarget::Screen => &[SiteKind::Screen],
            VfxTarget::Widget { .. } => &[SiteKind::Text],
        }
    }
}

/// What a condition yields when its own shape says, before any script compiles it.
fn value_type(condition: &Condition) -> Option<ValueType> {
    match condition {
        Expr::Query(query) => RiftQueryKind::from(query).value_type(),
        _ => None,
    }
}

impl EffectDef<RiftScripting> for PlayVfx {
    fn needs_subject(&self) -> bool {
        self.on.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let on = match &self.on {
            Some(expr) => {
                let (eval, ty) = cx.typed(expr)?;
                match ty {
                    ValueType::Actor | ValueType::Actors => eval,
                    ValueType::Point | ValueType::Area if self.site != VfxTarget::Path => eval,
                    ValueType::Point | ValueType::Area => {
                        return Err(ScriptFault::new(
                            "wrong type",
                            format!("on is {}; a path runs through actors", ty.name()),
                        ));
                    }
                    other => {
                        return Err(ScriptFault::new(
                            "wrong type",
                            format!(
                                "on is {}, not an actor, a set, a point or an area",
                                other.name()
                            ),
                        ));
                    }
                }
            }
            None => cx.actor(None)?,
        };
        let def = self.vfx.get(cx.content);
        let mut fillers: Vec<(String, ParamFiller)> = Vec::new();
        for (name, expr) in &self.params {
            let filler = match def.declares(name) {
                Some(ParamKind::Number) => ParamFiller::Number(cx.expr(expr, ValueType::Number)?),
                Some(ParamKind::Bool) => ParamFiller::Flag(cx.expr(expr, ValueType::Boolean)?),
                Some(ParamKind::Color) => {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("{name} is a color, which a script cannot fill"),
                    ));
                }
                None => {
                    return Err(ScriptFault::new(
                        "unknown parameter",
                        format!("{} declares no {name}", def.name),
                    ));
                }
            };
            fillers.push((name.clone(), filler));
        }
        let (vfx, target) = (self.vfx, self.site.clone());
        let cast = cx
            .owner_row()
            .filter(|row| row.kind == RunnerKind::Skill)
            .map(|row| row.id::<SkillDef>());
        Ok(Box::new(move |world, _, cx| {
            let filled = VfxParams(
                fillers
                    .iter()
                    .map(|(name, filler)| (name.clone(), filler.fill(world, cx)))
                    .collect(),
            );
            let value = on(world, cx);
            let from = cx.subject.and_then(|subject| position(world, subject));
            let struck = cast.map(|skill| skill::targets(world, cx, skill));
            match &target {
                VfxTarget::Place => {
                    for actor in value.as_actors() {
                        if world.get_entity(actor).is_err() {
                            continue;
                        }
                        let site = if world.get::<AreaObject>(actor).is_some() {
                            VfxSite::Object(actor)
                        } else {
                            VfxSite::Actor(actor)
                        };
                        let dealt = Delivery { from, struck: None };
                        spawn(world, vfx, site, filled.clone(), dealt);
                    }
                    let footprint = value.as_area().or_else(|| {
                        value.as_point().map(|point| PlacedShape {
                            at: point,
                            facing: Radians::default(),
                            shape: Shape::Circle {
                                radius: Meters(0.5),
                            },
                        })
                    });
                    if let Some(footprint) = footprint {
                        let dealt = Delivery { from, struck };
                        spawn(world, vfx, VfxSite::Area(footprint), filled.clone(), dealt);
                    }
                }
                VfxTarget::Path => {
                    let points = chained(world, cx.subject, &value.as_actors());
                    if points.len() >= 2 {
                        let dealt = Delivery::default();
                        spawn(world, vfx, VfxSite::Path(points), filled.clone(), dealt);
                    }
                }
                VfxTarget::Screen => {
                    for actor in value.as_actors() {
                        spawn_for(world, actor, vfx, VfxSite::Screen, filled.clone());
                    }
                }
                VfxTarget::Widget { name } => {
                    for actor in value.as_actors() {
                        let site = VfxSite::Widget(WidgetName(name.clone()));
                        spawn_for(world, actor, vfx, site, filled.clone());
                    }
                }
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::ShowsVfx]
    }
}

impl VfxTarget {
    fn is_place(&self) -> bool {
        *self == VfxTarget::Place
    }
}

/// A script's value for a parameter, read as the kind the vfx declares.
enum ParamFiller {
    Number(Eval),
    Flag(Eval),
}

impl ParamFiller {
    fn fill(&self, world: &World, cx: &FlowCx) -> ParamValue {
        match self {
            ParamFiller::Number(eval) => ParamValue::Number(eval(world, cx).as_num() as f32),
            ParamFiller::Flag(eval) => ParamValue::Bool(eval(world, cx).truthy()),
        }
    }
}

/// Where a path through actors runs: from the subject, when there is one, to the actor nearest it,
/// then the one nearest that, until every actor is passed.
fn chained(world: &World, from: Option<Entity>, actors: &[Entity]) -> Vec<Pos<Meters>> {
    let mut left: Vec<Pos<Meters>> = actors
        .iter()
        .filter_map(|actor| position(world, *actor))
        .collect();
    let start = match from.and_then(|from| position(world, from)) {
        Some(at) => at,
        None if left.is_empty() => return Vec::new(),
        None => left.remove(0),
    };
    let mut points = vec![start];
    let mut here = start;
    while !left.is_empty() {
        let (index, _) = left
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.distance_to(here).total_cmp(&b.distance_to(here)))
            .expect("left is not empty");
        here = left.swap_remove(index);
        points.push(here);
    }
    points
}

fn spawn_at(
    world: &mut World,
    vfx: VfxId,
    site: VfxSite,
    params: VfxParams,
    at: Pos<Meters>,
    heading: Radians,
    presence: Option<Presence>,
) -> Entity {
    let content = world.resource::<Content>().clone();
    let now = world.resource::<WallClock>().now;
    let area = world.resource::<WorldArea>().0;
    let instance = VfxInstance {
        vfx,
        site,
        params,
        heading,
        started: now,
        until: now.after(vfx.get(&content).lasts()),
    };
    let entity = world
        .spawn((Replicated, Position::at(at), AreaTag { area }, instance))
        .id();
    if let Some(presence) = presence {
        world.entity_mut(entity).insert(presence);
    }
    entity
}
