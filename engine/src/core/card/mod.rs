//! Card styles: the content that says how a card looks, apart from its picture and its words.

use bevy::prelude::Resource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::{card, sheet};

use crate::core::content::{
    Content, ContentFile, ContentId, Fixture, ImageRef, Nearby, RowCheck, serial,
};
use crate::core::math::{ColorGradient, Rgb};
use crate::core::units::{Degrees, Seconds};

pub mod dress;
pub mod stage;

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
    pub material: CardMaterial,
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

/// How the picture is framed within its part of the card, which is as large on every card.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardArt {
    #[default]
    Flush,
    /// Set in from the frame by a margin, with a thin border of its own.
    Inset { margin: CardPoints },
}

/// What a card is made of, and so what it does with the lamp's light. Each measure is a share
/// of one.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardMaterial {
    /// Takes no shine, and only shades a little as it turns.
    #[default]
    Matte,
    /// A wide, soft glow, the tighter the higher its gloss.
    Satin { gloss: f32 },
    /// A clear coat over the print: a small, bright highlight, the sharper the higher its gloss.
    Lacquer { gloss: f32 },
    /// The print itself is metal, the more mirror-like the higher its polish.
    Metal { polish: f32 },
    /// Splits the lamp's light into colours over `strength` of the print. The image shapes how
    /// the colours break up.
    Foil { pattern: ImageRef, strength: f32 },
    /// Raised as the image says, a normal map of which way each point of the face leans. Its
    /// ridges catch the lamp the brighter the higher its gloss.
    Embossed { relief: ImageRef, gloss: f32 },
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
            material: CardMaterial::default(),
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

/// How every card answers the pointer, and the lamp that lights it.
#[derive(Resource, JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardHandling {
    /// How far a card is turned when the pointer is at its edge.
    pub tilt: Degrees,
    /// How long a card takes to follow the pointer, and to lie flat again once it has left.
    pub settles_in: Seconds,
    /// How far in front of the window the eye is, in window heights. The nearer, the more a
    /// tilted card's far edge shrinks.
    pub eye: f32,
    pub lamp: CardLamp,
}

/// The light every card is lit by. It hangs still, or moves no faster than the day does: a
/// highlight moves only as a card does.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardLamp {
    /// Where over the window it hangs, as shares of the window's width and height from its top
    /// left corner.
    pub across: f32,
    pub down: f32,
    /// How far in front of the window it hangs, in window heights.
    pub height: f32,
    pub color: Rgb,
    /// How much of a card's light is the lamp's. The rest falls evenly, so at nought a matte
    /// card takes no shading at all.
    pub strength: f32,
    /// Whether the lamp is the light of the sky wherever one stands over the player: the sun, or
    /// by night the moon. The window then faces that sky where its light stands at noon: the
    /// light rises to the window's left, stands highest over its middle at noon and sets to its
    /// right, in front of the window all the while and as far from its middle as the lamp hangs
    /// here, so the nearer the horizon it stands the smaller the pool of light under it. It has
    /// that light's colour, and as much of the strength written here as that light has of a
    /// full sun's to an eye grown used to it. No cloud dims it. Under no sky the lamp hangs as
    /// it is written here.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub follows_sky: bool,
}

impl CardHandling {
    pub fn check(&self, at: &str, check: &mut RowCheck) {
        check.range(format!("{at}/tilt"), self.tilt.0, 0.0, MOST_TILT, "degrees");
        let shift = self.corner_shift();
        if shift > sheet::CROSS / 2.0 {
            check.error(
                format!("{at}/tilt"),
                "tilt too steep",
                format!(
                    "{} degrees, drawing a card's corner {shift:.0} points from where it is clicked",
                    self.tilt.0
                ),
                "a tilt, with the eye's distance, that keeps a card's close button under the pointer",
            );
        }
        check.at_least(
            format!("{at}/settles_in"),
            self.settles_in.0,
            0.0,
            "seconds",
        );
        check.above_up_to(
            format!("{at}/eye"),
            self.eye,
            0.0,
            FARTHEST_EYE,
            "window heights",
        );
        let lamp = &self.lamp;
        check.range(
            format!("{at}/lamp/across"),
            lamp.across,
            0.0,
            1.0,
            "a share",
        );
        check.range(format!("{at}/lamp/down"), lamp.down, 0.0, 1.0, "a share");
        check.above_up_to(
            format!("{at}/lamp/height"),
            lamp.height,
            0.0,
            FARTHEST_EYE,
            "window heights",
        );
        check.range(
            format!("{at}/lamp/strength"),
            lamp.strength,
            0.0,
            1.0,
            "a share",
        );
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
const MOST_ART_MARGIN: f32 = 24.0;
const MOST_GLOW: f32 = 48.0;
impl CardHandling {
    /// How far from where it lies flat a card's corner is drawn with the pointer on it, where
    /// the card is turned the most both ways, in the smallest window the tallest card fits in:
    /// there the eye stands nearest, and the far corner shrinks the most.
    fn corner_shift(&self) -> f32 {
        let (across, down) = (card::WIDTH / 2.0, card::HEIGHT / 2.0);
        let turn = self.tilt.0.to_radians();
        let eye = self.eye * card::HEIGHT;
        let sunk = (across + down) * turn.sin();
        across.hypot(down) * (1.0 - turn.cos() * eye / (eye + sunk))
    }
}

const MOST_TILT: f32 = 12.0;
const FARTHEST_EYE: f32 = 10.0;

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
        if let CardArt::Inset { margin } = self.art {
            check.above_up_to("/art/margin", margin.0, 0.0, MOST_ART_MARGIN, "points");
        }
        match &self.material {
            CardMaterial::Matte => {}
            CardMaterial::Satin { gloss }
            | CardMaterial::Lacquer { gloss }
            | CardMaterial::Embossed { gloss, .. } => share(check, "/material/gloss", *gloss),
            CardMaterial::Metal { polish } => share(check, "/material/polish", *polish),
            CardMaterial::Foil { strength, .. } => share(check, "/material/strength", *strength),
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
