//! Textures painted for a body: a base texture with colours laid over it wherever masks mark them,
//! and where a layer is metal, the surface it lays, baked once for each way of painting it, a
//! slice of a frame at a time, and shared by every mesh painted alike.

use std::sync::LazyLock;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::material::{METAL_ROUGHNESS, METALLIC, ROUGHNESS, WorldMaterial};
use crate::core::math::Rgb;
use crate::core::pace::Pace;

/// A texture painted from `base` and `layers`, laid in order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Paint {
    pub base: String,
    pub layers: Vec<PaintLayer>,
}

/// `colour`, laid as thick as one channel of `mask` is bright: fully where it is white, not at all
/// where it is black; the other way round when `inverted`. A `metal` layer lays a metal's surface
/// with its colour.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaintLayer {
    pub mask: String,
    pub channel: usize,
    pub inverted: bool,
    pub colour: Rgb,
    pub metal: bool,
}

/// A paint as baked: its colours, and the surface of its metal where any layer is metal.
#[derive(Clone, Debug)]
pub struct Painted {
    pub colour: Handle<Image>,
    pub surface: Option<Handle<Image>>,
}

/// Every paint asked for, baked once its images load, and the materials drawn with each.
#[derive(Resource, Default)]
pub struct Paints {
    baked: HashMap<Paint, PaintState>,
    materials: HashMap<(AssetId<WorldMaterial>, AssetId<Image>), Handle<WorldMaterial>>,
}

enum PaintState {
    Loading {
        base: Handle<Image>,
        masks: Vec<Handle<Image>>,
    },
    Baking(Bake),
    Done(Painted),
    Failed,
}

/// Bakes every paint whose images have loaded, as far as the frame's slice allows.
pub(super) fn advance_bakes(
    mut paints: ResMut<Paints>,
    mut images: ResMut<Assets<Image>>,
    pace: Res<Pace>,
) {
    for baking in paints.baked.values_mut() {
        if let PaintState::Baking(bake) = baking {
            loop {
                if pace.spent() {
                    return;
                }
                if let Some(baked) = bake.step() {
                    *baking = PaintState::Done(Painted {
                        colour: images.add(baked.colour),
                        surface: baked.surface.map(|surface| images.add(surface)),
                    });
                    break;
                }
            }
        }
    }
}

