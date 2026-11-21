//! The waters an area names, readied to lie as the view comes: each one's sheet, drawn with the
//! ground over its patches wherever the area's water map names it and water stands, through the
//! shader the water names; and what each is like from within, for its sheet and the ground under
//! it to read.

use bevy::prelude::*;

use crate::core::content::Content;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Pos;
use crate::core::render::world::gpu::draw::{GroundSheet, SheetOver};
use crate::core::render::world::water::{self, WaterLook, WaterMaterial};
use crate::core::units::{MeterSize, Meters, Radians, Seconds};
use crate::core::vfx::Stage;
use crate::core::vfx::board::ShaderBoardImage;
use crate::core::vfx::field::ShaderFieldImage;
use crate::core::vfx::render::{self as vfx_render, Run, SiteFrame};
use crate::core::vfx::uniform::ParamsUniform;
use crate::systems::area::Area;
use crate::systems::water::WaterId;

/// What each of `waters` is like from within, by its place among them.
pub(super) fn looks(content: &Content, waters: &[WaterId]) -> Vec<WaterLook> {
    waters
        .iter()
        .map(|water| {
            let def = water.get(content);
            WaterLook {
                clarity: def.clarity,
                depths: def.depths.color().to_linear(),
                murk: def.murk,
            }
        })
        .collect()
}

/// The sheet of each of `waters`, by its place among them, whose shader has compiled, with `area`
/// as its site, mirroring the picture of the sky `sky` where the area has one.
pub(super) fn sheets(
    world: &mut World,
    content: &Content,
    area: &Area,
    waters: &[WaterId],
    sky: Option<&Handle<Image>>,
) -> Vec<GroundSheet> {
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
    let frame = SiteFrame::of_area(&site, |at| area.heights.height_at(at));
    let run = Run {
        progress: 0.0,
        time: Seconds(0.0),
    };
    let mut sheets = Vec::new();
    for (slot, id) in waters.iter().enumerate() {
        let def = id.get(content);
        let (Ok(slot), Some(shader)) = (
            u8::try_from(slot),
            vfx_render::compiled_shader(world, def.shader),
        ) else {
            continue;
        };
        let params = ParamsUniform::pack(def.shader.get(content), |name| {
            def.params.get(name).copied()
        });
        let board = world.resource::<ShaderBoardImage>().image.clone();
        let field = world.resource::<ShaderFieldImage>().image.clone();
        let material = world
            .resource_mut::<Assets<WaterMaterial>>()
            .add(water::material(
                frame.uniform_over(run, frame.metres(Stage::GROUND)),
                params,
                shader,
                board,
                field,
            ));
        sheets.push(GroundSheet {
            over: SheetOver::Water {
                water: slot,
                sky: sky.cloned(),
            },
            color: def.depths.color().to_linear(),
            material: material.untyped(),
        });
    }
    sheets
}
