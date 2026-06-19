//! Pointer interaction state, the analog of CSS `:hover`/`:active`. [`PointerState`] tracks whether the
//! pointer is over an element and pressing it; observers maintain it from `bevy_picking` events, and a
//! recipe reads it to pick the hover/active styling. Elements opt in by carrying the component (the
//! stateful styling inserts it); the observers ignore everything else.

use bevy_ecs::prelude::*;
use bevy_picking::prelude::{Out, Over, Pointer, Press, Release};

/// Whether an element is currently hovered and/or pressed.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PointerState {
    pub hovered: bool,
    pub pressed: bool,
}

pub(crate) fn on_over(over: On<Pointer<Over>>, mut states: Query<&mut PointerState>) {
    if let Ok(mut state) = states.get_mut(over.entity) {
        state.hovered = true;
    }
}

pub(crate) fn on_out(out: On<Pointer<Out>>, mut states: Query<&mut PointerState>) {
    if let Ok(mut state) = states.get_mut(out.entity) {
        state.hovered = false;
        state.pressed = false;
    }
}

pub(crate) fn on_press(press: On<Pointer<Press>>, mut states: Query<&mut PointerState>) {
    if let Ok(mut state) = states.get_mut(press.entity) {
        state.pressed = true;
    }
}

pub(crate) fn on_release(release: On<Pointer<Release>>, mut states: Query<&mut PointerState>) {
    if let Ok(mut state) = states.get_mut(release.entity) {
        state.pressed = false;
    }
}
