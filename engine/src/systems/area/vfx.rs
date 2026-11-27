use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::{AppReplicationExt, Replicated};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AreaObject, AreaTag, PropDef};
use crate::core::content::{Content, Fixture, Loading, SchemaMark};
use crate::core::geometry::{PlacedShape, Shape, facing_from};
use crate::core::math::{Pos, Rgba};
use crate::core::script::{
    Answer, Arrow, Compiler, EffectCall, EffectDef, EffectStep, Eval, Expr, FlowCx, FlowKey,
    Instances, NEXT, Node, Reach, ReachDef, ScriptDef, ScriptFault, ValueType, Vocabulary,
};
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::{Meters, Radians, Seconds};
use crate::core::vfx::{
    ParamKind, ParamValue, PathStop, SiteKind, VfxDef, VfxId, VfxInstance, VfxParams, VfxPart,
    VfxPlay, VfxSite, check_play, check_played,
};
use crate::systems::WorldArea;
use crate::systems::movement::{Position, position};
use crate::systems::roster::{Among, TickRoster};
use crate::systems::skill::{self, SKILL_RUNNER, SkillDef};

pub fn register(app: &mut App) {
    app.replicate::<VfxInstance>();
}

/// Refuses a layer that cannot play on the site a script's `vfx` node or a prop plays its vfx
/// on, and a prop's play that fills what its vfx doesn't declare.
pub fn check_sites(loading: &mut Loading) {
    let content = loading.content();
    for (id, script) in loading.rows::<ScriptDef>(&content) {
        for (key, node) in &script.nodes {
            if let Node::Effect(effect) = node
                && let Some(play) = effect.downcast_ref::<PlayVfx>()
            {
                let by = format!("{}, node {key}", id.path(&content));
                check_played(loading, play.vfx, play.sites(), &by);
            }
        }
    }
    for (id, prop) in loading.rows::<PropDef>(&content) {
        let mut check = crate::core::content::RowCheck::default();
        for (index, play) in prop.effects.iter().enumerate() {
            check_played(loading, play.vfx, &[SiteKind::Object], id.path(&content));
            check_play(&content, play, &format!("/effects/{index}"), &mut check);
        }
        loading.row_errors(id, check);
    }
}

/// How long an instance outlives its last layer, so a client sees the end before the entity goes.
const LINGER: Seconds = Seconds(0.25);

/// How a vfx is dealt, beyond its site: by whom, which heads a vfx on an actor from where the
/// dealer stands and lets a layer leave from the dealer's hand, and `struck`, the actors a cast
/// hit, which an area's `on_actors_inside` then reaches instead of everyone standing in it.
#[derive(Default)]
pub struct Delivery {
    pub dealer: Option<Entity>,
    pub struck: Option<Vec<Entity>>,
}

/// The site a thing is to a vfx played on it: an object of the area as one, anyone else as an
/// actor.
pub fn site_on(world: &World, thing: Entity) -> VfxSite {
    match world.get::<AreaObject>(thing) {
        Some(_) => VfxSite::Object(thing),
        None => VfxSite::Actor(thing),
    }
}

/// Plays the vfx a fixture is on `site`, shown to everyone nearby.
pub fn play_fixture<F: Fixture<Row = VfxDef>>(world: &mut World, fixture: F, site: VfxSite) {
    let content = world.resource::<Content>().clone();
    show(world, &VfxPlay::of(fixture.id(&content)), site);
}

pub fn show(world: &mut World, play: &VfxPlay, site: VfxSite) -> Option<Entity> {
    spawn(world, play, site, Delivery::default())
}

/// Plays a vfx on a site over and over, until whoever shows it removes it.
pub fn show_lasting(world: &mut World, play: &VfxPlay, site: VfxSite) -> Option<Entity> {
    let shown = show(world, play, site)?;
    world.get_mut::<VfxInstance>(shown)?.until = None;
    Some(shown)
}

