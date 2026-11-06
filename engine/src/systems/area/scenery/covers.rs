//! The covers an area's terrains grow, readied to stand as the view comes: each cover's sheet,
//! drawn with the ground over its patches wherever its terrain paints; the blades the GPU grows
//! round the views; and the copies of the props its pieces strew, which the scenery's arena holds
//! chunk by chunk as it holds the area's props.

use std::sync::Arc;

use bevy::prelude::*;

use super::expand::StrewnCover;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Pos;
use crate::core::render::world::cover::{self, CoverMaterial, StrewnPiece};
use crate::core::render::world::gpu::draw::GroundSheet;
use crate::core::render::world::gpu::grass::{BladeShape, GrassPiece};
use crate::core::units::{MeterSize, Meters, Radians, Seconds};
use crate::core::vfx::ParamFill;
use crate::core::vfx::render::{self as vfx_render, Run, SiteFrame};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::uniform::{ParamsUniform, WeatherImage};
use crate::systems::area::Area;
use crate::systems::area::prop::PropId;
use crate::systems::area::terrain::{CoverSheet, PieceShape};

/// The covers of an area's terrains as its scenery stands them: each terrain's that has one.
pub(super) struct Covers {
    layers: Vec<CoverLayer>,
}

/// A cover a terrain grows, ready to stand: its slot among the area's terrains, what its sheet lies
/// on if it has one and the terrain's colour it is drawn in, its pieces strewing props and growing
/// blades, and the material its sheet and blades are drawn with.
struct CoverLayer {
    slot: usize,
    sheet: Option<CoverSheet>,
    ground_color: LinearRgba,
    copies: Vec<CopyPiece>,
    grass: Vec<GrassPiece>,
    blended: Handle<CoverMaterial>,
}

/// A piece of a cover strewing copies of a prop: the cover it belongs to, how it is spread, the
/// prop, the most its prop's model measures across, and the material its copies are drawn with.
#[derive(Clone)]
pub(super) struct CopyPiece {
    pub cover: StrewnCover,
    pub spread: cover::Spread,
    pub prop: PropId,
    pub extent: Meters,
    pub material: Handle<CoverMaterial>,
}

impl CopyPiece {
    /// How far across the piece's largest copies measure.
    pub(super) fn largest(&self) -> Meters {
        Meters(self.extent.0 * self.spread.size.1)
    }
}

impl Covers {
    pub(super) fn new(world: &mut World, content: &Content, area: &Area) -> Covers {
        Covers {
            layers: cover_layers(world, content, area),
        }
    }

    /// The pieces of the covers that grow blades.
    pub(super) fn grass(&self) -> Arc<[GrassPiece]> {
        self.layers
            .iter()
            .flat_map(|layer| layer.grass.iter().cloned())
            .collect()
    }

    /// The pieces of the covers that strew copies of a prop, in the order the scenery's arena
    /// draws them.
    pub(super) fn copies(&self) -> Arc<[CopyPiece]> {
        self.layers
            .iter()
            .flat_map(|layer| layer.copies.iter().cloned())
            .collect()
    }

    /// The sheets the covers lay over the ground.
    pub(super) fn sheets(&self) -> Vec<GroundSheet> {
        self.layers
            .iter()
            .filter_map(|layer| {
                Some(GroundSheet {
                    terrain: u8::try_from(layer.slot).ok()?,
                    on: match layer.sheet? {
                        CoverSheet::Ground => cover::SheetOn::Ground,
                        CoverSheet::Water => cover::SheetOn::Water,
                    },
                    color: layer.ground_color,
                    material: layer.blended.clone(),
                })
            })
            .collect()
    }
}

/// Each terrain with a cover, readied to lay it as the view comes: the material its sheet and
/// blades are drawn with, through the terrain's cover shader with the area as its site, its pieces
/// growing blades, and those strewing a prop with how each is spread and the material its copies
/// are drawn with.
fn cover_layers(world: &mut World, content: &Content, area: &Area) -> Vec<CoverLayer> {
    let bounds = area.size.bounds();
    let site = PlacedShape {
        at: Pos::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        ),
        facing: Radians::default(),
        shape: Shape::Rect {
            width: Meters(bounds.size.width),
            height: Meters(bounds.size.height),
        },
    };
    let models = content.built::<GltfInfos>();
    let prop_bounds = |prop: PropId| {
        let bounds = models.0.get(&prop.get(content).model.path())?.bounds;
        Some((bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z))
    };
    let measures = |prop: PropId| {
        let bounds = models.0.get(&prop.get(content).model.path())?.bounds;
        Some((
            Meters((bounds.max - bounds.min).max_element()),
            Meters(bounds.max.y),
        ))
    };
    let mut layers = Vec::new();
    for (slot, terrain) in area.terrains.iter().enumerate() {
        let ground = terrain.get(content);
        let Some(cover_def) = &ground.cover else {
            continue;
        };
        let Some(shader) = vfx_render::compiled_shader(world, cover_def.shader) else {
            continue;
        };
        let frame = SiteFrame::of_area(&site, |at| area.heights.height_at(at));
        let run = Run {
            progress: 0.0,
            time: Seconds(0.0),
        };
        let params = ParamsUniform::pack(cover_def.shader.get(content), |name| {
            match cover_def.params.get(name)? {
                ParamFill::Value(value) => Some(*value),
                ParamFill::Param { .. } => None,
            }
        });
        let material = |world: &mut World, strewn: Option<&StrewnPiece>| {
            let weather = world.resource::<WeatherImage>().0.clone();
            world
                .resource_mut::<Assets<CoverMaterial>>()
                .add(cover::material(
                    frame.uniform(run, Stage::Cover),
                    params,
                    shader.clone(),
                    weather,
                    strewn,
                ))
        };
        let blended = material(world, None);
        let ground_color = ground.color.color().to_linear();
        let mut copies = Vec::new();
        let mut grass = Vec::new();
        for (index, piece) in cover_def.pieces.iter().enumerate() {
            let spread = cover::Spread {
                piece: index as u32,
                per_square_meter: piece.per_square_meter(piece.shape.base_area(prop_bounds)),
                size: (piece.size.min, piece.size.max),
                color: (
                    piece.color.from.color().to_linear(),
                    piece.color.to.color().to_linear(),
                ),
            };
            match &piece.shape {
                PieceShape::Prop { prop } => {
                    let Some((extent, height)) = measures(*prop) else {
                        continue;
                    };
                    let strewn = StrewnPiece { spread, height };
                    copies.push(CopyPiece {
                        cover: StrewnCover {
                            slot,
                            seed: cover_def.seed,
                        },
                        spread,
                        prop: *prop,
                        extent,
                        material: material(world, Some(&strewn)),
                    });
                }
                PieceShape::Blade { width, height } => {
                    if let Ok(terrain) = u8::try_from(slot) {
                        grass.push(GrassPiece {
                            terrain,
                            piece: spread.piece,
                            seed: cover_def.seed,
                            per_square_meter: spread.per_square_meter,
                            size: spread.size,
                            color: spread.color,
                            shape: BladeShape {
                                width: width.0,
                                height: height.0,
                            },
                            material: blended.clone(),
                        });
                    }
                }
            }
        }
        layers.push(CoverLayer {
            slot,
            sheet: cover_def.sheet,
            ground_color,
            copies,
            grass,
            blended,
        });
    }
    layers
}
