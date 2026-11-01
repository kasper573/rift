use egui::emath::{Rot2, TSTransform};
use egui::epaint::{ClippedShape, Mesh, Primitive, Shadow};
use egui::{Color32, Context, LayerId, Pos2, Rect};

use crate::sheet;

pub const SHEET_OPEN_SECS: f32 = 0.18;
pub const SHEET_CLOSE_SECS: f32 = 0.12;
pub const DEAL_SECS: f32 = 0.22;
pub const FLICK_SECS: f32 = 0.15;
const LAND_AT: f32 = 0.19 / DEAL_SECS;

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    progress: f32,
}

impl Clock {
    pub const OPEN: Clock = Clock { progress: 1.0 };
    pub const CLOSED: Clock = Clock { progress: 0.0 };

    pub fn run(&mut self, open: bool, seconds: f32) {
        let step = if open {
            seconds / SHEET_OPEN_SECS
        } else {
            -seconds / SHEET_CLOSE_SECS
        };
        self.progress = (self.progress + step).clamp(0.0, 1.0);
    }

    pub fn progress(self) -> f32 {
        self.progress
    }

    pub fn is_closed(self) -> bool {
        self.progress <= 0.0
    }

    pub fn is_open(self) -> bool {
        self.progress >= 1.0
    }

    pub fn pose(self) -> SheetPose {
        let t = ease_out(self.progress);
        SheetPose {
            opacity: t,
            scale: 1.03 - 0.03 * t,
            dy: -4.0 + 4.0 * t,
            shadow: Shadow {
                offset: [0, lerp(20.0, 12.0, t).round() as i8],
                blur: lerp(26.0, 18.0, t).round() as u8,
                spread: 0,
                color: Color32::from_black_alpha(lerp(115.0, 128.0, t).round() as u8),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetPose {
    pub opacity: f32,
    pub scale: f32,
    pub dy: f32,
    pub shadow: Shadow,
}

impl SheetPose {
    pub const REST: SheetPose = SheetPose {
        opacity: 1.0,
        scale: 1.0,
        dy: 0.0,
        shadow: sheet::shadow(sheet::Depth::Front),
    };

    /// The pose of a sheet standing in `rect`, nudged by under a pixel so `pin` keeps the place it
    /// has within a pixel at rest: text there is never rounded to a neighbouring pixel while the
    /// shapes around it move on smoothly.
    pub fn transform(self, ctx: &Context, rect: Rect, pin: Pos2) -> TSTransform {
        let centre = rect.center().to_vec2();
        let posed = TSTransform::from_translation(centre + egui::Vec2::new(0.0, self.dy))
            * TSTransform::from_scaling(self.scale)
            * TSTransform::from_translation(-centre);
        let pixels = ctx.pixels_per_point();
        let within = |at: Pos2| {
            let at = at.to_vec2() * pixels;
            at - at.round()
        };
        let nudge = (within(pin) - within(posed * pin)) / pixels;
        TSTransform::from_translation(nudge) * posed
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardPose {
    pub center: Pos2,
    pub scale: f32,
    pub angle: f32,
    pub opacity: f32,
}

pub fn deal(t: f32, from: Rect, to: Rect) -> CardPose {
    let e = ease_out(t);
    let overshoot = if t > LAND_AT * 0.85 && t < 1.0 {
        let phase = ((t - LAND_AT * 0.85) / (1.0 - LAND_AT * 0.85)).clamp(0.0, 1.0);
        0.01 * (phase * std::f32::consts::PI).sin()
    } else {
        0.0
    };
    CardPose {
        center: from.center() + (to.center() - from.center()) * e,
        scale: 0.2 + 0.8 * e + overshoot,
        angle: (-14.0_f32).to_radians() * (1.0 - e),
        opacity: (e * 1.5).min(1.0),
    }
}

pub fn flick(t: f32, at: Rect) -> CardPose {
    let e = ease_in(t);
    CardPose {
        center: at.center() - egui::Vec2::new(0.0, 20.0 * e),
        scale: 1.0,
        angle: 4.0_f32.to_radians() * e,
        opacity: 1.0 - e,
    }
}

pub fn replaced(t: f32, at: Rect) -> CardPose {
    let e = ease_out(t);
    CardPose {
        center: at.center() + egui::Vec2::new(0.0, 26.0 * e),
        scale: 1.0,
        angle: 0.0,
        opacity: 1.0 - 0.45 * e,
    }
}

pub fn posed(ctx: &Context, layer: LayerId, rest: Rect, pose: CardPose) -> Vec<Mesh> {
    let shapes: Vec<ClippedShape> = ctx.graphics_mut(|graphics| {
        std::mem::take(graphics.entry(layer))
            .all_entries()
            .cloned()
            .collect()
    });
    let centre = rest.center().to_vec2();
    let scale = TSTransform::from_translation(centre)
        * TSTransform::from_scaling(pose.scale)
        * TSTransform::from_translation(-centre);
    let travel = TSTransform::from_translation(pose.center - rest.center());
    ctx.tessellate(shapes, ctx.pixels_per_point())
        .into_iter()
        .filter_map(|primitive| match primitive.primitive {
            Primitive::Mesh(mesh) => Some(mesh),
            Primitive::Callback(_) => None,
        })
        .map(|mut mesh| {
            mesh.transform(scale);
            mesh.rotate(Rot2::from_angle(pose.angle), rest.center());
            mesh.transform(travel);
            if pose.opacity < 1.0 {
                for vertex in &mut mesh.vertices {
                    vertex.color = vertex.color.gamma_multiply(pose.opacity);
                }
            }
            mesh
        })
        .collect()
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
