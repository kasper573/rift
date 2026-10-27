pub mod areas;
pub mod props;

use std::collections::{BTreeSet, HashMap};
use ui::egui;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::gpu_readback::ReadbackComplete;

use engine::core::assets::ImageRef;
use engine::core::gui::{GuiImage, ImageSource};
use engine::core::render::world::paint::{Paint, bake};
use image::RgbaImage;
use image::imageops::FilterType;
use ui::egui::{ColorImage, TextureHandle, TextureOptions};

use crate::project::ProjectFiles;

#[derive(Default)]
pub struct Thumbnails {
    entries: HashMap<String, Entry>,
    /// Each picture's size in pixels as its header says, by path, with the hash it was read at.
    dimensions: HashMap<String, (String, Option<[u32; 2]>)>,
}

enum Entry {
    Decoded(Thumbnail),
    Undecodable { hash: String },
}

#[derive(Clone)]
pub struct Thumbnail {
    pub texture: TextureHandle,
    /// The picture's own size in pixels, which a texture too big for the GPU is drawn down from.
    pub size: [usize; 2],
    pub hash: String,
}

/// A project's pictures, for drawing the engine's own gui in the editor.
pub struct ProjectImages<'a> {
    pub ctx: &'a egui::Context,
    pub files: &'a ProjectFiles,
    pub thumbnails: &'a mut Thumbnails,
}

impl ImageSource for ProjectImages<'_> {
    fn image(&mut self, asset: &ImageRef) -> Option<GuiImage> {
        let thumb = self
            .thumbnails
            .image(self.ctx, self.files, asset.as_str())?;
        Some(GuiImage {
            texture: thumb.texture.id(),
            size: thumb.texture.size_vec2(),
            path: asset.as_str().to_owned(),
        })
    }
}

pub const ICON: u32 = 32;
pub const FULL_UV: egui::Rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));

impl Thumbnails {
    pub fn image(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        path: &str,
    ) -> Option<Thumbnail> {
        let mut unlimited = u32::MAX;
        self.image_within(ctx, files, path, &mut unlimited)
    }

    /// The size in pixels of the picture at `path`, read from its header without decoding it.
    pub fn dimensions(&mut self, files: &ProjectFiles, path: &str) -> Option<[u32; 2]> {
        let hash = files.hash_of(path)?;
        if let Some((known, dimensions)) = self.dimensions.get(path)
            && known == hash
        {
            return *dimensions;
        }
        let dimensions = image::ImageReader::new(std::io::Cursor::new(&files.get(path)?[..]))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok())
            .map(|(width, height)| [width, height]);
        self.dimensions
            .insert(path.to_owned(), (hash.to_owned(), dimensions));
        dimensions
    }

    pub fn image_within(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        path: &str,
        budget: &mut u32,
    ) -> Option<Thumbnail> {
        let file = path.to_owned();
        self.load(ctx, files, file.clone(), file, budget, Some)
    }

    /// The texture `paint` makes, baked from the project's pictures as the game bakes it.
    pub fn painted(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        paint: &Paint,
    ) -> Option<Thumbnail> {
        let key = format!("paint:{paint:?}");
        let paths: BTreeSet<&str> = std::iter::once(paint.base.as_str())
            .chain(paint.layers.iter().map(|layer| layer.mask.as_str()))
            .collect();
        let hash = paths
            .iter()
            .map(|path| files.hash_of(path))
            .collect::<Option<Vec<&str>>>()?
            .join(" ");
        match self.entries.get(&key) {
            Some(Entry::Decoded(thumb)) if thumb.hash == hash => return Some(thumb.clone()),
            Some(Entry::Undecodable { hash: known }) if *known == hash => return None,
            _ => {}
        }
        let images: HashMap<&str, Image> = paths
            .iter()
            .filter_map(|path| {
                let decoded = image::load_from_memory(files.get(path)?).ok()?;
                Some((
                    *path,
                    Image::from_dynamic(decoded, true, RenderAssetUsages::default()),
                ))
            })
            .collect();
        let baked = images.get(paint.base.as_str()).and_then(|base| {
            let layers = paint
                .layers
                .iter()
                .map(|layer| Some((images.get(layer.mask.as_str())?, layer)))
                .collect::<Option<Vec<_>>>()?;
            bake(base, &layers)?.try_into_dynamic().ok()
        });
        let max_side = ctx.input(|i| i.max_texture_side) as u32;
        let thumb = baked.map(|image| {
            let image = image.into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            Thumbnail {
                texture: ctx.load_texture(
                    key.clone(),
                    to_color_image(shrink(image, max_side)),
                    TextureOptions::LINEAR,
                ),
                size,
                hash: hash.clone(),
            }
        });
        let entry = match &thumb {
            Some(thumb) => Entry::Decoded(thumb.clone()),
            None => Entry::Undecodable { hash },
        };
        self.entries.insert(key, entry);
        thumb
    }

    /// The `frame`th cell, each `size` pixels, of the sheet at `path`, cut down to the pixels it
    /// draws, so what it shows fills the room it is given.
    pub fn icon(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        path: &str,
        budget: &mut u32,
    ) -> Option<Thumbnail> {
        let file = path.to_owned();
        let key = format!("icon:{file}");
        self.load(ctx, files, key, file, budget, |image| {
            Some(shrink(image, ICON))
        })
    }

    fn load(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        key: String,
        file: String,
        budget: &mut u32,
        prepare: impl FnOnce(RgbaImage) -> Option<RgbaImage>,
    ) -> Option<Thumbnail> {
        let bytes = files.get(&file)?;
        let hash = files.hash_of(&file)?.to_owned();
        match self.entries.get(&key) {
            Some(Entry::Decoded(thumb)) if thumb.hash == hash => return Some(thumb.clone()),
            Some(Entry::Undecodable { hash: known }) if *known == hash => return None,
            _ => {}
        }
        if *budget == 0 {
            ctx.request_repaint();
            return None;
        }
        *budget -= 1;
        let max_side = ctx.input(|i| i.max_texture_side) as u32;
        let thumb = decode(bytes).and_then(prepare).map(|image| {
            let size = [image.width() as usize, image.height() as usize];
            let image = to_color_image(shrink(image, max_side));
            Thumbnail {
                texture: ctx.load_texture(key.clone(), image, TextureOptions::NEAREST),
                size,
                hash: hash.clone(),
            }
        });
        let entry = match &thumb {
            Some(thumb) => Entry::Decoded(thumb.clone()),
            None => Entry::Undecodable { hash },
        };
        self.entries.insert(key, entry);
        thumb
    }
}