/// Forgets the painted materials made from world materials that are gone.
pub(super) fn forget_dropped(
    mut events: MessageReader<AssetEvent<WorldMaterial>>,
    mut paints: ResMut<Paints>,
) {
    let gone = super::dropped(&mut events);
    if !gone.is_empty() {
        paints
            .materials
            .retain(|(material, _), _| !gone.contains(material));
    }
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
    /// The textures `paint` makes, once its base and masks have loaded and it is baked.
    pub fn texture(&mut self, paint: &Paint) -> Option<Painted> {
        let Painter {
            paints,
            assets,
            images,
            ..
        } = self;
        let baking = paints
            .baked
            .entry(paint.clone())
            .or_insert_with(|| PaintState::Loading {
                base: assets.load(paint.base.clone()),
                masks: paint
                    .layers
                    .iter()
                    .map(|layer| assets.load(layer.mask.clone()))
                    .collect(),
            });
        if let PaintState::Loading { base, masks } = baking {
            let loaded = |handle: &Handle<Image>| images.get(handle).is_some();
            let failed = |handle: &Handle<Image>| assets.load_state(handle).is_failed();
            if failed(base) || masks.iter().any(failed) {
                warn!("could not paint {}: an image did not load", paint.base);
                *baking = PaintState::Failed;
            } else if loaded(base) && masks.iter().all(loaded) {
                let layers: Option<Vec<(&Image, &PaintLayer)>> = masks
                    .iter()
                    .zip(&paint.layers)
                    .map(|(mask, layer)| Some((images.get(mask)?, layer)))
                    .collect();
                let started = images
                    .get(base.id())
                    .zip(layers)
                    .and_then(|(base, layers)| Bake::start(base, &layers));
                *baking = match started {
                    Some(bake) => PaintState::Baking(bake),
                    None => {
                        warn!("could not paint {}: its masks do not fit it", paint.base);
                        PaintState::Failed
                    }
                };
            }
        }
        match baking {
            PaintState::Done(texture) => Some(texture.clone()),
            PaintState::Loading { .. } | PaintState::Baking(_) | PaintState::Failed => None,
        }
    }

    /// Whether `paint` will never bake, so what waits for it draws without it.
    pub fn failed(&self, paint: &Paint) -> bool {
        matches!(self.paints.baked.get(paint), Some(PaintState::Failed))
    }

    /// `material` drawing `texture`'s colours in place of its own, and its surface where it has
    /// one, shared by every mesh painted alike.
    pub fn painted(
        &mut self,
        material: &Handle<WorldMaterial>,
        texture: &Painted,
    ) -> Option<Handle<WorldMaterial>> {
        let key = (material.id(), texture.colour.id());
        if let Some(painted) = self.paints.materials.get(&key) {
            return Some(painted.clone());
        }
        let mut painted = self.materials.get(material)?.clone();
        painted.base.base_color = Color::WHITE;
        painted.base.base_color_texture = Some(texture.colour.clone());
        if let Some(surface) = &texture.surface {
            painted.base.metallic = 1.0;
            painted.base.perceptual_roughness = 1.0;
            painted.base.metallic_roughness_texture = Some(surface.clone());
        }
        let painted = self.materials.add(painted);
        self.paints.materials.insert(key, painted.clone());
        Some(painted)
    }
}

/// A paint as baked, before its textures are added: `base` with each layer laid over it in order,
/// as thick as its mask's channel is bright, mixed in linear light as a shader would; and where a
/// layer is metal, the surface as glTF reads it, roughness in green and metalness in blue: the
/// world's own surface, turned to metal as thick as each metal layer lies. No surface when no
/// layer is metal, and the base keeps the world's surface.
pub struct Baked {
    pub colour: Image,
    pub surface: Option<Image>,
}

/// Bakes a paint whole.
pub fn bake(base: &Image, layers: &[(&Image, &PaintLayer)]) -> Option<Baked> {
    let mut bake = Bake::start(base, layers)?;
    loop {
        if let Some(baked) = bake.step() {
            return Some(baked);
        }
    }
}

/// A paint baking a band of rows at a time, so a bake can take a slice of every frame.
pub struct Bake {
    width: u32,
    height: u32,
    row: u32,
    colour: Vec<u8>,
    surface: Option<Vec<u8>>,
    masks: Vec<Vec<u8>>,
    layers: Vec<Laid>,
}

/// A layer as a bake reads it: its mask among the decoded ones, and its colour in linear light.
struct Laid {
    mask: usize,
    channel: usize,
    inverted: bool,
    colour: [f32; 3],
    metal: bool,
}

/// How many pixels a step bakes.
const STEP_PIXELS: u32 = 16 * 1024;

impl Bake {
    /// Begins baking `layers` over `base`: none when a mask is not the size of the base.
    pub fn start(base: &Image, layers: &[(&Image, &PaintLayer)]) -> Option<Bake> {
        let pixels = |image: &Image| Some(image.clone().try_into_dynamic().ok()?.to_rgba8());
        let base = pixels(base)?;
        let (width, height) = base.dimensions();
        let mut seen: Vec<&Image> = Vec::new();
        let mut masks = Vec::new();
        let mut laid = Vec::with_capacity(layers.len());
        for (mask, layer) in layers {
            let index = match seen
                .iter()
                .position(|earlier| std::ptr::eq(*earlier, *mask))
            {
                Some(index) => index,
                None => {
                    let pixels =
                        pixels(mask).filter(|mask| mask.dimensions() == (width, height))?;
                    seen.push(mask);
                    masks.push(pixels.into_raw());
                    masks.len() - 1
                }
            };
            laid.push(Laid {
                mask: index,
                channel: layer.channel.min(3),
                inverted: layer.inverted,
                colour: layer.colour.0.map(|byte| LINEAR[byte as usize]),
                metal: layer.metal,
            });
        }
        let surface = laid
            .iter()
            .any(|layer| layer.metal)
            .then(|| vec![0u8; (width * height * 4) as usize]);
        Some(Bake {
            width,
            height,
            row: 0,
            colour: base.into_raw(),
            surface,
            masks,
            layers: laid,
        })
    }

