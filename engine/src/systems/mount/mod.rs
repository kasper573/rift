//! Mounts: what a player rides. A mount is no creature of its own: it is its rider's ride, drawn
//! under them for as long as they sit on it and gone when they leave.

pub mod effect;
pub mod plate;
pub mod render;
pub mod seat;

use std::collections::BTreeSet;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, IntoStaticStr, VariantArray};

use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck,
};
use crate::core::math::Pos;
use crate::core::tick::{Cached, Planner};
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::actor::model::{ActorModel, ModelId};
use crate::systems::actor::skeleton::{self, Declared, JointName, MoveName, SkeletonDef};
use crate::systems::actor::{Appearance, Hitbox};
use crate::systems::area::{self, AreaId, AreaTag, HeldMesh};
use crate::systems::history::HistoryTopic;
use crate::systems::movement;
use crate::systems::notification::{self, Notification};
use crate::systems::region;
use crate::systems::script::{
    EffectReach, OwnerRow, Place, RiftScripts, RunnerKind, ScriptModule, ScriptedRow,
};
use crate::systems::skill::casting::CastAim;
use crate::systems::skill::{self, SkillDef, SkillId, Targeting};
use crate::systems::stat;
use crate::systems::status::aspect;
use crate::systems::text::LineText;

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate::<Riding>()
        .add_client_message::<seat::MountRequest>();
}

pub type MountId = ContentId<MountDef>;

/// Something a player rides. Whatever it changes about its rider comes from its script, which
/// runs on the rider for as long as they ride it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountDef {
    pub name: String,
    pub look: ModelId,
    pub travels: Travels,
    /// How far from the mount its rider may step off it.
    pub reach: Meters,
    /// Where its rider sits, and what they may do from there.
    pub rider: MountSeat,
    /// What comes of a rider doing what their seat does not let them.
    pub barred: BarredAct,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<MountModule>,
}

