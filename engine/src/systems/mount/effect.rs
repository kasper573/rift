//! What scripts do with mounts: call one, take someone off theirs, and ask whether someone rides.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::MountId;
use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, REFUSED, ScriptFault};
use crate::systems::script::effect::{self as effects, Aimed, Reaching};
use crate::systems::script::event::USED_POINT;
use crate::systems::script::{Condition, EffectCall, EffectReach, RiftScripting};
use crate::systems::skill::casting::CastAim;

/// Seats an actor as the driver of a mount of their own: where they stand, or for a mount that
/// floats, on the water at the spot the skill that runs this was aimed at.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("mount"))]
pub struct Mount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub mount: MountId,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when the mount cannot be called here, with the caster told why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef<RiftScripting> for Mount {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (mount, refused) = (self.mount, self.refused.clone());
        Ok(Box::new(move |world, flow, cx| {
            let aimed =
                effects::aimed_character(world, cx, &who).map_or(Aimed::Nobody, Aimed::Here);
            let aim = match cx.event.get(USED_POINT).and_then(|point| point.as_point()) {
                Some(spot) => CastAim::Point(spot),
                None => CastAim::None,
            };
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching {
                    done: NEXT,
                    refused: Some(&refused),
                    nobody: Some("Nobody to ride it"),
                },
                move |world, caster| super::call(world, caster, mount, aim),
            )
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Mounts(self.mount)]
    }
}

/// Takes an actor off what they ride, onto the nearest ground however far that is.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("mount"))]
pub struct Dismount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Dismount {
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
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                |world, rider| {
                    super::set_down(world, rider);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        Vec::new()
    }
}
