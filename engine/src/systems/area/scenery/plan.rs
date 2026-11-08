//! What the scenery stands for a sight, planned off the frame: the chunks of ground and the lights
//! a view could take in from anywhere its eye may stand, so a
//! view turning, tilting or zooming within its sight plans nothing anew, and one moving across
//! the ground plans anew only as it crosses into another cell of it. An eye that may stand any
//! way round the ground stands near everything at its distance from it, so a view whose eye keeps
//! its bearing says so, and is planned for that side alone.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use bevy::math::Vec3;
use bevy::platform::collections::HashSet;

use super::{Node, distance_to, nodes_for};
use crate::core::ground::Heightfield;
use crate::core::math::{Pos, Rect};
use crate::core::render::camera::{CameraFraming, Lens};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{Meters, Radians};
use crate::systems::area::Area;

/// Where a view may look from, as the scenery stands for it: the ground it looks at, how far
/// around that ground the covers, props and lights stand, how far off it the far field's props
/// still stand (or as far as the ground goes), its lens, and everywhere around that ground its
/// eye may stand. The scenery stands for every view the sight allows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub at: Pos<Meters>,
    pub reach: Meters,
    pub horizon: Option<Meters>,
    pub lens: Lens,
    pub eyes: Eyes,
}

/// Everywhere a view's eye may stand around the ground it looks at: from `nearest` to `farthest`
/// along the ground off it, at `bearings` round it, and from `lowest` to `highest` over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eyes {
    pub nearest: Meters,
    pub farthest: Meters,
    pub lowest: Meters,
    pub highest: Meters,
    pub bearings: Bearings,
}

/// Which way round the ground it looks at an eye may stand, by the bearing from that ground to it
/// (from the x axis toward the y axis of the ground).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bearings {
    /// Any way round, as an orbiting camera may turn.
    Any,
    /// Along the arc turning from `from` to `to`.
    Within { from: Radians, to: Radians },
}

impl Eyes {
    /// Everywhere a camera framed by `framing` may stand orbiting a point `lift` over the ground it
    /// looks at: at any of its zooms and pitches, any way round.
    pub fn orbiting(framing: &CameraFraming, lift: Meters) -> Eyes {
        let (lowest, highest) = (
            framing.lowest_pitch.radians().0,
            framing.highest_pitch.radians().0,
        );
        let (close, wide) = (framing.zooms.close.0, framing.zooms.wide.0);
        Eyes {
            nearest: Meters(close * highest.cos()),
            farthest: Meters(wide * lowest.cos()),
            lowest: Meters(lift.0 + close * lowest.sin()),
            highest: Meters(lift.0 + wide * highest.sin()),
            bearings: Bearings::Any,
        }
    }

    /// An eye standing at `eye` over the ground at `at`, `ground` metres high.
    pub fn standing(eye: Vec3, at: Pos<Meters>, ground: Meters) -> Eyes {
        let across = Meters(Pos::new(eye.x, eye.z).distance_to(at));
        let over = Meters(eye.y - ground.0);
        let bearing = Radians((eye.z - at.y).atan2(eye.x - at.x));
        Eyes {
            nearest: across,
            farthest: across,
            lowest: over,
            highest: over,
            bearings: Bearings::Within {
                from: bearing,
                to: bearing,
            },
        }
    }
}

impl Bearings {
    /// The bearings taken out to whole steps of a turn, each step's arc no longer than a cell
    /// at `nearest` off the ground looked at, so every bearing within the same steps plans alike.
    fn widened(self, nearest: Meters) -> Bearings {
        let Bearings::Within { from, to } = self else {
            return Bearings::Any;
        };
        let steps = (TAU * nearest.0 / PLAN_CELL.0)
            .ceil()
            .max(LEAST_BEARING_STEPS) as u32;
        let step = TAU / steps.next_power_of_two() as f32;
        let from = (from.0 / step).floor() * step;
        let to = ((to.0 / step).floor() + 1.0) * step;
        if to - from >= TAU {
            return Bearings::Any;
        }
        Bearings::Within {
            from: Radians(from),
            to: Radians(to),
        }
    }

