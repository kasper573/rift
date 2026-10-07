mod soundscape;

use crate::core::assets::AssetService;
use crate::core::math::{Offset, Rect};
use crate::core::tiling::{self, Cell, TilePos, Tiles};
use crate::systems::actor::{Actor, Hitbox};
use crate::systems::area;
use crate::systems::area::AreaTag;
use crate::systems::input::map::{ActionInput, InputAction};
use crate::systems::player::session::Viewpoint;
use bevy::prelude::*;

use crate::core::render::screen::ToScreen;
use crate::systems::movement::RenderPosition;
use crate::systems::scene::Scene;

const SAFE_TILE_INSET: f32 = 0.2;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugMode>()
            .init_resource::<ShowHitboxes>()
            .init_resource::<soundscape::ShownSoundscape>()
            .add_systems(
                Update,
                (
                    cycle.run_if(not(ui::typing)),
                    draw,
                    (soundscape::show, soundscape::read_out_hovered).chain(),
                    toggle_hitboxes.run_if(not(ui::typing)),
                    draw_hitboxes,
                )
                    .run_if(in_state(Scene::Area)),
            )
            .add_systems(OnExit(Scene::Area), (clear_hitboxes, soundscape::hide));
    }
}

#[derive(Resource, Default, Clone, Copy, PartialEq)]
enum DebugMode {
    #[default]
    Off,
    Nodes,
    Obscured,
    SafeZones,
    Soundscapes,
}

fn cycle(input: ActionInput, mut mode: ResMut<DebugMode>) {
    if input.just_pressed(InputAction::CycleDebugView) {
        *mode = match *mode {
            DebugMode::Off => DebugMode::Nodes,
            DebugMode::Nodes => DebugMode::Obscured,
            DebugMode::Obscured => DebugMode::SafeZones,
            DebugMode::SafeZones => DebugMode::Soundscapes,
            DebugMode::Soundscapes => DebugMode::Off,
        };
    }
}

fn draw(
    mode: Res<DebugMode>,
    viewpoint: Res<Viewpoint>,
    service: Res<AssetService>,
    areas: Query<&AreaTag>,
    mut gizmos: Gizmos,
) {
    if *mode == DebugMode::Off {
        return;
    }
    let Some(area_id) = viewpoint
        .0
        .and_then(|seen| areas.get(seen).ok())
        .map(|tag| tag.area)
    else {
        return;
    };
    let area = service.resolve(area_id.get().map, area::build_area);
    let red = Color::srgb(1.0, 0.0, 0.0);
    match *mode {
        DebugMode::Nodes => {
            for &node in area.grid.nodes() {
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
                outline(&mut gizmos, *rect, red);
            }
        }
        DebugMode::SafeZones => {
            let green = Color::srgb(0.2, 1.0, 0.4);
            for &node in area.grid.nodes().iter().filter(|&&node| area.safe(node)) {
                let tile = node.cell().bounds();
                outline(
                    &mut gizmos,
                    tile.inflate(-SAFE_TILE_INSET, -SAFE_TILE_INSET),
                    green,
                );
            }
            for zone in &area.safe_zones {
                outline(&mut gizmos, *zone, green);
            }
        }
        DebugMode::Soundscapes => soundscape::outline(&mut gizmos, &area.soundscape),
        DebugMode::Off => {}
    }
}

fn outline(gizmos: &mut Gizmos, rect: Rect<Tiles>, color: Color) {
    gizmos.rect_2d(rect.center().to_screen(), rect.size.to_screen(), color);
}

#[derive(Resource, Default)]
struct ShowHitboxes(bool);

#[derive(Component)]
struct HitboxOverlay;

const HITBOX_FILL: Color = Color::srgba(1.0, 0.0, 0.0, 0.35);
const HITBOX_Z: f32 = 100.0;

fn toggle_hitboxes(input: ActionInput, mut show: ResMut<ShowHitboxes>) {
    if input.just_pressed(InputAction::ToggleHitboxes) {
        show.0 = !show.0;
    }
}

fn draw_hitboxes(
    show: Res<ShowHitboxes>,
    actors: Query<(&RenderPosition, &Hitbox), With<Actor>>,
    overlays: Query<Entity, With<HitboxOverlay>>,
    mut commands: Commands,
) {
    clear(&overlays, &mut commands);
    if !show.0 {
        return;
    }
    for (render, hitbox) in &actors {
        let bounds = render.0.hitbox(hitbox.size);
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
