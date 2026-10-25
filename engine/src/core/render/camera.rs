//! The camera: a perspective view pitched down on a point of the ground, turned to any yaw, tilted
//! between looking down and looking out toward the horizon, set to one of three distances, and the
//! ray it casts through the pointer. The view is a pure
//! projection of the rig, so the probe, the plates and the picking all project the way the camera
//! draws.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde::{Deserialize, Serialize};

use crate::core::geometry::{Degrees, Radians};
use crate::core::ground::{HIGHEST, Heightfield, LOWEST};
use crate::core::gui;
use crate::core::math::{Pos, Rect};
use crate::core::time::Seconds;
use crate::core::units::Meters;

pub const FOV: Degrees = Degrees(32.0);
/// How far down the camera looks when brought home.
pub const PITCH: Degrees = Degrees(55.0);
/// How far one press of a turn key turns the camera.
pub const TURN_STEP: Degrees = Degrees(45.0);
/// How far one press of a tilt key tilts the camera, between looking out over the ground and
/// nearly straight down.
pub const TILT_STEP: Degrees = Degrees(10.0);
pub const LOWEST_PITCH: Degrees = Degrees(5.0);
pub const HIGHEST_PITCH: Degrees = Degrees(75.0);
const NEAR: f32 = 0.5;
/// How far a ray is followed through the world before it is given up as hitting nothing.
const REACH: f32 = 200.0;
const MARCH_STEP: f32 = 0.25;
const BISECTIONS: u32 = 12;
/// How long a zoom step takes to ease over.
const ZOOM_EASE: Seconds = Seconds(0.2);

#[derive(Component)]
pub struct WorldCamera;

/// The perspective every camera on the world draws with, over a viewport `width` by `height`.
pub fn projection(width: u32, height: u32) -> Projection {
    Projection::Perspective(PerspectiveProjection {
        fov: FOV.radians().0,
        near: NEAR,
        aspect_ratio: width as f32 / height.max(1) as f32,
        ..default()
    })
}

/// The three distances the camera is kept at, along its pitch line.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zoom {
    Close,
    #[default]
    Middle,
    Wide,
}

impl Zoom {
    /// How many steps the zoom has.
    pub const STEPS: u8 = 3;

    pub fn distance(self) -> Meters {
        match self {
            Zoom::Close => Meters(18.0),
            Zoom::Middle => Meters(28.0),
            Zoom::Wide => Meters(40.0),
        }
    }

    pub fn closer(self) -> Zoom {
        match self {
            Zoom::Close | Zoom::Middle => Zoom::Close,
            Zoom::Wide => Zoom::Middle,
        }
    }

    pub fn wider(self) -> Zoom {
        match self {
            Zoom::Close => Zoom::Middle,
            Zoom::Middle | Zoom::Wide => Zoom::Wide,
        }
    }

    /// The step as the probe reports it: 0 close, 1 middle, 2 wide.
    pub fn step(self) -> u8 {
        match self {
            Zoom::Close => 0,
            Zoom::Middle => 1,
            Zoom::Wide => 2,
        }
    }
}

/// Where the camera looks from: at a point of the world, from a yaw and a distance that ease toward
/// what the player last asked for. A client resource, never replicated.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraRig {
    target: Vec3,
    yaw: Radians,
    wanted_yaw: Radians,
    pitch: Radians,
    wanted_pitch: Radians,
    distance: Meters,
    zoom: Zoom,
}

impl Default for CameraRig {
    fn default() -> CameraRig {
        CameraRig {
            target: Vec3::ZERO,
            yaw: Radians(0.0),
            wanted_yaw: Radians(0.0),
            pitch: PITCH.radians(),
            wanted_pitch: PITCH.radians(),
            distance: Zoom::default().distance(),
            zoom: Zoom::default(),
        }
    }
}

/// How the player likes the camera to move.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraPreferences {
    /// Degrees a second the camera turns at.
    pub turn_speed: Degrees,
    /// Turns and zooms snap instead of easing, and nothing sways or fades over time.
    pub reduce_motion: bool,
}

impl Default for CameraPreferences {
    fn default() -> CameraPreferences {
        CameraPreferences {
            turn_speed: Degrees(180.0),
            reduce_motion: false,
        }
    }
}

impl CameraRig {
    /// Points the camera at `at`, `height` above the ground there.
    pub fn look_at(&mut self, at: Pos<Meters>, height: Meters) {
        self.target = world_point(at, height);
    }