    /// The arc as its start and how far it turns, none when it goes all the way round.
    fn arc(self) -> Option<(f32, f32)> {
        match self {
            Bearings::Any => None,
            Bearings::Within { from, to } => Some((from.0, to.0 - from.0)),
        }
    }
}

/// The fewest steps of a turn an eye's bearing is planned for.
const LEAST_BEARING_STEPS: f32 = 16.0;

impl Sight {
    /// The sight the scenery plans for in place of this one: its ground taken to the middle of
    /// the cell it lies in, its reach and its eyes' distances along the ground to whole cells,
    /// their bearings to steps of a turn about a cell long, and their heights to half octaves, so
    /// that every view within the same steps plans alike. An eye no farther off its ground than
    /// that ground may lie off the cell's middle may stand any way round it.
    ///
    /// The plan stands everything as near as the nearest place an eye may stand, so where the
    /// ground could lie within those places they stay about a cell wide however far off the eye
    /// is: half an octave of a far eye's distance would stand the finest scenery over a band of
    /// ground hundreds of metres wide. What lies under the eye never lies within its heights,
    /// so those may widen with its height.
    pub(super) fn widened(self) -> Sight {
        let cell = PLAN_CELL.0;
        let middle = |at: f32| ((at / cell).floor() + 0.5) * cell;
        let nearest = Meters((self.eyes.nearest.0 / cell).floor() * cell);
        Sight {
            at: Pos::new(middle(self.at.x), middle(self.at.y)),
            reach: Meters((self.reach.0 / cell).ceil() * cell),
            horizon: self.horizon,
            lens: self.lens,
            eyes: Eyes {
                nearest,
                farthest: Meters((self.eyes.farthest.0 / cell).ceil() * cell),
                lowest: octaves_down(self.eyes.lowest),
                highest: octaves_up(self.eyes.highest),
                bearings: if self.eyes.nearest.0 <= OFF_MIDDLE {
                    Bearings::Any
                } else {
                    self.eyes.bearings.widened(nearest)
                },
            },
        }
    }
}

/// How much of a view's ground a plan holds for: views looking at ground within the same cell of
/// this side plan alike.
const PLAN_CELL: Meters = Meters(16.0);

/// The farthest ground within a cell lies from its middle.
const OFF_MIDDLE: f32 = PLAN_CELL.0 / std::f32::consts::SQRT_2;

/// A widened sight as the plan measures from it: where off the cell's middle any of its eyes may
/// stand along the ground and how high, with the ground looked at lying anywhere in the cell, as
/// high as the ground there may lie off the middle's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Seen {
    pub sight: Sight,
    /// How far around the cell's middle the scenery stands, out to the far side of the cell.
    pub reach: Meters,
    /// How far an eye may stand along the ground off the ground looked at.
    across: (f32, f32),
    /// The arc of bearings it may stand at, none for any.
    arc: Option<(f32, f32)>,
    /// How high any eye may stand.
    heights: (f32, f32),
}

impl Seen {
    pub(super) fn of(sight: Sight, heights: &Heightfield) -> Seen {
        let at = sight.at;
        let middle = heights.height_at(at).0;
        let half = PLAN_CELL.0 / 2.0;
        let steps = 4;
        let rise = (0..=steps)
            .flat_map(|row| (0..=steps).map(move |column| (column, row)))
            .map(|(column, row)| {
                let off = |step: i32| -half + PLAN_CELL.0 * step as f32 / steps as f32;
                let ground = heights.height_at(Pos::new(at.x + off(column), at.y + off(row)));
                (ground.0 - middle).abs()
            })
            .fold(0.0, f32::max);
        let eyes = sight.eyes;
        Seen {
            sight,
            reach: Meters(sight.reach.0 + OFF_MIDDLE),
            across: (eyes.nearest.0, eyes.farthest.0),
            arc: eyes.bearings.arc(),
            heights: (
                middle + eyes.lowest.0 - rise,
                middle + eyes.highest.0 + rise,
            ),
        }
    }

