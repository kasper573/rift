use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use egui::{Pos2, Rect, Vec2};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::content::{Content, RowCheck};
use crate::systems::player::Xp;
use crate::systems::script::condition;
use crate::systems::script::{Condition, condition::CompiledCondition};
use crate::systems::skill::{Rank, SkillId};

/// How far apart two nodes must sit on the canvas, in canvas units, not to overlap.
pub const NODE_EXTENT: f32 = 56.0;

#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SkillTree(pub BTreeMap<String, SkillNode>);

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillNode {
    pub skill: SkillId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<AccessRequirement>,
    pub at: CanvasPoint,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccessRequirement {
    Skill {
        node: String,
        #[schemars(extend("default" = 1))]
        #[serde(default = "first_rank", skip_serializing_if = "Rank::is_first")]
        rank: Rank,
    },
    Level {
        level: u32,
    },
    Condition {
        condition: Condition,
    },
}

#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasPoint {
    pub x: f32,
    pub y: f32,
}

/// The art behind a class's tree: an image drawn centred within a box of the canvas, which by
/// convention the designer draws around the nodes.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeBackdrop {
    pub image: AssetRef,
    pub frame: CanvasRect,
    #[serde(default)]
    pub fit: BackdropFit,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How a backdrop image meets its frame, as a stylesheet's object-fit would put it.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackdropFit {
    /// Scaled to show whole within the frame, leaving the frame bare along one axis.
    #[default]
    Contain,
    /// Scaled to fill the frame, cropped along one axis.
    Cover,
    /// Stretched to the frame's size.
    Stretch,
}

/// An arrow of the canvas: unlocking `from` leads toward `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlockArrow<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub rank: Rank,
}

impl SkillTree {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn node(&self, key: &str) -> Option<&SkillNode> {
        self.0.get(key)
    }

    pub fn node_of(&self, skill: SkillId) -> Option<(&str, &SkillNode)> {
        self.0
            .iter()
            .find(|(_, node)| node.skill == skill)
            .map(|(key, node)| (key.as_str(), node))
    }

    pub fn arrows(&self) -> Vec<UnlockArrow<'_>> {
        self.0
            .iter()
            .flat_map(|(to, node)| {
                node.requires
                    .iter()
                    .filter_map(move |requirement| match requirement {
                        AccessRequirement::Skill { node: from, rank } => Some(UnlockArrow {
                            from: from.as_str(),
                            to: to.as_str(),
                            rank: *rank,
                        }),
                        _ => None,
                    })
            })
            .collect()
    }

    /// The canvas the nodes take up, or nothing for an empty tree.
    pub fn bounds(&self) -> Option<Rect> {
        self.0
            .values()
            .map(|node| node.at.footprint())
            .reduce(|all, next| all.union(next))
    }

    pub fn roots(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(|(_, node)| {
                !node
                    .requires
                    .iter()
                    .any(|requirement| matches!(requirement, AccessRequirement::Skill { .. }))
            })
            .map(|(key, _)| key.as_str())
    }

    /// The nodes on every cycle of node requirements.
    pub fn cycles(&self) -> BTreeSet<&str> {
        let mut on_cycle = BTreeSet::new();
        for start in self.0.keys() {
            let mut stack = vec![start.as_str()];
            let mut seen = BTreeSet::new();
            while let Some(key) = stack.pop() {
                let Some(node) = self.0.get(key) else {
                    continue;
                };
                for requirement in &node.requires {
                    if let AccessRequirement::Skill { node: next, .. } = requirement {
                        if next == start {
                            on_cycle.insert(start.as_str());
                        } else if seen.insert(next.as_str()) {
                            stack.push(next.as_str());
                        }
                    }
                }
            }
        }
        on_cycle
    }

    pub fn check(&self, check: &mut RowCheck) {
        let cycles = self.cycles();
        let keys: Vec<&String> = self.0.keys().collect();
        for (key, node) in &self.0 {
            if let Some(twin) = self
                .0
                .iter()
                .find(|(other, twin)| *other != key && twin.skill == node.skill)
            {
                check.error(
                    format!("/tree/{key}/skill"),
                    "skill twice",
                    format!("the skill of {}", twin.0),
                    "each skill in one node of the tree",
                );
            }
            if let Some((other, _)) = self
                .0
                .iter()
                .find(|(other, twin)| *other < key && twin.at.overlaps(node.at))
            {
                check.error(
                    format!("/tree/{key}/at"),
                    "nodes overlap",
                    format!("({}, {}) over {other}", node.at.x, node.at.y),
                    format!("nodes at least {NODE_EXTENT} apart"),
                );
            }
            for (index, requirement) in node.requires.iter().enumerate() {
                let AccessRequirement::Skill { node: named, .. } = requirement else {
                    continue;
                };
                if named == key {
                    check.error(
                        format!("/tree/{key}/requires/{index}/node"),
                        "requires itself",
                        named,
                        "another node of the tree",
                    );
                } else if !self.0.contains_key(named) {
                    check.problem(
                        format!("/tree/{key}/requires/{index}/node"),
                        crate::core::content::ContentProblem::new(
                            "unknown node",
                            named,
                            "a node of this tree",
                        )
                        .near(crate::core::content::id::nearest(
                            keys.iter().copied(),
                            named,
                        )),
                    );
                } else if cycles.contains(key.as_str()) && cycles.contains(named.as_str()) {
                    check.error(
                        format!("/tree/{key}/requires/{index}"),
                        "cycle",
                        format!("{key} and {named} need each other"),
                        "requirements that lead somewhere",
                    );
                }
            }
        }
    }
}

