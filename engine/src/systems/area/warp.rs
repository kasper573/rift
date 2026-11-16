use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::transition::Crossing;
use super::{AreaId, AreaTag, ObjectShape};
use crate::core::content::{Content, SchemaMark};
use crate::core::math::Pos;
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, ScriptFault};
use crate::core::units::Meters;
use crate::systems::area_instance::AreaInstanceWorld;
use crate::systems::npc::Npc;
use crate::systems::player::roster::ServerRoster;
use crate::systems::player::{self, SpawnPoint};
use crate::systems::script::effect::{self as effects, Aimed, Reaching};
use crate::systems::script::{
    Condition, EffectCall, EffectReach, Fill, OwnerRow, Place, RiftScripting, RiftScripts,
};
use crate::systems::text::LineText;
use crate::systems::{area_instance, dialogue, mount, movement, reach, region};

pub struct WarpObject {
    pub name: String,
    pub shape: ObjectShape,
    pub to: Place,
}

pub fn warp_objects(content: &Content, scripts: &RiftScripts, area: AreaId) -> Vec<WarpObject> {
    super::objects(super::placed(content, area))
        .filter_map(|(index, object)| {
            let to = scripts
                .may_do(OwnerRow::of_object(area, index))
                .reaches
                .iter()
                .find_map(|reach| match reach {
                    EffectReach::Warp(place) => Some(*place),
                    _ => None,
                })?;
            Some(WarpObject {
                name: object.name.clone().unwrap_or_default(),
                shape: object.shape,
                to,
            })
        })
        .collect()
}

/// Moves an actor to a place, setting its spawn point.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("area"))]
pub struct Warp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub to: Fill<Place>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Warp {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let to = cx.place(&self.to)?;
        if to.area.get(cx.content).instance.is_some() {
            return Err(ScriptFault::new(
                "warps into an instance",
                "an enter_instance effect, the only way into an instanced area",
            ));
        }
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some("Nobody to warp"),
                },
                move |world, actor| warp(world, actor, to),
            )
        }))
    }
    fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        cx.peek_place(&self.to)
            .map(EffectReach::Warp)
            .into_iter()
            .collect()
    }
}

/// Moves an actor to their spawn point in the area they stand in: where they last arrived in it,
/// and where they come back after a fall. It is a warp like any other.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("area"))]
pub struct ReturnToSpawn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for ReturnToSpawn {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some("Nobody to warp"),
                },
                |world, actor| {
                    let spawn = world.get::<SpawnPoint>(actor).map(|spawn| spawn.0);
                    let area = world.get::<AreaTag>(actor).map(|tag| tag.area);
                    let (Some(at), Some(area)) = (spawn, area) else {
                        return Err("Has no spawn point".into());
                    };
                    warp(world, actor, Place { area, at })
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        Vec::new()
    }
}

/// Moves an actor to where another stands, setting its spawn point.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("area"))]
pub struct WarpToActor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub to: Condition,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for WarpToActor {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let to = cx.actor_anywhere(Some(&self.to))?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_anywhere(world, cx, &who);
            let stands = match effects::aimed_anywhere(world, cx, &to) {
                Aimed::Here(other) => standing(world, other),
                Aimed::Elsewhere(client) => world
                    .get_resource::<ServerRoster>()
                    .and_then(|roster| roster.place_of(client)),
                Aimed::Nobody => None,
            };
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some("Nobody to warp"),
                },
                move |world, actor| warp(world, actor, stands.ok_or("Nobody to warp to")?),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        Vec::new()
    }
}

pub fn warp(world: &mut World, actor: Entity, to: Place) -> Result<(), LineText> {
    let here = world.get::<AreaTag>(actor).map(|tag| tag.area);
    if here != Some(to.area) && world.get::<Npc>(actor).is_some() {
        return Err("Npcs stay in their area".into());
    }
    let instanced = to.area.get(world.resource::<Content>()).instance.is_some();
    if here != Some(to.area) && instanced {
        return Err("Nobody is followed into an instance".into());
    }
    dialogue::close_for(world, actor);
    area_instance::entrance::close(world, actor);
    reach::forget(world, actor);
    mount::warping(world, actor, to);
    world.entity_mut(actor).insert(SpawnPoint(to.at));
    if here == Some(to.area) {
        land(world, actor, to.at);
    } else {
        world.entity_mut(actor).insert(Crossing {
            dest_area: to.area,
            dest: to.at,
            instance: None,
        });
    }
    Ok(())
}

/// Sends a player's character to `to` in another world: the instance `instance` of the place's
/// area, or the world everyone in that area shares. They keep riding where their mount may be at
/// `to`, as through any warp.
pub fn cross(world: &mut World, character: Entity, to: Place, instance: Option<AreaInstanceWorld>) {
    dialogue::close_for(world, character);
    area_instance::entrance::close(world, character);
    reach::forget(world, character);
    mount::warping(world, character, to);
    world.entity_mut(character).insert((
        SpawnPoint(to.at),
        Crossing {
            dest_area: to.area,
            dest: to.at,
            instance,
        },
    ));
}

/// Puts an actor down at `at` in its own area, or beside it when someone else rests there. A
/// player's character arrives there as it does in another area: held until its client has drawn
/// what stands around it.
pub fn land(world: &mut World, actor: Entity, at: Pos<Meters>) {
    region::hold_around(world, at);
    movement::relocate_beside(world, actor, at);
    player::hold_arrival(world, actor);
}

fn standing(world: &World, actor: Entity) -> Option<Place> {
    Some(Place {
        area: world.get::<AreaTag>(actor)?.area,
        at: movement::position(world, actor)?,
    })
}
