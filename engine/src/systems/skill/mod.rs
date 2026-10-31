pub mod aim;
pub mod bar;
pub mod card;
pub mod casting;
pub mod client;
pub mod hotbar;
pub mod learn;
pub mod prompt;
pub mod reset;
pub mod window;

pub use casting::{CastAim, UseSkill, UseSkillRequest, cast};
pub use hotbar::{AssignBasicAttackRequest, AssignHotbarRequest, BasicAttackPick};
pub use learn::LearnSkillsRequest;
pub use reset::{ResetAnswerRequest, ResetPrompt, ResetSkills};

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::assets::ImageRef;
use crate::core::audio::playback::SfxId;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck, serial,
};
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Pos;
use crate::core::script::FlowCx;
use crate::core::time::{Seconds, UnixMillis};
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::AreaTag;
use crate::systems::class::{self, NodeKey, SkillPoints, SkillTree};
use crate::systems::equipment;
use crate::systems::item::{ItemId, ItemKind, ItemStack};
use crate::systems::movement;
use crate::systems::npc::{Npc, NpcDef};
use crate::systems::script::event::{USED_POINT, USED_RANK, USED_TARGET};
use crate::systems::script::query::{Among, TickRoster};
use crate::systems::script::{
    Condition, EventKind, OwnerRow, RiftScripts, RunnerKind, ScriptModule, ScriptedRow,
};
use crate::systems::stat::{self, StatKind};

pub const HOTBAR_SLOTS: u32 = 40;
/// An area hits the nearest few, so a crowd never multiplies one cast without bound.
pub const AREA_TARGETS: usize = 8;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::{Channel, ClientMessageAppExt};

    use crate::systems::mirror::AppMirrorExt;
    app.replicate_private::<Learned>()
        .replicate_private::<Cooldowns>()
        .replicate_private::<Hotbar>()
        .replicate_private::<BasicAttack>()
        .replicate_private::<ResetPrompt>()
        .add_mapped_client_message::<UseSkillRequest>(Channel::Ordered)
        .add_client_message::<LearnSkillsRequest>(Channel::Ordered)
        .add_client_message::<AssignHotbarRequest>(Channel::Ordered)
        .add_client_message::<AssignBasicAttackRequest>(Channel::Ordered)
        .add_client_message::<ResetAnswerRequest>(Channel::Ordered);
}

/// Every request a client may send about its skills, answered in one place.
pub fn requests(world: &mut World) {
    casting::request(world);
    learn::requests(world);
    hotbar::requests(world);
    reset::answers(world);
}

pub type SkillId = ContentId<SkillDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillDef {
    pub name: String,
    pub icon: ImageRef,
    #[serde(default, skip_serializing_if = "SkillKind::is_active")]
    pub kind: SkillKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<SfxId>,
    pub target: Targeting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = 4.0))]
    pub range: Option<Meters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Shape>,
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub cooldown: Seconds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub basic_attack: bool,
    pub levels: Vec<SkillLevel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<SkillModule>,
}

impl ContentFile for SkillDef {
    const TABLE: &'static str = "skill";

