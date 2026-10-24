use bevy::prelude::*;

use super::AreaObject;
use crate::core::content::Content;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{RowDepth, TILE, dynamic_z, place_sprite};
use crate::core::units::Meters;
use crate::systems::area::{self, AreaTag};
use crate::systems::movement::Position;
use crate::systems::screen::ClientScreen;

pub struct ObjectRenderPlugin;

impl Plugin for ObjectRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(attach_look).add_systems(
            Update,
            place_objects
                .run_if(in_state(ClientScreen::Area))
                .in_set(WorldViewSystems),
        );
    }
}

fn attach_look(
    add: On<Add<AreaObject>>,
    objects: Query<&AreaObject>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(object) = objects.get(add.entity) else {
        return;
    };
    let Some(look) = object.look.clone() else {
        return;
    };
    let size = object.shape.bounds().size;
    commands.entity(add.entity).insert((
        Sprite {
            image: assets.load(look),
            custom_size: Some(Vec2::new(size.width * TILE.0, size.height * TILE.0)),
            ..default()
        },
        Transform::default(),
        RowDepth::default(),
        Visibility::default(),
    ));
}

fn place_objects(
    content: Res<Content>,
    mut objects: Query<(
        &AreaObject,
        &Position,
        &AreaTag,
        &mut Transform,
        &mut RowDepth,
    )>,
) {
    for (object, position, tag, mut transform, mut depth) in &mut objects {
        let area = area::load(&content, tag.area);
        *depth = RowDepth(dynamic_z(
            area.size.height,
            area.sort_base(),
            Meters(position.pos.y),
        ));
        place_sprite(&mut transform, object.shape.center());
    }
}
