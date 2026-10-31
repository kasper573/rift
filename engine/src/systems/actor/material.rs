//! What a part is painted with: a base texture, and masks marking where each of its skeleton's
//! tints paints over it.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::skeleton::{Colours, SkeletonDef, SkeletonId, TintName};
use crate::core::assets::ImageRef;
use crate::core::content::media::ImageSizes;
use crate::core::content::{Content, ContentFile, ContentId, Loading, Nearby, RowCheck, serial};
use crate::core::render::world::paint::{Paint, PaintLayer};

pub type MaterialId = ContentId<MaterialDef>;

/// A base texture, and masks over it the size of the base: each channel of a mask marks where one
/// of `skeleton`'s tints paints over the base, and the tints paint in the order the skeleton lists
/// them.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialDef {
    pub skeleton: SkeletonId,
    pub base: ImageRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<MaterialMask>,
}

/// An image marking, in each of its channels, where one tint paints: in white, or in black when
/// it is `inverted`.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialMask {
    pub image: ImageRef,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub inverted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<TintName>,
}

const CHANNELS: [&str; 4] = ["r", "g", "b", "a"];

impl MaterialMask {
    /// Each channel that paints a tint, by its place in a pixel, red first.
    pub fn painting(&self) -> impl Iterator<Item = (usize, &TintName)> {
        [&self.r, &self.g, &self.b, &self.a]
            .into_iter()
            .enumerate()
            .filter_map(|(channel, tint)| Some((channel, tint.as_ref()?)))
    }
}

impl MaterialDef {
    /// The material painted for a body of `skeleton` in `colours`: each tint the skeleton lists,
    /// in its order, laid in its colour wherever a mask's channel marks it.
    pub fn paint(&self, skeleton: &SkeletonDef, colours: &Colours) -> Paint {
        let colours = colours.over(&skeleton.tint_colours());
        let layers = skeleton
            .tints
            .iter()
            .flat_map(|tint| {
                self.masks
                    .iter()
                    .flat_map(move |mask| {
                        mask.painting()
                            .filter(move |(_, painted)| **painted == tint.name)
                            .map(move |(channel, _)| (mask, channel))
                    })
                    .map(|(mask, channel)| PaintLayer {
                        mask: mask.image.path(),
                        channel,
                        inverted: mask.inverted,
                        colour: colours.0.get(&tint.name).copied().unwrap_or(tint.colour),
                        metal: tint.metal,
                    })
            })
            .collect();
        Paint {
            base: self.base.path(),
            layers,
        }
    }
}

impl ContentFile for MaterialDef {
    const TABLE: &'static str = "material";

    fn fresh(near: &Nearby) -> Result<MaterialDef, String> {
        Ok(MaterialDef {
            skeleton: near.file()?,
            base: near.image()?,
            masks: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        let mut painted = BTreeSet::new();
        for (index, mask) in self.masks.iter().enumerate() {
            for (channel, tint) in mask.painting() {
                if !painted.insert(tint) {
                    cx.error(
                        format!("/masks/{index}/{}", CHANNELS[channel]),
                        "tint twice",
                        tint.as_str(),
                        "every tint painted by one channel",
                    );
                }
            }
        }
    }
}

/// Every tint a material's masks paint is one of its skeleton's, and every mask is the size of
/// its base.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    for (id, material) in loading.rows::<MaterialDef>(&content) {
        let skeleton = material.skeleton.get(&content);
        let mut check = RowCheck::default();
        let base = sizes.0.get(material.base.as_str()).copied();
        for (index, mask) in material.masks.iter().enumerate() {
            for (channel, tint) in mask.painting() {
                check_tint(
                    &content,
                    skeleton,
                    material.skeleton,
                    tint,
                    &format!("/masks/{index}/{}", CHANNELS[channel]),
                    &mut check,
                );
            }
            let size = sizes.0.get(mask.image.as_str()).copied();
            if let (Some(base), Some(size)) = (base, size)
                && size != base
            {
                check.error(
                    format!("/masks/{index}/image"),
                    "wrong size",
                    format!("{} × {} px", size.0, size.1),
                    format!("{} × {} px, the size of the base", base.0, base.1),
                );
            }
        }
        loading.row_errors(id, check);
    }
}

/// Every tint `colours` names is one of `skeleton`'s.
pub fn check_colours(
    content: &Content,
    skeleton: SkeletonId,
    colours: &Colours,
    pointer: &str,
    cx: &mut RowCheck,
) {
    let def = skeleton.get(content);
    for tint in colours.0.keys() {
        check_tint(
            content,
            def,
            skeleton,
            tint,
            &format!("{pointer}/{tint}"),
            cx,
        );
    }
}

fn check_tint(
    content: &Content,
    def: &SkeletonDef,
    skeleton: SkeletonId,
    tint: &TintName,
    pointer: &str,
    cx: &mut RowCheck,
) {
    if !def.tints.iter().any(|known| known.name == *tint) {
        let names: Vec<TintName> = def.tints.iter().map(|known| known.name.clone()).collect();
        cx.error(
            pointer,
            "unknown tint",
            tint.as_str(),
            super::model::expected_name("tints", &names, skeleton.path(content)),
        );
    }
}
