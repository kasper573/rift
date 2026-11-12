//! Things that hang plumb from what carries them, however it turns: a lantern from the hand that
//! holds its bail.

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;

/// Hangs its entity plumb from its parent: laid in the parent's frame as `rigid` has it, then
/// turned about `pivot`, the point of that frame it hangs by, the least it takes for its +Y to
/// point up.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Plumb {
    pub rigid: Transform,
    pub pivot: Vec3,
}

impl Plumb {
    /// Where the entity lies in the frame of a parent the world sees turned by `carried`.
    pub fn hung(&self, carried: Quat) -> Transform {
        let leaning = carried * self.rigid.rotation * Vec3::Y;
        let righting = carried.inverse() * Quat::from_rotation_arc(leaning, Vec3::Y) * carried;
        Transform {
            translation: self.pivot + righting * (self.rigid.translation - self.pivot),
            rotation: righting * self.rigid.rotation,
            scale: self.rigid.scale,
        }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn hang(
    mut hung: ParamSet<(
        (Query<(Entity, &Plumb, &ChildOf)>, TransformHelper),
        Query<&mut Transform>,
    )>,
) {
    let (plumbs, carriers) = hung.p0();
    let placed: Vec<(Entity, Transform)> = plumbs
        .iter()
        .filter_map(|(entity, plumb, carrier)| {
            let carried = carriers.compute_global_transform(carrier.parent()).ok()?;
            Some((entity, plumb.hung(carried.rotation())))
        })
        .collect();
    let mut transforms = hung.p1();
    for (entity, hangs) in placed {
        if let Ok(mut transform) = transforms.get_mut(entity) {
            transform.set_if_neq(hangs);
        }
    }
}