    /// Bakes the next band of rows; the textures once the last is baked, after which the bake is
    /// spent.
    pub fn step(&mut self) -> Option<Baked> {
        let rows = (STEP_PIXELS / self.width.max(1)).max(1);
        let from = self.row;
        let to = (from + rows).min(self.height);
        self.row = to;
        let range = (from * self.width * 4) as usize..(to * self.width * 4) as usize;
        let first = (from * self.width) as usize;
        let srgb = &*SRGB;
        let surface = self
            .surface
            .as_mut()
            .map(|surface| &mut surface[range.clone()]);
        let mut surface = surface
            .map(|surface| surface.as_chunks_mut::<4>().0.iter_mut())
            .into_iter()
            .flatten();
        for (offset, pixel) in self.colour[range]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            let index = first + offset;
            let mut mixed = [pixel[0], pixel[1], pixel[2]].map(|byte| LINEAR[byte as usize]);
            let mut metal = 0.0f32;
            for layer in &self.layers {
                let bright = self.masks[layer.mask][index * 4 + layer.channel] as f32 / 255.0;
                let amount = if layer.inverted { 1.0 - bright } else { bright };
                for (mixed, colour) in mixed.iter_mut().zip(layer.colour) {
                    *mixed += (colour - *mixed) * amount;
                }
                let target = if layer.metal { 1.0 } else { 0.0 };
                metal += (target - metal) * amount;
            }
            for (byte, mixed) in pixel.iter_mut().zip(mixed) {
                *byte = srgb[(mixed.clamp(0.0, 1.0) * SRGB_STEPS as f32).round() as usize];
            }
            if let Some(pixel) = surface.next() {
                let roughness = ROUGHNESS + (METAL_ROUGHNESS - ROUGHNESS) * metal;
                *pixel = [
                    0,
                    (roughness * 255.0).round() as u8,
                    (METALLIC * metal * 255.0).round() as u8,
                    255,
                ];
            }
        }
        if to < self.height {
            return None;
        }
        let image = |data: Vec<u8>, format: TextureFormat| {
            Image::new(
                Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                data,
                format,
                RenderAssetUsages::RENDER_WORLD,
            )
        };
        Some(Baked {
            colour: image(
                std::mem::take(&mut self.colour),
                TextureFormat::Rgba8UnormSrgb,
            ),
            surface: self
                .surface
                .take()
                .map(|surface| image(surface, TextureFormat::Rgba8Unorm)),
        })
    }
}

/// Linear light for every sRGB byte.
static LINEAR: LazyLock<[f32; 256]> =
    LazyLock::new(|| std::array::from_fn(|byte| to_linear(byte as f32 / 255.0)));

// A bake converts every pixel of a texture back to sRGB, and `powf` runs in software on the web,
// where that took longer than all else an arrival does.
const SRGB_STEPS: usize = u16::MAX as usize;

/// The sRGB byte for linear light in steps of `1 / SRGB_STEPS`, fine enough that every byte comes
/// back from light as it went in.
static SRGB: LazyLock<Vec<u8>> = LazyLock::new(|| {
    (0..=SRGB_STEPS)
        .map(|step| (to_srgb(step as f32 / SRGB_STEPS as f32) * 255.0).round() as u8)
        .collect()
});

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
