//! What comes between the camera and whom it follows dithers out of the way over a fifth of a
//! second, and back when the way is clear. A faded thing keeps its shadow.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use super::camera::CameraRig;
use super::toon::ToonMaterial;
use crate::core::time::Seconds;

/// How long a fade takes, out or back.
pub const FADE: Seconds = Seconds(0.2);

/// A thing that fades when it hides the camera's subject.
#[derive(Component, Default)]
pub struct Fades {
    shown: f32,
}

pub(super) fn fade_occluders(
    time: Res<Time>,
    rig: Res<CameraRig>,
    children: Query<&Children>,
    bounds: Query<(&Aabb, &GlobalTransform)>,
    mut fading: Query<(Entity, &mut Fades)>,
    materials_of: Query<&MeshMaterial3d<ToonMaterial>>,
    mut materials: ResMut<Assets<ToonMaterial>>,
) {
    let eye = rig.eye();
    let target = rig.target();
    let step = time.delta_secs() / FADE.0;
    for (root, mut fades) in &mut fading {
        let crossed = children
            .iter_descendants(root)
            .chain(std::iter::once(root))
            .filter_map(|entity| bounds.get(entity).ok())
            .any(|(aabb, transform)| segment_crosses(eye, target, aabb, transform));
        let wanted = if crossed { 0.0 } else { 1.0 };
        if fades.shown == wanted {
            continue;
        }
        fades.shown = if (wanted - fades.shown).abs() <= step {
            wanted
        } else {
            fades.shown + step.copysign(wanted - fades.shown)
        };
        for entity in children.iter_descendants(root) {
            if let Ok(material) = materials_of.get(entity)
                && let Some(mut material) = materials.get_mut(&material.0)
            {
                material.extension.style.shown = fades.shown;
            }
        }
    }
}

/// Whether the segment from `from` to `to` passes through the box, which is tested in the box's
/// own frame.
fn segment_crosses(from: Vec3, to: Vec3, aabb: &Aabb, transform: &GlobalTransform) -> bool {
    let world_from_local = transform.affine();
    let local_from_world = world_from_local.inverse();
    let a = local_from_world.transform_point3(from);
    let b = local_from_world.transform_point3(to);
    let min = Vec3::from(aabb.min());
    let max = Vec3::from(aabb.max());
    let direction = b - a;
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        if direction[axis].abs() < 1e-6 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let t1 = (min[axis] - a[axis]) / direction[axis];
        let t2 = (max[axis] - a[axis]) / direction[axis];
        enter = enter.max(t1.min(t2));
        exit = exit.min(t1.max(t2));
        if enter > exit {
            return false;
        }
    }
    true
}
