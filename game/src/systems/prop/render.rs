use bevy::prelude::*;

use super::Prop;
use crate::core::assets::AssetService;
use crate::core::render::{TILE, dynamic_z, sprite_transform};
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::actor::Hitbox;
use crate::systems::area::{self, AreaTag};
use crate::systems::movement::Position;

pub struct PropPlugin;

impl Plugin for PropPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(attach_look).add_systems(
            Update,
            place_props.run_if(in_state(crate::systems::scene::Scene::Area)),
        );
    }
}

fn attach_look(
    add: On<Add, Prop>,
    props: Query<(&Prop, &Hitbox)>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok((prop, hitbox)) = props.get(add.entity) else {
        return;
    };
    let Some(look) = prop.def.get().look else {
        return;
    };
    commands.entity(add.entity).insert((
        Sprite {
            image: assets.load(look.0),
            custom_size: Some(Vec2::new(
                hitbox.size.width * TILE.0,
                hitbox.size.height * TILE.0,
            )),
            ..default()
        },
        Transform::default(),
        Visibility::default(),
    ));
}

fn place_props(
    service: Res<AssetService>,
    mut props: Query<(&Position, &Hitbox, &AreaTag, &mut Transform), With<Prop>>,
) {
    for (position, hitbox, tag, mut transform) in &mut props {
        let area = service.resolve(tag.area.get().map, area::build_area);
        let z = dynamic_z(
            area.size.height,
            area.dynamic_layer() as f32,
            Tiles(position.pos.y),
        );
        let center = position.pos.hitbox(hitbox.size).center();
        *transform = sprite_transform(center, z);
    }
}
