//! The editor's view of an area: a camera orbiting a point of the ground, and the projection that
//! puts ground points on the viewport and the pointer back on the ground, as the game's own camera
//! projects.

use std::sync::Arc;

use bevy::prelude::{Ray3d, Transform, Vec2, Vec3};
use engine::core::geometry::{Degrees, Radians};
use engine::core::math::{Pos, Rect};
use engine::core::render::camera::{FOV, PITCH, View, ground_hit, world_point};
use engine::core::units::{MeterPos, Meters, TileSize};
use engine::systems::area::Area;
use serde::{Deserialize, Serialize};
use ui::egui::{PointerButton, Pos2, Rect as ERect, Response, Ui};

use crate::keymap::EditorKey;

/// How close the camera may come to its target, and how far it may draw back.
pub const NEAREST: Meters = Meters(3.0);
pub const FARTHEST: Meters = Meters(400.0);
/// How far down the camera looks when it looks straight down.
pub const TOP_DOWN: Degrees = Degrees(89.0);
const LOWEST_PITCH: Degrees = Degrees(5.0);
/// Degrees of turn and tilt a pixel of drag gives.
const ORBIT_PER_PIXEL: f32 = 0.4;
/// How much of the frame an area fills when the camera frames it.
const FRAME_MARGIN: f32 = 1.15;
/// How far apart the points of a ground line are, so it bends with the terrain.
const LINE_STEP: f32 = 0.5;

/// A camera looking at a point of the ground from a yaw, a pitch and a distance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrbitCamera {
    pub target: Pos<Meters>,
    pub yaw: Radians,
    pub pitch: Radians,
    pub distance: Meters,
}

impl Default for OrbitCamera {
    fn default() -> OrbitCamera {
        OrbitCamera {
            target: Pos::new(0.0, 0.0),
            yaw: Radians(0.0),
            pitch: PITCH.radians(),
            distance: Meters(30.0),
        }
    }
}

impl OrbitCamera {
    /// A camera at the game's pitch with `bounds` in frame.
    pub fn framing(bounds: Rect<Meters>) -> OrbitCamera {
        let mut camera = OrbitCamera::default();
        camera.frame(bounds);
        camera
    }

    /// Looks at the middle of `bounds` from far enough to see all of it.
    pub fn frame(&mut self, bounds: Rect<Meters>) {
        let center = bounds.center();
        self.target = Pos::new(center.x, center.y);
        let span = bounds.size.width.max(bounds.size.height).max(1.0);
        let half_fov = FOV.radians().0 / 2.0;
        self.distance =
            Meters((span * FRAME_MARGIN / 2.0 / half_fov.tan()).clamp(NEAREST.0, FARTHEST.0));
    }

    /// Looks straight down from north.
    pub fn top_down(&mut self) {
        self.yaw = Radians(0.0);
        self.pitch = TOP_DOWN.radians();
    }

    /// Turns by `delta.x` and tilts by `delta.y` pixels of drag.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw = Radians(self.yaw.0 - delta.x.to_radians() * ORBIT_PER_PIXEL);
        self.pitch = Radians(
            (self.pitch.0 + delta.y.to_radians() * ORBIT_PER_PIXEL)
                .clamp(LOWEST_PITCH.radians().0, TOP_DOWN.radians().0),
        );
    }

    /// Slides the target along the ground by `delta` metres across and away from the camera.
    pub fn pan(&mut self, delta: Vec2) {
        let (sin, cos) = self.yaw.0.sin_cos();
        let right = Vec2::new(cos, -sin);
        let forward = Vec2::new(-sin, -cos);
        let moved = right * delta.x + forward * delta.y;
        self.target = Pos::new(self.target.x + moved.x, self.target.y + moved.y);
    }

    /// Draws back by `factor` (closer under one).
    pub fn zoom(&mut self, factor: f32) {
        self.distance = Meters((self.distance.0 * factor).clamp(NEAREST.0, FARTHEST.0));
    }

    /// Metres of ground a pixel of the viewport spans at the target, over `height` pixels.
    pub fn metres_per_pixel(&self, height: f32) -> f32 {
        2.0 * self.distance.0 * (FOV.radians().0 / 2.0).tan() / height.max(1.0)
    }

    /// Where the camera stands and looks, its target `ground` high.
    pub fn transform(&self, ground: Meters) -> Transform {
        let target = world_point(self.target, ground);
        let (sin, cos) = self.pitch.0.sin_cos();
        let back = Vec3::new(0.0, self.distance.0 * sin, self.distance.0 * cos);
        let eye = target + bevy::prelude::Quat::from_rotation_y(self.yaw.0) * back;
        Transform::from_translation(eye).looking_at(target, Vec3::Y)
    }

    pub fn view(&self, ground: Meters, viewport: Vec2) -> View {
        View::new(self.transform(ground), viewport)
    }

    /// Whether two cameras look alike enough that neither needs remembering over the other.
    pub fn near(&self, other: &OrbitCamera) -> bool {
        self.target.distance(other.target).0 < 0.01
            && (self.yaw.0 - other.yaw.0).abs() < 0.001
            && (self.pitch.0 - other.pitch.0).abs() < 0.001
            && (self.distance.0 - other.distance.0).abs() < 0.01
    }
}

