use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::transition::Crossing;
use super::{AreaId, AreaTag, ObjectShape, Warps};
use crate::core::content::{Content, SchemaMark};
use crate::core::math::Pos;
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, ScriptFault, Vocabulary};
use crate::core::script::{EffectCall, Expr, Fill, OwnerRow, Reach, Scripts};
use crate::core::text::LineText;
use crate::core::units::Meters;
use crate::systems::aim::{self as effects, Aimed, Reaching};
use crate::systems::area::Place;
use crate::systems::area_instance::AreaInstanceWorld;
use crate::systems::messages;
use crate::systems::npc::Npc;
use crate::systems::player::roster::ServerRoster;
use crate::systems::player::{self, SpawnPoint};
use crate::systems::{area_instance, dialogue, mount, movement, reach, region};

pub struct WarpObject {
    pub name: String,
    pub shape: ObjectShape,
    pub to: Place,
}

pub fn warp_objects(content: &Content, scripts: &Scripts, area: AreaId) -> Vec<WarpObject> {
    super::objects(super::placed(content, area))
        .filter_map(|(index, object)| {
            let to = scripts
                .may_do(OwnerRow::of_object(area, index))
                .reaching::<Warps>()
                .next()
                .map(|warps| warps.0)?;
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
    pub who: Option<Expr>,
    pub to: Fill<Place>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for Warp {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
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
            let nobody = messages::of(world.resource::<Content>())
                .map
                .nobody_to_warp()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some(nobody.as_str()),
                },
                move |world, actor| warp(world, actor, to),
            )
        }))
    }
    fn reach(&self, cx: &Compiler) -> Vec<Reach> {
        cx.peek_place(&self.to)
            .map(|place| Reach::new(Warps(place)))
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
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for ReturnToSpawn {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_anywhere(world, cx, &who);
            let nobody = messages::of(world.resource::<Content>())
                .map
                .nobody_to_warp()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some(nobody.as_str()),
                },
                |world, actor| {
                    let spawn = world.get::<SpawnPoint>(actor).map(|spawn| spawn.0);
                    let area = world.get::<AreaTag>(actor).map(|tag| tag.area);
                    let (Some(at), Some(area)) = (spawn, area) else {
                        let words = &messages::of(world.resource::<Content>()).map;
                        return Err(LineText::from(words.no_spawn_point()));
                    };
                    warp(world, actor, Place { area, at })
                },
            )
        }))
    }
}

/// Moves an actor to where another stands, setting its spawn point.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("area"))]
pub struct WarpToActor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub to: Expr,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for WarpToActor {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
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
            let nobody = messages::of(world.resource::<Content>())
                .map
                .nobody_to_warp()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: None,
                    nobody: Some(nobody.as_str()),
                },
                move |world, actor| {
                    let Some(stands) = stands else {
                        let words = &messages::of(world.resource::<Content>()).map;
                        return Err(LineText::from(words.nobody_to_warp_to()));
                    };
                    warp(world, actor, stands)
                },
            )
        }))
    }
}

pub fn warp(world: &mut World, actor: Entity, to: Place) -> Result<(), LineText> {
    let here = world.get::<AreaTag>(actor).map(|tag| tag.area);
    if here != Some(to.area) && world.get::<Npc>(actor).is_some() {
        let words = &messages::of(world.resource::<Content>()).map;
        return Err(LineText::from(words.npcs_stay()));
    }
    let instanced = to.area.get(world.resource::<Content>()).instance.is_some();
    if here != Some(to.area) && instanced {
        let words = &messages::of(world.resource::<Content>()).map;
        return Err(LineText::from(words.no_following_into_instance()));
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

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<Warp>("warp")
        .effect::<WarpToActor>("warp_to_actor")
        .effect::<ReturnToSpawn>("return_to_spawn");
}