    fn fresh(near: &Nearby) -> Result<SkillDef, String> {
        let first = SkillLevel {
            cost: SkillPoints(1),
            stats: BTreeMap::new(),
        };
        Ok(SkillDef {
            name: near.name(),
            icon: near.image()?,
            kind: SkillKind::Active,
            sfx: None,
            target: Targeting::SingleEnemy,
            range: None,
            area: None,
            cooldown: Seconds(0.0),
            requires: None,
            cost: Vec::new(),
            basic_attack: false,
            levels: vec![first],
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count("/levels", self.levels.len(), 1, 20, "ranks");
        if self.kind == SkillKind::Passive {
            let passive = "nothing: a passive skill is never used, it holds while known";
            if self.target != Targeting::Caster {
                check.error(
                    "/target",
                    "passive with a target",
                    self.target.label(),
                    "self",
                );
            }
            if self.area.is_some() {
                check.error("/area", "passive with an area", "an area", passive);
            }
            if self.range.is_some() {
                check.error("/range", "passive with a range", "a range", passive);
            }
            if self.cooldown.0 > 0.0 {
                check.error(
                    "/cooldown",
                    "passive with a cooldown",
                    self.cooldown.0,
                    passive,
                );
            }
            if !self.cost.is_empty() {
                check.error("/cost", "passive with a cost", "a cost", passive);
            }
            if self.basic_attack {
                check.error(
                    "/basic_attack",
                    "passive basic attack",
                    "true",
                    "false: a basic attack is used on a click",
                );
            }
        }
        if self.basic_attack {
            let answers = "nothing: a click on an enemy must always answer";
            if !self.cost.is_empty() {
                check.error("/cost", "basic attack with a cost", "a cost", answers);
            }
            if self.cooldown.0 > 0.0 {
                check.error(
                    "/cooldown",
                    "basic attack with a cooldown",
                    self.cooldown.0,
                    answers,
                );
            }
        }
        match (self.target.aimed_at_point(), self.area.is_some()) {
            (true, false) => check.error(
                "/area",
                "area kind without an area",
                self.target.label(),
                "a shape the skill covers at the aimed point",
            ),
            (false, true) => check.error(
                "/area",
                "area on a single kind",
                self.target.label(),
                "no area: a single or self skill has no shape",
            ),
            _ => {}
        }
        if self.cooldown.0 < 0.0 {
            check.error("/cooldown", "out of range", self.cooldown.0, "0 or more");
        }
        if let Some(range) = self.range
            && range.0 <= 0.0
        {
            check.error("/range", "out of range", range.0, "more than 0 m");
        }
        for (index, stack) in self.cost.iter().enumerate() {
            if stack.count == 0 {
                check.error(
                    format!("/cost/{index}/count"),
                    "empty stack",
                    0,
                    "at least 1",
                );
            }
        }
    }
}

impl NamedContent for SkillDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for SkillDef {
    type Module = SkillModule;

    fn modules(&self) -> &[SkillModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<SkillModule> {
        &mut self.modules
    }

    fn rule(_: &SkillModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for SkillDef {
    const OWNER: RunnerKind = RunnerKind::Skill;

    fn script(module: &SkillModule) -> Option<&ScriptModule> {
        match module {
            SkillModule::Script(script) => Some(script),
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillModule {
    Script(ScriptModule),
}

/// An active skill is used, aimed and performed; a passive one holds while it is known, its
/// script running as an aspect of the character.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
pub enum SkillKind {
    #[default]
    Active,
    Passive,
}

impl SkillKind {
    pub fn is_active(&self) -> bool {
        *self == SkillKind::Active
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Targeting {
    SingleAlly,
    SingleEnemy,
    AreaAlly,
    AreaEnemy,
    #[serde(rename = "self")]
    #[strum(serialize = "self")]
    Caster,
}

impl Targeting {
    pub fn aimed_at_point(self) -> bool {
        matches!(self, Targeting::AreaAlly | Targeting::AreaEnemy)
    }

    pub fn hostile(self) -> bool {
        matches!(self, Targeting::SingleEnemy | Targeting::AreaEnemy)
    }

    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn describe(self) -> &'static str {
        match self {
            Targeting::SingleAlly => "One ally",
            Targeting::SingleEnemy => "One enemy",
            Targeting::AreaAlly => "Allies around the aimed point",
            Targeting::AreaEnemy => "Enemies around the aimed point",
            Targeting::Caster => "Yourself",
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillLevel {
    pub cost: SkillPoints,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stats: BTreeMap<SkillStat, f32>,
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
pub enum SkillStat {
    Power,
    Cooldown,
    Range,
    Radius,
    Duration,
}

impl SkillStat {
    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn unit(self) -> Option<&'static str> {
        match self {
            SkillStat::Cooldown | SkillStat::Duration => Some("s"),
            SkillStat::Range | SkillStat::Radius => Some("m"),
            SkillStat::Power => None,
        }
    }
}

/// How far into a skill's ranks someone is: `NONE` outside them, `FIRST` at the first.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[serde(transparent)]
pub struct Rank(pub u8);

impl Rank {
    pub const NONE: Rank = Rank(0);
    pub const FIRST: Rank = Rank(1);

    pub fn first() -> Rank {
        Rank::FIRST
    }

    pub fn is_first(&self) -> bool {
        *self == Rank::FIRST
    }
}

/// The ranks a character learned in its class tree, by node.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Learned(pub BTreeMap<NodeKey, Rank>);

impl Learned {
    pub fn rank<K: Ord + ?Sized>(&self, node: &K) -> Rank
    where
        NodeKey: Borrow<K>,
    {
        self.0.get(node).copied().unwrap_or(Rank::NONE)
    }

    /// The skills the learned nodes stand for, at their ranks.
    pub fn skills<'a>(&'a self, tree: &'a SkillTree) -> impl Iterator<Item = (SkillId, Rank)> + 'a {
        self.0.iter().filter_map(|(key, rank)| {
            let node = tree.node(key)?;
            (*rank > Rank::NONE).then_some((node.skill, *rank))
        })
    }

    pub fn cost(&self, content: &Content, tree: &SkillTree) -> SkillPoints {
        self.skills(tree)
            .fold(SkillPoints(0), |sum, (skill, rank)| {
                sum + skill.get(content).cost_to(rank)
            })
    }

    pub fn ranks(&self) -> u32 {
        self.0.values().map(|rank| u32::from(rank.0)).sum()
    }

    pub fn skills_held(&self) -> u32 {
        self.0.values().filter(|rank| **rank > Rank::NONE).count() as u32
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Cooldowns(pub BTreeMap<SkillId, UnixMillis>);

impl Cooldowns {
    pub fn ready_at(&self, skill: SkillId) -> Option<UnixMillis> {
        self.0.get(&skill).copied()
    }

    pub fn cooling(&self, skill: SkillId, now: UnixMillis) -> bool {
        self.ready_at(skill).is_some_and(|ready| ready > now)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hotbar(pub Vec<HotbarEntry>);

/// A place on the hotbar, counted from zero; the player sees it numbered from one.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct HotbarSlot(pub u32);

impl HotbarSlot {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn number(self) -> u32 {
        self.0 + 1
    }

    /// The first `count` slots, in order.
    pub fn first(count: u32) -> impl Iterator<Item = HotbarSlot> {
        (0..count).map(HotbarSlot)
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotbarEntry {
    #[default]
    Empty,
    Skill(SkillId),
    Item(ItemId),
}

impl Hotbar {
    pub fn entry(&self, slot: HotbarSlot) -> HotbarEntry {
        self.0.get(slot.index()).copied().unwrap_or_default()
    }

    /// Puts `entry` in `slot`, taking it out of any other slot: the bar holds each skill or item
    /// once, so slotting it again moves it.
    pub fn set(&mut self, slot: HotbarSlot, entry: HotbarEntry) {
        if entry != HotbarEntry::Empty {
            for held in &mut self.0 {
                if *held == entry {
                    *held = HotbarEntry::Empty;
                }
            }
        }
        if self.0.len() <= slot.index() {
            self.0.resize(slot.index() + 1, HotbarEntry::Empty);
        }
        self.0[slot.index()] = entry;
        while self.0.last() == Some(&HotbarEntry::Empty) {
            self.0.pop();
        }
    }

    pub fn first_free(&self, within: u32) -> Option<HotbarSlot> {
        HotbarSlot::first(within).find(|slot| self.entry(*slot) == HotbarEntry::Empty)
    }

    pub fn slot_of(&self, entry: HotbarEntry) -> Option<HotbarSlot> {
        self.0
            .iter()
            .position(|held| *held == entry)
            .map(|slot| HotbarSlot(slot as u32))
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BasicAttack(pub Option<SkillId>);

impl SkillDef {
    pub fn rank_count(&self) -> u8 {
        self.levels.len() as u8
    }

    /// The ranks someone may hold the skill at.
    pub fn ranks(&self) -> RangeInclusive<Rank> {
        Rank::FIRST..=Rank(self.rank_count())
    }

    pub fn cost_to(&self, rank: Rank) -> SkillPoints {
        self.levels
            .iter()
            .take(usize::from(rank.0))
            .fold(SkillPoints(0), |sum, level| sum + level.cost)
    }

    /// A stat summed over the ranks reached.
    pub fn stat(&self, rank: Rank, stat: SkillStat) -> f32 {
        self.levels
            .iter()
            .take(usize::from(rank.0))
            .filter_map(|level| level.stats.get(&stat))
            .sum()
    }

    pub fn cooldown_at(&self, rank: Rank) -> Seconds {
        Seconds((self.cooldown.0 + self.stat(rank, SkillStat::Cooldown)).max(0.0))
    }

    pub fn area_at(&self, rank: Rank) -> Option<Shape> {
        self.area
            .map(|shape| shape.grown(Meters(self.stat(rank, SkillStat::Radius))))
    }
}

/// The one complaint about a row holding a skill at a rank outside its ranks.
pub fn check_rank(
    check: &mut RowCheck,
    pointer: String,
    content: &Content,
    skill: SkillId,
    rank: Rank,
) {
    // A skill whose row failed to load has its own errors reported, and no ranks to check against.
    let Some(def) = content.table::<SkillDef>().get(skill) else {
        return;
    };
    if !def.ranks().contains(&rank) {
        check.error(
            pointer,
            "rank out of range",
            rank.0,
            format!(
                "1 to {}, the ranks of {}",
                def.rank_count(),
                skill.path(content)
            ),
        );
    }
}

/// Every skill an actor may use now, at the rank it has it: an npc's listed skills, the ranks
/// learned in its class tree, and what its worn equipment or an npc's basic attack grants at rank
/// one.
pub fn known(world: &World, actor: Entity) -> BTreeMap<SkillId, Rank> {
    let content = world.resource::<Content>();
    let tree = class::of(world, actor).map(|class| &class.get(content).tree);
    let worn = equipment::worn(world, actor);
    known_from(
        content,
        tree,
        world.get::<Learned>(actor),
        worn,
        world.get::<Npc>(actor).map(|npc| npc.def.get(content)),
    )
}

/// `known`, from the parts a character is made of, so a snapshot is checked the way the world
/// answers for the character it becomes.
pub fn known_from(
    content: &Content,
    tree: Option<&SkillTree>,
    learned: Option<&Learned>,
    worn: impl Iterator<Item = ItemId>,
    npc: Option<&NpcDef>,
) -> BTreeMap<SkillId, Rank> {
    let mut known = BTreeMap::new();
    if let (Some(tree), Some(learned)) = (tree, learned) {
        known.extend(learned.skills(tree));
    }
    for skill in worn.flat_map(|item| grants_of(content, item)) {
        known.entry(*skill).or_insert(Rank::FIRST);
    }
    if let Some(npc) = npc {
        known.extend(npc.skills.iter().map(|listed| (listed.skill, listed.rank)));
        if let Some(skill) = npc.basic_attack {
            known.entry(skill).or_insert(Rank::FIRST);
        }
    }
    known
}

/// Why an actor could not use the skill at all, whatever its aim or readiness.
pub fn unusable(world: &World, actor: Entity, skill: SkillId) -> Option<String> {
    let def = skill.get(world.resource::<Content>());
    if rank_of(world, actor, skill) == Rank::NONE {
        Some(format!("You don't know {}", def.name))
    } else if def.kind == SkillKind::Passive {
        Some(format!("{} is passive: it holds on its own", def.name))
    } else {
        None
    }
}

/// Why a passive skill takes no hotbar slot.
pub fn unslotted(def: &SkillDef) -> String {
    format!("{} is passive: it holds without a slot", def.name)
}

pub fn rank_of(world: &World, actor: Entity, skill: SkillId) -> Rank {
    known(world, actor)
        .get(&skill)
        .copied()
        .unwrap_or(Rank::NONE)
}

/// The skills an actor's worn equipment grants, in slot order.
pub fn granted(world: &World, actor: Entity) -> Vec<SkillId> {
    let content = world.resource::<Content>();
    equipment::worn(world, actor)
        .flat_map(|item| grants_of(content, item).iter().copied())
        .collect()
}

pub fn grants_of(content: &Content, item: ItemId) -> &[SkillId] {
    match &item.get(content).kind {
        ItemKind::Equipment { grants, .. } => grants,
        _ => &[],
    }
}

/// The first skill flagged a basic attack among what the worn items grant, in slot order.
pub fn first_flagged_grant(
    content: &Content,
    worn: impl Iterator<Item = ItemId>,
) -> Option<SkillId> {
    worn.flat_map(|item| grants_of(content, item).iter().copied())
        .find(|skill| skill.get(content).basic_attack)
}

/// The reach of a skill from an actor: the row's range, or the actor's Range stat when unset,
/// plus whatever its ranks add.
pub fn range_of(world: &World, actor: Entity, skill: SkillId, rank: Rank) -> Meters {
    let def = skill.get(world.resource::<Content>());
    let base = def
        .range
        .unwrap_or_else(|| Meters(stat::effective(world, actor, StatKind::Range)));
    Meters((base.0 + def.stat(rank, SkillStat::Range)).max(0.0))
}

/// Whom a cast lands on, read from the use that raised the running script: the caster, the aimed
/// actor when it still stands where the skill may reach it, or the nearest actors inside the area.
pub fn targets(world: &World, cx: &FlowCx, skill: SkillId) -> Vec<Entity> {
    let Some(caster) = cx.subject else {
        return Vec::new();
    };
    let def = skill.get(world.resource::<Content>());
    match def.target {
        Targeting::Caster => vec![caster],
        Targeting::SingleAlly | Targeting::SingleEnemy => used_target(world, cx)
            .filter(|who| casting::fits(world, caster, *who, def.target))
            .into_iter()
            .collect(),
        Targeting::AreaAlly | Targeting::AreaEnemy => {
            let Some(footprint) = footprint(world, cx, skill) else {
                return Vec::new();
            };
            let Some(roster) = world.get_resource::<TickRoster>() else {
                return Vec::new();
            };
            let Some(area) = world.get::<AreaTag>(caster).map(|tag| tag.area) else {
                return Vec::new();
            };
            let mut inside: Vec<(Entity, Meters)> = roster
                .near(Among::Anyone, footprint.at, footprint.reach())
                .filter(|listed| listed.area == area && footprint.contains(listed.at))
                .filter(|listed| casting::fits(world, caster, listed.entity, def.target))
                .map(|listed| (listed.entity, listed.at.distance(footprint.at)))
                .collect();
            inside.sort_by(|(a, da), (b, db)| da.0.total_cmp(&db.0).then(a.cmp(b)));
            inside
                .into_iter()
                .take(AREA_TARGETS)
                .map(|(entity, _)| entity)
                .collect()
        }
    }
}

/// The ground a cast covers, from the caster toward the aimed point.
pub fn footprint(world: &World, cx: &FlowCx, skill: SkillId) -> Option<PlacedShape> {
    let caster = cx.subject?;
    let def = skill.get(world.resource::<Content>());
    let shape = def.area_at(used_rank(cx))?;
    let from = movement::position(world, caster)?;
    let point = aimed_point(world, cx, skill).unwrap_or(from);
    Some(PlacedShape::cast(shape, from, point))
}

pub fn aimed_point(world: &World, cx: &FlowCx, skill: SkillId) -> Option<Pos<Meters>> {
    let caster = cx.subject?;
    match skill.get(world.resource::<Content>()).target {
        Targeting::Caster => movement::position(world, caster),
        Targeting::SingleAlly | Targeting::SingleEnemy => {
            movement::position(world, used_target(world, cx)?)
        }
        Targeting::AreaAlly | Targeting::AreaEnemy => cx.event.get(USED_POINT)?.as_point(),
    }
}

fn used_target(world: &World, cx: &FlowCx) -> Option<Entity> {
    let target = cx.event.get(USED_TARGET)?.as_actor()?;
    world.get_entity(target).ok().map(|entity| entity.id())
}

fn used_rank(cx: &FlowCx) -> Rank {
    cx.event
        .get(USED_RANK)
        .and_then(|rank| u8::try_from(rank.as_int()).ok())
        .map_or(Rank::FIRST, Rank)
}

/// The rules a skill's script keeps: an active skill answers `used` and no aspect event, a passive
/// one the other way around.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<RiftScripts>();
    for (id, def) in loading.rows::<SkillDef>(&content) {
        let may_do = scripts.may_do(OwnerRow::of_row(id));
        let aspect_events = [
            EventKind::Activated,
            EventKind::Deactivated,
            EventKind::Interval,
        ];
        let (rule, expected) = match def.kind {
            SkillKind::Passive if may_do.answers(EventKind::Used) => (
                "passive answers used",
                "activated, deactivated or interval: a passive skill is never used",
            ),
            SkillKind::Active if aspect_events.into_iter().any(|kind| may_do.answers(kind)) => (
                "active answers an aspect event",
                "used: an active skill's script runs when it is used",
            ),
            SkillKind::Active if !may_do.answers(EventKind::Used) => (
                "skill without used",
                "a script answering used on every active skill",
            ),
            _ => continue,
        };
        loading.row_error(
            id,
            "/modules",
            ContentProblem::new(rule, format!("skill/{}", id.path(&content)), expected),
        );
    }
}
