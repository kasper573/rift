//! The times of day an npc is away. An away npc has no body in the world; where it lives a sign
//! may stand in its place and say why.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{NpcDef, NpcId, NpcModule};
use crate::core::content::{Content, RowCheck};
use crate::core::script::{Arrow, DialogueNode, Node};
use crate::core::units::{Degrees, Meters};
use crate::systems::GameSettings;
use crate::systems::area::object::{self, AreaObject, ObjectShape};
use crate::systems::area::{DayCycle, PropId, Resident, TimeOfDayName};
use crate::systems::combat::Attackers;
use crate::systems::dialogue;
use crate::systems::interact::{Interactive, Verb};
use crate::systems::player::PlayedBy;
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::{
    OwnerRow, Procedure, RiftEvent, RiftScripting, ScriptLine, Speaker, sky,
};
use crate::systems::text::Span;
use crate::systems::visibility::Presence;

/// When an npc is away: during each of its spans. With none it is never away.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Absence {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<AbsenceSpan>,
}

/// A time of day an npc is away for. With a `notice`, a sign stands where the npc lives for as
/// long, and says the notice to whoever reads it; without one nothing is left.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbsenceSpan {
    pub time: TimeOfDayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<AbsenceNotice>,
}

/// What an away npc leaves behind: the `text` its sign says, and the `sign` that stands for it
/// when not the game's own.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbsenceNotice {
    pub text: Vec<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign: Option<PropId>,
}

/// The sign a game stands for its away npcs, and from how far any of their signs is read.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbsenceSign {
    pub prop: PropId,
    pub reach: Meters,
}

/// Which of an npc's absence spans, by its place among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsenceSpanAt(pub u32);

impl NpcDef {
    pub fn absence(&self) -> Option<&Absence> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Absence(absence) => Some(absence),
            _ => None,
        })
    }
}

impl Absence {
    /// The span the npc is away for while the time of day `time` holds, if it is away then.
    pub fn span_during(&self, time: &TimeOfDayName) -> Option<AbsenceSpanAt> {
        self.spans
            .iter()
            .position(|span| span.time == *time)
            .map(|at| AbsenceSpanAt(at as u32))
    }

    /// Holds its spans to the game's times of day, each listed once, and every notice to a sign
    /// to stand for it and a reach to be read from.
    pub fn check(
        &self,
        pointer: &str,
        daylight: Option<&DayCycle>,
        sign: Option<AbsenceSign>,
        talked_to: bool,
        cx: &mut RowCheck,
    ) {
        for (index, span) in self.spans.iter().enumerate() {
            let at = format!("{pointer}/spans/{index}");
            DayCycle::check_named(daylight, format!("{at}/time"), &span.time, cx);
            if self.spans[..index]
                .iter()
                .any(|other| other.time == span.time)
            {
                cx.error(
                    format!("{at}/time"),
                    "listed twice",
                    &span.time.0,
                    "each time of day once",
                );
            }
            let Some(notice) = &span.notice else {
                continue;
            };
            if notice.sign.is_none() && sign.is_none() {
                cx.error(
                    format!("{at}/notice"),
                    "no sign",
                    "a notice with no sign to stand for it",
                    "a sign of its own, or the game's absence_sign",
                );
            }
            if sign.is_none() && !talked_to {
                cx.error(
                    format!("{at}/notice"),
                    "no reach",
                    "a notice nothing says from how far it is read",
                    "the game's absence_sign, or an npc that answers an interaction",
                );
            }
        }
    }
}

/// The span of its absence `npc` is away for under the world's sky, if it is away.
pub fn away(world: &World, npc: NpcId) -> Option<AbsenceSpanAt> {
    let absence = npc.get(world.resource::<Content>()).absence()?;
    absence.span_during(&sky::time_of_day(world)?.name)
}

/// The characters an away npc stays for: whoever it talks or trades with, and whoever fights it.
pub fn engaged(world: &World, sessions: &mut dialogue::Sessions, npc: Entity) -> Vec<Entity> {
    let mut engaged = dialogue::talking_with(sessions.iter(world), npc);
    engaged.extend(
        world
            .get::<Attackers>(npc)
            .into_iter()
            .flat_map(|attackers| &attackers.ids)
            .filter(|&&attacker| world.get::<PlayedBy>(attacker).is_some()),
    );
    engaged.sort_unstable();
    engaged.dedup();
    engaged
}

