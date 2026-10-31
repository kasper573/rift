pub mod tree;

pub use tree::{
    AccessRequirement, BackdropFit, CanvasPoint, CanvasRect, NODE_EXTENT, NodeKey, SkillNode,
    SkillTree, TreeBackdrop,
};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui;

use crate::core::assets::ImageRef;
use crate::core::content::{
    Content, ContentFile, ContentId, Loading, NamedContent, Nearby, RowCheck,
};
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault, ValueType};
use crate::core::vfx::VfxSite;
use crate::systems::actor::ModelId;
use crate::systems::dialogue::bust::BustId;
use crate::systems::notification;
use crate::systems::player::Xp;
use crate::systems::script::{
    Condition, EffectCall, EventKind, Reach, RiftInstances, RiftScripting, area_owner,
    effect as effects,
};
use crate::systems::skill::{self, Learned, SkillDef};
use crate::systems::stat::{Stat, Stats};
use crate::systems::vfx::{self, VfxFixture, VfxShaderFixture};

pub fn register(app: &mut App) {
    use crate::systems::mirror::AppMirrorExt;
    app.replicate::<Class>();
}

pub type ClassId = ContentId<ClassDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ClassDef {
    pub name: String,
    pub icon: ImageRef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flavour: String,
    /// What a new character of the class looks like.
    pub model: ModelId,
    /// The portrait a conversation shows beside the player's lines, in a game that shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bust: Option<BustId>,
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

    /// The levels reached at `xp`, in order: level one is the first listed.
    pub fn reached(&self, xp: u32) -> impl Iterator<Item = &ClassLevel> {
        self.levels.iter().filter(move |tier| tier.xp <= xp)
    }

    pub fn level_at(&self, xp: u32) -> u32 {
        self.reached(xp).count() as u32
    }

    pub fn granted_points(&self, xp: u32) -> SkillPoints {
        self.reached(xp)
            .fold(SkillPoints(0), |sum, tier| sum + tier.skill_points)
    }

    pub fn stats_at(&self, xp: u32) -> impl Iterator<Item = Stat> + '_ {
        self.reached(xp).flat_map(|tier| tier.stats.listed())
    }
}

impl ContentFile for ClassDef {
    const TABLE: &'static str = "class";

    fn fresh(near: &Nearby) -> Result<ClassDef, String> {
        let first = ClassLevel {
            xp: 0,
            stats: Stats::default(),
            skill_points: SkillPoints(1),
        };
        Ok(ClassDef {
            name: near.name(),
            icon: near.image()?,
            flavour: String::new(),
            model: near.file()?,
            bust: None,
            levels: vec![first],
            tree: SkillTree::default(),
            backdrop: None,
        })
    }

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

impl NamedContent for ClassDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ClassLevel {
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Stats::is_empty")]
    pub stats: Stats,
    #[serde(default, skip_serializing_if = "SkillPoints::is_zero")]
    pub skill_points: SkillPoints,
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

impl SkillPoints {
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Class {
    pub def: ClassId,
}

pub fn of(world: &World, entity: Entity) -> Option<ClassId> {
    world.get::<Class>(entity).map(|class| class.def)
}

pub fn xp_of(world: &World, entity: Entity) -> u32 {
    world.get::<Xp>(entity).map_or(0, |xp| xp.amount)
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
        vfx::fixture::play(world, VfxFixture::LevelUp, VfxSite::Actor(entity));
        let shown = format!("Level {reached}");
        notification::pop_as(world, entity, shown, VfxShaderFixture::LevelPop);
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

pub async fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<ClassDef>(&content) {
        let mut check = RowCheck::default();
        for (key, node) in &def.tree.0 {
            for (index, requirement) in node.requires.iter().enumerate() {
                if let AccessRequirement::Skill { node: other, rank } = requirement
                    && let Some(named) = def.tree.node(other)
                {
                    skill::check_rank(
                        &mut check,
                        format!("/tree/{key}/requires/{index}/rank"),
                        &content,
                        named.skill,
                        *rank,
                    );
                }
            }
            if content
                .table::<SkillDef>()
                .get(node.skill)
                .is_some_and(|skill| skill.rank_count() == 0)
            {
                check.error(
                    format!("/tree/{key}/skill"),
                    "no ranks",
                    node.skill.path(&content),
                    "a skill with at least one rank",
                );
            }
        }
        loading.row_errors(id, check);
    }
}

/// Grants experience as kills do.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "class"))]
pub struct GrantXp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[schemars(extend("default" = 10))]
    pub xp: Condition,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for GrantXp {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let xp = cx.expr(&self.xp, ValueType::Integer)?;
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                let amount = u32::try_from(xp(world, cx).as_int()).unwrap_or(0);
                grant_xp(world, player, amount);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::GrantsXp]
    }
}

pub(crate) fn level_stats(world: &World, entity: Entity, into: &mut Vec<Stat>) {
    let content = world.resource::<Content>();
    if let Some(class) = of(world, entity) {
        into.extend(class.get(content).stats_at(xp_of(world, entity)));
    }
}
