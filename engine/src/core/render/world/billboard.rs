//! Quads that always face the camera, for sprites and effects drawn flat in a world of depth.

use bevy::prelude::*;

use super::camera::CameraRig;

#[derive(Component, Default)]
pub struct Billboard;

pub(super) fn face_camera(rig: Res<CameraRig>, mut boards: Query<&mut Transform, With<Billboard>>) {
    let facing = rig.transform().rotation;
    for mut transform in &mut boards {
        if transform.rotation != facing {
            transform.rotation = facing;
        }
    }
}