/// Plays a vfx on a site everyone nearby sees.
pub fn spawn(
    world: &mut World,
    play: &VfxPlay,
    site: VfxSite,
    delivery: Delivery,
) -> Option<Entity> {
    let at = match &site {
        VfxSite::Actor(entity) | VfxSite::Object(entity) => position(world, *entity)?,
        VfxSite::Area(footprint) => footprint.at,
        VfxSite::Path(stops) => stop_at(world, stops.first()?)?,
    };
    let from = delivery.dealer.and_then(|dealer| position(world, dealer));
    let heading = match &site {
        VfxSite::Area(footprint) => Some(footprint.facing),
        VfxSite::Path(stops) => Some(facing_from(at, stop_at(world, stops.last()?)?)),
        _ => from
            .filter(|from| *from != at)
            .map(|from| facing_from(from, at)),
    };
    let params = play.params();
    if let VfxSite::Area(footprint) = &site
        && let Some(inner) = play.vfx.get(world.resource::<Content>()).on_actors_inside
    {
        for (actor, stood) in reached(world, footprint, delivery.struck.as_deref()) {
            let instance = VfxInstance {
                part: VfxPart::Whole,
                heading: Some(footprint.facing),
                ..instance(world, inner, VfxSite::Actor(actor), params.clone())
            };
            spawn_at(world, instance, stood);
        }
    }
    let instance = VfxInstance {
        part: play.part.clone(),
        dealer: delivery.dealer,
        heading,
        ..instance(world, play.vfx, site, params)
    };
    Some(spawn_at(world, instance, at))
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

/// Ends every vfx whose last layer has ended, and goes on with every script that waited for a vfx
/// it played to land.
pub fn expire(world: &mut World, instances: &mut QueryState<(Entity, &VfxInstance)>) {
    let now = world.resource::<WallClock>().now;
    let landed: Vec<(Entity, FlowKey)> = world
        .query::<(Entity, &Landing)>()
        .iter(world)
        .filter(|(_, landing)| landing.at <= now)
        .map(|(entity, landing)| (entity, landing.flow))
        .collect();
    for (entity, flow) in landed {
        world.despawn(entity);
        world
            .resource_mut::<Instances>()
            .answer(flow, Answer::Output(NEXT));
    }
    let done: Vec<Entity> = instances
        .iter(world)
        .filter(|(_, instance)| {
            instance
                .until
                .is_some_and(|until| until.after(LINGER) <= now)
        })
        .map(|(entity, _)| entity)
        .collect();
    for entity in done {
        world.despawn(entity);
    }
}

/// Keeps every vfx that plays on a thing where the thing stands, so whoever comes within sight
/// of the thing sees what plays on it.
pub fn follow(world: &mut World, instances: &mut QueryState<(Entity, &VfxInstance, &Position)>) {
    let moved: Vec<(Entity, Pos<Meters>)> = instances
        .iter(world)
        .filter_map(|(entity, instance, stood)| {
            let (VfxSite::Actor(thing) | VfxSite::Object(thing)) = &instance.site else {
                return None;
            };
            let at = position(world, *thing)?;
            (at != stood.pos).then_some((entity, at))
        })
        .collect();
    for (entity, at) in moved {
        if let Some(mut stood) = world.get_mut::<Position>(entity) {
            stood.pos = at;
        }
    }
}

/// Plays a vfx on whatever `on` names: each actor of a set, a point or an area; or on a path from
/// the subject through the actors named, each nearest to the last. The subject deals it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("vfx"))]
pub struct PlayVfx {
    pub vfx: VfxId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub on: Option<Expr>,
    #[serde(default, skip_serializing_if = "VfxTarget::is_place")]
    pub site: VfxTarget,
    /// The part of each thing it plays on that it covers.
    #[serde(
        default,
        skip_serializing_if = "crate::core::content::serial::is_default"
    )]
    pub part: VfxPart,
    /// Keyed by the vfx's declared parameters, each given or left to the layers' own values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("vfx"))]
    pub params: BTreeMap<String, VfxArg>,
    /// Goes on once the vfx lands: when its last layer that travels its path arrives, or, with
    /// none travelling, when it ends. A shot lands before the blow it deals.
    #[serde(
        default,
        skip_serializing_if = "crate::core::content::serial::is_false"
    )]
    pub wait: bool,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// A script's value for one of a vfx's parameters: a colour, as it is given, or what an
/// expression yields for a number or a flag.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VfxArg {
    Color(Rgba),
    Expr(Expr),
}

#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VfxTarget {
    /// On each actor `on` names, or on the area or point it names.
    #[default]
    Place,
    /// Along a path from the subject: through every actor `on` names, nearest first, or to the
    /// point it names, or the middle of the area.
    Path,
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
        }
    }
}

/// What a condition yields when its own shape says, before any script compiles it.
fn value_type(condition: &Expr) -> Option<ValueType> {
    match condition {
        Expr::Query(query) => query.static_type(),
        _ => None,
    }
}

