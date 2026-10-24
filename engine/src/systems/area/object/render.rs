use bevy::prelude::*;

use super::AreaObject;
use crate::core::content::Content;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{RowDepth, TILE, dynamic_z, place_sprite};
use crate::core::units::{Meters, TilePos};
use crate::systems::actor::Hitbox;
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
    objects: Query<(&AreaObject, &Hitbox)>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok((object, hitbox)) = objects.get(add.entity) else {
        return;
    };
    let Some(look) = object.look.clone() else {
        return;
    };
    commands.entity(add.entity).insert((
        Sprite {
            image: assets.load(look),
            custom_size: Some(Vec2::new(
                hitbox.size.width * TILE.0,
                hitbox.size.height * TILE.0,
            )),
            ..default()
        },
        Transform::default(),
        RowDepth::default(),
        Visibility::default(),
    ));
}

fn place_objects(
    content: Res<Content>,
    mut objects: Query<
        (&Position, &Hitbox, &AreaTag, &mut Transform, &mut RowDepth),
        With<AreaObject>,
    >,
) {
    for (position, hitbox, tag, mut transform, mut depth) in &mut objects {
        let area = area::load(&content, tag.area);
        *depth = RowDepth(dynamic_z(
            area.size.height,
            area.sort_base(),
            Meters(position.pos.y),
        ));
        let center = position.pos.hitbox(hitbox.size).center();
        place_sprite(&mut transform, center);
    }
}
