use bevy_ecs::entity::Entity;
use engine::core::content::Fixture;
use engine::core::math::{Offset, Pos, Rect, WindowPx};
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{MeterPos, Meters};
use engine::core::vfx::{VfxDef, VfxSite};
use engine::systems::actor::Action;
use engine::systems::area::AreaDef;
use engine::systems::attention::AttentionMark;
use engine::systems::history::HistoryTopic;
use engine::systems::interface::InterfaceIcon;
use engine::systems::item::ItemDef;
use engine::systems::movement::SPACING;
use engine::systems::npc::NpcDef;
use engine::systems::probe::{
    ProbeBody, ProbeObject, ProbeQuest, ProbeSnapshot, ProbeTreeView, ProbeUiElement, ProbeVfx,
    ProbeWarp,
};
use engine::systems::quest::{QuestDef, QuestResult};
use engine::systems::skill::{HotbarEntry, bar::HotbarPress};
use engine::systems::status::StatusEffectDef;

use crate::content::{content, id};

// What a snapshot shows, asked by content name.

/// The first frame with the player standing in an area, done crossing into it.
pub fn in_world(snapshot: &ProbeSnapshot) -> bool {
    snapshot.me.is_some()
        && snapshot.view.is_some()
        && snapshot.area.is_some()
        && snapshot.transition == ScreenTransitionPhase::Idle
}

/// The probe names the new area as soon as the character is there; the crossing's transition is
/// still uncovering it until the screen settles.
pub fn arrived_in(snapshot: &ProbeSnapshot, area: &str) -> bool {
    snapshot.area == Some(id::<AreaDef>(area)) && in_world(snapshot)
}

pub fn area_name(snapshot: &ProbeSnapshot) -> Option<&'static str> {
    snapshot.area.map(|area| area.name(content()))
}

pub fn at_rest(body: &ProbeBody) -> bool {
    body.action == Action::Idle && body.resting
}

pub fn me_at_rest(snapshot: &ProbeSnapshot) -> bool {
    snapshot.me.as_ref().is_some_and(at_rest)
}

pub fn townsperson<'a>(snapshot: &'a ProbeSnapshot, npc: &str) -> Option<&'a ProbeBody> {
    let npc = id::<NpcDef>(npc);
    snapshot.actors.iter().find(|actor| actor.npc == Some(npc))
}

/// The fallen don't count.
pub fn nearest_npc<'a>(snapshot: &'a ProbeSnapshot, npc: &str) -> Option<&'a ProbeBody> {
    let me = snapshot.me.as_ref()?.at;
    snapshot
        .actors
        .iter()
        .filter(|actor| actor.health > 0.0 && npc_is(actor, npc))
        .min_by(|a, b| a.at.distance_to(me).total_cmp(&b.at.distance_to(me)))
}

pub fn npc_is(body: &ProbeBody, npc: &str) -> bool {
    body.npc == Some(id::<NpcDef>(npc))
}

/// Whether at least `count` living `npc`s stand still within arm's reach of the resting player,
/// no two bodies overlapping.
pub fn surrounded_by(snapshot: &ProbeSnapshot, npc: &str, count: usize) -> bool {
    let Some(mine) = snapshot.me.as_ref().filter(|me| me.resting).map(|me| me.at) else {
        return false;
    };
    let around: Vec<Pos<Meters>> = snapshot
        .actors
        .iter()
        .filter(|actor| actor.health > 0.0 && npc_is(actor, npc) && actor.resting)
        .map(|actor| actor.at)
        .filter(|at| at.distance(mine) <= Meters(1.5))
        .collect();
    let mut bodies = around.clone();
    bodies.push(mine);
    let apart = bodies.iter().enumerate().all(|(index, a)| {
        bodies[index + 1..]
            .iter()
            .all(|b| a.distance(*b) >= Meters(SPACING.0 - 1e-3))
    });
    around.len() >= count && apart
}

pub fn marked(snapshot: &ProbeSnapshot, npc: &str, mark: AttentionMark) -> bool {
    let mark = mark.id(content());
    townsperson(snapshot, npc).is_some_and(|body| body.marks.contains(&mark))
}

pub fn object_named<'a>(snapshot: &'a ProbeSnapshot, name: &str) -> Option<&'a ProbeObject> {
    snapshot
        .objects
        .iter()
        .find(|object| object.name.as_deref() == Some(name))
}

pub fn warp_named<'a>(snapshot: &'a ProbeSnapshot, name: &str) -> &'a ProbeWarp {
    snapshot
        .warps
        .iter()
        .find(|warp| warp.name == name)
        .unwrap_or_else(|| panic!("no warp named {name} here"))
}

