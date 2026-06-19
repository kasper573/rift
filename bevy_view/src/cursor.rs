use bevy_ecs::prelude::*;
use bevy_picking::hover::HoverMap;

pub use bevy_window::CursorIcon;

/// A cursor an element opts into showing while the pointer is over it. Set it with
/// [`Element::cursor`](crate::Element::cursor); an element without one never changes the cursor.
#[derive(Component, Clone)]
pub struct HoverCursor(pub CursorIcon);

/// Forces a cursor regardless of what is hovered — set it for the duration of a gesture (e.g. a
/// window resize) so the cursor holds even as the pointer leaves the element, and clear it when the
/// gesture ends.
#[derive(Resource, Default)]
pub struct CursorLock(pub Option<CursorIcon>);

/// The cursor the UI wants right now: the [`CursorLock`] if set, otherwise the topmost hovered
/// element's [`HoverCursor`]. Returns `None` when the UI has no opinion, leaving the game's own
/// cursor in charge — so a game applies it as `bevy_view::hovered_cursor(world).unwrap_or(game_cursor)`.
pub fn hovered_cursor(world: &World) -> Option<CursorIcon> {
    if let Some(locked) = world
        .get_resource::<CursorLock>()
        .and_then(|lock| lock.0.clone())
    {
        return Some(locked);
    }
    let hover_map = world.get_resource::<HoverMap>()?;
    let mut topmost: Option<(f32, CursorIcon)> = None;
    for hits in hover_map.values() {
        for (&entity, hit) in hits.iter() {
            if let Some(cursor) = world.get::<HoverCursor>(entity)
                && topmost.as_ref().is_none_or(|(depth, _)| hit.depth > *depth)
            {
                topmost = Some((hit.depth, cursor.0.clone()));
            }
        }
    }
    topmost.map(|(_, cursor)| cursor)
}