    pub fn target(&self) -> Vec3 {
        self.target
    }

    pub fn turn(&mut self, by: Degrees) {
        self.wanted_yaw = Radians(self.wanted_yaw.0 + by.radians().0);
    }

    /// Turns the camera straight away, as a drag does.
    pub fn turn_now(&mut self, by: Degrees) {
        self.turn(by);
        self.yaw = self.wanted_yaw;
    }

    /// Tilts the camera `by` degrees further down, or up toward the horizon when negative, within
    /// its limits.
    pub fn tilt(&mut self, by: Degrees) {
        self.wanted_pitch = Radians(
            (self.wanted_pitch.0 + by.radians().0)
                .clamp(LOWEST_PITCH.radians().0, HIGHEST_PITCH.radians().0),
        );
    }

    /// Tilts the camera straight away.
    pub fn tilt_now(&mut self, by: Degrees) {
        self.tilt(by);
        self.pitch = self.wanted_pitch;
    }

    pub fn home(&mut self) {
        self.wanted_yaw = Radians(0.0);
        self.wanted_pitch = PITCH.radians();
        self.zoom = Zoom::default();
    }

    pub fn zoom_in(&mut self) {
        self.zoom = self.zoom.closer();
    }

    pub fn zoom_out(&mut self) {
        self.zoom = self.zoom.wider();
    }

    pub fn yaw(&self) -> Radians {
        self.yaw
    }

    pub fn pitch(&self) -> Radians {
        self.pitch
    }

    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// Whether the camera has come to rest where it was last asked to go.
    pub fn settled(&self) -> bool {
        self.yaw == self.wanted_yaw
            && self.pitch == self.wanted_pitch
            && self.distance == self.zoom.distance()
    }

    pub fn eye(&self) -> Vec3 {
        let (sin, cos) = self.pitch.0.sin_cos();
        let back = Vec3::new(0.0, self.distance.0 * sin, self.distance.0 * cos);
        self.target + Quat::from_rotation_y(self.yaw.0) * back
    }

    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.eye()).looking_at(self.target, Vec3::Y)
    }

    pub fn view(&self, viewport: Vec2) -> View {
        View::new(self.transform(), viewport)
    }

    fn ease(&mut self, dt: Seconds, preferences: &CameraPreferences) {
        let wanted = self.zoom.distance();
        if preferences.reduce_motion {
            self.yaw = self.wanted_yaw;
            self.pitch = self.wanted_pitch;
            self.distance = wanted;
            return;
        }
        let turn = preferences.turn_speed.radians().0 * dt.0;
        let toward = |from: Radians, to: Radians| {
            let apart = to.0 - from.0;
            if apart.abs() <= turn {
                to
            } else {
                Radians(from.0 + turn.copysign(apart))
            }
        };
        self.yaw = toward(self.yaw, self.wanted_yaw);
        self.pitch = toward(self.pitch, self.wanted_pitch);
        let caught = (dt.0 / ZOOM_EASE.0).min(1.0);
        let left = wanted.0 - self.distance.0;
        self.distance = if left.abs() < 0.01 {
            wanted
        } else {
            Meters(self.distance.0 + left * caught)
        };
    }
}

/// A camera's projection of the world onto a viewport, as the renderer and everything that places
/// points in the window share it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    world_from_view: GlobalTransform,
    clip_from_view: Mat4,
    viewport: Vec2,
}

impl View {
    pub fn new(camera: Transform, viewport: Vec2) -> View {
        let aspect = viewport.x / viewport.y.max(1.0);
        View {
            world_from_view: GlobalTransform::from(camera),
            clip_from_view: bevy::math::proj::perspective_infinite_reverse(
                FOV.radians().0,
                aspect,
                NEAR,
            ),
            viewport,
        }
    }

    pub fn viewport(&self) -> Vec2 {
        self.viewport
    }

    pub fn eye(&self) -> Vec3 {
        self.world_from_view.translation()
    }

    pub fn view_proj(&self) -> Mat4 {
        self.clip_from_view * self.world_from_view.to_matrix().inverse()
    }

    /// The ray from the eye through a point of the viewport, in window pixels from its top left.
    pub fn ray(&self, window: Vec2) -> Ray3d {
        let ndc = (window / self.viewport * 2.0 - Vec2::ONE) * Vec2::new(1.0, -1.0);
        let view_from_clip = self.clip_from_view.inverse();
        let near = view_from_clip.project_point3(ndc.extend(1.0));
        let far = view_from_clip.project_point3(ndc.extend(f32::EPSILON));
        let origin = self.world_from_view.transform_point(near);
        let direction = self.world_from_view.affine().transform_vector3(far - near);
        Ray3d::new(origin, Dir3::new(direction).unwrap_or(Dir3::NEG_Y))
    }

