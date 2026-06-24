//! Item presentation: plays an item's sound, at the actor that used it, when the server confirms the
//! use. Emitted into the core audio mixer by id.

use bevy::prelude::*;
use world::systems::items::ItemConsumed;
use world::systems::movement::Position;

use crate::core::audio::PlaySfx;

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            use_sounds.run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

fn use_sounds(
    mut consumed: MessageReader<ItemConsumed>,
    positions: Query<&Position>,
    mut play: MessageWriter<PlaySfx>,
) {
    for consumed in consumed.read() {
        let Some(id) = consumed.item.get().sfx.as_ref() else {
            continue;
        };
        let Ok(position) = positions.get(consumed.actor) else {
            continue;
        };
        play.write(PlaySfx {
            id: id.0.clone(),
            at: position.pos,
        });
    }
}
