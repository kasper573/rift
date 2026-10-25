use std::collections::HashMap;
use ui::egui;
use ui::egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use engine::core::content::Content;
use engine::core::math::{Pos, Rect};
use engine::core::terrain::SAMPLES_PER_METER;
use engine::core::units::{GROUND_ORIGIN, Meters, TileSize};
use engine::systems::area::{self, AreaDef, AreaId};

use crate::views::navigation::Viewport;

/// How much a metre of rise darkens or lightens the ground, so slopes read on a flat picture.
const RELIEF: f32 = 0.08;

/// The ground of an area or blueprint as a flat picture: its materials' colours blended as the
/// paint weighs them over every half metre, lit from the top-left by the heights.
pub struct GroundArt {
    texture: TextureHandle,
    bounds: Rect<Meters>,
    generation: u64,
}

impl GroundArt {
    /// Draws the ground where `canvas` puts it, clipped to the painter.
    pub fn draw(&self, painter: &egui::Painter, canvas: Viewport<Meters>, tint: Color32) {
        painter.image(
            self.texture.id(),
            canvas.rect(self.bounds),
            crate::thumbs::FULL_UV,
            tint,
        );
    }

    pub fn bounds(&self) -> Rect<Meters> {
        self.bounds
    }
}

/// The ground pictures the views draw, each kept until the content it was made from changes.
#[derive(Default)]
pub struct GroundArts(HashMap<String, GroundArt>);

impl GroundArts {
    /// The picture of `area`, made again when `generation` is not the one it was made at.
    pub fn of(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        generation: u64,
        content: &Content,
        area: &area::Area,
    ) -> &GroundArt {
        let stale = self
            .0
            .get(key)
            .is_none_or(|art| art.generation != generation);
        if stale {
            let image = picture(content, area);
            let texture = ctx.load_texture(format!("ground {key}"), image, TextureOptions::NEAREST);
            self.0.insert(
                key.to_owned(),
                GroundArt {
                    texture,
                    bounds: area.size.bounds(),
                    generation,
                },
            );
        }
        &self.0[key]
    }

    /// The picture of the area at `id`, as the game loads it.
    pub fn of_area(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        generation: u64,
        content: &Content,
        id: AreaId,
    ) -> &GroundArt {
        let area = area::load(content, id);
        self.of(ctx, key, generation, content, area)
    }

    pub fn forget(&mut self, key: &str) {
        self.0.remove(key);
    }
}

fn picture(content: &Content, area: &area::Area) -> ColorImage {
    let colors: Vec<[f32; 3]> = area
        .terrain
        .materials
        .iter()
        .map(|ground| {
            let [r, g, b, _] = <[u8; 4]>::from(ground.get(content).color);
            [r as f32, g as f32, b as f32]
        })
        .collect();
    let (columns, rows) = area.terrain.size.paint_pixels();
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let mut pixels = Vec::with_capacity((columns * rows) as usize);
    for row in 0..rows {
        for column in 0..columns {
            let weights = area.paint.weights(column, row);
            let total: f32 = weights.iter().map(|w| *w as f32).sum();
            let mut color = [0.0f32; 3];
            if total > 0.0 {
                for (weight, material) in weights.iter().zip(&colors) {
                    for (channel, value) in color.iter_mut().zip(material) {
                        *channel += value * (*weight as f32) / total;
                    }
                }
            } else {
                color = colors[0];
            }
            let at = Pos::<Meters>::new(
                GROUND_ORIGIN.x + (column as f32 + 0.5) * step,
                GROUND_ORIGIN.y + (row as f32 + 0.5) * step,
            );
            let here = area.heights.height_at(at).0;
            let lit = area.heights.height_at(Pos::new(at.x - step, at.y - step)).0;
            let shade = (1.0 + (lit - here) * RELIEF).clamp(0.5, 1.5);
            let channel = |value: f32| (value * shade).round().clamp(0.0, 255.0) as u8;
            pixels.push(Color32::from_rgb(
                channel(color[0]),
                channel(color[1]),
                channel(color[2]),
            ));
        }
    }
    ColorImage {
        size: [columns as usize, rows as usize],
        source_size: egui::Vec2::new(columns as f32, rows as f32),
        pixels,
    }
}

/// The ground of the area at `path` drawn into `rect`, `center` in its middle at `px_per_meter`.
pub fn draw_area(
    editor: &mut crate::app::Editor,
    painter: &egui::Painter,
    key: &str,
    path: &str,
    rect: egui::Rect,
    center: Pos<Meters>,
    px_per_meter: f32,
) -> bool {
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let Some(content) = project.content.clone() else {
        return false;
    };
    let Some(id) = content.by_path::<AreaDef>(path) else {
        return false;
    };
    let generation = project.generation;
    let art =
        editor
            .views
            .area
            .grounds
            .of_area(&painter.ctx().clone(), key, generation, &content, id);
    let canvas = Viewport {
        screen: rect,
        center,
        zoom: px_per_meter / crate::views::navigation::PX_PER_METER,
    };
    art.draw(&painter.with_clip_rect(rect), canvas, Color32::WHITE);
    true
}
