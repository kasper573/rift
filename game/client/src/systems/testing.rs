use std::cell::RefCell;

use bevy::prelude::*;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use world::core::math::Pos;
use world::core::tiling::Tiles;
use world::systems::player::session;

use crate::Scene;

thread_local! {
    static PENDING: RefCell<Vec<Pos<Tiles>>> = const { RefCell::new(Vec::new()) };
}

pub struct TestingPlugin;

impl Plugin for TestingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, install_hook)
            .add_systems(Update, drain.run_if(in_state(Scene::Area)));
    }
}

fn install_hook() {
    let hook = Closure::<dyn Fn(f32, f32)>::new(|x: f32, y: f32| {
        PENDING.with(|pending| pending.borrow_mut().push(Pos::new(x, y)));
    });
    js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("click_world_tile"),
        hook.as_ref(),
    )
    .expect("expose click_world_tile on the JS global");
    hook.forget(); // hand the closure to JS for the page's lifetime
}

fn drain(world: &mut World) {
    let pending: Vec<Pos<Tiles>> =
        PENDING.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    for tile in pending {
        session::move_to(world, tile);
    }
}
