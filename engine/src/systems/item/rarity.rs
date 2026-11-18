use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::SfxId;
use crate::core::card::CardStyleId;
use crate::core::content::media::ImageSizes;
use crate::core::content::serial;
use crate::core::content::{
    ContentFile, ContentId, ImageRef, Loading, NamedContent, Nearby, RowCheck,
};
use crate::core::math::{ColorGradient, Rgb, Rgba};
use crate::core::units::Seconds;
use crate::core::vfx::{ParamKind, ParamValue, SiteKind, VfxId, VfxParams, check_played};

pub type ItemRarityId = ContentId<ItemRarityDef>;

/// The pixels of a rarity's icon: what a slot's pip and a name draw it in.
pub const RARITY_ICON_SIZE: (u32, u32) = (16, 16);

/// How rare an item is, and all that shows it: where it stands among the game's rarities, and the
/// icon and colours its items are marked with wherever they show.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRarityDef {
    pub name: String,
    /// Where it stands among the game's rarities, the rarest highest. No two share a rank.
    pub rank: RarityRank,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ImageRef>,
    pub gradient: ColorGradient,
    /// What the cards of its items are drawn in. Left out, the game's default card style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_style: Option<CardStyleId>,
    pub dropped: DroppedRarity,
}

/// What an item of a rarity does as it lies dropped.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DroppedRarity {
    pub named: DropNaming,
    /// Played over and over on the item for as long as it lies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vfx: Option<VfxId>,
    /// Played once on the item as it lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing_vfx: Option<VfxId>,
    /// Heard once as it lands, over the item's own drop sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<SfxId>,
    /// Whether the item is marked on the minimap, with the rarity's icon.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub on_minimap: bool,
}

/// The colour parameters a vfx a rarity plays may declare, which the rarity fills with the two
/// colours of its gradient.
pub const RARITY_FROM: &str = "rarity_from";
pub const RARITY_TO: &str = "rarity_to";

impl ItemRarityDef {
    /// What the rarity plays its vfx with: its two colours, for a vfx that asks for them.
    pub fn vfx_params(&self) -> VfxParams {
        let color = |rgb: Rgb| {
            let [red, green, blue] = rgb.0;
            ParamValue::Color(Rgba::from([red, green, blue, 255]))
        };
        VfxParams(vec![
            (RARITY_FROM.to_owned(), color(self.gradient.from)),
            (RARITY_TO.to_owned(), color(self.gradient.to)),
        ])
    }
}

/// When a dropped item is named without being asked.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DropNaming {
    /// Only under the pointer, or when the player asks for names.
    OnKey,
    /// For so long from the moment it lands.
    For { seconds: Seconds },
    /// For as long as it lies within reach of the player.
    Always,
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
    Hash,
)]
pub struct RarityRank(pub u32);

impl ContentFile for ItemRarityDef {
    const TABLE: &'static str = "item_rarity";

    fn fresh(near: &Nearby) -> Result<ItemRarityDef, String> {
        let above_all = near
            .all::<ItemRarityDef>()
            .iter()
            .map(|rarity| rarity.rank.0)
            .max()
            .map_or(1, |highest| highest.saturating_add(1));
        Ok(ItemRarityDef {
            name: near.name(),
            rank: RarityRank(above_all),
            icon: None,
            gradient: ColorGradient {
                from: Rgb::WHITE,
                to: Rgb::WHITE,
            },
            card_style: None,
            dropped: DroppedRarity {
                named: DropNaming::OnKey,
                vfx: None,
                landing_vfx: None,
                sfx: None,
                on_minimap: false,
            },
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        if let DropNaming::For { seconds } = self.dropped.named {
            check.at_least("/dropped/named/seconds", seconds.0, 0.0, "seconds");
        }
        if self.dropped.on_minimap && self.icon.is_none() {
            check.error(
                "/dropped/on_minimap",
                "mark without an icon",
                "no icon",
                "an icon, which is its mark on the minimap",
            );
        }
    }
}

impl NamedContent for ItemRarityDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

/// No two rarities share a rank, every rarity's icon is the size it is drawn, and the vfx it
/// plays take its colours as colours.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    let mut ranked: BTreeMap<RarityRank, ItemRarityId> = BTreeMap::new();
    for (id, rarity) in loading.rows::<ItemRarityDef>(&content) {
        let mut check = RowCheck::default();
        if let Some(first) = ranked.get(&rarity.rank) {
            check.error(
                "/rank",
                "rank twice",
                format!("{} is {} too", first.path(&content), rarity.rank.0),
                "each rank once",
            );
        } else {
            ranked.insert(rarity.rank, id);
        }
        if let Some(icon) = &rarity.icon
            && let Some(&size) = sizes.0.get(icon.as_str())
            && size != RARITY_ICON_SIZE
        {
            check.error(
                "/icon",
                "wrong size",
                format!("{} × {} pixels", size.0, size.1),
                format!("{} × {} pixels", RARITY_ICON_SIZE.0, RARITY_ICON_SIZE.1),
            );
        }
        let played = [
            ("vfx", rarity.dropped.vfx),
            ("landing_vfx", rarity.dropped.landing_vfx),
        ];
        for (field, vfx) in played {
            let Some(vfx) = vfx else {
                continue;
            };
            check_played(loading, vfx, &[SiteKind::Object], id.path(&content));
            for name in [RARITY_FROM, RARITY_TO] {
                if let Some(kind) = vfx.get(&content).declares(name)
                    && kind != ParamKind::Color
                {
                    check.error(
                        format!("/dropped/{field}"),
                        "wrong type",
                        format!("{} declares {name} as no color", vfx.path(&content)),
                        format!("a vfx whose {name} is a color, which the rarity fills"),
                    );
                }
            }
        }
        loading.row_errors(id, check);
    }
}
