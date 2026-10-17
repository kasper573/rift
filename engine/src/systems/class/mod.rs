pub mod tree;

pub use tree::{
    AccessRequirement, BackdropFit, CanvasPoint, CanvasRect, NODE_EXTENT, SkillNode, SkillTree,
    TreeBackdrop,
};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::content::{Content, ContentId, ContentRow, Loading, RowCheck, serial};
use crate::core::script::{Compiler, EffectDef, EffectStep, Name, ScriptFault, ValueType};
use crate::core::vfx::VfxSite;
use crate::systems::player::Xp;
use crate::systems::script::effect::{self as effects, Aim, Arrow, Link};
use crate::systems::script::{
    Condition, EffectCall, EventKind, Reach, RiftInstances, RiftScripting, area_owner,
};
use crate::systems::skill::{Learned, SkillDef};
use crate::systems::stat::{self, Stat, StatMap};
use crate::systems::vfx::{self, VfxFixture};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Class>();
    stat::source(app, level_stats);
}

pub type ClassId = ContentId<ClassDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ClassDef {
    pub name: String,
    pub icon: AssetRef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flavour: String,
    pub levels: Vec<ClassLevel>,
    #[serde(default, skip_serializing_if = "SkillTree::is_empty")]
    pub tree: SkillTree,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<TreeBackdrop>,
}

impl ClassDef {
    /// The canvas the tree is shown on: its backdrop's frame and every node's footprint. A
    /// window never shows past it.
    pub fn canvas(&self) -> Option<egui::Rect> {
        let nodes = self.tree.bounds();
        let frame = self.backdrop.as_ref().map(|backdrop| backdrop.frame.rect());
        match (nodes, frame) {
            (Some(nodes), Some(frame)) => Some(nodes.union(frame)),
            (nodes, frame) => nodes.or(frame),
        }
    }
}

impl ContentRow for ClassDef {
    const TABLE: &'static str = "class";

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count("/levels", self.levels.len(), 1, 200, "levels");
        for (index, level) in self.levels.iter().enumerate() {
            let before = index.checked_sub(1).map(|earlier| self.levels[earlier].xp);
            match before {
                None if level.xp != 0 => check.error(
                    "/levels/0/xp",
                    "first level",
                    level.xp,
                    "0: the first level is the one a character starts at",
                ),
                Some(earlier) if level.xp <= earlier => check.error(
                    format!("/levels/{index}/xp"),
                    "levels out of order",
                    level.xp,
                    format!("more than {earlier}, the level before"),
                ),
                _ => {}
            }
        }
        self.tree.check(check);
        if let Some(backdrop) = &self.backdrop {
            backdrop.check(check);
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ClassLevel {
    pub xp: u32,
    #[serde(default, skip_serializing_if = "StatMap::is_empty")]
    pub stats: StatMap,
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub skill_points: u32,
}

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
    derive_more::Add,
    derive_more::Sub,
    derive_more::AddAssign,
)]
#[serde(transparent)]
pub struct SkillPoints(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Class {
    pub def: ClassId,
}

impl ClassDef {
    /// The levels reached at `xp`: level one is the first listed.
    pub fn level_at(&self, xp: u32) -> u32 {
        self.levels.iter().filter(|tier| tier.xp <= xp).count() as u32
    }

    pub fn granted_points(&self, xp: u32) -> SkillPoints {
        SkillPoints(
            self.levels
                .iter()
                .filter(|tier| tier.xp <= xp)
                .map(|tier| tier.skill_points)
                .sum(),
        )
    }

    pub fn total_points(&self) -> SkillPoints {
        SkillPoints(self.levels.iter().map(|tier| tier.skill_points).sum())
    }

    pub fn stats_at(&self, xp: u32) -> impl Iterator<Item = Stat> + '_ {
        self.levels
            .iter()
            .filter(move |tier| tier.xp <= xp)
            .flat_map(|tier| tier.stats.listed())
    }
}