impl EffectDef for PlayVfx {
    fn needs_subject(&self) -> bool {
        self.on.is_none()
    }
    crate::outputs!(NEXT => next);
    fn pauses(&self) -> bool {
        self.wait
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let on = match &self.on {
            Some(expr) => {
                let (eval, ty) = cx.typed(expr)?;
                match ty {
                    ValueType::Actor | ValueType::Actors | ValueType::Point | ValueType::Area => {
                        eval
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
        for (name, arg) in &self.params {
            let filler = match (def.declares(name), arg) {
                (Some(ParamKind::Number), VfxArg::Expr(expr)) => {
                    ParamFiller::Number(cx.expr(expr, ValueType::Number)?)
                }
                (Some(ParamKind::Bool), VfxArg::Expr(expr)) => {
                    ParamFiller::Flag(cx.expr(expr, ValueType::Boolean)?)
                }
                (Some(ParamKind::Color), VfxArg::Color(color)) => ParamFiller::Color(*color),
                (Some(kind), _) => {
                    return Err(ScriptFault::new(
                        "wrong parameter kind",
                        format!("{name} takes a {}", kind.label()),
                    ));
                }
                (None, _) => {
                    return Err(ScriptFault::new(
                        "unknown parameter",
                        format!("{} declares no {name}", def.name),
                    ));
                }
            };
            fillers.push((name.clone(), filler));
        }
        let (vfx, target, part, wait) = (self.vfx, self.site.clone(), self.part.clone(), self.wait);
        let lands = def.lands();
        let cast = cx
            .owner_row()
            .filter(|row| row.kind == SKILL_RUNNER)
            .map(|row| row.id::<SkillDef>());
        Ok(Box::new(move |world, flow, cx| {
            let play = VfxPlay {
                vfx,
                part: part.clone(),
                fill: fillers
                    .iter()
                    .map(|(name, filler)| (name.clone(), filler.fill(world, cx)))
                    .collect(),
            };
            let value = on(world, cx);
            let dealer = cx.subject;
            let struck = cast.map(|skill| skill::targets(world, cx, skill));
            let mut played = false;
            match &target {
                VfxTarget::Place => {
                    for actor in value.as_actors() {
                        if world.get_entity(actor).is_err() {
                            continue;
                        }
                        let site = site_on(world, actor);
                        let dealt = Delivery {
                            dealer,
                            struck: None,
                        };
                        played |= spawn(world, &play, site, dealt).is_some();
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
                        let dealt = Delivery { dealer, struck };
                        played |= spawn(world, &play, VfxSite::Area(footprint), dealt).is_some();
                    }
                }
                VfxTarget::Path => {
                    let end = value.as_area().map(|area| area.middle());
                    let stops = match end.or_else(|| value.as_point()) {
                        Some(point) => cx
                            .subject
                            .map(PathStop::Actor)
                            .into_iter()
                            .chain([PathStop::Point(point)])
                            .collect(),
                        None => chained(world, cx.subject, &value.as_actors()),
                    };
                    if stops.len() >= 2 {
                        let dealt = Delivery {
                            dealer,
                            struck: None,
                        };
                        played |= spawn(world, &play, VfxSite::Path(stops), dealt).is_some();
                    }
                }
            }
            if !(wait && played) {
                return EffectStep::Done;
            }
            let at = world.resource::<WallClock>().now.after(lands);
            world.spawn(Landing { flow, at });
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(ShowsVfx)]
    }
}

impl VfxTarget {
    fn is_place(&self) -> bool {
        *self == VfxTarget::Place
    }
}

/// A script waiting for a vfx it played to land, and when it does.
#[derive(Component)]
struct Landing {
    flow: FlowKey,
    at: UnixMillis,
}

/// A script's value for a parameter, read as the kind the vfx declares.
enum ParamFiller {
    Number(Eval),
    Flag(Eval),
    Color(Rgba),
}

impl ParamFiller {
    fn fill(&self, world: &World, cx: &FlowCx) -> ParamValue {
        match self {
            ParamFiller::Number(eval) => ParamValue::Number(eval(world, cx).as_num() as f32),
            ParamFiller::Flag(eval) => ParamValue::Bool(eval(world, cx).truthy()),
            ParamFiller::Color(color) => ParamValue::Color(*color),
        }
    }
}

/// Where a path through actors runs: from the subject, when there is one, to the actor nearest it,
/// then the one nearest that, until every actor is passed.
fn chained(world: &World, from: Option<Entity>, actors: &[Entity]) -> Vec<PathStop> {
    let mut left: Vec<(Entity, Pos<Meters>)> = actors
        .iter()
        .filter(|actor| Some(**actor) != from)
        .filter_map(|actor| Some((*actor, position(world, *actor)?)))
        .collect();
    let start = match from.and_then(|from| Some((from, position(world, from)?))) {
        Some(start) => start,
        None if left.is_empty() => return Vec::new(),
        None => left.remove(0),
    };
    let mut stops = vec![PathStop::Actor(start.0)];
    let mut here = start.1;
    while !left.is_empty() {
        let (index, _) = left
            .iter()
            .enumerate()
            .min_by(|(_, (_, a)), (_, (_, b))| a.distance_to(here).total_cmp(&b.distance_to(here)))
            .expect("left is not empty");
        let (actor, at) = left.swap_remove(index);
        here = at;
        stops.push(PathStop::Actor(actor));
    }
    stops
}

/// Where a stop of a path stands now.
fn stop_at(world: &World, stop: &PathStop) -> Option<Pos<Meters>> {
    match stop {
        PathStop::Actor(actor) => position(world, *actor),
        PathStop::Point(at) => Some(*at),
    }
}

/// A vfx starting now on `site`, ending as its last layer does, dealt by nobody and headed east.
fn instance(world: &World, vfx: VfxId, site: VfxSite, params: VfxParams) -> VfxInstance {
    let now = world.resource::<WallClock>().now;
    VfxInstance {
        vfx,
        site,
        part: VfxPart::Whole,
        params,
        dealer: None,
        heading: None,
        started: now,
        until: Some(now.after(vfx.get(world.resource::<Content>()).lasts())),
    }
}

fn spawn_at(world: &mut World, instance: VfxInstance, at: Pos<Meters>) -> Entity {
    let area = world.resource::<WorldArea>().0;
    world
        .spawn((Replicated, Position::at(at), AreaTag { area }, instance))
        .id()
}

/// A script that may show a visual effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowsVfx;

impl ReachDef for ShowsVfx {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.effect::<PlayVfx>("vfx");
}
