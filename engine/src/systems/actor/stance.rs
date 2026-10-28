//! How a wearer holds itself with what its hands show: a stance is a small content file items name
//! and skeleton sets are kept by. The stance is never stored; it is derived from what the hands
//! show, the same on the server and on every client.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentFile, ContentId, NamedContent, Nearby, RowCheck};
use crate::systems::item::{ItemId, Wear};

pub type StanceId = ContentId<StanceDef>;

/// How a wearer holds what puts it in the stance: how many hands it takes, which hand its clips
/// were made for, which of those hands the item hangs from, and what the stance becomes beside
/// another in the next hand.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StanceDef {
    pub name: String,
    pub hands: HandCount,
    pub hand: HandRole,
    #[serde(default, skip_serializing_if = "PropHand::is_first")]
    pub prop: PropHand,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paired: Vec<Pairing>,
}

/// How many hands an item of the stance takes: one, or two side by side.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
#[schemars(extend("x-count" = true))]
pub struct HandCount(pub u8);

/// The hand a stance's clips were made for, where equipping puts its item unless the wearer
/// holds it in another.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandRole {
    /// The first hand of the body.
    Main,
    /// The second hand of the body.
    Off,
    /// The first hand free.
    Either,
}

/// Which of the hands an item of the stance takes its prop hangs from, as the stance's clips hold
/// it: a bow in the hand that grips it, not the one that draws.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropHand {
    #[default]
    First,
    Last,
}

impl PropHand {
    pub fn is_first(&self) -> bool {
        *self == PropHand::First
    }
}

/// What the stance becomes when the next hand shows an item of the stance `with`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pairing {
    pub with: StanceId,
    pub becomes: StanceId,
}

/// A hand of a body, by its place in the skeleton's order: 0 is the main hand, 1 the off hand.
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
pub struct HandIndex(pub u8);

impl HandIndex {
    pub const MAIN: HandIndex = HandIndex(0);
    pub const OFF: HandIndex = HandIndex(1);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl ContentFile for StanceDef {
    const TABLE: &'static str = "stance";

    fn fresh(near: &Nearby) -> Result<StanceDef, String> {
        Ok(StanceDef {
            name: near.name(),
            hands: HandCount(1),
            hand: HandRole::Main,
            prop: PropHand::First,
            paired: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 64);
        if !(1..=2).contains(&self.hands.0) {
            cx.error(
                "/hands",
                "out of range",
                format!("{} hands", self.hands.0),
                "1 or 2 hands",
            );
        }
        for (index, pairing) in self.paired.iter().enumerate() {
            if self.paired[..index]
                .iter()
                .any(|earlier| earlier.with == pairing.with)
            {
                cx.error(
                    format!("/paired/{index}/with"),
                    "paired twice",
                    format!("paired names stance {} twice", pairing.with.raw()),
                    "each stance paired once",
                );
            }
        }
    }
}

impl NamedContent for StanceDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl StanceDef {
    pub fn paired_with(&self, with: StanceId) -> Option<StanceId> {
        self.paired
            .iter()
            .find(|pairing| pairing.with == with)
            .map(|pairing| pairing.becomes)
    }
}

/// The stance of an item a hand can show, when it is one.
pub fn of_item(content: &Content, item: ItemId) -> Option<StanceId> {
    match item.get(content).wear()? {
        Wear::Held { stance, .. } => Some(*stance),
        Wear::Worn { .. } => None,
    }
}

/// The stance the hands put their wearer in: the main stance is the first hand's, in the body's
/// order, showing an item with a stance; the off stance the next hand's past it; the two paired
/// when either lists the other, whichever hand holds which, else the main stance alone.
pub fn of_hands(content: &Content, hands: &BTreeMap<HandIndex, ItemId>) -> Option<StanceId> {
    let mut styled = hands
        .iter()
        .filter_map(|(&hand, &item)| Some((hand, of_item(content, item)?)));
    let (main_hand, main) = styled.next()?;
    let spans = main.get(content).hands.0.max(1);
    let past = HandIndex(main_hand.0.saturating_add(spans));
    let off = styled
        .find(|(hand, _)| *hand >= past)
        .map(|(_, stance)| stance);
    Some(
        off.and_then(|off| {
            main.get(content)
                .paired_with(off)
                .or_else(|| off.get(content).paired_with(main))
        })
        .unwrap_or(main),
    )
}