    pub(super) fn at(&self) -> Pos<Meters> {
        self.sight.at
    }

    pub(super) fn lens(&self) -> Lens {
        self.sight.lens
    }

    /// The order what lies at `point` stands in: what lies ahead of the eyes, as they look at the
    /// ground, before what lies behind them, the nearer the eyes the sooner. Eyes any way round
    /// have nothing behind them.
    pub(super) fn order(&self, point: Pos<Meters>) -> (bool, f32) {
        let at = self.at();
        let Some((start, turn)) = self.arc else {
            return (false, point.distance_to(at));
        };
        let (sin, cos) = (start + turn / 2.0).sin_cos();
        let eye = Pos::new(at.x + cos * self.across.0, at.y + sin * self.across.0);
        let ahead = (eye.x - point.x) * cos + (eye.y - point.y) * sin;
        (ahead < 0.0, point.distance_to(eye))
    }

    /// The nearest any eye may come to ground over `bounds` lying from `lowest` to `highest`.
    pub(super) fn over(&self, bounds: Rect<Meters>, lowest: Meters, highest: Meters) -> f32 {
        let at = self.at();
        let farthest = (at.x - bounds.min().x)
            .abs()
            .max((bounds.max().x - at.x).abs())
            .hypot(
                (at.y - bounds.min().y)
                    .abs()
                    .max((bounds.max().y - at.y).abs()),
            );
        self.nearest(
            (distance_to(bounds, at), farthest),
            arc_of(bounds, at),
            (lowest.0, highest.0),
        )
    }

    /// How finely the view shows what lies `from_eye` off it.
    pub(super) fn detail(&self, from_eye: f32) -> Detail {
        Detail((2.0 * from_eye.max(NEAREST_GROUND.0).log2()).floor() as i32)
    }

    /// The nearest any eye may come to what lies from `across.0` to `across.1` along the ground
    /// off the cell's middle, at the bearings of `arc` (none for any) and from `heights.0` to
    /// `heights.1` high. Along the ground it is the gap to where the eyes stand round the middle,
    /// less as far as the ground looked at may lie off the middle; an arc of bearings away from
    /// the eyes' puts at least the nearest eye's distance times the sine of the angle between.
    fn nearest(&self, across: (f32, f32), arc: Option<(f32, f32)>, heights: (f32, f32)) -> f32 {
        let gap =
            |(low, high): (f32, f32), (from, to): (f32, f32)| (from - high).max(low - to).max(0.0);
        let turned = match (arc, self.arc) {
            (Some(arc), Some(eyes)) => {
                self.across.0 * apart(arc, eyes).min(std::f32::consts::FRAC_PI_2).sin()
            }
            _ => 0.0,
        };
        let along = (gap(across, self.across).max(turned) - OFF_MIDDLE).max(0.0);
        along
            .hypot(gap(heights, self.heights))
            .max(NEAREST_GROUND.0)
    }
}

/// The arc of bearings from `at` to the points of `bounds`, as its start and how far it turns:
/// none when `at` lies within them.
fn arc_of(bounds: Rect<Meters>, at: Pos<Meters>) -> Option<(f32, f32)> {
    if bounds.contains(at) {
        return None;
    }
    let middle = bounds.center();
    let toward = (middle.y - at.y).atan2(middle.x - at.x);
    let (least, most) = crate::core::geometry::corners(bounds)
        .into_iter()
        .map(|corner| wrapped((corner.y - at.y).atan2(corner.x - at.x) - toward + PI) - PI)
        .fold((0.0f32, 0.0f32), |(least, most), turn| {
            (least.min(turn), most.max(turn))
        });
    Some((toward + least, most - least))
}