impl CanvasPoint {
    pub fn overlaps(self, other: CanvasPoint) -> bool {
        (self.x - other.x).abs() < NODE_EXTENT && (self.y - other.y).abs() < NODE_EXTENT
    }

    /// The canvas a node takes up: its disc, its label and the room kept clear around it.
    pub fn footprint(self) -> Rect {
        Rect::from_center_size(Pos2::new(self.x, self.y), Vec2::splat(NODE_EXTENT * 2.0))
    }
}

impl CanvasRect {
    pub fn rect(self) -> Rect {
        Rect::from_min_size(
            Pos2::new(self.x, self.y),
            Vec2::new(self.width, self.height),
        )
    }

    pub fn from_rect(rect: Rect) -> CanvasRect {
        CanvasRect {
            x: rect.min.x,
            y: rect.min.y,
            width: rect.width(),
            height: rect.height(),
        }
    }
}

impl BackdropFit {
    /// Where an image of `image` size lands within `frame`, centred; whatever falls outside the
    /// frame is clipped away.
    pub fn place(self, frame: Rect, image: Vec2) -> Rect {
        let image = image.max(Vec2::splat(1.0));
        let scale = match self {
            BackdropFit::Contain => (frame.width() / image.x).min(frame.height() / image.y),
            BackdropFit::Cover => (frame.width() / image.x).max(frame.height() / image.y),
            BackdropFit::Stretch => {
                return frame;
            }
        };
        Rect::from_center_size(frame.center(), image * scale)
    }
}

impl TreeBackdrop {
    pub fn check(&self, check: &mut RowCheck) {
        check.range(
            "/backdrop/frame/width",
            self.frame.width,
            1.0,
            8192.0,
            "a width of",
        );
        check.range(
            "/backdrop/frame/height",
            self.frame.height,
            1.0,
            8192.0,
            "a height of",
        );
    }
}

impl AccessRequirement {
    /// Whether an actor who has `ranks` meets it, with `xp` and the world answering the rest.
    pub fn met(
        &self,
        world: &World,
        actor: Entity,
        class: &super::ClassDef,
        ranks: &BTreeMap<String, Rank>,
    ) -> bool {
        match self {
            AccessRequirement::Skill { node, rank } => {
                ranks.get(node).is_some_and(|held| held >= rank)
            }
            AccessRequirement::Level { level } => {
                let xp = world.get::<Xp>(actor).map_or(0, |xp| xp.amount);
                class.level_at(xp) >= *level
            }
            AccessRequirement::Condition { condition } => condition::holds(world, actor, condition),
        }
    }

    pub fn describe(&self, content: &Content, tree: &SkillTree) -> String {
        match self {
            AccessRequirement::Skill { node, rank } => {
                let name = tree
                    .node(node)
                    .map(|node| node.skill.get(content).name.clone())
                    .unwrap_or_else(|| node.clone());
                format!("{name} rank {}", rank.0)
            }
            AccessRequirement::Level { level } => format!("Level {level}"),
            AccessRequirement::Condition { condition } => condition::describe(content, condition),
        }
    }

    pub fn compiled(&self, content: &Content) -> Option<CompiledCondition> {
        match self {
            AccessRequirement::Condition { condition } => {
                Some(CompiledCondition::new(content, condition))
            }
            _ => None,
        }
    }
}

fn first_rank() -> Rank {
    Rank(1)
}

impl Rank {
    pub fn is_first(&self) -> bool {
        self.0 == 1
    }
}
