pub mod gestures;

use crate::core::assets::AssetService;
use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::area::{self, AreaTag};
use crate::systems::player::Owner;
use crate::systems::player::session::{self, MyClient};
use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};

use crate::core::render::TILE;
use crate::core::render::screen::ToScreen;
use crate::systems::scene::Scene;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        gestures::plugin(app);
        app.add_systems(Startup, setup_highlight)
            .add_systems(Update, touch_as_mouse)
            .add_systems(
                Update,
                (
                    respawn_when_dead.run_if(not(ui::typing)),
                    update_tile_highlight,
                )
                    .run_if(in_state(Scene::Area)),
            );
    }
}

#[derive(Resource)]
pub struct ActiveTileHighlight {
    pub pos: Pos<Tiles>,
    pub image: Handle<Image>,
}

#[derive(Component)]
struct TileHighlight;

fn setup_highlight(mut commands: Commands) {
    commands.spawn((
        TileHighlight,
        Sprite {
            custom_size: Some(Vec2::splat(TILE.0)),
            ..default()
        },
        Transform::default(),
        Visibility::Hidden,
    ));
}

fn update_tile_highlight(
    highlight: Option<Res<ActiveTileHighlight>>,
    me: Res<MyClient>,
    service: Res<AssetService>,
    players: Query<(&Owner, &AreaTag)>,
    mut sprite: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<TileHighlight>>,
) {
    let Ok((mut sprite, mut transform, mut visibility)) = sprite.single_mut() else {
        return;
    };
    let Some(highlight) = highlight else {
        *visibility = Visibility::Hidden;
        return;
    };
    let Some(z) = highlight_z(&service, &me, &players) else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Visible;
    sprite.image = highlight.image.clone();
    transform.translation = highlight.pos.to_screen().extend(z);
}

fn highlight_z(
    service: &AssetService,
    me: &MyClient,
    players: &Query<(&Owner, &AreaTag)>,
) -> Option<f32> {
    let my = me.0?;
    let (_, tag) = players.iter().find(|(owner, _)| owner.client == my)?;
    Some(
        service
            .resolve(tag.area.get().map, area::build_area)
            .dynamic_layer() as f32,
    )
}

fn touch_as_mouse(
    touches: Res<Touches>,
    window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cursor_moved: MessageWriter<CursorMoved>,
    mut mouse_button: MessageWriter<MouseButtonInput>,
) {
    let (entity, mut window) = window.into_inner();
    if let Some(touch) = touches.iter().next() {
        window.set_cursor_position(Some(touch.position()));
        cursor_moved.write(CursorMoved {
            window: entity,
            position: touch.position(),
            delta: Some(touch.delta()),
        });
    }
    for touch in touches.iter_just_pressed() {
        window.set_cursor_position(Some(touch.position()));
        cursor_moved.write(CursorMoved {
            window: entity,
            position: touch.position(),
            delta: None,
        });
        mouse_button.write(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window: entity,
        });
    }
    for _ in touches.iter_just_released() {
        mouse_button.write(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window: entity,
        });
    }
}

fn respawn_when_dead(world: &mut World) {
    if session::is_dead(world)
        && world
            .resource::<ButtonInput<KeyCode>>()
            .get_just_pressed()
            .next()
            .is_some()
    {
        session::respawn(world);
    }
}
