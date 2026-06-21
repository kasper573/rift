use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use ui::text_colored;

use crate::Screen;

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(OnEnter(Screen::Playing), spawn)
            .add_systems(OnExit(Screen::Playing), despawn)
            .add_systems(Update, readout.run_if(in_state(Screen::Playing)));
    }
}

#[derive(Component)]
struct FpsHud;

#[derive(Component)]
struct FpsText;

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
        children![(FpsText, text_colored("-- fps", Color::WHITE))],
    ));
}

fn readout(diagnostics: Res<DiagnosticsStore>, mut texts: Query<&mut Text, With<FpsText>>) {
    let reading = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|fps| fps.smoothed())
        .map_or_else(|| "-- fps".to_owned(), |fps| format!("{fps:.0} fps"));
    for mut text in &mut texts {
        text.0 = reading.clone();
    }
}

fn despawn(huds: Query<Entity, With<FpsHud>>, mut commands: Commands) {
    for entity in &huds {
        commands.entity(entity).despawn();
    }
}
