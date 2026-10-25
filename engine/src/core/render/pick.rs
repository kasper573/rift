//! What the pointer is over: the ray through it and the ground it meets, and the hit tests the
//! systems pick actors and things out of the world with.

use bevy::prelude::*;

use super::camera::{self, GroundHeights};
use crate::core::math::Pos;
use crate::core::units::Meters;

/// The pointer as it points into the world: its ray and, where that meets the ground, the point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointed {
    pub ray: Ray3d,
    pub ground: Option<Pos<Meters>>,
}

/// Where the pointer points, while it is over the world rather than the interface.
pub fn pointed(world: &mut World) -> Option<Pointed> {
    let ray = camera::cursor_ray(world)?;
    let ground = world.resource::<GroundHeights>().hit(ray);
    Some(Pointed { ray, ground })
}

/// How far along the ray a capsule `radius` wide and `height` tall standing on `foot` is hit, if
/// it is.
pub fn capsule_hit(ray: Ray3d, foot: Vec3, radius: f32, height: f32) -> Option<f32> {
    let radius = radius.max(0.05);
    let low = foot + Vec3::Y * radius.min(height / 2.0);
    let high = foot + Vec3::Y * (height - radius).max(height / 2.0);
    let (along, apart) = ray_segment(ray, low, high);
    (apart <= radius).then_some(along)
}

/// How far along the ray a point `reach` metres about `center` is hit, if it is.
pub fn sphere_hit(ray: Ray3d, center: Vec3, reach: f32) -> Option<f32> {
    let along = (center - ray.origin).dot(*ray.direction).max(0.0);
    let nearest = ray.origin + *ray.direction * along;
    (nearest.distance(center) <= reach).then_some(along)
}

/// How far along the ray its nearest point to the segment lies, and how far apart the two are
/// there.
fn ray_segment(ray: Ray3d, a: Vec3, b: Vec3) -> (f32, f32) {
    let d1 = *ray.direction;
    let d2 = b - a;
    let r = ray.origin - a;
    let e = d2.dot(d2);
    let c = d1.dot(r);
    if e < 1e-6 {
        let along = (-c).max(0.0);
        return (along, (ray.origin + d1 * along).distance(a));
    }
    let f = d2.dot(r);
    let bb = d1.dot(d2);
    let denom = e - bb * bb;
    let mut along = if denom.abs() > 1e-6 {
        ((bb * f - c * e) / denom).max(0.0)
    } else {
        0.0
    };
    let mut u = (bb * along + f) / e;
    if u < 0.0 {
        u = 0.0;
        along = (-c).max(0.0);
    } else if u > 1.0 {
        u = 1.0;
        along = (bb - c).max(0.0);
    }
    let on_ray = ray.origin + d1 * along;
    let on_segment = a + d2 * u;
    (along, on_ray.distance(on_segment))
}
