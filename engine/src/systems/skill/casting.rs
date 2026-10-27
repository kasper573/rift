use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Cooldowns, Rank, SkillId, Targeting};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{
    Arrow, Compiler, EffectDef, EffectStep, Eval, NEXT, REFUSED, ScriptFault, Value, ValueType,
};
use crate::core::time::WallClock;
use crate::core::units::Meters;
use crate::systems::actor::{self, playing};
use crate::systems::combat::{self, Relation};
use crate::systems::item::Inventory;
use crate::systems::player::{self, player_requests};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::script::effect::{self as effects, Audience};
use crate::systems::script::{
    Condition, EffectCall, EventKind, Owner, Reach, RiftInstances, RiftScripting, RunnerKind,
    condition, event,
};
use crate::systems::{notification, stat};

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

/// A script casts a skill as a player would: aimed, checked, walked into range and performed.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "skill"))]
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
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

pub fn request(world: &mut World) {
    player_requests::<UseSkillRequest>(world, true, |world, caster, request| {
        cast(world, caster, request.skill, request.aim, false)
    });
}

/// Sets out to use a skill: refused at once when the caster does not know it, it is cooling down,
/// its requirements or cost are not met, or the aim does not fit its kind.
pub fn cast(
    world: &mut World,
    caster: Entity,
    skill: SkillId,
    aim: CastAim,
    repeat: bool,
) -> Result<(), String> {
    if let Some(refusal) = super::unusable(world, caster, skill) {
        return Err(refusal);
    }
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    ready(world, caster, skill)?;
    let target = match (def.target, aim) {
        (Targeting::Caster, _) => ReachTarget::Actor(caster),
        (Targeting::SingleAlly | Targeting::SingleEnemy, CastAim::Actor(target)) => {
            if !fits(world, caster, target, def.target) {
                return Err(format!("{} can't target them", def.name));
            }
            ReachTarget::Actor(target)
        }
        (Targeting::SingleAlly | Targeting::SingleEnemy, _) => {
            return Err(format!("{} needs a target", def.name));
        }
        (Targeting::AreaAlly | Targeting::AreaEnemy, CastAim::Point(point)) => {
            ReachTarget::Point(point)
        }
        (Targeting::AreaAlly | Targeting::AreaEnemy, _) => {
            return Err(format!("{} needs a point to aim at", def.name));
        }
    };
    reach::intend(world, caster, target, ReachAct::Cast { skill, repeat });
    Ok(())
}

/// Whether a skill may be used now by someone who knows it: not cooling down, its requirements
/// holding and its cost in the bag.
pub fn ready(world: &World, caster: Entity, skill: SkillId) -> Result<(), String> {
    let content = world.resource::<Content>();
    let def = skill.get(content);
    let now = world.resource::<WallClock>().now;
    if world
        .get::<Cooldowns>(caster)
        .is_some_and(|cooldowns| cooldowns.cooling(skill, now))
    {
        return Err(format!("{} is cooling down", def.name));
    }
    if let Some(unmet) = condition::first_unmet(world, caster, def.requires.iter()) {
        return Err(format!("{} needs {unmet}", def.name));
    }
    for stack in &def.cost {
        let held = world
            .get::<Inventory>(caster)
            .map_or(0, |inventory| inventory.count(stack.item));
        if held < stack.count {
            return Err(format!(
                "{} needs {} {}",
                def.name,
                stack.count,
                stack.item.get(content).name
            ));
        }
    }
    Ok(())
}

/// The one rule for whom a skill of a kind may land on: an actor the caster could reach, not held
/// in an arrival, standing to the caster as the kind wants.
pub fn fits(world: &World, caster: Entity, target: Entity, targeting: Targeting) -> bool {
    if !reach::pursuable(world, caster, target) || player::held(world, target) {
        return false;
    }
    let relation = combat::relation(world, caster, target);
    match targeting {
        Targeting::SingleEnemy | Targeting::AreaEnemy => relation == Relation::Enemy,
        Targeting::SingleAlly | Targeting::AreaAlly => {
            matches!(relation, Relation::Own | Relation::Ally)
        }
        Targeting::Caster => relation == Relation::Own,
    }
}

/// Walks every caster toward what it aims at and performs the skill on arrival, once ready: the
/// cost is taken, the cooldown starts and `used` is raised on the skill's script. A caster stands
/// through what it plays, since a step would cut its swing short of the hit, and between casts it
/// settles, spreading out from others that crowd its tile.
pub fn casts(world: &mut World, intents: &mut reach::Intents) {
    for (caster, intent) in reach::casting(world, intents) {
        let (ReachAct::Cast { skill, repeat }, target) = (intent.act, intent.target) else {
            continue;
        };
        let rank = super::rank_of(world, caster, skill);
        if stat::is_dead(world, caster) || rank == Rank::NONE {
            reach::forget(world, caster);
            continue;
        }
        if playing::playing(world, caster) {
            continue;
        }
        let range = super::range_of(world, caster, skill, rank);
        match reach::pursue(world, caster, range) {
            Pursuit::Lost => {
                reach::forget(world, caster);
                continue;
            }
            Pursuit::Approaching => continue,
            Pursuit::Arrived => {}
        }
        if !playing::ready(world, caster) {
            reach::settle(world, caster, range);
            continue;
        }
        let targeting = skill.get(world.resource::<Content>()).target;
        if let ReachTarget::Actor(target) = target
            && !fits(world, caster, target, targeting)
        {
            reach::forget(world, caster);
            continue;
        }
        if let Err(reason) = ready(world, caster, skill) {
            if world.get::<player::PlayedBy>(caster).is_some() {
                notification::error(world, caster, reason);
            }
            reach::forget(world, caster);
            continue;
        }
        perform(world, caster, skill, rank, target);
        if !repeat {
            reach::drop_intent(world, caster);
        }
    }
}

/// Everyone reaching for a cast gives chase, at twice their pace.
pub fn chase(world: &World, entity: Entity, into: &mut Vec<stat::Stat>) {
    if reach::current(world, entity).is_some_and(|intent| intent.cast().is_some()) {
        let base = stat::base(world, entity, stat::StatKind::MovementSpeed);
        into.push(stat::StatKind::MovementSpeed.of(base * (CHASE_SPEED_MULTIPLIER - 1.0)));
    }
}

const CHASE_SPEED_MULTIPLIER: f32 = 2.0;

fn perform(world: &mut World, caster: Entity, skill: SkillId, rank: Rank, target: ReachTarget) {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let now = world.resource::<WallClock>().now;
    if let Some(toward) = target.position(world) {
        actor::face_toward(world, caster, toward);
    }
    if !def.cost.is_empty()
        && let Some(mut inventory) = world.get_mut::<Inventory>(caster)
        && inventory.exchange(&content, &def.cost, &[]).is_err()
    {
        return;
    }
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
            match cast(world, caster, skill, aim, false) {
                Ok(()) => EffectStep::Output(NEXT),
                Err(_) => EffectStep::Output(REFUSED),
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Casts(self.skill)]
    }
}