/// Moves `camera` as the pointer asks of `response`: the right button orbits, the middle button or
/// Space with the left drags the ground along, the wheel draws back and in. Answers whether the
/// pointer is busy with the camera.
pub fn navigate(ui: &Ui, response: &Response, camera: &mut OrbitCamera, height: f32) -> bool {
    if response.dragged_by(PointerButton::Secondary) {
        let delta = response.drag_delta();
        camera.orbit(Vec2::new(delta.x, delta.y));
        return true;
    }
    let space = ui.input(|i| EditorKey::Pan.held(i));
    let panning = response.dragged_by(PointerButton::Middle)
        || (space && response.dragged_by(PointerButton::Primary));
    if panning {
        let delta = response.drag_delta();
        let per_pixel = camera.metres_per_pixel(height);
        camera.pan(Vec2::new(-delta.x, delta.y) * per_pixel);
        return true;
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        let pinch = ui.input(|i| i.zoom_delta());
        if pinch != 1.0 {
            camera.zoom(1.0 / pinch);
        } else if scroll.y != 0.0 {
            camera.zoom((-scroll.y / 240.0).exp2());
        }
    }
    false
}

/// The viewport an area is drawn in: the camera's projection over the screen rectangle it fills,
/// and the ground it looks at.
#[derive(Clone)]
pub struct AreaCanvas {
    pub screen: ERect,
    pub camera: OrbitCamera,
    view: View,
    area: Arc<Area>,
}

impl AreaCanvas {
    pub fn new(screen: ERect, camera: OrbitCamera, area: Arc<Area>) -> AreaCanvas {
        let ground = area.heights.height_at(camera.target);
        let view = camera.view(ground, Vec2::new(screen.width(), screen.height()));
        AreaCanvas {
            screen,
            camera,
            view,
            area,
        }
    }

    pub fn area(&self) -> &Arc<Area> {
        &self.area
    }

    /// Where a point of the ground shows on the screen.
    pub fn to_screen(&self, at: Pos<Meters>) -> Pos2 {
        self.lifted(at, Meters(0.0))
    }

    /// Where a point `height` above the ground shows on the screen; a point behind the camera is
    /// put far off it.
    pub fn lifted(&self, at: Pos<Meters>, height: Meters) -> Pos2 {
        let ground = self.area.heights.height_at(at);
        match self.view.project(world_point(at, ground + height)) {
            Some(window) => self.screen.min + ui::egui::Vec2::new(window.x, window.y),
            None => Pos2::new(-1.0e5, -1.0e5),
        }
    }

    /// The ray through a screen point.
    pub fn ray(&self, screen: Pos2) -> Ray3d {
        let window = screen - self.screen.min;
        self.view.ray(Vec2::new(window.x, window.y))
    }

    /// The ground under a screen point, when it points at the area.
    pub fn to_tile(&self, screen: Pos2) -> Option<Pos<Meters>> {
        ground_hit(
            &self.area.heights,
            self.area.size.bounds().inflate(0.5, 0.5),
            self.ray(screen),
        )
    }

    /// Screen pixels a metre of ground takes at the camera's target.
    pub fn scale(&self) -> f32 {
        1.0 / self.camera.metres_per_pixel(self.screen.height())
    }

