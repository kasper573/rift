//! What comes between the camera and whom it follows clears a circle around them over a fifth of
//! a second, dithered and soft at its rim, and closes it again when the way is clear; what the camera stands
//! inside of clears away whole, since it would otherwise fill the picture. A faded thing keeps the
//! rest of itself and its whole shadow: it still stands there, only seen through.

use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::camera::CameraRig;
use super::material::{FadeStyle, WorldMaterial};
use crate::core::render::Look;
use crate::core::units::Meters;
use crate::core::units::Seconds;

/// How what hides the camera's subject clears away.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClearingLook {
    /// How long a fade takes, out or back.
    pub fades: Seconds,
    /// How far round the subject a fading thing is cleared, on the ground it stands on.
    pub radius: Meters,
}

/// A thing that fades when it hides the camera's subject. Whole, it shares its materials with
/// everything that looks alike; while it fades its meshes wear materials of their own, so its
/// fade moves on its own, and `shared` keeps the ones they wear again once it is whole.
#[derive(Component)]
pub struct Fades {
    shown: f32,
    shared: Vec<(Entity, Handle<WorldMaterial>)>,
}

impl Default for Fades {
    fn default() -> Fades {
        Fades {
            shown: 1.0,
            shared: Vec::new(),
        }
    }
}

/// The things that fade, with the meshes and materials they are drawn from.
#[derive(SystemParam)]
pub(super) struct Fading<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    bounds: Query<'w, 's, (&'static Aabb, &'static GlobalTransform)>,
    roots: Query<'w, 's, (Entity, &'static mut Fades)>,
    materials_of: Query<'w, 's, &'static mut MeshMaterial3d<WorldMaterial>>,
    materials: ResMut<'w, Assets<WorldMaterial>>,
}

pub(super) fn fade_occluders(
    time: Res<Time>,
    rig: Res<CameraRig>,
    look: Res<Look>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut fading: Fading,
) {
    let Fading {
        children,
        bounds,
        roots,
        materials_of,
        materials,
    } = &mut fading;
    let Ok(window) = window.single() else {
        return;
    };
    let eye = rig.eye();
    let target = rig.target();
    let view = rig.view(Vec2::new(
        window.resolution.width(),
        window.resolution.height(),
    ));
    let Some((focus, focus_radius)) = view.project(target).and_then(|focus| {
        let rim = view.project(target + rig.transform().right() * look.clearing.radius.0)?;
        Some((focus, focus.distance(rim)))
    }) else {
        return;
    };
    let step = time.delta_secs() / look.clearing.fades.0.max(f32::EPSILON);
    for (root, mut fades) in roots.iter_mut() {
        let (mut crossed, mut around_eye) = (false, false);
        for (aabb, transform) in children
            .iter_descendants(root)
            .chain(std::iter::once(root))
            .filter_map(|entity| bounds.get(entity).ok())
        {
            crossed |= segment_crosses(eye, target, aabb, transform);
            around_eye |= contains(eye, aabb, transform);
        }
        let wanted = if crossed { 0.0 } else { 1.0 };
        if fades.shown == 1.0 && wanted == 1.0 {
            continue;
        }
        fades.shown = if (wanted - fades.shown).abs() <= step {
            wanted
        } else {
            fades.shown + step.copysign(wanted - fades.shown)
        };
        if fades.shown == 1.0 {
            for (mesh, shared) in std::mem::take(&mut fades.shared) {
                if let Ok(mut worn) = materials_of.get_mut(mesh) {
                    worn.0 = shared;
                }
            }
            continue;
        }
        if fades.shared.is_empty() {
            for mesh in children.iter_descendants(root) {
                let Ok(mut worn) = materials_of.get_mut(mesh) else {
                    continue;
                };
                let Some(own) = materials.get(&worn.0).cloned() else {
                    continue;
                };
                let shared = std::mem::replace(&mut worn.0, materials.add(own));
                fades.shared.push((mesh, shared));
            }
        }
        let style = FadeStyle {
            shown: fades.shown,
            focus,
            focus_radius: if around_eye { f32::MAX } else { focus_radius },
        };
        for (mesh, _) in &fades.shared {
            let Ok(worn) = materials_of.get(*mesh) else {
                continue;
            };
            if materials
                .get(&worn.0)
                .is_some_and(|material| material.extension.style != style)
                && let Some(mut material) = materials.get_mut(&worn.0)
            {
                material.extension.style = style;
            }
        }
    }
}

/// Whether `point` lies in the box, tested in the box's own frame.
fn contains(point: Vec3, aabb: &Aabb, transform: &GlobalTransform) -> bool {
    let local = transform.affine().inverse().transform_point3(point);
    let (min, max) = (Vec3::from(aabb.min()), Vec3::from(aabb.max()));
    local.cmpge(min).all() && local.cmple(max).all()
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