/// A spot within `rect` with no interface element under or near it, for a drag meant for the
/// surface itself rather than a control on it: the first found on a coarse grid in from the
/// edges. Near counts, since the interface takes a pointer a few pixels off a control as on it.
pub fn bare_spot(snapshot: &ProbeSnapshot, rect: Rect<WindowPx>) -> Option<Pos<WindowPx>> {
    const INSET: f32 = 20.0;
    const STEP: f32 = 40.0;
    const CLEARANCE: f32 = 10.0;
    let mut y = rect.min().y + INSET;
    while y < rect.max().y - INSET {
        let mut x = rect.min().x + INSET;
        while x < rect.max().x - INSET {
            let spot = Pos::new(x, y);
            if !snapshot
                .ui
                .iter()
                .any(|element| element.rect.inflate(CLEARANCE, CLEARANCE).contains(spot))
            {
                return Some(spot);
            }
            x += STEP;
        }
        y += STEP;
    }
    None
}

pub fn holding(snapshot: &ProbeSnapshot, item: &str) -> u32 {
    let item = id::<ItemDef>(item);
    snapshot
        .bag
        .iter()
        .filter(|stack| stack.item == item)
        .map(|stack| stack.count)
        .sum()
}

pub fn carried(snapshot: &ProbeSnapshot) -> u32 {
    snapshot.bag.iter().map(|stack| stack.count).sum()
}

pub fn on_quest<'a>(snapshot: &'a ProbeSnapshot, quest: &str) -> Option<&'a ProbeQuest> {
    let quest = id::<QuestDef>(quest);
    snapshot
        .quests
        .active
        .iter()
        .find(|active| active.quest == quest)
}

pub fn finished(snapshot: &ProbeSnapshot, quest: &str) -> Option<QuestResult> {
    let quest = id::<QuestDef>(quest);
    snapshot
        .quests
        .finished
        .iter()
        .find(|done| done.quest == quest)
        .map(|done| done.result)
}

/// Notices come and go: on a machine too busy to draw every frame, one can be up and gone between
/// two of them. History keeps every one, so it is how to tell that a notice came.
pub fn noticed(snapshot: &ProbeSnapshot, row: &str) -> bool {
    snapshot
        .history
        .records
        .iter()
        .any(|record| record.text == row)
}

/// A notice that says whom or what it's from: narration, alerts, milestones, intros, and words
/// called out loud rather than said in a conversation.
pub fn told_by(snapshot: &ProbeSnapshot, by: &str) -> bool {
    snapshot
        .history
        .records
        .iter()
        .any(|record| record.topic != HistoryTopic::Talk && record.by.as_deref() == Some(by))
}

pub fn erred(snapshot: &ProbeSnapshot, saying: &str) -> bool {
    snapshot
        .history
        .records
        .iter()
        .any(|record| record.topic == HistoryTopic::Error && record.text.contains(saying))
}

