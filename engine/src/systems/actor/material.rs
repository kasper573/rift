//! What a part is painted with: a base texture, and masks marking where each of its skeleton's
//! tints paints over it.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::skeleton::{Colours, SkeletonDef, SkeletonId, TintName};
use crate::core::assets::ImageRef;
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ErrorPlace, FileWriter, Loading,
    Nearby, PartSpec, Parts, RowCheck, Shape,
};
use crate::core::render::world::paint::{Paint, PaintLayer};

pub type MaterialId = ContentId<MaterialDef>;

/// A base texture, `base.png`, and masks over it, `masks/<mask>.png`, the size of the base: each
/// channel of a mask marks in white where one of `skeleton`'s tints paints over the base, and the
/// tints paint in the order the skeleton lists them.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialDef {
    pub skeleton: SkeletonId,
    /// The tint each channel of a mask paints, by the mask's name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub masks: BTreeMap<String, MaskChannels>,
    #[serde(skip)]
    pub base: ImageRef,
    #[serde(skip)]
    pub mask_files: BTreeMap<String, ImageRef>,
}

/// The tint each channel of a mask paints, where it paints one.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskChannels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<TintName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<TintName>,
}

pub const MATERIAL_BASE: PartSpec = PartSpec::File {
    name: "base",
    exts: &["png"],
};

pub const MATERIAL_MASKS: PartSpec = PartSpec::Keyed {
    folder: "masks",
    exts: &["png"],
};

const CHANNELS: [&str; 4] = ["r", "g", "b", "a"];

impl MaskChannels {
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
                    .flat_map(move |(mask, channels)| {
                        channels
                            .painting()
                            .filter(move |(_, painted)| **painted == tint.name)
                            .filter_map(move |(channel, _)| {
                                Some((self.mask_files.get(mask)?.path(), channel))
                            })
                    })
                    .map(|(mask, channel)| PaintLayer {
                        mask,
                        channel,
                        colour: colours.0.get(&tint.name).copied().unwrap_or(tint.colour),
                    })
            })
            .collect();
        Paint {
            base: self.base.path(),
            layers,
        }
    }
}

impl Bundle for MaterialDef {
    const PARTS: &'static [PartSpec] = &[MATERIAL_BASE, MATERIAL_MASKS];

    fn read_parts(&mut self, parts: &mut Parts) {
        let Some(base) = parts.required(MATERIAL_BASE) else {
            return;
        };
        self.base = ImageRef::new(parts.path(base));
        let size = parts.image_size(base);
        let mut check = RowCheck::default();
        let keyed = parts.keyed(MATERIAL_MASKS);
        for (mask, file) in &keyed {
            if !self.masks.contains_key(*mask) {
                parts.error(
                    file,
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        "unused mask",
                        format!("no channels named for the mask {mask}"),
                        format!("a {mask} entry in masks naming what its channels paint"),
                    ),
                );
                continue;
            }
            let mask_size = parts.image_size(file);
            if mask_size != size {
                let shown = |size: Option<(u32, u32)>| {
                    size.map_or("no size".to_owned(), |(w, h)| format!("{w} × {h} px"))
                };
                parts.error(
                    file,
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        "wrong size",
                        shown(mask_size),
                        format!("{}, the size of the base", shown(size)),
                    ),
                );
                continue;
            }
            self.mask_files
                .insert((*mask).to_owned(), ImageRef::new(parts.path(file)));
        }
        for mask in self.masks.keys() {
            if !keyed.iter().any(|(key, _)| key == mask) {
                check.error(
                    format!("/masks/{mask}"),
                    "missing file",
                    format!("no {}", MATERIAL_MASKS.named(mask, "png")),
                    format!("the mask's image in {}/", parts.bundle()),
                );
            }
        }
        parts.config_errors(check);
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentFile for MaterialDef {
    const TABLE: &'static str = "material";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<MaterialDef, String> {
        Ok(MaterialDef {
            skeleton: near.file()?,
            masks: BTreeMap::new(),
            base: ImageRef::default(),
            mask_files: BTreeMap::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        let mut painted = BTreeSet::new();
        for (mask, channels) in &self.masks {
            cx.text(format!("/masks/{mask}"), mask, 1, 32);
            for (channel, tint) in channels.painting() {
                if !painted.insert(tint) {
                    cx.error(
                        format!("/masks/{mask}/{}", CHANNELS[channel]),
                        "tint twice",
                        tint.as_str(),
                        "every tint painted by one channel",
                    );
                }
            }
        }
    }
}

/// Every tint a material's masks paint is one of its skeleton's.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, material) in loading.rows::<MaterialDef>(&content) {
        let skeleton = material.skeleton.get(&content);
        let mut check = RowCheck::default();
        for (mask, channels) in &material.masks {
            for (channel, tint) in channels.painting() {
                check_tint(
                    &content,
                    skeleton,
                    material.skeleton,
                    tint,
                    &format!("/masks/{mask}/{}", CHANNELS[channel]),
                    &mut check,
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