pub fn of(world: &World, entity: Entity) -> Option<ClassId> {
    world.get::<Class>(entity).map(|class| class.def)
}

pub fn level(world: &World, entity: Entity) -> u32 {
    let content = world.resource::<Content>();
    let Some(class) = of(world, entity) else {
        return 0;
    };
    class.get(content).level_at(xp_of(world, entity))
}

pub fn granted_points(world: &World, entity: Entity) -> SkillPoints {
    let content = world.resource::<Content>();
    of(world, entity).map_or(SkillPoints(0), |class| {
        class.get(content).granted_points(xp_of(world, entity))
    })
}

/// The points the learned ranks cost, by the skill rows as they are now.
pub fn spent_points(world: &World, entity: Entity) -> SkillPoints {
    let content = world.resource::<Content>();
    let Some(class) = of(world, entity) else {
        return SkillPoints(0);
    };
    let tree = &class.get(content).tree;
    world
        .get::<Learned>(entity)
        .map_or(SkillPoints(0), |learned| learned.cost(content, tree))
}

/// Granted minus spent; a row that shrank its grant leaves nothing to spend until a reset.
pub fn points_to_spend(world: &World, entity: Entity) -> SkillPoints {
    let granted = granted_points(world, entity);
    let spent = spent_points(world, entity);
    SkillPoints(granted.0.saturating_sub(spent.0))
}

pub fn grant_xp(world: &mut World, entity: Entity, amount: u32) {
    let before = level(world, entity);
    if let Some(mut xp) = world.get_mut::<Xp>(entity) {
        xp.gain(amount);
    }
    let reached = level(world, entity);
    if reached > before {
        let level = &[("level", reached as f32)];
        vfx::fixture::play(world, VfxFixture::LevelUp, VfxSite::Actor(entity), level);
        vfx::fixture::play_for(world, entity, VfxFixture::LevelUp, VfxSite::Screen, level);
        let owner = area_owner(world);
        world.resource_mut::<RiftInstances>().raise(
            owner,
            EventKind::LeveledUp,
            Some(entity),
            Vec::new(),
            |_| true,
        );
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<ClassDef>(&content) {
        let mut check = RowCheck::default();
        for (key, node) in &def.tree.0 {
            let ranks = node.skill.get(&content).levels.len() as u8;
            for (index, requirement) in node.requires.iter().enumerate() {
                if let AccessRequirement::Skill { node: other, rank } = requirement
                    && let Some(named) = def.tree.0.get(other)
                {
                    let has = named.skill.get(&content).levels.len() as u8;
                    if rank.0 == 0 || rank.0 > has {
                        check.error(
                            format!("/tree/{key}/requires/{index}/rank"),
                            "rank out of range",
                            rank.0,
                            format!("1 to {has}, the ranks of {}", named.skill.key(&content)),
                        );
                    }
                }
            }
            if ranks == 0 {
                check.error(
                    format!("/tree/{key}/skill"),
                    "no ranks",
                    node.skill.key(&content),
                    "a skill with at least one rank",
                );
            }
        }
        loading.row_errors(id, check);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct GrantXp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub xp: Condition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for GrantXp {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![("next", self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let xp = cx.expr(&self.xp, ValueType::Integer)?;
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                let amount = xp(world, cx).as_int().clamp(0, i64::from(u32::MAX)) as u32;
                grant_xp(world, player, amount);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::GrantsXp]
    }
}

fn xp_of(world: &World, entity: Entity) -> u32 {
    world.get::<Xp>(entity).map_or(0, |xp| xp.amount)
}

fn level_stats(world: &World, entity: Entity) -> Vec<Stat> {
    let content = world.resource::<Content>();
    let Some(class) = of(world, entity) else {
        return Vec::new();
    };
    class.get(content).stats_at(xp_of(world, entity)).collect()
}

impl SkillDef {
    pub fn rank_count(&self) -> u8 {
        self.levels.len() as u8
    }
}
