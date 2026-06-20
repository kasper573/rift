use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_view::{View, ViewRoot};
use ui::Text;

use crate::Screen;

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(OnEnter(Screen::Playing), spawn)
            .add_systems(OnExit(Screen::Playing), despawn);
    }
}

#[derive(Component)]
struct FpsHud;

fn spawn(mut commands: Commands) {
    commands.spawn((
        FpsHud,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            bottom: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(100),
        Pickable::IGNORE,
        ViewRoot::new(readout),
    ));
}

fn readout(_: &World) -> View {
    Text::dynamic(|w: &World| fps(w)).color(Color::WHITE).into()
}

fn fps(world: &World) -> String {
    world
        .get_resource::<DiagnosticsStore>()
        .and_then(|diagnostics| {
            diagnostics
                .get(&FrameTimeDiagnosticsPlugin::FPS)?
                .smoothed()
        })
        .map_or_else(|| "-- fps".to_owned(), |fps| format!("{fps:.0} fps"))
}

fn despawn(huds: Query<Entity, With<FpsHud>>, mut commands: Commands) {
    for entity in &huds {
        commands.entity(entity).despawn();
    }
}
