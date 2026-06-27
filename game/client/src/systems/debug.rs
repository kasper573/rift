use bevy::prelude::*;
use world::core::assets::AssetService;
use world::core::math::Offset;
use world::core::tiling::{self, TilePos};
use world::systems::actor::{Actor, Hitbox};
use world::systems::area;
use world::systems::area::AreaTag;
use world::systems::movement::Position;
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

use crate::Scene;
use crate::core::render::screen::ToScreen;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugMode>()
            .init_resource::<ShowHitboxes>()
            .add_systems(
                Update,
                (cycle, draw, toggle_hitboxes, draw_hitboxes).run_if(in_state(Scene::Area)),
            )
            .add_systems(OnExit(Scene::Area), clear_hitboxes);
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
    service: Res<AssetService>,
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
    let area = service.resolve(area_id, |a| area::build_area(a, area_id));
    let red = Color::srgb(1.0, 0.0, 0.0);
    match *mode {
        DebugMode::Nodes => {
            for &node in &area.walkable_nodes {
                for (dx, dy) in tiling::NEIGHBORS_8 {
                    let neighbor = node + Offset::new(dx as f32, dy as f32);
                    if area.grid.walkable(neighbor) {
                        gizmos.line_2d(node.to_screen(), neighbor.to_screen(), red);
                    }
                }
            }
        }
        DebugMode::Obscured => {
            for rect in &area.obscuring_rects {
                let min = rect.origin.to_screen();
                let max = (rect.origin + rect.size).to_screen();
                gizmos.line_2d(Vec2::new(min.x, min.y), Vec2::new(max.x, min.y), red);
                gizmos.line_2d(Vec2::new(max.x, min.y), Vec2::new(max.x, max.y), red);
                gizmos.line_2d(Vec2::new(max.x, max.y), Vec2::new(min.x, max.y), red);
                gizmos.line_2d(Vec2::new(min.x, max.y), Vec2::new(min.x, min.y), red);
            }
        }
        DebugMode::Off => {}
    }
}

#[derive(Resource, Default)]
struct ShowHitboxes(bool);

#[derive(Component)]
struct HitboxOverlay;

const HITBOX_FILL: Color = Color::srgba(1.0, 0.0, 0.0, 0.35);
const HITBOX_Z: f32 = 100.0;

fn toggle_hitboxes(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowHitboxes>) {
    if keys.just_pressed(KeyCode::F2) {
        show.0 = !show.0;
    }
}

fn draw_hitboxes(
    show: Res<ShowHitboxes>,
    actors: Query<(&Position, &Hitbox), With<Actor>>,
    overlays: Query<Entity, With<HitboxOverlay>>,
    mut commands: Commands,
) {
    clear(&overlays, &mut commands);
    if !show.0 {
        return;
    }
    for (position, hitbox) in &actors {
        let bounds = position.pos.hitbox(hitbox.size);
        let size = hitbox.size.to_screen();
        let center = bounds.center().to_screen();
        commands.spawn((
            HitboxOverlay,
            Sprite {
                color: HITBOX_FILL,
                custom_size: Some(size),
                ..default()
            },
            Transform::from_translation(center.extend(HITBOX_Z)),
        ));
    }
}

fn clear_hitboxes(overlays: Query<Entity, With<HitboxOverlay>>, mut commands: Commands) {
    clear(&overlays, &mut commands);
}

fn clear(overlays: &Query<Entity, With<HitboxOverlay>>, commands: &mut Commands) {
    for entity in overlays {
        commands.entity(entity).despawn();
    }
}
