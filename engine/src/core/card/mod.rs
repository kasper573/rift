//! Card styles: the content that says how a card looks, apart from its picture and its words.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::{
    Content, ContentFile, ContentId, Fixture, ImageRef, Nearby, RowCheck, serial,
};
use crate::core::math::{ColorGradient, Rgb};

pub mod dress;

pub type CardStyleId = ContentId<CardStyleDef>;

/// The whole appearance of a card, apart from its picture and what it says: its colours, and
/// fields that each pick one of a few looks, with that look's own measures. A card whose file
/// names no style is drawn in the game's default.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardStyleDef {
    /// The colour behind the picture, where the card brings none of its own.
    pub ground: ColorGradient,
    /// The two colours the looks paint with. Needed only by a look that paints in them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<ColorGradient>,
    /// The icon the looks draw. Needed only by a look that draws it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ImageRef>,
    /// The word the looks write. Needed only by a look that writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub border: CardBorder,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub corners: CardCorners,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub crest: CardCrest,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub name_band: CardNameBand,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub label_line: CardLabelLine,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub art: CardArt,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub paper: CardPaper,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub dividers: CardDividers,
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub halo: CardHalo,
}

impl CardStyleDef {
    /// The style a card is drawn in: the one its file names, or the game's default.
    pub fn of(content: &Content, named: Option<CardStyleId>) -> &CardStyleDef {
        named
            .unwrap_or_else(|| CardStyleFixture::Default.id(content))
            .get(content)
    }
}

/// A length on a card, in the points the interface is drawn in.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardPoints(pub f32);

/// The stroke round a card.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardBorder {
    #[default]
    Ink,
    /// A stroke in the gradient along the card's edge.
    Gradient { width: CardPoints },
    /// A stroke in the gradient along the card's edge, and one of ink a gap inside it.
    Double { width: CardPoints, gap: CardPoints },
    /// An image tiled along the edge and tinted by the gradient.
    Patterned { image: ImageRef, width: CardPoints },
}

/// The shape of a card's corners.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardCorners {
    Round {
        radius: CardPoints,
    },
    /// Cut off at an angle.
    Cut {
        size: CardPoints,
    },
    /// Stepped inward.
    Notched {
        size: CardPoints,
    },
}

impl Default for CardCorners {
    fn default() -> CardCorners {
        CardCorners::Round {
            radius: CardPoints(ui::card::CORNER),
        }
    }
}

/// What rises over a card's top edge.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardCrest {
    #[default]
    None,
    /// A tab holding the style's icon.
    Icon { size: CardPoints },
    /// A banner carrying the style's label.
    Banner,
}

/// How the band under the picture is filled, and whether the style's icon stands before the name.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardNameBand {
    Plain {
        #[serde(default, skip_serializing_if = "serial::is_false")]
        icon: bool,
    },
    /// Washed in the gradient at both ends, as strongly as `strength` of one says.
    Washed {
        strength: f32,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        icon: bool,
    },
    /// Filled with the gradient, under a light name.
    Filled {
        #[serde(default, skip_serializing_if = "serial::is_false")]
        icon: bool,
    },
    /// A ribbon that reaches past the card's edge on both sides.
    Ribbon {
        overhang: CardPoints,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        icon: bool,
    },
}

impl Default for CardNameBand {
    fn default() -> CardNameBand {
        CardNameBand::Plain { icon: false }
    }
}

/// Where a card writes the style's label.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardLabelLine {
    #[default]
    None,
    /// On a strip under the name band.
    Strip,
    /// As a seal stamped on the paper.
    Seal,
}

/// How the picture is framed and how tall it stands. Without a height it is as tall as the card
/// makes its picture.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardArt {
    Flush {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<CardPoints>,
    },
    /// Set in from the frame by a margin, with a thin border of its own.
    Inset {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<CardPoints>,
        margin: CardPoints,
    },
    /// Arched at the top.
    Arched {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<CardPoints>,
    },
}

impl Default for CardArt {
    fn default() -> CardArt {
        CardArt::Flush { height: None }
    }
}

/// What lies under a card's text.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardPaper {
    #[default]
    Cream,
    /// Cream tinted with the gradient, as strongly as `strength` of one says.
    Tinted { strength: f32 },
    /// Cream with the style's icon on it, large and as faint as `strength` of one says.
    Watermarked { strength: f32 },
}

/// The rule between the sections of a card's text.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardDividers {
    #[default]
    Line,
    /// A line in the gradient that fades at both ends.
    Gradient,
    /// A line broken by the style's icon.
    Ornament,
}

/// What surrounds a card as it lies on the screen.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardHalo {
    #[default]
    Shadow,
    /// A glow in the gradient that spreads so far.
    Glow { spread: CardPoints },
}

impl ContentFile for CardStyleDef {
    const TABLE: &'static str = "card_style";

