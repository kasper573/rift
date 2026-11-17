use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Cooldowns, Rank, SkillId, Targeting};
use crate::core::content::Content;
use crate::core::content::{SchemaMark, serial};
use crate::core::math::Pos;
use crate::core::par;
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, Eval, NEXT, REFUSED, ScriptFault, Value, ValueType,
};
use crate::core::tick::{Cached, Planner};
use crate::core::time::WallClock;
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::{self, playing};
use crate::systems::combat::{self, Relation};
use crate::systems::item::Inventory;
use crate::systems::mention::Mention;
use crate::systems::mount::{self, RiderAct};
use crate::systems::movement::{self, Berths};
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, player_requests};
use crate::systems::reach::{self, Pursuit, PursuitPlan, ReachAct, ReachIntent, ReachTarget};
use crate::systems::script::effect::{self as effects, Audience};
use crate::systems::script::{
    Condition, EffectCall, EffectReach, EventKind, Owner, RiftInstances, RiftScripting, RunnerKind,
    condition, event,
};
use crate::systems::stat::{self, Vital};
use crate::systems::text::LineText;
use crate::systems::{notification, visibility};

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct UseSkillRequest {
    pub skill: SkillId,
    #[entities]
    pub aim: CastAim,
}

/// What a cast is aimed at: an actor for the single kinds, a point for the area kinds, nothing for
/// a skill on yourself.
#[derive(Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum CastAim {
    Actor(#[entities] Entity),
    Point(Pos<Meters>),
    None,
}

/// How a cast is set out on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastWay {
    /// Once, by a caster who knows the skill.
    Once,
    /// Again and again until the target falls or the caster turns away, as a basic attack lands.
    Repeated,
    /// Once, lent by a script to a caster who need not know the skill, at its first rank.
    Lent,
}

/// A script casts a skill as a player would: aimed, checked, walked into range and performed.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("skill"))]
pub struct UseSkill {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub skill: SkillId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub at: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when the skill can't be cast, without telling anyone why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
    /// Lends the skill for this one cast, so a caster who doesn't know it may cast it.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub lent: bool,
}

pub fn request(world: &mut World) {
    player_requests::<UseSkillRequest>(world, RequestNeeds::Command, |world, caster, request| {
        if let CastAim::Actor(target) = request.aim
            && !visibility::in_sight(world, caster, target)
        {
            return Err("Can't see them".into());
        }
        if !mount::left_by(world, caster, request.skill) {
            let act = if super::carried_as_item(world, caster, request.skill) {
                RiderAct::Items
            } else {
                RiderAct::Skills
            };
            mount::seat::act(world, caster, act)?;
        }
        cast(world, caster, request.skill, request.aim, CastWay::Once)
    });
}

/// Sets out to use a skill: refused at once when the caster does not know it and it isn't lent,
/// it is cooling down, its requirements or cost are not met, or the aim does not fit its kind.
pub fn cast(
    world: &mut World,
    caster: Entity,
    skill: SkillId,
    aim: CastAim,
    way: CastWay,
) -> Result<(), LineText> {
    let refusal = match way {
        CastWay::Lent => super::passive(world, skill),
        CastWay::Once | CastWay::Repeated => super::unusable(world, caster, skill),
    };
    if let Some(refusal) = refusal {
        return Err(refusal);
    }
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    if let Some(mount) = mount::called_by(world, skill) {
        // The skill of the mount its caster rides is how they leave it, which is no cast:
        // nothing is spent, and no cooldown starts.
        if mount::rides_on(world, caster, mount) {
            let spot = match aim {
                CastAim::Point(spot) => Some(spot),
                CastAim::Actor(_) | CastAim::None => None,
            };
            return mount::step_off(world, caster, spot);
        }
        // A mount called at a spot is refused for the spot, where the spot is weighed.
        if def.target != Targeting::Spot
            && let Some(refusal) = mount::uncallable(world, caster, mount, aim)
        {
            return Err(refusal.into());
        }
    }
    ready(world, caster, skill)?;
    let target = match (def.target, aim) {
        (Targeting::Caster, _) => ReachTarget::Actor(caster),
        (Targeting::SingleAlly | Targeting::SingleEnemy, CastAim::Actor(target)) => {
            if !fits(world, caster, target, def.target) {
                return Err(named(skill, &content).then(" can't target them"));
            }
            ReachTarget::Actor(target)
        }
        (Targeting::SingleAlly | Targeting::SingleEnemy, _) => {
            return Err(named(skill, &content).then(" needs a target"));
        }
        (Targeting::AreaAlly | Targeting::AreaEnemy, CastAim::Point(point)) => {
            ReachTarget::Point(point)
        }
        (Targeting::AreaAlly | Targeting::AreaEnemy, _) => {
            return Err(named(skill, &content).then(" needs a point to aim at"));
        }
        (Targeting::Spot, CastAim::Point(spot)) => {
            if let Some(refusal) = spot_refused(world, caster, skill, spot) {
                return Err(refusal.into());
            }
            ReachTarget::Point(spot)
        }
        (Targeting::Spot, _) => {
            return Err(named(skill, &content).then(" needs a spot to aim at"));
        }
    };
    reach::intend(world, caster, target, ReachAct::Cast { skill, way });
    Ok(())
}

