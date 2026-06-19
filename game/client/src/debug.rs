//! F1 cycles debug overlays drawn as gizmos in world space: the area's walkable node graph, then
//! the obscuring object rectangles.

use bevy::prelude::*;
use world::area;
use world::math::{Offset, Pos, Tiles};
use world::protocol::{AreaTag, Owner};
use world::session::MyClient;

use crate::Screen;
use crate::render::TILE;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugMode>()
            .add_systems(Update, (cycle, draw).run_if(in_state(Screen::Playing)));
    }
}

#[derive(Resource, Default, Clone, Copy, PartialEq)]
enum DebugMode {
    #[default]
    Off,
    Nodes,
    Obscured,
}

fn cycle(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<DebugMode>) {
    if keys.just_pressed(KeyCode::F1) {
        *mode = match *mode {
            DebugMode::Off => DebugMode::Nodes,
            DebugMode::Nodes => DebugMode::Obscured,
            DebugMode::Obscured => DebugMode::Off,
        };
    }
}

fn draw(
    mode: Res<DebugMode>,
    me: Res<MyClient>,
    players: Query<(&Owner, &AreaTag)>,
    mut gizmos: Gizmos,
) {
    if *mode == DebugMode::Off {
        return;
    }
    let Some(my) = me.0 else {
        return;
    };
    let Some(area_id) = players
        .iter()
        .find(|(owner, _)| owner.client == my)
        .map(|(_, tag)| tag.area)
    else {
        return;
    };
    let area = &area::areas()[area_id.index()];
    let red = Color::srgb(1.0, 0.0, 0.0);
    match *mode {
        DebugMode::Nodes => {
            for &node in &area.walkable_nodes {
                for (dx, dy) in [
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                    (1, 1),
                    (1, -1),
                    (-1, 1),
                    (-1, -1),
                ] {
                    let neighbor = Pos::new(node.x + dx as f32, node.y + dy as f32);
                    if area.grid.walkable(neighbor) {
                        gizmos.line_2d(center(node), center(neighbor), red);
                    }
                }
            }
        }
        DebugMode::Obscured => {
            for rect in &area.obscuring_rects {
                let min = corner(rect.origin);
                let max = corner(rect.origin + rect.size);
                gizmos.line_2d(Vec2::new(min.x, min.y), Vec2::new(max.x, min.y), red);
                gizmos.line_2d(Vec2::new(max.x, min.y), Vec2::new(max.x, max.y), red);
                gizmos.line_2d(Vec2::new(max.x, max.y), Vec2::new(min.x, max.y), red);
                gizmos.line_2d(Vec2::new(min.x, max.y), Vec2::new(min.x, min.y), red);
            }
        }
        DebugMode::Off => {}
    }
}

fn center(tile: Pos<Tiles>) -> Vec2 {
    corner(tile + Offset::splat(0.5))
}

fn corner(p: Pos<Tiles>) -> Vec2 {
    Vec2::new(p.x * TILE.0, -p.y * TILE.0)
}
