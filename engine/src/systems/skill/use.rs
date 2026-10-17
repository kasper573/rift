use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Cooldowns, Rank, SkillDef, SkillId, SkillKind, Targeting};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{
    Compiler, EffectDef, EffectStep, Eval, Name, ScriptFault, Value, ValueType,
};
use crate::core::tiling::Tiles;
use crate::core::time::WallClock;
use crate::systems::actor::{self, playing};
use crate::systems::area;
use crate::systems::combat::{self, Relation};
use crate::systems::item::Inventory;
use crate::systems::movement::{halt, on_tile};
use crate::systems::notification;
use crate::systems::player::{self, sender_player};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::script::effect::{self as effects, Aim as Who, Arrow, Audience, Link};
use crate::systems::script::{
    Condition, EffectCall, EventKind, Owner as ScriptOwner, OwnerKind, Reach, RiftInstances,
    RiftScripting, condition,
};
use crate::systems::visibility;
use crate::systems::{drain_requests, stat};

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct UseSkillRequest {
    pub skill: SkillId,
    #[entities]
    pub aim: Aim,
}

/// What a use is aimed at: an actor for the single kinds, a point for the area kinds, nothing for
/// a skill on yourself.
#[derive(Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Actor(#[entities] Entity),
    Point(Pos<Tiles>),
    None,
}

/// A script casts a skill as a player would: aimed, checked, walked into range and performed.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct UseSkill {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Who,
    pub skill: SkillId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Arrow,
}

pub fn request(world: &mut World) {
    for request in drain_requests::<UseSkillRequest>(world) {
        let Some(caster) = sender_player(world, request.client_id) else {
            continue;
        };
        if !player::may_command(world, caster) {
            continue;
        }
        let UseSkillRequest { skill, aim } = request.message;
        if let Err(reason) = cast(world, caster, skill, aim, false) {
            notification::error(world, caster, reason);
        }
    }
}

/// Sets out to use a skill: refused at once when the caster does not know it, it is cooling down,
/// its requirements or cost are not met, or the aim does not fit its kind.
pub fn cast(
    world: &mut World,
    caster: Entity,
    skill: SkillId,
    aim: Aim,
    repeat: bool,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let rank = super::rank_of(world, caster, skill);
    if rank == Rank::NONE {
        return Err(format!("You don't know {}", def.name));
    }
    if def.kind == SkillKind::Passive {
        return Err(format!("{} is passive: it holds on its own", def.name));
    }
    ready(world, caster, skill, rank)?;
    let target = match (def.target, aim) {
        (Targeting::Caster, _) => ReachTarget::Actor(caster),
        (Targeting::SingleAlly | Targeting::SingleEnemy, Aim::Actor(target)) => {
            if !fits(world, caster, target, def.target) {
                return Err(format!("{} can't target them", def.name));
            }
            ReachTarget::Actor(target)
        }
        (Targeting::SingleAlly | Targeting::SingleEnemy, _) => {
            return Err(format!("{} needs a target", def.name));
        }
        (Targeting::AreaAlly | Targeting::AreaEnemy, Aim::Point(point)) => {
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
pub fn ready(world: &World, caster: Entity, skill: SkillId, rank: Rank) -> Result<(), String> {
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
    let _ = rank;
    Ok(())
}

/// Whether an actor stands to the caster as the skill's kind wants.
pub fn fits(world: &World, caster: Entity, target: Entity, targeting: Targeting) -> bool {
    if world.get_entity(target).is_err()
        || stat::is_dead(world, target)
        || player::held(world, target)
        || !area::same_area(world, caster, target)
        || !visibility::coexist(world, caster, target)
    {
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
/// cost is taken, the cooldown starts and `used` is raised on the skill's script.
pub fn casts(world: &mut World, intents: &mut reach::Intents) {
    for (caster, intent) in reach::casting(world, intents) {
        let (ReachAct::Cast { skill, repeat }, target) = (intent.act, intent.target) else {
            continue;
        };
        if stat::is_dead(world, caster) {
            reach::forget(world, caster);
            continue;
        }
        let rank = super::rank_of(world, caster, skill);
        if rank == Rank::NONE {
            reach::forget(world, caster);
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
        halt(world, caster);
        if !on_tile(world, caster) || !playing::ready(world, caster) {
            continue;
        }
        let targeting = skill.get(world.resource::<Content>()).target;
        if let ReachTarget::Actor(target) = target
            && !fits(world, caster, target, targeting)
        {
            reach::forget(world, caster);
            continue;
        }
        if let Err(reason) = ready(world, caster, skill, rank) {
            if world.get::<player::Owner>(caster).is_some() {
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
pub fn chase(world: &World, entity: Entity) -> Vec<stat::Stat> {
    if reach::current(world, entity).is_some_and(|intent| intent.cast().is_some()) {
        let base = stat::base(world, entity, stat::StatKind::MovementSpeed);
        vec![stat::StatKind::MovementSpeed.of(base * (CHASE_SPEED_MULTIPLIER - 1.0))]
    } else {
        Vec::new()
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
        match world.get_mut::<Cooldowns>(caster) {
            Some(mut cooldowns) => {
                cooldowns.0.insert(skill, ready_at);
            }
            None => {
                world
                    .entity_mut(caster)
                    .insert(Cooldowns([(skill, ready_at)].into_iter().collect()));
            }
        }
    }
    world
        .entity_mut(caster)
        .insert(playing::LastAction { at: now });
    if let Some(sfx) = def.sfx {
        notification::play_sound(world, Some(caster), Some(caster), sfx, Audience::Everyone);
    }
    world.resource_mut::<RiftInstances>().raise(
        ScriptOwner::Row(OwnerKind::Skill, skill.raw()),
        EventKind::Used,
        Some(caster),
        vec![
            Value::Actor(target.actor()),
            Value::Point(target.point()),
            Value::Int(i64::from(rank.0)),
        ],
        |_| true,
    );
}

const NEXT: Name = "next";
const REFUSED: Name = "refused";

impl EffectDef<RiftScripting> for UseSkill {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![
            (NEXT, self.next.as_deref()),
            (REFUSED, self.refused.as_deref()),
        ]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let skill = self.skill;
        let at: Option<(Eval, ValueType)> = match &self.at {
            Some(at) => {
                let (eval, ty) = cx.typed(at)?;
                if !matches!(ty, ValueType::Actor | ValueType::Point) {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("at is {}, expected an actor or a point", ty.name()),
                    ));
                }
                Some((eval, ty))
            }
            None => None,
        };
        Ok(Box::new(move |world, _, cx| {
            let Some(caster) = effects::aimed(world, cx, &who) else {
                return EffectStep::Output(REFUSED);
            };
            let aim = match &at {
                None => Aim::None,
                Some((eval, _)) => match eval(world, cx) {
                    Value::Actor(Some(actor)) => Aim::Actor(actor),
                    Value::Point(Some(point)) => Aim::Point(point),
                    _ => Aim::None,
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

impl SkillDef {
    pub fn aims_at(&self) -> &'static str {
        match self.target {
            Targeting::SingleAlly | Targeting::SingleEnemy => "an actor",
            Targeting::AreaAlly | Targeting::AreaEnemy => "a point",
            Targeting::Caster => "yourself",
        }
    }
}
