use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};

use super::TILE;
use super::camera::WorldCamera;
use super::present::{self, SCALE};
use super::screen::ToScreen;
use crate::core::math::{Direction, Pos};
use crate::core::tiling::Tiles;
use crate::core::time::Seconds;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenTransition {
    #[default]
    Crumble,
    TileWave,
    Sweep,
    Iris,
    Mosaic,
    Dither,
    Fade,
}

impl ScreenTransition {
    pub const ALL: [ScreenTransition; 7] = [
        ScreenTransition::Crumble,
        ScreenTransition::TileWave,
        ScreenTransition::Sweep,
        ScreenTransition::Iris,
        ScreenTransition::Mosaic,
        ScreenTransition::Dither,
        ScreenTransition::Fade,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScreenTransition::Crumble => "Crumble",
            ScreenTransition::TileWave => "Tile wave",
            ScreenTransition::Sweep => "Sweep",
            ScreenTransition::Iris => "Iris",
            ScreenTransition::Mosaic => "Mosaic",
            ScreenTransition::Dither => "Dither dissolve",
            ScreenTransition::Fade => "Fade",
        }
    }

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

#[derive(Resource, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTransitionPhase {
    #[default]
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

pub fn covering(phase: Res<ScreenTransitionPhase>) -> bool {
    *phase == ScreenTransitionPhase::Out
}

pub fn freeze(world: &mut World, viewed_at: Pos<Tiles>, heading: Direction) {
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
    world.insert_resource(ScreenTransitionPhase::Out);
}

pub fn reveal(world: &mut World, viewed_at: Pos<Tiles>) {
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
    world.insert_resource(ScreenTransitionPhase::Idle);
}

/// A long frame pauses the transition rather than skipping part of it.
const LONGEST_STEP: Seconds = Seconds(1.0 / 30.0);

#[derive(Resource, Default)]
pub(super) struct ActiveTransition(Option<Cut>);

impl ActiveTransition {
    pub(super) fn advance(
        &mut self,
        frame: Seconds,
        camera: Option<(&Camera, &GlobalTransform)>,
    ) -> (ScreenTransitionPhase, CutUniform) {
        let Some(cut) = &mut self.0 else {
            return (ScreenTransitionPhase::Idle, CutUniform::IDLE);
        };
        cut.played += frame.min(LONGEST_STEP);
        let progress = cut.progress();
        if progress.phase == ScreenTransitionPhase::Idle {
            self.0 = None;
            return (ScreenTransitionPhase::Idle, CutUniform::IDLE);
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
        let uniform = CutUniform {
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
        };
        (progress.phase, uniform)
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
    viewed_at: Pos<Tiles>,
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
        let uncover_from = self.arrival.as_ref().map(|arrival| {
            if arrival.at > pace.cover {
                arrival.at
            } else {
                pace.cover
            }
        });
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

fn world_camera_marks(world: &mut World, viewed_at: Pos<Tiles>) -> Option<TargetMarks> {
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    target_marks(camera, transform, viewed_at)
}

fn target_marks(
    camera: &Camera,
    transform: &GlobalTransform,
    viewed_at: Pos<Tiles>,
) -> Option<TargetMarks> {
    let project = |at: Pos<Tiles>| {
        camera
            .world_to_viewport(transform, at.to_screen().extend(0.0))
            .ok()
    };
    Some(TargetMarks {
        viewed: project(viewed_at)?,
        tile_origin: project(Pos::origin())?,
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

fn screen_heading(heading: Direction) -> Vec2 {
    let (x, y) = match heading {
        Direction::E => (1.0, 0.0),
        Direction::SE => (1.0, 1.0),
        Direction::S => (0.0, 1.0),
        Direction::SW => (-1.0, 1.0),
        Direction::W => (-1.0, 0.0),
        Direction::NW => (-1.0, -1.0),
        Direction::N => (0.0, -1.0),
        Direction::NE => (1.0, -1.0),
    };
    Vec2::new(x, y).normalize()
}
