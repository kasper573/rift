use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::egui::{Pos2, Rect, Vec2};

use crate::core::content::ImageRef;
use crate::core::content::{ContentNames, ContentProblem, RowCheck, id};
use crate::systems::script::{Condition, condition};
use crate::systems::skill::{Rank, SkillId};

/// How far apart two nodes must sit on the canvas, in canvas units, not to overlap.
pub const NODE_EXTENT: f32 = 56.0;

/// The name of a node within its class tree, as the row keys it.
#[derive(
    JsonSchema, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeKey(pub String);

#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SkillTree(pub BTreeMap<NodeKey, SkillNode>);

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
        node: NodeKey,
        #[schemars(extend("default" = 1))]
        #[serde(default = "Rank::first", skip_serializing_if = "Rank::is_first")]
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
    pub image: ImageRef,
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
#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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
    pub from: &'a NodeKey,
    pub to: &'a NodeKey,
    pub rank: Rank,
}

impl SkillTree {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn node<K: Ord + ?Sized>(&self, key: &K) -> Option<&SkillNode>
    where
        NodeKey: Borrow<K>,
    {
        self.0.get(key)
    }

    pub fn node_of(&self, skill: SkillId) -> Option<(&NodeKey, &SkillNode)> {
        self.0.iter().find(|(_, node)| node.skill == skill)
    }

    pub fn arrows(&self) -> Vec<UnlockArrow<'_>> {
        self.0
            .iter()
            .flat_map(|(to, node)| {
                node.requires
                    .iter()
                    .filter_map(move |requirement| match requirement {
                        AccessRequirement::Skill { node: from, rank } => Some(UnlockArrow {
                            from,
                            to,
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

    /// Whether reaching `node` takes `on` first, through any chain of node requirements.
    pub fn depends_on<K: Ord + ?Sized>(&self, node: &K, on: &K) -> bool
    where
        NodeKey: Borrow<K>,
    {
        let mut stack: Vec<&NodeKey> = self.0.get(node).map_or(Vec::new(), |node| {
            node.requires.iter().filter_map(required_node).collect()
        });
        let mut seen = BTreeSet::new();
        while let Some(key) = stack.pop() {
            if key.borrow() == on {
                return true;
            }
            if let Some(node) = self.0.get::<NodeKey>(key)
                && seen.insert(key)
            {
                stack.extend(node.requires.iter().filter_map(required_node));
            }
        }
        false
    }

    /// The nodes on every cycle of node requirements.
    pub fn cycles(&self) -> BTreeSet<&NodeKey> {
        self.0
            .keys()
            .filter(|key| self.depends_on(*key, *key))
            .collect()
    }

    pub fn check(&self, check: &mut RowCheck) {
        let cycles = self.cycles();
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
                        ContentProblem::new("unknown node", named, "a node of this tree")
                            .near(id::nearest(self.0.keys().map(|key| &key.0), named.as_str())),
                    );
                } else if cycles.contains(key) && cycles.contains(named) {
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

fn required_node(requirement: &AccessRequirement) -> Option<&NodeKey> {
    match requirement {
        AccessRequirement::Skill { node, .. } => Some(node),
        AccessRequirement::Level { .. } | AccessRequirement::Condition { .. } => None,
    }
}

impl CanvasPoint {
    pub fn overlaps(self, other: CanvasPoint) -> bool {
        (self.x - other.x).abs() < NODE_EXTENT && (self.y - other.y).abs() < NODE_EXTENT
    }

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
    /// The one rule of access: whether someone holding `ranks` at `level` meets it, with
    /// `condition` answering for the world.
    pub fn met_by(
        &self,
        ranks: &BTreeMap<NodeKey, Rank>,
        level: u32,
        condition: impl Fn(&Condition) -> bool,
    ) -> bool {
        match self {
            AccessRequirement::Skill { node, rank } => {
                ranks.get(node).is_some_and(|held| held >= rank)
            }
            AccessRequirement::Level { level: needed } => level >= *needed,
            AccessRequirement::Condition { condition: wanted } => condition(wanted),
        }
    }

    /// `met_by` for an actor of the world, holding `ranks`.
    pub fn met(
        &self,
        world: &World,
        actor: Entity,
        class: &super::ClassDef,
        ranks: &BTreeMap<NodeKey, Rank>,
    ) -> bool {
        let level = class.level_at(super::xp_of(world, actor));
        self.met_by(ranks, level, |condition| {
            condition::holds(world, actor, condition)
        })
    }

    pub fn describe(&self, content: &impl ContentNames, tree: &SkillTree) -> String {
        match self {
            AccessRequirement::Skill { node, rank } => {
                let name = tree
                    .node(node)
                    .map(|node| content.name_of(node.skill))
                    .unwrap_or_else(|| node.to_string());
                format!("{name} rank {}", rank.0)
            }
            AccessRequirement::Level { level } => format!("Level {level}"),
            AccessRequirement::Condition { condition } => {
                condition::describe(content, condition).to_string()
            }
        }
    }
}

impl NodeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for NodeKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NodeKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeKey {
    fn from(key: &str) -> NodeKey {
        NodeKey(key.to_owned())
    }
}

impl From<String> for NodeKey {
    fn from(key: String) -> NodeKey {
        NodeKey(key)
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
