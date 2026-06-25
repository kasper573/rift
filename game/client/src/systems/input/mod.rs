pub mod gestures;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use world::core::math::Pos;
use world::core::tiling::Tiles;
use world::systems::area::{self, AreaTag};
use world::systems::player::Owner;
use world::systems::player::session::{self, MyClient};

use crate::GameScene;
use crate::core::render::TILE;
use crate::core::render::screen::ToScreen;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        gestures::plugin(app);
        app.add_systems(Startup, setup_highlight)
            .add_systems(Update, touch_as_mouse)
            .add_systems(
                Update,
                (respawn_when_dead, update_tile_highlight).run_if(in_state(GameScene::Playing)),
            );
    }
}

/// What the active gesture wants highlighted on the map — a tile and the image to mark it with. The
/// gesture dispatch sets it as a resource; [`update_tile_highlight`] mirrors it onto the sprite.
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
    let Some(z) = highlight_z(&me, &players) else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Visible;
    sprite.image = highlight.image.clone();
    transform.translation = highlight.pos.to_screen().extend(z);
}

/// The base depth of the local player's area dynamic layer — every actor sorts strictly above it, so
/// the highlight shares the actors' layer yet always renders behind them.
fn highlight_z(me: &MyClient, players: &Query<(&Owner, &AreaTag)>) -> Option<f32> {
    let my = me.0?;
    let (_, tag) = players.iter().find(|(owner, _)| owner.client == my)?;
    Some(area::areas().get(tag.area.index())?.dynamic_layer() as f32)
}

/// Bridges touch to the mouse so taps act as left-clicks for both the map and the UI, without the
/// rest of the game (or bevy's picking) needing to know about touch: the first active finger drives
/// the cursor, and a touch beginning/ending becomes a left-button press/release.
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
