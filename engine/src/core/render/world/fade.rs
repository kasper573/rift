//! What comes between the camera and whom it follows clears a circle around them over a fifth of
//! a second, dithered and soft at its rim, and closes it again when the way is clear; what the camera stands
//! inside of clears away whole, since it would otherwise fill the picture. A faded thing keeps the
//! rest of itself and its whole shadow: it still stands there, only seen through.

use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::math::Affine3A;
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

/// The boxes a thing that fades is drawn in, kept as tested: each in its own frame with the way
/// into it from the world's, and all of them round in the world's, so the way from the camera to
/// its subject is tested against a thing only where it passes near.
#[derive(Component)]
pub(super) struct FadeBounds {
    around: (Vec3, Vec3),
    boxes: Vec<FrameBox>,
}

struct FrameBox {
    world_from_local: Affine3A,
    local_from_world: Affine3A,
    min: Vec3,
    max: Vec3,
}

/// The things that fade, with the meshes and materials they are drawn from.
#[derive(SystemParam)]
pub(super) struct Fading<'w, 's> {
    commands: Commands<'w, 's>,
    children: Query<'w, 's, &'static Children>,
    bounds: Query<'w, 's, (&'static Aabb, &'static GlobalTransform)>,
    roots: Query<'w, 's, (Entity, &'static mut Fades, Option<&'static FadeBounds>)>,
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
        commands,
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
    for (root, mut fades, kept) in roots.iter_mut() {
        let made = kept
            .is_none_or(|kept| !kept.stands(root, children, bounds))
            .then(|| bounds_of(root, children, bounds));
        let Some(laid) = made.as_ref().or(kept) else {
            continue;
        };
        let (crossed, around_eye) = laid.seen(eye, target);
        if let Some(made) = made {
            commands.entity(root).insert(made);
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

impl FadeBounds {
    /// Whether `root` and everything under it stand as they did when these were made.
    fn stands(
        &self,
        root: Entity,
        children: &Query<&Children>,
        bounds: &Query<(&Aabb, &GlobalTransform)>,
    ) -> bool {
        let mut kept = self.boxes.iter();
        children
            .iter_descendants(root)
            .chain(std::iter::once(root))
            .filter_map(|entity| bounds.get(entity).ok())
            .all(|(aabb, transform)| {
                kept.next().is_some_and(|framed| {
                    framed.world_from_local == transform.affine()
                        && (framed.min, framed.max)
                            == (Vec3::from(aabb.min()), Vec3::from(aabb.max()))
                })
            })
            && kept.next().is_none()
    }

    /// Whether the way from `eye` to `target` passes through any of the boxes, and whether `eye`
    /// stands inside one.
    fn seen(&self, eye: Vec3, target: Vec3) -> (bool, bool) {
        let (low, high) = self.around;
        let near = segment_crosses(eye, target, low, high)
            || (eye.cmpge(low).all() && eye.cmple(high).all());
        if !near {
            return (false, false);
        }
        self.boxes
            .iter()
            .fold((false, false), |(crossed, inside), framed| {
                let (from, to) = (
                    framed.local_from_world.transform_point3(eye),
                    framed.local_from_world.transform_point3(target),
                );
                (
                    crossed || segment_crosses(from, to, framed.min, framed.max),
                    inside || (from.cmpge(framed.min).all() && from.cmple(framed.max).all()),
                )
            })
    }
}

/// The boxes `root` and everything under it are drawn in, as they stand now.
fn bounds_of(
    root: Entity,
    children: &Query<&Children>,
    bounds: &Query<(&Aabb, &GlobalTransform)>,
) -> FadeBounds {
    let mut around = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let boxes = children
        .iter_descendants(root)
        .chain(std::iter::once(root))
        .filter_map(|entity| bounds.get(entity).ok())
        .map(|(aabb, transform)| {
            let world_from_local = transform.affine();
            let (min, max) = (Vec3::from(aabb.min()), Vec3::from(aabb.max()));
            let center = world_from_local.transform_point3((min + max) / 2.0);
            let half = world_from_local.matrix3.abs() * Vec3A::from((max - min) / 2.0);
            around.0 = around.0.min(center - Vec3::from(half));
            around.1 = around.1.max(center + Vec3::from(half));
            FrameBox {
                world_from_local,
                local_from_world: world_from_local.inverse(),
                min,
                max,
            }
        })
        .collect();
    FadeBounds { around, boxes }
}

/// Whether the segment from `a` to `b` passes through the box from `min` to `max`.
fn segment_crosses(a: Vec3, b: Vec3, min: Vec3, max: Vec3) -> bool {
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