/// Stands the sign `resident`'s npc leaves where it lives for the span `span` of its absence, if
/// that span leaves a notice.
pub fn leave_sign(world: &mut World, resident: &Resident, span: AbsenceSpanAt) -> Option<Entity> {
    let content = world.resource::<Content>().clone();
    let sign = sign_of(&content, resident.npc, span)?;
    let standing = AreaObject {
        index: None,
        name: None,
        title: None,
        shape: ObjectShape::Point(resident.at),
        look: Some(sign.path(&content).to_owned()),
        yaw: sign_yaw(resident.facing),
        warp: None,
    };
    let stood = object::stand(world, standing, OwnerRow::of_notice(resident.npc, span));
    if let Some(shown) = &resident.shown {
        world
            .entity_mut(stood)
            .insert(Presence::When(CompiledCondition::new(&content, shown)));
    }
    Some(stood)
}

/// How a sign stands for an npc facing `facing`: its face the way the npc looked.
pub fn sign_yaw(facing: Degrees) -> Degrees {
    Degrees(facing.0 - QUARTER_TURN)
}

/// The sign that stands for `npc` through the span `span` of its absence, if that span leaves a
/// notice: the notice's own, else the game's.
pub fn sign_of(content: &Content, npc: NpcId, span: AbsenceSpanAt) -> Option<PropId> {
    notice_of(content, npc, span)?.sign.or(content
        .try_built::<GameSettings>()?
        .absence_sign
        .map(|sign| sign.prop))
}

/// Where in `def`'s file the notice of the span `span` of its absence is written.
pub fn notice_pointer(def: &NpcDef, span: AbsenceSpanAt) -> String {
    let module = def
        .modules
        .iter()
        .position(|module| matches!(module, NpcModule::Absence(_)))
        .unwrap_or_default();
    format!("/modules/{module}/spans/{}/notice", span.0)
}

/// The script each notice of `npc` is read through, as the nodes of a script file would say it:
/// read from `reach` away, its sign says the notice as the npc.
pub fn notice_scripts(
    npc: NpcId,
    def: &NpcDef,
    own: &[Arc<Procedure>],
    settings: Option<&GameSettings>,
) -> Vec<(AbsenceSpanAt, BTreeMap<String, Node<RiftScripting>>)> {
    let reach = settings
        .and_then(|settings| settings.absence_sign)
        .map(|sign| sign.reach)
        .or_else(|| Interactive::of(own).map(|interactive| interactive.reach));
    let (Some(absence), Some(reach)) = (def.absence(), reach) else {
        return Vec::new();
    };
    absence
        .spans
        .iter()
        .enumerate()
        .filter_map(|(at, span)| {
            let notice = span.notice.as_ref()?;
            let mut says = Arrow::default();
            says.point(Some(SAYS));
            let nodes = BTreeMap::from([
                (
                    READ.to_owned(),
                    Node::On {
                        event: RiftEvent::Interacted {
                            verb: Verb::Read,
                            reach,
                            marks: Vec::new(),
                            news: false,
                        },
                        next: says,
                    },
                ),
                (
                    SAYS.to_owned(),
                    Node::Dialogue(DialogueNode {
                        lines: vec![ScriptLine {
                            by: Speaker::Npc(npc),
                            gesture: None,
                            camera: None,
                            text: notice.text.clone(),
                        }],
                        choices: Vec::new(),
                        next: Arrow::default(),
                        left: Arrow::default(),
                    }),
                ),
            ]);
            Some((AbsenceSpanAt(at as u32), nodes))
        })
        .collect()
}

/// How far a prop is turned to face the way an actor facing nowhere does.
const QUARTER_TURN: f32 = 90.0;
const READ: &str = "read";
const SAYS: &str = "says";

fn notice_of(content: &Content, npc: NpcId, span: AbsenceSpanAt) -> Option<&AbsenceNotice> {
    npc.get(content)
        .absence()?
        .spans
        .get(span.0 as usize)?
        .notice
        .as_ref()
}
