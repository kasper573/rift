//! What scripts do with mounts: call one, take someone off theirs, and ask whether someone rides.

use crate::core::script::Reach;
use crate::core::script::Vocabulary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::MountId;
use super::Mounts;
use crate::core::content::{Content, SchemaMark};
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, REFUSED, ScriptFault};
use crate::core::script::{EffectCall, Expr};
use crate::systems::aim::{self as effects, Aimed, Reaching};
use crate::systems::item::USED_POINT;
use crate::systems::messages;
use crate::systems::skill::casting::CastAim;

/// Seats an actor on a mount of their own: where they stand, or for a mount that
/// floats, on the water at the spot the skill that runs this was aimed at.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("mount"))]
pub struct Mount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    pub mount: MountId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when the mount cannot be called here, with the caster told why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef for Mount {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (mount, refused) = (self.mount, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed =
                effects::aimed_character(world, cx, &who).map_or(Aimed::Nobody, Aimed::Here);
            let aim = match cx.event.get(USED_POINT).and_then(|point| point.as_point()) {
                Some(spot) => CastAim::Point(spot),
                None => CastAim::None,
            };
            let nobody = messages::of(world.resource::<Content>())
                .mount
                .nobody_to_ride()
                .to_owned();
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: Some(&refused),
                    nobody: Some(nobody.as_str()),
                },
                move |world, caster| super::call(world, caster, mount, aim),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Mounts(self.mount))]
    }
}

/// Takes an actor off what they ride, as stepping off does: onto ground within the mount's reach.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("mount"))]
pub struct Dismount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken where no ground is within the mount's reach: left unwired, the rider is told why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef for Dismount {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        let refused = self.refused.clone();
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: Some(&refused),
                    nobody: None,
                },
                |world, rider| super::step_off(world, rider, None),
            )
        }))
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<Mount>("mount")
        .effect::<Dismount>("dismount");
}