    /// The ground the canvas may show anything of.
    pub fn visible(&self) -> Rect<Meters> {
        self.area.size.bounds().inflate(4.0, 4.0)
    }

    /// The box on the screen around a rectangle of ground.
    pub fn rect(&self, rect: Rect<Meters>) -> ERect {
        let corners = engine::core::geometry::corners(rect);
        ERect::from_points(&corners.map(|corner| self.to_screen(corner)))
    }

    /// A rectangle of ground as a line going round it over the terrain.
    pub fn outline(&self, rect: Rect<Meters>) -> Vec<Pos2> {
        let corners = engine::core::geometry::corners(rect);
        self.closed(&corners)
    }

    /// A line through `points` on the ground, bent over the terrain, closed back to its start.
    pub fn closed(&self, points: &[Pos<Meters>]) -> Vec<Pos2> {
        let mut round: Vec<Pos<Meters>> = points.to_vec();
        if let Some(first) = points.first() {
            round.push(*first);
        }
        self.polyline(&round)
    }

    /// A line through `points` on the ground, bent over the terrain.
    pub fn polyline(&self, points: &[Pos<Meters>]) -> Vec<Pos2> {
        let mut out = Vec::new();
        for pair in points.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let length = from.distance(to).0;
            let steps = (length / LINE_STEP).ceil().max(1.0) as usize;
            for step in 0..steps {
                let t = step as f32 / steps as f32;
                out.push(self.to_screen(from.lerp(to, t)));
            }
        }
        if let Some(last) = points.last() {
            out.push(self.to_screen(*last));
        }
        out
    }

    /// A circle of `radius` round `center` on the ground, bent over the terrain.
    pub fn circle(&self, center: Pos<Meters>, radius: f32) -> Vec<Pos2> {
        let segments = ((radius * 8.0).ceil() as usize).clamp(16, 96);
        (0..segments)
            .map(|step| {
                let angle = std::f32::consts::TAU * step as f32 / segments as f32;
                self.to_screen(Pos::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                ))
            })
            .collect()
    }

    /// An ellipse filling `rect` on the ground, bent over the terrain.
    pub fn ellipse(&self, rect: Rect<Meters>) -> Vec<Pos2> {
        let center = rect.center();
        let (rx, ry) = (rect.size.width / 2.0, rect.size.height / 2.0);
        let segments = ((rx.max(ry) * 8.0).ceil() as usize).clamp(16, 96);
        (0..segments)
            .map(|step| {
                let angle = std::f32::consts::TAU * step as f32 / segments as f32;
                self.to_screen(Pos::new(
                    center.x + rx * angle.cos(),
                    center.y + ry * angle.sin(),
                ))
            })
            .collect()
    }

    /// Whether the ray through `screen` passes through the box standing on `bounds`, `height`
    /// high over the ground, and how far along the ray it enters.
    pub fn hits_box(&self, screen: Pos2, bounds: Rect<Meters>, height: f32) -> Option<f32> {
        let ray = self.ray(screen);
        let floor = engine::core::geometry::corners(bounds)
            .iter()
            .map(|corner| self.area.heights.height_at(*corner).0)
            .fold(f32::INFINITY, f32::min);
        let ceiling = engine::core::geometry::corners(bounds)
            .iter()
            .map(|corner| self.area.heights.height_at(*corner).0)
            .fold(f32::NEG_INFINITY, f32::max)
            + height;
        let min = Vec3::new(bounds.min().x, floor, bounds.min().y);
        let max = Vec3::new(bounds.max().x, ceiling, bounds.max().y);
        ray_box(ray, min, max)
    }
}

/// How far along `ray` it enters the box from `min` to `max`, if it does.
fn ray_box(ray: Ray3d, min: Vec3, max: Vec3) -> Option<f32> {
    let origin = ray.origin;
    let direction = *ray.direction;
    let (mut enter, mut exit) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        if direction[axis].abs() < 1e-6 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let t1 = (min[axis] - origin[axis]) / direction[axis];
        let t2 = (max[axis] - origin[axis]) / direction[axis];
        enter = enter.max(t1.min(t2));
        exit = exit.min(t1.max(t2));
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}
