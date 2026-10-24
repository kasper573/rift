use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use super::TILE;
use super::camera::WorldCamera;
use super::present::{self, SCALE};
use super::screen::ToScreen;
use crate::core::geometry::Radians;
use crate::core::math::Pos;
use crate::core::time::Seconds;
use crate::core::units::{Cell, CellPos, Meters};

#[derive(
    Serialize,
    Deserialize,
    VariantArray,
    AsRefStr,
    EnumString,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
)]
pub enum ScreenTransition {
    #[default]
    Crumble,
    #[strum(serialize = "Tile wave")]
    TileWave,
    Sweep,
    Iris,
    Mosaic,
    #[strum(serialize = "Dither dissolve")]
    Dither,
    Fade,
}

impl ScreenTransition {
    fn pace(self) -> Pace {
        let (cover, uncover) = match self {
            ScreenTransition::Crumble => (0.55, 0.65),
            ScreenTransition::TileWave | ScreenTransition::Iris => (0.45, 0.6),
            ScreenTransition::Sweep => (0.4, 0.45),
            ScreenTransition::Mosaic => (0.4, 0.55),
            ScreenTransition::Dither => (0.4, 0.5),
            ScreenTransition::Fade => (0.3, 0.4),
        };
        Pace {
            cover: Seconds(cover),
            uncover: Seconds(uncover),
        }
    }
}

#[derive(Resource, Default, Clone, Copy)]
pub struct ScreenTransitionPreference(pub ScreenTransition);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTransitionPhase {
    Idle,
    Out,
    Hold,
    In,
}

/// Systems that work with the world on view. They hold off while a transition covers the old view:
/// the area being entered isn't on view yet, and whatever they would load for it then loads under
/// the full cover, where a long frame can't stutter the animation.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorldViewSystems;

pub fn covering(transition: Res<ActiveTransition>) -> bool {
    transition.phase() == ScreenTransitionPhase::Out
}

pub fn freeze(world: &mut World, viewed_at: Pos<Meters>, heading: Radians) {
    let Some(from) = world_camera_marks(world, viewed_at) else {
        return;
    };
    let style = world.resource::<ScreenTransitionPreference>().0;
    present::hold_frame(world);
    world.resource_mut::<ActiveTransition>().0 = Some(Cut {
        style,
        played: Seconds(0.0),
        from,
        heading: screen_heading(heading),
        arrival: None,
    });
}

pub fn reveal(world: &mut World, viewed_at: Pos<Meters>) {
    if let Some(cut) = world.resource_mut::<ActiveTransition>().0.as_mut()
        && cut.arrival.is_none()
    {
        cut.arrival = Some(Arrival {
            at: cut.played,
            viewed_at,
        });
    }
}

pub fn abort(world: &mut World) {
    world.resource_mut::<ActiveTransition>().0 = None;
}

/// A long frame pauses the transition rather than skipping part of it.
const LONGEST_STEP: Seconds = Seconds(1.0 / 30.0);

#[derive(Resource, Default)]
pub struct ActiveTransition(Option<Cut>);

impl ActiveTransition {
    pub fn phase(&self) -> ScreenTransitionPhase {
        self.0
            .as_ref()
            .map_or(ScreenTransitionPhase::Idle, |cut| cut.progress().phase)
    }

    pub(super) fn advance(
        &mut self,
        frame: Seconds,
        camera: Option<(&Camera, &GlobalTransform)>,
    ) -> CutUniform {
        let Some(cut) = &mut self.0 else {
            return CutUniform::IDLE;
        };
        cut.played += frame.min(LONGEST_STEP);
        let progress = cut.progress();
        if progress.phase == ScreenTransitionPhase::Idle {
            self.0 = None;
            return CutUniform::IDLE;
        }
        let size = camera
            .and_then(|(camera, _)| camera.logical_viewport_size())
            .unwrap_or(Vec2::ONE);
        let to = cut
            .arrival
            .as_ref()
            .zip(camera)
            .and_then(|(arrival, (camera, transform))| {
                target_marks(camera, transform, arrival.viewed_at)
            })
            .unwrap_or(cut.from);
        CutUniform {
            style: cut.style as u32,
            out_progress: progress.cover,
            in_progress: progress.uncover,
            time: cut.played.0,
            tile: tile_texels(),
            pixel: SCALE,
            from_radius: radius(cut.from, size),
            to_radius: radius(to, size),
            from_anchor: cut.from.viewed,
            to_anchor: to.viewed,
            from_grid: cut.from.tile_origin,
            to_grid: to.tile_origin,
            sweep: -cut.heading,
            from_reach: reach(cut.from, size),
            to_reach: reach(to, size),
        }
    }
}

