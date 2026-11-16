//! A line drawn round a thing in the world, to set it apart from what it stands among.

use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy_mod_outline::{OutlinePlugin, OutlineStencil, OutlineVolume, PropagateOutline};

use crate::core::units::WindowPx;

/// A line of `color`, `width` wide on the screen, round an entity and all it is made of.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub color: Color,
    pub width: WindowPx,
}

/// Left out of the line round the thing it hangs on, with all it is made of: what plays on a
/// thing is no part of its shape. It is a source of lines of its own that draws none, which is
/// where the line of what it hangs on stops.
#[derive(Component, Clone, Copy, Debug, Default)]
#[require(PropagateOutline)]
pub struct Unoutlined;

pub struct OutlineScenePlugin;

impl Plugin for OutlineScenePlugin {
    fn build(&self, app: &mut App) {
        // The outline is drawn by the renderer, which an app that only reads the game has none of.
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        app.add_plugins(OutlinePlugin::JUMP_FLOOD)
            .add_systems(PostUpdate, (draw_outlines, drop_outlines));
    }
}

fn draw_outlines(mut commands: Commands, outlined: Query<(Entity, &Outline), Changed<Outline>>) {
    for (entity, outline) in &outlined {
        commands.entity(entity).insert((
            OutlineVolume {
                visible: true,
                width: outline.width.0,
                colour: outline.color,
            },
            OutlineStencil::default(),
            PropagateOutline,
        ));
    }
}

fn drop_outlines(mut commands: Commands, mut gone: RemovedComponents<Outline>) {
    for entity in gone.read() {
        if let Ok(mut entity) = commands.get_entity(entity) {
            entity.remove::<(OutlineVolume, OutlineStencil, PropagateOutline)>();
        }
    }
}
