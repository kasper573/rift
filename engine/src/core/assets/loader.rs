//! PNG files read as images at a pace: a row at a time, breathing between rows, so a large
//! texture decodes over many frames on the one thread a browser gives the client.

use std::io::{self, Cursor};

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::image::{Image, ImageLoaderSettings};
use bevy::reflect::TypePath;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::core::pace::Pace;

/// Reads a PNG as an image of RGBA bytes, in sRGB unless its settings ask for linear light.
#[derive(TypePath)]
pub struct PngLoader {
    pub pace: Pace,
}

impl AssetLoader for PngLoader {
    type Asset = Image;
    type Settings = ImageLoaderSettings;
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &ImageLoaderSettings,
        _: &mut LoadContext<'_>,
    ) -> Result<Image, io::Error> {
        let mut encoded = Vec::new();
        reader.read_to_end(&mut encoded).await?;
        let mut decoder = png::Decoder::new(Cursor::new(encoded));
        decoder.set_transformations(
            png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
        );
        let mut reader = decoder.read_info().map_err(io::Error::other)?;
        let (width, height) = (reader.info().width, reader.info().height);
        let (color, _) = reader.output_color_type();
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        while let Some(row) = reader.next_row().map_err(io::Error::other)? {
            match color {
                png::ColorType::Rgba => pixels.extend_from_slice(row.data()),
                png::ColorType::GrayscaleAlpha => {
                    for pixel in row.data().as_chunks::<2>().0 {
                        pixels.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
                    }
                }
                other => {
                    return Err(io::Error::other(format!(
                        "a PNG of {other:?} samples after normalising"
                    )));
                }
            }
            self.pace.breathe().await;
        }
        let format = if settings.is_srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        };
        let mut image = Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            format,
            settings.asset_usage,
        );
        image.sampler = settings.sampler.clone();
        Ok(image)
    }

    fn extensions(&self) -> &[&str] {
        &["png"]
    }
}