/// The least angle between a bearing of the arc `a` and one of the arc `b`, each its start and
/// how far it turns.
fn apart(a: (f32, f32), b: (f32, f32)) -> f32 {
    if wrapped(b.0 - a.0) <= a.1 || wrapped(a.0 - b.0) <= b.1 {
        return 0.0;
    }
    wrapped(b.0 - (a.0 + a.1)).min(wrapped(a.0 - (b.0 + b.1)))
}

/// `angle` taken round into a single turn from nothing.
fn wrapped(angle: f32) -> f32 {
    angle.rem_euclid(TAU)
}

/// How finely the scenery shows what lies somewhere off the eye: as finely as what lies 2^(n/2)
/// metres off it, taken to the half octave nearer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Detail(i32);

impl Detail {
    /// How far off the eye what shows at this detail is taken to be.
    pub(super) fn distance(self) -> Meters {
        Meters((self.0 as f32 / 2.0).exp2())
    }
}

/// The nearest the ground is taken to come to the eye, so an eye on or below it still shows a
/// detail.
const NEAREST_GROUND: Meters = Meters(0.5);

/// What a plan is made from that stays as long as the area stands: the area and its lights.
pub(super) struct Planner {
    pub area: Arc<Area>,
    pub lights: SpatialIndex,
}

/// What the scenery stands for a sight: where it is seen from, the ground's chunks and the
/// lights.
pub(super) struct Plan {
    pub seen: Seen,
    /// The chunks of ground, finest first.
    pub ground: Vec<Node>,
    pub lights: LightsPlan,
}

/// The lights the plan stands, nearest the view first, and those it keeps that stand already.
pub(super) struct LightsPlan {
    pub near: Vec<usize>,
    pub kept: HashSet<usize>,
}

/// How far past its reach the view keeps what stands, so pacing back and forth over the edge
/// stands nothing twice.
pub(super) const KEEP_SHARE: f32 = 1.25;

impl Planner {
    pub(super) fn plan(&self, sight: Sight, heights: &Heightfield) -> Plan {
        let seen = Seen::of(sight, heights);
        let mut ground = nodes_for(heights, seen.at(), OFF_MIDDLE);
        ground.sort_by_key(|node| node.level);
        Plan {
            ground,
            lights: self.plan_lights(&seen),
            seen,
        }
    }

    fn plan_lights(&self, seen: &Seen) -> LightsPlan {
        let at = seen.at();
        let apart = |index: usize| self.area.lights[index].at.distance_to(at);
        let mut near: Vec<usize> = self
            .lights
            .near(at, seen.reach)
            .filter(|&index| apart(index) <= seen.reach.0)
            .collect();
        near.sort_by(|&a, &b| apart(a).total_cmp(&apart(b)));
        let keep = seen.reach.0 * KEEP_SHARE;
        let kept = self
            .lights
            .near(at, Meters(keep))
            .filter(|&index| apart(index) <= keep)
            .collect();
        LightsPlan { near, kept }
    }
}

/// `distance` taken down to the half octave below it, and to nothing from under the nearest
/// ground above nothing.
fn octaves_down(distance: Meters) -> Meters {
    if distance.0 < 0.0 {
        return Meters(-octaves_up(Meters(-distance.0)).0);
    }
    if distance.0 < NEAREST_GROUND.0 {
        return Meters(0.0);
    }
    Meters(((2.0 * distance.0.log2()).floor() / 2.0).exp2())
}

/// `distance` taken up to the half octave above it, and to the nearest ground from under it.
fn octaves_up(distance: Meters) -> Meters {
    if distance.0 < 0.0 {
        return Meters(-octaves_down(Meters(-distance.0)).0);
    }
    Meters(((2.0 * distance.0.max(NEAREST_GROUND.0).log2()).ceil() / 2.0).exp2())
}
