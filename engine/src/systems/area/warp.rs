use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AreaTag;
use super::transition::Crossing;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault};
use crate::systems::npc::Npc;
use crate::systems::player::SpawnPoint;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, Fill, Place, RiftScripting, effect as effects,
};
use crate::systems::{dialogue, movement, reach, region};

/// Moves an actor to a place, setting its spawn point.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "area"))]
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
        let who = effects::aim(cx, &self.who)?;
        let to = cx.place(&self.to)?;
        Ok(Box::new(move |world, _, cx| {
            let warped = effects::aimed(world, cx, &who)
                .ok_or_else(|| "Nobody to warp".to_owned())
                .and_then(|actor| warp(world, actor, to));
            if let Err(reason) = warped {
                effects::show_error(world, cx, reason);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        cx.peek_place(&self.to)
            .map(EffectReach::Warp)
            .into_iter()
            .collect()
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
        region::reach(world, to.at);
        movement::relocate_beside(world, actor, to.at);
    } else {
        world.entity_mut(actor).insert(Crossing {
            dest_area: to.area,
            dest: to.at,
        });
    }
    Ok(())
}
