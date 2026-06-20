use bevy_ecs::prelude::*;
use bevy_picking::hover::HoverMap;

pub use bevy_window::CursorIcon;

/// An element without a cursor never changes it, so the game's own cursor stays in charge.
#[derive(Component, Clone)]
pub struct HoverCursor(pub CursorIcon);

/// Locks a cursor for the duration of a gesture so it holds even as the pointer leaves the element.
#[derive(Resource, Default)]
pub struct CursorLock(pub Option<CursorIcon>);

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