/// What `spot` has against `caster` using the skill there, in the game's words: it lies out of
/// the skill's range of them, or the mount the skill calls may not be called at it. For the
/// driver of that mount the skill is how they leave it, and the spot is where they would stand.
/// The server weighs a spot by it as the skill is cast, and a client as the spot is aimed at.
pub fn spot_refused(
    world: &World,
    caster: Entity,
    skill: SkillId,
    spot: Pos<Meters>,
) -> Option<String> {
    let mount = mount::called_by(world, skill);
    if mount.is_some_and(|mount| mount::rides_on(world, caster, mount)) {
        return mount::landing(world, caster, Some(spot)).err();
    }
    let range = super::range_of(world, caster, skill, super::rank_of(world, caster, skill));
    let near = movement::position(world, caster).is_some_and(|from| from.distance(spot) <= range);
    if !near {
        return Some(world.resource::<GameSettings>().words.spot_too_far.clone());
    }
    mount.and_then(|mount| mount::uncallable(world, caster, mount, CastAim::Point(spot)))
}

/// Whether a skill may be used now by someone who knows it: not cooling down, its requirements
/// holding, its mana held and its cost in the bag.
pub fn ready(world: &World, caster: Entity, skill: SkillId) -> Result<(), LineText> {
    let content = world.resource::<Content>();
    let def = skill.get(content);
    let now = world.resource::<WallClock>().now;
    if world
        .get::<Cooldowns>(caster)
        .is_some_and(|cooldowns| cooldowns.cooling(skill, now))
    {
        return Err(named(skill, content).then(" is cooling down"));
    }
    if let Some(unmet) = condition::first_unmet(world, caster, def.requires.iter()) {
        return Err(named(skill, content)
            .then(" needs ")
            .then(LineText::from(&unmet)));
    }
    for stack in &def.cost {
        let held = world
            .get::<Inventory>(caster)
            .map_or(0, |inventory| inventory.count(stack.item));
        if held < stack.count {
            return Err(named(skill, content).then(" needs ").then(stack.named()));
        }
    }
    let mana = def.mana_at(super::rank_of(world, caster, skill));
    if stat::current(world, caster, Vital::Mana) < mana {
        return Err(named(skill, content).then(format!(" needs {mana} mana")));
    }
    Ok(())
}

fn named(skill: SkillId, content: &Content) -> LineText {
    Mention::Skill(skill).line(content)
}

/// The one rule for whom a skill of a kind may land on: an actor the caster could reach, not held
/// in an arrival, standing to the caster as the kind wants.
pub fn fits(world: &World, caster: Entity, target: Entity, targeting: Targeting) -> bool {
    if !reach::pursuable(world, caster, target) || player::arriving(world, target) {
        return false;
    }
    let relation = combat::relation(world, caster, target);
    match targeting {
        Targeting::SingleEnemy | Targeting::AreaEnemy => relation == Relation::Enemy,
        Targeting::SingleAlly | Targeting::AreaAlly => {
            matches!(relation, Relation::Own | Relation::Ally)
        }
        Targeting::Caster => relation == Relation::Own,
        Targeting::Spot => false,
    }
}

/// Walks every caster toward what it aims at and performs the skill on arrival, once ready: the
/// cost is taken, the cooldown starts and `used` is raised on the skill's script. A caster stands
/// through what it plays, since a step would cut its swing short of the hit, and between casts it
/// settles, spreading out from others that crowd its tile. Every caster's step is worked out on every
/// core from the world as the tick found it, then written in the order the casters were found.
#[derive(Default)]
pub struct StepCasts {
    intents: Cached<(Entity, &'static ReachIntent), ()>,
    steps: Vec<(Entity, ReachIntent, CastStep)>,
}

impl Planner for StepCasts {
    fn prepare(&mut self, world: &mut World) {
        self.intents.ready(world);
    }