/// Where the tile shows in the window, if it does, clear of the interface over it.
pub fn clickable_point(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> Option<Pos<WindowPx>> {
    let point = snapshot.ground_point(tile)?;
    let window = Rect::from_size(snapshot.screen?);
    (window.contains(point) && !snapshot.covered.iter().any(|cover| cover.contains(point)))
        .then_some(point)
}

/// Ground a click walks onto: in view, and clear of anyone or anything a click would use instead.
pub fn open(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> bool {
    clickable_point(snapshot, tile).is_some() && !taken(snapshot, tile)
}

/// Someone or something stands there, or lies there.
pub fn taken(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> bool {
    snapshot
        .actors
        .iter()
        .any(|actor| stands_over(actor, tile, Meters(0.0)))
        || snapshot
            .objects
            .iter()
            .any(|object| object.hitbox.contains(tile))
        || snapshot
            .items
            .iter()
            .any(|item| item.at.distance(tile) <= Meters(0.5))
}

/// Anyone walking can step under the pointer between the frame that shows ground clear and the
/// click landing there, so ground within a tile of them is left alone while they pass.
pub fn passed_by(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> bool {
    let me = snapshot.me.as_ref().map(|me| me.id);
    snapshot
        .actors
        .iter()
        .filter(|actor| Some(actor.id) != me && matches!(actor.action, Action::Walk | Action::Run))
        .any(|actor| stands_over(actor, tile, PASSING))
}

/// Anyone at all can be under the pointer by the time a click lands: who walks may walk onto the
/// ground, and who stands may set off. Ground picked freely keeps a tile clear of everyone.
pub fn beside_anyone(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> bool {
    let me = snapshot.me.as_ref().map(|me| me.id);
    snapshot
        .actors
        .iter()
        .filter(|actor| Some(actor.id) != me)
        .any(|actor| stands_over(actor, tile, PASSING))
}

const PASSING: Meters = Meters(1.0);

/// Whether the body, grown by `margin`, covers the ground at `tile`.
fn stands_over(body: &ProbeBody, tile: Pos<Meters>, margin: Meters) -> bool {
    body.at.distance(tile) <= body.capsule.radius + margin
}

/// Stepping into a warp takes it.
pub fn in_warp(snapshot: &ProbeSnapshot, tile: Pos<Meters>) -> bool {
    snapshot.warps.iter().any(|warp| warp.rect.contains(tile))
}

/// A piece of the interface, picked out by what it says or shows, or by where it sits.
pub struct UiTarget {
    describe: String,
    test: UiTest,
}

type UiTest = Box<dyn Fn(&ProbeUiElement, &ProbeSnapshot) -> bool>;

impl UiTarget {
    pub fn text(text: &str) -> UiTarget {
        let text = text.to_owned();
        UiTarget::new(format!("{text:?}"), move |element, _| {
            element.text.as_deref() == Some(text.as_str())
        })
    }

    pub fn text_field() -> UiTarget {
        UiTarget::new("the text field", |element, _| element.editable)
    }

    pub fn typing_field() -> UiTarget {
        UiTarget::text_field().and("with focus", |element, _| element.focused)
    }

    /// A clickable row or tab: a button named by its text, where the same text may show as a label
    /// too.
    pub fn button(text: &str) -> UiTarget {
        let text = text.to_owned();
        UiTarget::new(format!("button {text:?}"), move |element, _| {
            element.button && element.text.as_deref() == Some(text.as_str())
        })
    }

    pub fn starting(prefix: &str) -> UiTarget {
        let prefix = prefix.to_owned();
        UiTarget::new(format!("{prefix:?}…"), move |element, _| {
            element
                .text
                .as_deref()
                .is_some_and(|text| text.starts_with(&prefix))
        })
    }

    pub fn containing(part: &str) -> UiTarget {
        let part = part.to_owned();
        UiTarget::new(format!("…{part:?}…"), move |element, _| {
            element
                .text
                .as_deref()
                .is_some_and(|text| text.contains(&part))
        })
    }

    /// The item named `name`, by the icon the interface shows it with.
    pub fn item(name: &str) -> UiTarget {
        let content = content();
        let item = content
            .table::<ItemDef>()
            .get(id(name))
            .unwrap_or_else(|| panic!("no item named {name}"));
        UiTarget::showing(name, item.icon.path())
    }

    /// The interface's own icon for `icon`, as this game draws it.
    pub fn interface_icon(icon: InterfaceIcon) -> UiTarget {
        UiTarget::showing(icon.name(), icon.get(content()).image.path())
    }

    fn showing(describe: &str, path: String) -> UiTarget {
        UiTarget::new(describe, move |element, _| {
            element.image.as_deref() == Some(path.as_str())
        })
    }

    pub fn new(
        describe: impl Into<String>,
        test: impl Fn(&ProbeUiElement, &ProbeSnapshot) -> bool + 'static,
    ) -> UiTarget {
        UiTarget {
            describe: describe.into(),
            test: Box::new(test),
        }
    }

    /// Narrowed to the copies that also pass `also`.
    pub fn and(
        self,
        describe: &str,
        also: impl Fn(&ProbeUiElement, &ProbeSnapshot) -> bool + 'static,
    ) -> UiTarget {
        let UiTarget {
            describe: first,
            test,
        } = self;
        UiTarget::new(format!("{first} {describe}"), move |element, snapshot| {
            test(element, snapshot) && also(element, snapshot)
        })
    }

    pub fn find<'a>(&self, snapshot: &'a ProbeSnapshot) -> Option<&'a ProbeUiElement> {
        snapshot
            .ui
            .iter()
            .find(|element| self.matches(element, snapshot))
    }

    pub fn matches(&self, element: &ProbeUiElement, snapshot: &ProbeSnapshot) -> bool {
        (self.test)(element, snapshot)
    }

    pub fn describe(&self) -> &str {
        &self.describe
    }
}

impl From<&str> for UiTarget {
    fn from(text: &str) -> UiTarget {
        UiTarget::text(text)
    }
}

// The skills window and the hotbar, as the probe reports them.

pub fn tree_view(snapshot: &ProbeSnapshot) -> Option<&ProbeTreeView> {
    snapshot.skills.as_ref()?.view.as_ref()
}

pub fn learned(snapshot: &ProbeSnapshot, node: &str) -> u8 {
    snapshot
        .skills
        .as_ref()
        .and_then(|skills| {
            skills
                .learned
                .iter()
                .find(|(key, _)| key == node)
                .map(|(_, rank)| rank.0)
        })
        .unwrap_or(0)
}

pub fn planned(snapshot: &ProbeSnapshot) -> usize {
    snapshot
        .skills
        .as_ref()
        .map_or(0, |skills| skills.planned.len())
}

/// The rank a node is picked up to in the open tree, or 0 when it is not picked.
pub fn planned_rank(snapshot: &ProbeSnapshot, key: &str) -> u8 {
    snapshot
        .skills
        .as_ref()
        .and_then(|skills| skills.planned.iter().find(|(node, _)| node == key))
        .map_or(0, |(_, rank)| rank.0)
}

pub fn to_spend(snapshot: &ProbeSnapshot) -> u32 {
    snapshot.skills.as_ref().map_or(0, |skills| skills.to_spend)
}

pub fn aiming(snapshot: &ProbeSnapshot) -> bool {
    snapshot
        .skills
        .as_ref()
        .is_some_and(|skills| skills.aiming.is_some())
}

pub fn reset_prompted(snapshot: &ProbeSnapshot) -> bool {
    snapshot
        .skills
        .as_ref()
        .is_some_and(|skills| skills.reset_prompt)
}

pub fn pressed(snapshot: &ProbeSnapshot, slot: usize) -> Option<HotbarPress> {
    snapshot.hotbar.get(slot).and_then(|slot| slot.pressed)
}

pub fn cooling(snapshot: &ProbeSnapshot, slot: usize) -> bool {
    snapshot.hotbar.get(slot).is_some_and(|slot| slot.cooling)
}

pub fn blocked(snapshot: &ProbeSnapshot, slot: usize) -> bool {
    snapshot.hotbar.get(slot).is_some_and(|slot| slot.blocked)
}

pub fn slot_skill(snapshot: &ProbeSnapshot, slot: usize) -> Option<&'static str> {
    match snapshot.hotbar.get(slot)?.entry {
        HotbarEntry::Skill(skill) => Some(skill.name(content())),
        _ => None,
    }
}

pub fn slot_item(snapshot: &ProbeSnapshot, slot: usize) -> Option<&'static str> {
    match snapshot.hotbar.get(slot)?.entry {
        HotbarEntry::Item(item) => Some(item.name(content())),
        _ => None,
    }
}

// Statuses, vfx and company, as the probe reports them.

pub fn status_on_me(snapshot: &ProbeSnapshot, effect: &str) -> bool {
    let wanted = id::<StatusEffectDef>(effect);
    snapshot
        .statuses
        .iter()
        .any(|status| status.effect == wanted)
}

/// Every instance of a vfx the client saw start lately, over or not: what a wait for a cast to land
/// looks for, since a short vfx may come and go between two frames of a slow client.
pub fn played<'a>(snapshot: &'a ProbeSnapshot, vfx: &str) -> Vec<&'a ProbeVfx> {
    let wanted = id::<VfxDef>(vfx);
    snapshot
        .vfx
        .iter()
        .filter(|instance| instance.vfx == wanted)
        .collect()
}