#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub(super) struct CutUniform {
    style: u32,
    out_progress: f32,
    in_progress: f32,
    time: f32,
    tile: f32,
    pixel: f32,
    from_radius: f32,
    to_radius: f32,
    from_anchor: Vec2,
    to_anchor: Vec2,
    from_grid: Vec2,
    to_grid: Vec2,
    sweep: Vec2,
    from_reach: f32,
    to_reach: f32,
}

impl CutUniform {
    pub(super) const IDLE: CutUniform = CutUniform {
        style: 0,
        out_progress: 1.0,
        in_progress: 1.0,
        time: 0.0,
        tile: 1.0,
        pixel: 1.0,
        from_radius: 1.0,
        to_radius: 1.0,
        from_anchor: Vec2::ZERO,
        to_anchor: Vec2::ZERO,
        from_grid: Vec2::ZERO,
        to_grid: Vec2::ZERO,
        sweep: Vec2::X,
        from_reach: 1.0,
        to_reach: 1.0,
    };
}

struct Pace {
    cover: Seconds,
    uncover: Seconds,
}

struct Cut {
    style: ScreenTransition,
    played: Seconds,
    from: TargetMarks,
    heading: Vec2,
    arrival: Option<Arrival>,
}

struct Arrival {
    at: Seconds,
    viewed_at: Pos<Meters>,
}

#[derive(Clone, Copy)]
struct TargetMarks {
    viewed: Vec2,
    tile_origin: Vec2,
}

struct Progress {
    cover: f32,
    uncover: f32,
    phase: ScreenTransitionPhase,
}

impl Cut {
    fn progress(&self) -> Progress {
        let pace = self.style.pace();
        let uncover_from = self
            .arrival
            .as_ref()
            .map(|arrival| Seconds(arrival.at.0.max(pace.cover.0)));
        let cover = fraction(self.played, pace.cover);
        let uncover = uncover_from.map_or(0.0, |from| fraction(self.played - from, pace.uncover));
        let phase = if cover < 1.0 {
            ScreenTransitionPhase::Out
        } else if uncover_from.is_none_or(|from| self.played < from) {
            ScreenTransitionPhase::Hold
        } else if uncover < 1.0 {
            ScreenTransitionPhase::In
        } else {
            ScreenTransitionPhase::Idle
        };
        Progress {
            cover,
            uncover,
            phase,
        }
    }
}

fn fraction(elapsed: Seconds, span: Seconds) -> f32 {
    (elapsed.0 / span.0).clamp(0.0, 1.0)
}

fn tile_texels() -> f32 {
    TILE.0 * SCALE
}

fn world_camera_marks(world: &mut World, viewed_at: Pos<Meters>) -> Option<TargetMarks> {
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    target_marks(camera, transform, viewed_at)
}

fn target_marks(
    camera: &Camera,
    transform: &GlobalTransform,
    viewed_at: Pos<Meters>,
) -> Option<TargetMarks> {
    let project = |at: Pos<Meters>| {
        camera
            .world_to_viewport(transform, at.to_screen().extend(0.0))
            .ok()
    };
    Some(TargetMarks {
        viewed: project(viewed_at)?,
        tile_origin: project(CellPos::new(0, 0).bounds().min())?,
    })
}

fn corners(size: Vec2) -> [Vec2; 4] {
    [
        Vec2::ZERO,
        Vec2::new(size.x, 0.0),
        Vec2::new(0.0, size.y),
        size,
    ]
}

fn radius(marks: TargetMarks, size: Vec2) -> f32 {
    corners(size)
        .into_iter()
        .map(|corner| corner.distance(marks.viewed))
        .fold(0.0, f32::max)
        + SCALE
}

fn reach(marks: TargetMarks, size: Vec2) -> f32 {
    let cell = |at: Vec2| ((at - marks.tile_origin) / tile_texels()).floor();
    let center = cell(marks.viewed);
    corners(size - Vec2::ONE)
        .into_iter()
        .map(|corner| {
            let apart = (cell(corner) - center).abs();
            apart.x + apart.y
        })
        .fold(1.0, f32::max)
}

fn screen_heading(heading: Radians) -> Vec2 {
    let heading = heading.heading();
    Vec2::new(heading.x, heading.y)
}
