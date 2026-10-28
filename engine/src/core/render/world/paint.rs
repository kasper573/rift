//! Textures painted for a body: a base texture with colours laid over it wherever masks mark them,
//! baked once for each way of painting it and shared by every mesh painted alike.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::material::WorldMaterial;
use crate::core::math::Rgb;

/// A texture painted from `base` and `layers`, laid in order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Paint {
    pub base: String,
    pub layers: Vec<PaintLayer>,
}

/// `colour`, laid as thick as one channel of `mask` is bright: fully where it is white, not at all
/// where it is black; the other way round when `inverted`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaintLayer {
    pub mask: String,
    pub channel: usize,
    pub inverted: bool,
    pub colour: Rgb,
}

/// Every paint asked for, baked once its images load, and the materials drawn with each.
#[derive(Resource, Default)]
pub struct Paints {
    baked: HashMap<Paint, Baking>,
    materials: HashMap<(AssetId<WorldMaterial>, AssetId<Image>), Handle<WorldMaterial>>,
}

enum Baking {
    Loading {
        base: Handle<Image>,
        masks: Vec<Handle<Image>>,
    },
    Done(Handle<Image>),
    Failed,
}

/// What paints a mesh: the paints baked so far, and the images and materials they are made of.
#[derive(SystemParam)]
pub struct Painter<'w> {
    paints: ResMut<'w, Paints>,
    assets: Res<'w, AssetServer>,
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<WorldMaterial>>,
}

impl Painter<'_> {
    /// The texture `paint` makes, once its base and masks have loaded and it is baked.
    pub fn texture(&mut self, paint: &Paint) -> Option<Handle<Image>> {
        let Painter {
            paints,
            assets,
            images,
            ..
        } = self;
        let baking = paints
            .baked
            .entry(paint.clone())
            .or_insert_with(|| Baking::Loading {
                base: assets.load(paint.base.clone()),
                masks: paint
                    .layers
                    .iter()
                    .map(|layer| assets.load(layer.mask.clone()))
                    .collect(),
            });
        if let Baking::Loading { base, masks } = baking {
            let loaded = |handle: &Handle<Image>| images.get(handle).is_some();
            let failed = |handle: &Handle<Image>| assets.load_state(handle).is_failed();
            if failed(base) || masks.iter().any(failed) {
                warn!("could not paint {}: an image did not load", paint.base);
                *baking = Baking::Failed;
            } else if loaded(base) && masks.iter().all(loaded) {
                let layers: Option<Vec<(&Image, &PaintLayer)>> = masks
                    .iter()
                    .zip(&paint.layers)
                    .map(|(mask, layer)| Some((images.get(mask)?, layer)))
                    .collect();
                let baked = images
                    .get(base.id())
                    .zip(layers)
                    .and_then(|(base, layers)| bake(base, &layers));
                *baking = match baked {
                    Some(image) => Baking::Done(images.add(image)),
                    None => {
                        warn!("could not paint {}: its masks do not fit it", paint.base);
                        Baking::Failed
                    }
                };
            }
        }
        match baking {
            Baking::Done(texture) => Some(texture.clone()),
            Baking::Loading { .. } | Baking::Failed => None,
        }
    }

    /// Whether `paint` will never bake, so what waits for it draws without it.
    pub fn failed(&self, paint: &Paint) -> bool {
        matches!(self.paints.baked.get(paint), Some(Baking::Failed))
    }

    /// `material` drawing `texture` in place of its own colour, shared by every mesh painted alike.
    pub fn painted(
        &mut self,
        material: &Handle<WorldMaterial>,
        texture: &Handle<Image>,
    ) -> Option<Handle<WorldMaterial>> {
        let key = (material.id(), texture.id());
        if let Some(painted) = self.paints.materials.get(&key) {
            return Some(painted.clone());
        }
        let mut painted = self.materials.get(material)?.clone();
        painted.base.base_color = Color::WHITE;
        painted.base.base_color_texture = Some(texture.clone());
        let painted = self.materials.add(painted);
        self.paints.materials.insert(key, painted.clone());
        Some(painted)
    }
}

/// `base` with each layer laid over it in order, as thick as its mask's channel is bright, mixed in
/// linear light as a shader would: none when a mask is not the size of the base.
pub fn bake(base: &Image, layers: &[(&Image, &PaintLayer)]) -> Option<Image> {
    let pixels = |image: &Image| Some(image.clone().try_into_dynamic().ok()?.to_rgba8());
    let base = pixels(base)?;
    let (width, height) = base.dimensions();
    let mut decoded: Vec<(&Image, _)> = Vec::new();
    let mut masks = Vec::with_capacity(layers.len());
    for (mask, _) in layers {
        let index = match decoded
            .iter()
            .position(|(seen, _)| std::ptr::eq(*seen, *mask))
        {
            Some(index) => index,
            None => {
                let pixels = pixels(mask).filter(|mask| mask.dimensions() == (width, height))?;
                decoded.push((*mask, pixels));
                decoded.len() - 1
            }
        };
        masks.push(index);
    }
    let linear: Vec<f32> = (0..=255u8)
        .map(|byte| to_linear(byte as f32 / 255.0))
        .collect();
    let colours: Vec<[f32; 3]> = layers
        .iter()
        .map(|(_, layer)| layer.colour.0.map(|byte| linear[byte as usize]))
        .collect();
    let mut data = base.into_raw();
    for (index, pixel) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let mut mixed = [pixel[0], pixel[1], pixel[2]].map(|byte| linear[byte as usize]);
        for (((_, layer), &mask), colour) in layers.iter().zip(&masks).zip(&colours) {
            let bright = decoded[mask].1.as_raw()[index * 4 + layer.channel.min(3)] as f32 / 255.0;
            let amount = if layer.inverted { 1.0 - bright } else { bright };
            for (mixed, colour) in mixed.iter_mut().zip(colour) {
                *mixed += (colour - *mixed) * amount;
            }
        }
        for (byte, mixed) in pixel.iter_mut().zip(mixed) {
            *byte = (to_srgb(mixed) * 255.0).round() as u8;
        }
    }
    Some(Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

fn to_linear(srgb: f32) -> f32 {
    if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}