    fn plan(&mut self, world: &World) {
        let casters = reach::casting(self.intents.query(world).iter());
        self.steps = casters
            .par_iter()
            .with_min_len(par::grain(casters.len()))
            .map(|&(caster, intent)| (caster, intent, step_of(world, caster, intent)))
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        for (caster, intent, step) in self.steps.drain(..) {
            apply(world, caster, intent, step);
        }
    }
}

/// What a caster's tick comes to, worked out from the world as the tick found it.
enum CastStep {
    /// The cast is given up, told to the player who meant it when it was refused.
    Forget {
        refused: Option<LineText>,
    },
    /// Playing a cast, or waiting on the delay before the next.
    Wait,
    Pursue(PursuitPlan),
    Settle(Berths),
    Perform {
        skill: SkillId,
        rank: Rank,
    },
}

fn step_of(world: &World, caster: Entity, intent: ReachIntent) -> CastStep {
    let (ReachAct::Cast { skill, way }, target) = (intent.act, intent.target) else {
        return CastStep::Wait;
    };
    let rank = match way {
        CastWay::Lent => super::rank_of(world, caster, skill).max(Rank::FIRST),
        CastWay::Once | CastWay::Repeated => super::rank_of(world, caster, skill),
    };
    if stat::is_dead(world, caster) || rank == Rank::NONE {
        return CastStep::Forget { refused: None };
    }
    if playing::playing(world, caster) {
        return CastStep::Wait;
    }
    let range = super::range_of(world, caster, skill, rank);
    let pursuit = PursuitPlan::of(world, caster, range);
    if !pursuit.arrived() {
        return CastStep::Pursue(pursuit);
    }
    if !playing::ready(world, caster) {
        return match reach::settling(world, caster, range) {
            Some(berths) => CastStep::Settle(berths),
            None => CastStep::Wait,
        };
    }
    let targeting = skill.get(world.resource::<Content>()).target;
    if let ReachTarget::Actor(target) = target
        && !fits(world, caster, target, targeting)
    {
        return CastStep::Forget { refused: None };
    }
    if let Err(reason) = ready(world, caster, skill) {
        let refused = world.get::<player::PlayedBy>(caster).map(|_| reason);
        return CastStep::Forget { refused };
    }
    CastStep::Perform { skill, rank }
}

fn apply(world: &mut World, caster: Entity, intent: ReachIntent, step: CastStep) {
    match step {
        CastStep::Forget { refused } => {
            if let Some(reason) = refused {
                notification::error(world, caster, reason);
            }
            reach::forget(world, caster);
        }
        CastStep::Wait => {}
        CastStep::Pursue(pursuit) => {
            if pursuit.apply(world, caster) == Pursuit::Lost {
                reach::forget(world, caster);
            }
        }
        CastStep::Settle(berths) => berths.settle(world, caster),
        CastStep::Perform { skill, rank } => {
            perform(world, caster, skill, rank, intent.target);
            if let ReachAct::Cast {
                way: CastWay::Once | CastWay::Lent,
                ..
            } = intent.act
            {
                reach::drop_intent(world, caster);
            }
        }
    }
}

/// Everyone reaching for a cast gives chase, at the game's chase pace.
pub fn chase(world: &World, entity: Entity, into: &mut dyn FnMut(stat::Stat)) {
    if reach::chasing(world, entity) {
        let base = stat::base(world, entity, stat::StatKind::MovementSpeed);
        let pace = world.resource::<GameSettings>().chase_pace;
        into(stat::StatKind::MovementSpeed.of(base * (pace - 1.0)));
    }
}

fn perform(world: &mut World, caster: Entity, skill: SkillId, rank: Rank, target: ReachTarget) {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let now = world.resource::<WallClock>().now;
    if let Some(toward) = target.position(world) {
        actor::face_toward(world, caster, toward);
    }
    let mana = def.mana_at(rank);
    if stat::current(world, caster, Vital::Mana) < mana {
        return;
    }
    if !def.cost.is_empty()
        && let Some(mut inventory) = world.get_mut::<Inventory>(caster)
        && inventory.exchange(&content, &def.cost, &[]).is_err()
    {
        return;
    }
    stat::drain(world, caster, Vital::Mana, mana);
    let cooldown = def.cooldown_at(rank);
    if cooldown.0 > 0.0 {
        let ready_at = now.after(cooldown);
        if let Some(mut cooldowns) = world.get_mut::<Cooldowns>(caster) {
            cooldowns.0.insert(skill, ready_at);
        }
    }
    world
        .entity_mut(caster)
        .insert(playing::LastAction { at: now });
    if let Some(sfx) = def.sfx {
        notification::play_sound(world, Some(caster), Some(caster), sfx, Audience::Everyone);
    }
    world.resource_mut::<RiftInstances>().raise(
        Owner::Row(RunnerKind::Skill, skill.raw()),
        EventKind::Used,
        Some(caster),
        event::used_payload(target.actor(), target.point(), i64::from(rank.0)),
        |_| true,
    );
}

impl EffectDef<RiftScripting> for UseSkill {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let skill = self.skill;
        let way = if self.lent {
            CastWay::Lent
        } else {
            CastWay::Once
        };
        let at: Option<Eval> = match &self.at {
            Some(at) => {
                let (eval, ty) = cx.typed(at)?;
                if !matches!(ty, ValueType::Actor | ValueType::Point) {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("at is {}, expected an actor or a point", ty.name()),
                    ));
                }
                Some(eval)
            }
            None => None,
        };
        Ok(Box::new(move |world, _, cx| {
            let Some(caster) = effects::aimed(world, cx, &who) else {
                return EffectStep::Output(REFUSED);
            };
            let aim = match &at {
                None => CastAim::None,
                Some(eval) => match eval(world, cx) {
                    Value::Actor(Some(actor)) => CastAim::Actor(actor),
                    Value::Point(Some(point)) => CastAim::Point(point),
                    _ => CastAim::None,
                },
            };
            match cast(world, caster, skill, aim, way) {
                Ok(()) => EffectStep::Output(NEXT),
                Err(_) => EffectStep::Output(REFUSED),
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Casts(self.skill)]
    }
}