    fn fresh(_: &Nearby) -> Result<CardStyleDef, String> {
        Ok(CardStyleDef {
            ground: ColorGradient {
                from: Rgb::WHITE,
                to: Rgb::WHITE,
            },
            gradient: None,
            icon: None,
            label: None,
            border: CardBorder::default(),
            corners: CardCorners::default(),
            crest: CardCrest::default(),
            name_band: CardNameBand::default(),
            label_line: CardLabelLine::default(),
            art: CardArt::default(),
            paper: CardPaper::default(),
            dividers: CardDividers::default(),
            halo: CardHalo::default(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        if let Some(label) = &self.label {
            check.text("/label", label, 1, MOST_LABEL);
        }
        self.check_measures(check);
        for (pointer, need) in self.needs() {
            let (has, found, expected) = match need {
                StyleNeed::Gradient => (
                    self.gradient.is_some(),
                    "no gradient",
                    "a gradient, which this look paints in",
                ),
                StyleNeed::Icon => (
                    self.icon.is_some(),
                    "no icon",
                    "an icon, which this look draws",
                ),
                StyleNeed::Label => (
                    self.label.is_some(),
                    "no label",
                    "a label, which this look writes",
                ),
            };
            if !has {
                check.error(pointer, "look without what it paints with", found, expected);
            }
        }
    }
}

/// The style of every card whose file names none: `fixtures/default.card_style.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum CardStyleFixture {
    Default,
}

impl Fixture for CardStyleFixture {
    type Row = CardStyleDef;
    const FOLDER: Option<&'static str> = None;
}

const MOST_LABEL: usize = 24;
const CREST_ICON: (f32, f32) = (8.0, 48.0);
const MOST_OVERHANG: f32 = 24.0;
const ART_HEIGHT: (f32, f32) = (40.0, 320.0);
const MOST_ART_MARGIN: f32 = 24.0;
const MOST_GLOW: f32 = 48.0;

enum StyleNeed {
    Gradient,
    Icon,
    Label,
}

impl CardStyleDef {
    fn check_measures(&self, check: &mut RowCheck) {
        let room = ui::card::BORDER_ROOM;
        match &self.border {
            CardBorder::Ink => {}
            CardBorder::Gradient { width } | CardBorder::Patterned { width, .. } => {
                check.above_up_to("/border/width", width.0, 0.0, room, "points");
            }
            CardBorder::Double { width, gap } => {
                check.above_up_to("/border/width", width.0, 0.0, room / 2.0, "points");
                let left = (room - 2.0 * width.0).max(0.0);
                check.range("/border/gap", gap.0, 0.0, left, "points");
            }
        }
        match self.corners {
            CardCorners::Round { radius } => {
                check.range(
                    "/corners/radius",
                    radius.0,
                    0.0,
                    ui::card::ROUND_ROOM,
                    "points",
                );
            }
            CardCorners::Cut { size } | CardCorners::Notched { size } => {
                check.above_up_to(
                    "/corners/size",
                    size.0,
                    0.0,
                    ui::card::CORNER_ROOM,
                    "points",
                );
            }
        }
        if let CardCrest::Icon { size } = self.crest {
            check.range("/crest/size", size.0, CREST_ICON.0, CREST_ICON.1, "points");
        }
        match self.name_band {
            CardNameBand::Washed { strength, .. } => share(check, "/name_band/strength", strength),
            CardNameBand::Ribbon { overhang, .. } => {
                check.above_up_to(
                    "/name_band/overhang",
                    overhang.0,
                    0.0,
                    MOST_OVERHANG,
                    "points",
                );
            }
            CardNameBand::Plain { .. } | CardNameBand::Filled { .. } => {}
        }
        let (CardArt::Flush { height }
        | CardArt::Inset { height, .. }
        | CardArt::Arched { height }) = self.art;
        if let Some(height) = height {
            check.range(
                "/art/height",
                height.0,
                ART_HEIGHT.0,
                ART_HEIGHT.1,
                "points",
            );
        }
        if let CardArt::Inset { margin, .. } = self.art {
            check.above_up_to("/art/margin", margin.0, 0.0, MOST_ART_MARGIN, "points");
        }
        if let CardPaper::Tinted { strength } | CardPaper::Watermarked { strength } = self.paper {
            share(check, "/paper/strength", strength);
        }
        if let CardHalo::Glow { spread } = self.halo {
            check.above_up_to("/halo/spread", spread.0, 0.0, MOST_GLOW, "points");
        }
    }

    /// What each look the style picks paints with, by where the look is written.
    fn needs(&self) -> Vec<(&'static str, StyleNeed)> {
        let mut needs = Vec::new();
        if self.border != CardBorder::Ink {
            needs.push(("/border", StyleNeed::Gradient));
        }
        match self.crest {
            CardCrest::None => {}
            CardCrest::Icon { .. } => needs.push(("/crest", StyleNeed::Icon)),
            CardCrest::Banner => needs.push(("/crest", StyleNeed::Label)),
        }
        let with_icon = match self.name_band {
            CardNameBand::Plain { icon } => icon,
            CardNameBand::Washed { icon, .. }
            | CardNameBand::Filled { icon }
            | CardNameBand::Ribbon { icon, .. } => {
                needs.push(("/name_band", StyleNeed::Gradient));
                icon
            }
        };
        if with_icon {
            needs.push(("/name_band/icon", StyleNeed::Icon));
        }
        if self.label_line != CardLabelLine::None {
            needs.push(("/label_line", StyleNeed::Label));
        }
        match self.paper {
            CardPaper::Cream => {}
            CardPaper::Tinted { .. } => needs.push(("/paper", StyleNeed::Gradient)),
            CardPaper::Watermarked { .. } => needs.push(("/paper", StyleNeed::Icon)),
        }
        match self.dividers {
            CardDividers::Line => {}
            CardDividers::Gradient => needs.push(("/dividers", StyleNeed::Gradient)),
            CardDividers::Ornament => needs.push(("/dividers", StyleNeed::Icon)),
        }
        if self.halo != CardHalo::Shadow {
            needs.push(("/halo", StyleNeed::Gradient));
        }
        needs
    }
}

fn share(check: &mut RowCheck, pointer: &'static str, strength: f32) {
    check.above_up_to(pointer, strength, 0.0, 1.0, "a share of one,");
}
