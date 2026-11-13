use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::transition::Crossing;
use super::{AreaId, AreaTag, ObjectShape};
use crate::core::content::{Content, SchemaMark};
use crate::core::math::Pos;
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, ScriptFault};
use crate::core::units::Meters;
use crate::systems::npc::Npc;
use crate::systems::player::roster::ServerRoster;
use crate::systems::player::{self, SpawnPoint};
use crate::systems::script::effect::{self as effects, Aimed, Reaching};
use crate::systems::script::{
    Condition, EffectCall, EffectReach, Fill, OwnerRow, Place, RiftScripting, RiftScripts,
};
use crate::systems::{dialogue, movement, reach, region};

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
                    .and_then(|roster| roster.player(client))
                    .map(|enrolled| enrolled.place),
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

pub fn warp(world: &mut World, actor: Entity, to: Place) -> Result<(), String> {
    let here = world.get::<AreaTag>(actor).map(|tag| tag.area);
    if here != Some(to.area) && world.get::<Npc>(actor).is_some() {
        return Err("Npcs stay in their area".to_owned());
    }
    dialogue::close_for(world, actor);
    reach::forget(world, actor);
    world.entity_mut(actor).insert(SpawnPoint(to.at));
    if here == Some(to.area) {
        land(world, actor, to.at);
    } else {
        world.entity_mut(actor).insert(Crossing {
            dest_area: to.area,
            dest: to.at,
        });
    }
    Ok(())
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
