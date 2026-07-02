use std::cell::RefCell;

use bevy::prelude::*;
use world::core::math::Pos;
use world::core::tiling::Tiles;
use world::systems::player::session;

use crate::Scene;
use crate::core::platform;

pub struct TestingPlugin;

impl Plugin for TestingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, install_hook)
            .add_systems(Update, drain.run_if(in_state(Scene::Area)));
    }
}

thread_local! {
    static PENDING: RefCell<Vec<Pos<Tiles>>> = const { RefCell::new(Vec::new()) };
}

fn install_hook() {
    platform::expose_global_fn("click_world_tile", |x, y| {
        PENDING.with(|pending| pending.borrow_mut().push(Pos::new(x, y)));
    });
}

fn drain(world: &mut World) {
    let pending: Vec<Pos<Tiles>> =
        PENDING.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    for tile in pending {
        session::move_to(world, tile);
    }
}