pub fn played_on(snapshot: &ProbeSnapshot, vfx: &str, body: Entity) -> bool {
    played(snapshot, vfx)
        .iter()
        .any(|instance| instance.site == VfxSite::Actor(body))
}

/// The instances of a vfx still playing.
pub fn playing<'a>(snapshot: &'a ProbeSnapshot, vfx: &str) -> Vec<&'a ProbeVfx> {
    played(snapshot, vfx)
        .into_iter()
        .filter(|instance| !instance.over)
        .collect()
}

/// The tile two east of the other player: where a friend walks to stand with them, in full view.
pub fn beside_other_player(snapshot: &ProbeSnapshot) -> Option<Pos<Meters>> {
    other_player(snapshot).map(|star| star.at + Offset::new(2.0, 0.0))
}

/// The other player once they stand within `reach` tiles of me.
pub fn other_player_within(snapshot: &ProbeSnapshot, reach: f32) -> Option<ProbeBody> {
    let me = snapshot.me.as_ref()?.at;
    other_player(snapshot)
        .filter(|star| star.at.distance_to(me) <= reach)
        .cloned()
}

/// Another player in view: the first one that is not me.
pub fn other_player(snapshot: &ProbeSnapshot) -> Option<&ProbeBody> {
    let me = snapshot.me.as_ref()?.id;
    snapshot
        .actors
        .iter()
        .find(|actor| actor.player && actor.id != me)
}