pub fn uv(size: [usize; 2], x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    let (sw, sh) = (size[0] as f32, size[1] as f32);
    egui::Rect::from_min_max(
        egui::pos2(x / sw, y / sh),
        egui::pos2((x + w) / sw, (y + h) / sh),
    )
}

pub fn square_uv(size: [usize; 2]) -> egui::Rect {
    let (w, h) = (size[0] as f32, size[1] as f32);
    if h >= w {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, w / h))
    } else {
        let margin = (1.0 - h / w) / 2.0;
        egui::Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    }
}

pub fn flip_uv(uv: egui::Rect, horizontal: bool, vertical: bool) -> egui::Rect {
    let (mut min, mut max) = (uv.min, uv.max);
    if horizontal {
        std::mem::swap(&mut min.x, &mut max.x);
    }
    if vertical {
        std::mem::swap(&mut min.y, &mut max.y);
    }
    egui::Rect::from_min_max(min, max)
}

fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.into_rgba8())
}

fn to_color_image(image: RgbaImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, image.as_raw())
}

fn shrink(image: RgbaImage, longest: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    if w <= longest && h <= longest {
        return image;
    }
    let scale = longest as f64 / w.max(h) as f64;
    let sw = ((w as f64 * scale).round() as u32).max(1);
    let sh = ((h as f64 * scale).round() as u32).max(1);
    image::imageops::resize(&image, sw, sh, FilterType::Nearest)
}

/// What the renderer read back of a thumbnail, kept until the thumbnails that asked take it.
#[derive(Resource, Default)]
pub struct ReadThumbnails(Vec<(Entity, Vec<u8>)>);

/// Takes what the renderer read back, for the thumbnails to pick up on their next frame.
pub fn took(read: On<ReadbackComplete>, mut taken: ResMut<ReadThumbnails>, mut commands: Commands) {
    taken.0.push((read.entity, read.data.clone()));
    commands.entity(read.entity).despawn();
}

/// The pixels read back for `readback`, once they are.
pub(crate) fn taken(world: &mut World, readback: Entity) -> Option<Vec<u8>> {
    let mut read = world.resource_mut::<ReadThumbnails>();
    let index = read.0.iter().position(|(entity, _)| *entity == readback)?;
    Some(read.0.remove(index).1)
}
