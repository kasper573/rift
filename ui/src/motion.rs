use egui::emath::easing;
use egui::emath::{Rot2, TSTransform};
use egui::epaint::{ClippedShape, Mesh, Primitive, Shadow};
use egui::{Color32, Context, LayerId, Pos2, Rect, Vec2};

use crate::sheet;

pub const SHEET_OPEN_SECS: f32 = 0.18;
pub const SHEET_CLOSE_SECS: f32 = 0.12;
pub const DEAL_SECS: f32 = 0.22;
pub const FLICK_SECS: f32 = 0.15;
pub const FAN_OPEN_SECS: f32 = 0.5;
pub const FAN_CLOSE_SECS: f32 = 0.32;
const LAND_AT: f32 = 0.19 / DEAL_SECS;

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetClock {
    progress: f32,
}

impl SheetClock {
    pub const OPEN: SheetClock = SheetClock { progress: 1.0 };
    pub const CLOSED: SheetClock = SheetClock { progress: 0.0 };

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

const SWELL_SCALE: f32 = 1.08;
const SWELL_SECONDS: f64 = 0.25;

/// A swell that draws the eye to something shown again: armed by the bump, timed from the next
/// draw.
#[derive(Default, Clone, Copy)]
pub struct Swell {
    bumped: bool,
    started: Option<f64>,
}

impl Swell {
    pub fn bump(&mut self) {
        self.bumped = true;
    }

    pub fn start_if_bumped(&mut self, now: f64) {
        if std::mem::take(&mut self.bumped) {
            self.started = Some(now);
        }
    }

    /// How far the bumped thing is scaled now, easing back to its size.
    pub fn scale(self, now: f64) -> f32 {
        self.started.map_or(1.0, |at| {
            let t = ((now - at) / SWELL_SECONDS).clamp(0.0, 1.0) as f32;
            1.0 + (SWELL_SCALE - 1.0) * (1.0 - easing::cubic_out(t))
        })
    }
}

/// The items of a place as they come and go, each keyed: a new one enters, one gone leaves from
/// where it stood, by the place's [`Transit`].
pub struct Transits<T> {
    items: Vec<Transiting<T>>,
}

impl<T> Default for Transits<T> {
    fn default() -> Transits<T> {
        Transits { items: Vec::new() }
    }
}

impl<T: Clone> Transits<T> {
    /// Brings the place up to what it shows now, in the order given. An item that just left keeps
    /// the slot it had while it fades out.
    pub fn sync(
        &mut self,
        now: f64,
        transit: Transit,
        current: impl IntoIterator<Item = (u64, T)>,
    ) -> Vec<(T, TransitPose)> {
        let mut previous: Vec<Option<Transiting<T>>> = std::mem::take(&mut self.items)
            .into_iter()
            .map(Some)
            .collect();
        let mut items: Vec<Transiting<T>> = Vec::new();
        for (key, item) in current {
            let kept = previous
                .iter_mut()
                .find_map(|slot| slot.take_if(|transiting| transiting.key == key));
            items.push(match kept {
                Some(mut transiting) => {
                    transiting.item = item;
                    if transiting.leaving {
                        transiting.leaving = false;
                        transiting.since = now;
                    }
                    transiting
                }
                None => Transiting {
                    key,
                    item,
                    since: now,
                    leaving: false,
                },
            });
        }
        for (index, slot) in previous.into_iter().enumerate() {
            let Some(mut transiting) = slot else {
                continue;
            };
            if !transiting.leaving {
                transiting.leaving = true;
                transiting.since = now;
            }
            if now - transiting.since < transit.exit.seconds {
                items.insert(index.min(items.len()), transiting);
            }
        }
        self.items = items;
        self.items
            .iter()
            .map(|transiting| (transiting.item.clone(), transiting.pose(now, transit)))
            .collect()
    }
}

/// How the items of a place enter and leave.
#[derive(Clone, Copy)]
pub struct Transit {
    pub enter: TransitMove,
    pub exit: TransitMove,
}

/// The way an item travels on entering or leaving: from or to `offset` and `scale`, over
/// `seconds`.
#[derive(Clone, Copy)]
pub struct TransitMove {
    offset: Vec2,
    scale: f32,
    seconds: f64,
}

impl TransitMove {
    pub const fn new(offset: Vec2, scale: f32, seconds: f64) -> TransitMove {
        TransitMove {
            offset,
            scale,
            seconds,
        }
    }
}

/// Where an item stands in its transit: how opaque, moved and scaled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitPose {
    pub opacity: f32,
    pub offset: Vec2,
    pub scale: f32,
}

impl TransitPose {
    /// The transform that gives whatever is drawn in `rect` this pose, scaled about its centre.
    pub fn around(self, rect: Rect) -> TSTransform {
        let centre = rect.center().to_vec2();
        TSTransform::new(centre * (1.0 - self.scale) + self.offset, self.scale)
    }
}

struct Transiting<T> {
    key: u64,
    item: T,
    since: f64,
    leaving: bool,
}

impl<T> Transiting<T> {
    fn pose(&self, now: f64, transit: Transit) -> TransitPose {
        let (step, away) = if self.leaving {
            let t = ((now - self.since) / transit.exit.seconds).clamp(0.0, 1.0) as f32;
            (transit.exit, easing::cubic_in(t))
        } else {
            let t = ((now - self.since) / transit.enter.seconds).clamp(0.0, 1.0) as f32;
            (transit.enter, 1.0 - easing::cubic_out(t))
        };
        TransitPose {
            opacity: 1.0 - away,
            offset: step.offset * away,
            scale: 1.0 + (step.scale - 1.0) * away,
        }
    }
}

impl Default for TransitPose {
    fn default() -> TransitPose {
        TransitPose {
            opacity: 1.0,
            offset: Vec2::ZERO,
            scale: 1.0,
        }
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