    /// Where a point of the world shows in the viewport, none behind the eye.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.view_proj() * world.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x + 1.0) / 2.0 * self.viewport.x,
            (1.0 - ndc.y) / 2.0 * self.viewport.y,
        ))
    }
}

/// The ground on view: the shown area's heights and how far it reaches, for everything that stands
/// on the ground or points at it.
#[derive(Resource, Clone, Default)]
pub struct GroundHeights {
    pub heights: Heightfield,
    pub bounds: Rect<Meters>,
}

impl GroundHeights {
    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        self.heights.height_at(at)
    }

    /// Where the ray meets the ground, if it does within the area.
    pub fn hit(&self, ray: Ray3d) -> Option<Pos<Meters>> {
        ground_hit(&self.heights, self.bounds, ray)
    }
}

/// The ground point a ray lands on: the ray is marched a quarter metre at a time from where it
/// enters the heights' range until it dips under the ground, then the crossing is bisected.
pub fn ground_hit(heights: &Heightfield, bounds: Rect<Meters>, ray: Ray3d) -> Option<Pos<Meters>> {
    let origin = ray.origin;
    let direction = *ray.direction;
    let under = |t: f32| {
        let point = origin + direction * t;
        let at = ground_pos(point);
        bounds.contains(at) && point.y <= heights.height_at(at).0
    };
    let mut t = if origin.y > HIGHEST.0 && direction.y < 0.0 {
        (HIGHEST.0 - origin.y) / direction.y
    } else {
        0.0
    };
    if under(t) {
        return Some(ground_pos(origin + direction * t));
    }
    let end = t + REACH;
    while t < end {
        let next = t + MARCH_STEP;
        let point = origin + direction * next;
        if point.y < LOWEST.0 && direction.y < 0.0 {
            return None;
        }
        if under(next) {
            let (mut above, mut below) = (t, next);
            for _ in 0..BISECTIONS {
                let mid = (above + below) / 2.0;
                if under(mid) {
                    below = mid;
                } else {
                    above = mid;
                }
            }
            return Some(ground_pos(origin + direction * below));
        }
        t = next;
    }
    None
}

/// A point of the ground `height` above it, in the world the camera sees: x east, y up, z south.
pub fn world_point(at: Pos<Meters>, height: Meters) -> Vec3 {
    Vec3::new(at.x, height.0, at.y)
}

pub fn ground_pos(point: Vec3) -> Pos<Meters> {
    Pos::new(point.x, point.z)
}

pub fn ground_height(world: &World, at: Pos<Meters>) -> Meters {
    world
        .get_resource::<GroundHeights>()
        .map_or(Meters(0.0), |ground| ground.height_at(at))
}

/// The camera's view through the primary window.
pub fn view_of(world: &mut World) -> Option<View> {
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?;
    let viewport = Vec2::new(window.resolution.width(), window.resolution.height());
    Some(world.resource::<CameraRig>().view(viewport))
}

/// The ray through the pointer, while it is over the world rather than the interface.
pub fn cursor_ray(world: &mut World) -> Option<Ray3d> {
    if gui::pointer_claimed(world) {
        return None;
    }
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?
        .cursor_position()?;
    Some(view_of(world)?.ray(cursor))
}

/// The ground under the pointer.
pub fn cursor_ground(world: &mut World) -> Option<Pos<Meters>> {
    let ray = cursor_ray(world)?;
    world.resource::<GroundHeights>().hit(ray)
}

/// Where a point `height` above the ground at `at` shows in the window.
pub fn window_point(world: &mut World, at: Pos<Meters>, height: Meters) -> Option<Vec2> {
    let ground = ground_height(world, at);
    view_of(world)?.project(world_point(at, ground + height))
}

pub(super) fn ease(
    time: Res<Time>,
    preferences: Res<CameraPreferences>,
    mut rig: ResMut<CameraRig>,
) {
    if !rig.settled() {
        rig.ease(Seconds(time.delta_secs()), &preferences);
    }
}

pub(super) fn place(rig: Res<CameraRig>, mut camera: Query<&mut Transform, With<WorldCamera>>) {
    if let Ok(mut transform) = camera.single_mut() {
        *transform = rig.transform();
    }
}