/// How a mount travels, and so where it may go.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum_discriminants(
    name(MountKind),
    derive(JsonSchema, Serialize, Deserialize, Hash, IntoStaticStr, VariantArray),
    serde(rename_all = "snake_case"),
    strum(serialize_all = "snake_case")
)]
pub enum Travels {
    /// Over the ground a walker walks.
    Riding,
    /// Over everything within the area's bounds. Its riders are drawn `height` over the land or
    /// the water below, their drawn height following what lies below over `eases`.
    Flying { height: Meters, eases: Seconds },
    /// Wherever water stands at least `draught` deep.
    Floating { draught: Meters },
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountSeat {
    /// The joint of the mount's model its rider sits on: their feet at the joint, facing the way
    /// it faces.
    pub joint: JointName,
    /// The move its rider makes over and over for as long as they sit there, in place of
    /// standing and walking.
    pub rides: MoveName,
    /// What its rider may do from it. Anything else is barred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub may: Vec<RiderAct>,
}

/// What a seat may let its rider do. A rider may always steer, chat, emote, open their menus and
/// step off.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RiderAct {
    /// Casting skills and attacking.
    Skills,
    /// Using items.
    Items,
    /// Working a resource.
    Gather,
    /// Talking, trading, using a bench or a door.
    Interact,
    /// Picking up what lies on the ground.
    PickUp,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BarredAct {
    /// Nothing happens but the rider being told.
    Refuse,
    /// The rider steps off where they stand and then does it. Where they cannot stand, it is
    /// refused.
    StepOff,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MountModule {
    Script(ScriptModule),
}

impl ContentFile for MountDef {
    const TABLE: &'static str = "mount";

    fn fresh(near: &Nearby) -> Result<MountDef, String> {
        const JOINTED: &str = "a model whose skeleton names a hand's joint";
        let look: ModelId = near.file()?;
        let skeleton: SkeletonDef = near.read(near.read(look)?.skeleton)?;
        let joint = skeleton.hands.first().map(|hand| hand.joint.clone());
        Ok(MountDef {
            name: near.name(),
            look,
            travels: Travels::Riding,
            reach: Meters(3.0),
            rider: MountSeat {
                joint: joint.ok_or_else(|| format!("it needs {JOINTED}: make one first"))?,
                rides: skeleton::nearest_move(near)?,
                may: Vec::new(),
            },
            barred: BarredAct::Refuse,
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.at_least("/reach", self.reach.0, 0.0, "meters");
        match self.travels {
            Travels::Riding => {}
            Travels::Flying { height, eases } => {
                check.at_least("/travels/height", height.0, 0.0, "meters");
                check.at_least("/travels/eases", eases.0, 0.0, "seconds");
            }
            Travels::Floating { draught } => {
                check.at_least("/travels/draught", draught.0, 0.0, "meters");
            }
        }
    }
}

impl NamedContent for MountDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for MountDef {
    type Module = MountModule;

    fn modules(&self) -> &[MountModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<MountModule> {
        &mut self.modules
    }

    fn rule(_: &MountModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for MountDef {
    const OWNER: RunnerKind = RunnerKind::Mount;

    fn script(module: &MountModule) -> Option<&ScriptModule> {
        match module {
            MountModule::Script(script) => Some(script),
        }
    }
}

/// On every rider: the mount they ride. Both the server and every client pick a rider's mesh and
/// pace from it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Riding {
    pub mount: MountId,
}

/// The moves `mounts` are ridden with: made over and over for as long as a rider sits, where any
/// other move is made once and lands.
pub fn sat_moves<'a>(mounts: impl IntoIterator<Item = &'a MountDef>) -> BTreeSet<MoveName> {
    mounts
        .into_iter()
        .map(|mount| mount.rider.rides.clone())
        .collect()
}

/// Whether `entity` sits on a mount.
pub fn rides(world: &World, entity: Entity) -> bool {
    world.get::<Riding>(entity).is_some()
}

/// How the mount `entity` rides travels: nothing for whoever is on foot.
pub fn travels(content: &Content, world: &World, entity: Entity) -> Option<Travels> {
    world
        .get::<Riding>(entity)
        .map(|riding| riding.mount.get(content).travels)
}

/// Whether what `entity` rides carries them over water and steep ground alike: a mount that
/// flies or floats wades through nothing.
pub fn lifted(content: &Content, world: &World, entity: Entity) -> bool {
    matches!(
        travels(content, world, entity),
        Some(Travels::Flying { .. } | Travels::Floating { .. })
    )
}

/// How far from their mount `rider` may step off it.
pub fn reach_of(world: &World, rider: Entity) -> Option<Meters> {
    let riding = world.get::<Riding>(rider)?;
    Some(riding.mount.get(world.resource::<Content>()).reach)
}

/// Whether what `rider` rides is on its way somewhere.
pub fn under_way(world: &World, rider: Entity) -> bool {
    rides(world, rider) && movement::moving(world, rider)
}

/// Every depth of water some mount of the game floats at, the shallowest first.
pub fn draughts(content: &Content) -> Vec<Meters> {
    let mut draughts: Vec<Meters> = content
        .table::<MountDef>()
        .rows()
        .filter_map(|mount| match mount.travels {
            Travels::Floating { draught } => Some(draught),
            Travels::Riding | Travels::Flying { .. } => None,
        })
        .collect();
    draughts.sort_by(|a, b| a.0.total_cmp(&b.0));
    draughts.dedup();
    draughts
}

/// The mesh a mount that travels as `travels` goes on in `area`, as far as the tiles held: none
/// in an area that bars its kind.
pub fn mesh_in(content: &Content, area: AreaId, travels: Travels) -> Option<HeldMesh> {
    if !welcome_in(content, area, travels) {
        return None;
    }
    let area = area::load(content, area);
    match travels {
        Travels::Riding => Some(area.mesh()),
        Travels::Flying { .. } => Some(area.mesh_for(false, true)),
        Travels::Floating { draught } => area.water_mesh(draught),
    }
}

/// The skills that call `mount`: those whose script's `mount` effect names it.
pub fn callers(content: &Content, mount: MountId) -> Vec<SkillId> {
    let scripts = content.built::<RiftScripts>();
    content
        .ids::<SkillDef>()
        .filter(|skill| {
            let reaches = &scripts.may_do(OwnerRow::of_row(*skill)).reaches;
            reaches.contains(&EffectReach::Mounts(mount))
        })
        .collect()
}

/// The mount a skill calls: the one its script's `mount` effect names.
pub fn called_by(world: &World, skill: SkillId) -> Option<MountId> {
    let scripts = world.resource::<RiftScripts>();
    let reaches = &scripts.may_do(OwnerRow::of_row(skill)).reaches;
    reaches.iter().find_map(|reach| match reach {
        EffectReach::Mounts(mount) => Some(*mount),
        _ => None,
    })
}

/// Whether using `skill` is how `rider` leaves the mount they ride: the skill that called it.
pub fn left_by(world: &World, rider: Entity, skill: SkillId) -> bool {
    called_by(world, skill).is_some_and(|mount| rides_on(world, rider, mount))
}

/// Whether `rider` rides a `mount`.
pub fn rides_on(world: &World, rider: Entity, mount: MountId) -> bool {
    world
        .get::<Riding>(rider)
        .is_some_and(|riding| riding.mount == mount)
}

/// Why `actor` could not use `skill` from where they are, whatever they aim it at, in the game's
/// words: their seat does not let them, or the mount it calls may not be called here. Nothing for
/// the skill that called the mount they ride, which is how they leave it.
pub fn shut(world: &World, actor: Entity, skill: SkillId) -> Option<String> {
    if left_by(world, actor, skill) {
        return None;
    }
    let act = if skill::carried_as_item(world, actor, skill) {
        RiderAct::Items
    } else {
        RiderAct::Skills
    };
    seat::barred(world, actor, act)
        .or_else(|| called_by(world, skill).and_then(|mount| unwelcome(world, actor, mount)))
}

/// Why `caster` may not call `mount` at `aim`, in the game's words.
pub fn uncallable(world: &World, caster: Entity, mount: MountId, aim: CastAim) -> Option<String> {
    if let Some(refusal) = unwelcome(world, caster, mount) {
        return Some(refusal);
    }
    let content = world.resource::<Content>();
    let words = &world.resource::<GameSettings>().words;
    let def = mount.get(content);
    let tag = world.get::<AreaTag>(caster)?;
    match def.travels {
        Travels::Riding | Travels::Flying { .. } => None,
        Travels::Floating { draught } => {
            let floats = match aim {
                CastAim::Point(spot) => area::load(content, tag.area)
                    .water_mesh(draught)
                    .is_some_and(|water| water.walkable(spot)),
                CastAim::Actor(_) | CastAim::None => false,
            };
            (!floats).then(|| words.no_water_in_reach.clone())
        }
    }
}

/// Why `caster` may call no `mount` where they are, wherever they would call it to, in the
/// game's words: they already ride, or their area bars mounts of its kind.
fn unwelcome(world: &World, caster: Entity, mount: MountId) -> Option<String> {
    let content = world.resource::<Content>();
    let words = &world.resource::<GameSettings>().words;
    if world.get::<Riding>(caster).is_some() {
        return Some(words.step_off_first.clone());
    }
    let travels = mount.get(content).travels;
    let welcome = world
        .get::<AreaTag>(caster)
        .is_some_and(|tag| welcome_in(content, tag.area, travels));
    (!welcome).then(|| words.mount_barred_here.clone())
}

/// Seats `caster` on a `mount` of their own: where they stand, or for a mount that floats on the
/// water at the spot `aim` picks.
pub fn call(
    world: &mut World,
    caster: Entity,
    mount: MountId,
    aim: CastAim,
) -> Result<(), LineText> {
    if let Some(refusal) = uncallable(world, caster, mount, aim) {
        return Err(refusal.into());
    }
    let riding = Riding { mount };
    let travels = mount.get(world.resource::<Content>()).travels;
    match (travels, aim) {
        (Travels::Floating { .. }, CastAim::Point(spot)) => movement::hop(world, caster, spot),
        _ => movement::halt(world, caster),
    }
    seat(world, caster, riding);
    Ok(())
}

/// The rider leaves their seat: for the spot `at` when they picked one, else for the free ground
/// nearest them. Refused when that lies farther from the mount than its reach, which is the only
/// way off a mount: nothing moves a rider to ground anywhere else.
pub fn step_off(world: &mut World, rider: Entity, at: Option<Pos<Meters>>) -> Result<(), LineText> {
    if !rides(world, rider) {
        return Ok(());
    }
    let spot = landing(world, rider, at)?;
    unseat(world, rider);
    movement::hop_beside(world, rider, spot);
    Ok(())
}

/// Where `rider` would stand if they stepped off for the spot `at`, or with none for the free
/// ground nearest them: refused, in the game's words, where that is no ground a walker stands on
/// within the mount's reach of them.
pub fn landing(
    world: &World,
    rider: Entity,
    at: Option<Pos<Meters>>,
) -> Result<Pos<Meters>, String> {
    let content = world.resource::<Content>();
    let refusal = || {
        world
            .resource::<GameSettings>()
            .words
            .no_ground_in_reach
            .clone()
    };
    let (Some(riding), Some(from), Some(tag)) = (
        world.get::<Riding>(rider),
        movement::position(world, rider),
        world.get::<AreaTag>(rider),
    ) else {
        return Err(refusal());
    };
    let ground = area::load(content, tag.area).mesh();
    let toward = at.unwrap_or(from);
    if at.is_some_and(|at| !ground.walkable(at)) || ground.nearest(toward).is_none() {
        return Err(refusal());
    }
    let spot = movement::place_beside(world, tag.area, &ground, toward);
    if from.distance(spot) > riding.mount.get(content).reach {
        return Err(refusal());
    }
    Ok(spot)
}

/// Whether a rider of a mount that travels as `travels` stays on it arriving at `place`: the area
/// must allow its kind, and the spot must be one it travels over. Every warp asks this.
pub fn carried_to(content: &Content, place: Place, travels: Travels) -> bool {
    welcome_in(content, place.area, travels) && rides_at(content, place.area, travels, place.at)
}

/// What a rider takes through a warp into another world, to ride on once they arrive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossingRide {
    pub mount: MountId,
}

/// The ride `rider` would take into another world with them.
pub fn crossing(world: &World, rider: Entity) -> Option<CrossingRide> {
    let riding = world.get::<Riding>(rider)?;
    Some(CrossingRide {
        mount: riding.mount,
    })
}

/// Seats `rider` on what they rode before they crossed into this world, where it may be at the
/// spot they arrived at. Otherwise they arrive on foot, told why.
pub fn arrive(world: &mut World, rider: Entity, ride: CrossingRide) {
    let content = world.resource::<Content>().clone();
    let Some(place) = standing(world, rider) else {
        return;
    };
    region::hold_around(world, place.at);
    let travels = ride.mount.get(&content).travels;
    if carried_to(&content, place, travels) {
        seat(world, rider, Riding { mount: ride.mount });
    } else {
        movement::hop_beside(world, rider, place.at);
        tell_on_foot(world, rider, ride.mount, place.area);
    }
}

/// Takes `actor` off their mount before a warp to `to`, where it may not be at the spot the warp
/// leads to, telling them why. Only the world of an area holds its ground, so whether the spot
/// carries a mount in another world is settled there, as its rider arrives.
pub fn warping(world: &mut World, actor: Entity, to: Place) {
    let Some(riding) = world.get::<Riding>(actor).copied() else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let travels = riding.mount.get(&content).travels;
    let carried = if world.get::<AreaTag>(actor).map(|tag| tag.area) == Some(to.area) {
        region::hold_around(world, to.at);
        carried_to(&content, to, travels)
    } else {
        welcome_in(&content, to.area, travels)
    };
    if !carried {
        unseat(world, actor);
        tell_on_foot(world, actor, riding.mount, to.area);
    }
}

/// Whether `rider` stands, on what they ride, where they could stand on foot.
pub fn over_ground(world: &World, rider: Entity) -> bool {
    let content = world.resource::<Content>();
    match (
        movement::position(world, rider),
        world.get::<AreaTag>(rider),
    ) {
        (Some(at), Some(tag)) => area::load(content, tag.area).mesh().walkable(at),
        _ => false,
    }
}

/// Takes every fallen rider off their mount where they fell: nobody is moved for it.
#[derive(Default)]
pub struct UnseatFallen {
    riders: Cached<(Entity, &'static Riding), ()>,
    fallen: Vec<Entity>,
}

impl Planner for UnseatFallen {
    fn prepare(&mut self, world: &mut World) {
        self.riders.ready(world);
    }

    fn plan(&mut self, world: &World) {
        self.fallen = self
            .riders
            .query(world)
            .iter()
            .map(|(rider, _)| rider)
            .filter(|rider| stat::is_dead(world, *rider))
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        for rider in self.fallen.drain(..) {
            unseat(world, rider);
        }
    }
}

/// What every mount must hold against the rest of the game: its rider seated on a joint of its own
/// model, riding with a move some skeleton declares, and a skill that calls it aimed as it
/// travels.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let declared = content.built::<Declared>();
    for (id, mount) in loading.rows::<MountDef>(&content) {
        let model = ActorModel::loaded(&content, mount.look);
        let seat = &mount.rider;
        if !model.is_none_or(|model| model.has_joint(&seat.joint)) {
            loading.row_error(
                id,
                "/rider/joint",
                ContentProblem::new(
                    "unknown joint",
                    seat.joint.as_str(),
                    "a joint of the mount's model",
                ),
            );
        }
        if !declared.moves.contains(&seat.rides) {
            loading.row_error(
                id,
                "/rider/rides",
                ContentProblem::new(
                    "undeclared move",
                    seat.rides.as_str(),
                    "a move some skeleton's set declares",
                ),
            );
        }
    }
    let scripts = content.built::<RiftScripts>();
    for (id, skill) in loading.rows::<SkillDef>(&content) {
        let may_do = scripts.may_do(OwnerRow::of_row(id));
        let mounts: Vec<MountId> = may_do
            .reaches
            .iter()
            .filter_map(|reach| match reach {
                EffectReach::Mounts(mount) => Some(*mount),
                _ => None,
            })
            .collect();
        for mount in mounts {
            let floats = matches!(mount.get(&content).travels, Travels::Floating { .. });
            let wanted = if floats {
                Targeting::Spot
            } else {
                Targeting::Caster
            };
            if skill.target != wanted {
                loading.row_error(
                    id,
                    "/target",
                    ContentProblem::new(
                        "aimed unlike its mount",
                        format!("{} calls {}", skill.target.label(), mount.path(&content)),
                        format!("target {}", wanted.label()),
                    ),
                );
            }
        }
    }
}

/// Whether a mount that travels as `travels` may be ridden at `at` in `area`.
fn rides_at(content: &Content, area: AreaId, travels: Travels, at: Pos<Meters>) -> bool {
    mesh_in(content, area, travels).is_some_and(|mesh| mesh.walkable(at))
}

/// Whether `area` lets in mounts that travel as `travels`.
fn welcome_in(content: &Content, area: AreaId, travels: Travels) -> bool {
    !area
        .get(content)
        .bars_mounts
        .contains(&MountKind::from(travels))
}

/// Seats `rider` on the mount `riding` names, where they are: what a pointer lands on is the
/// mount, and its script holds on them.
fn seat(world: &mut World, rider: Entity, riding: Riding) {
    let content = world.resource::<Content>().clone();
    let look = riding.mount.get(&content).look;
    let hitbox = Hitbox(ActorModel::get(&content, look).hitbox());
    world.entity_mut(rider).insert((riding, hitbox));
    aspect::resettle(world, rider);
}

/// Takes `rider` out of their seat where they are: they are themselves to hit again, and their
/// mount's script no longer holds on them.
pub(crate) fn unseat(world: &mut World, rider: Entity) {
    let content = world.resource::<Content>().clone();
    let own = world
        .get::<Appearance>(rider)
        .map(|appearance| Hitbox(ActorModel::get(&content, appearance.model).hitbox()));
    let mut seated = world.entity_mut(rider);
    if seated.take::<Riding>().is_none() {
        return;
    }
    if let Some(own) = own {
        seated.insert(own);
    }
    aspect::resettle(world, rider);
}

/// Tells `rider` they arrive on foot in `area`, their `mount` left behind: the area bars its kind,
/// or the spot is none it travels over.
fn tell_on_foot(world: &mut World, rider: Entity, mount: MountId, area: AreaId) {
    let content = world.resource::<Content>().clone();
    let def = mount.get(&content);
    let words = &world.resource::<GameSettings>().words;
    let said = if welcome_in(&content, area, def.travels) {
        words.mount_not_here.replace("{mount}", &def.name)
    } else {
        words.mount_barred_here.clone()
    };
    let told = Notification::feed(HistoryTopic::Notification, said);
    notification::notify(world, rider, None, told);
}

fn standing(world: &World, actor: Entity) -> Option<Place> {
    Some(Place {
        area: world.get::<AreaTag>(actor)?.area,
        at: movement::position(world, actor)?,
    })
}
