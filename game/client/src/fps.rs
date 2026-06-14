//! A windowless FPS readout pinned to the bottom-left of the HUD, shown only while playing.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::Screen;

pub struct FpsPlugin;

impl Plugin for FpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(OnEnter(Screen::Playing), spawn)
            .add_systems(OnExit(Screen::Playing), despawn)
            .add_systems(Update, update.run_if(in_state(Screen::Playing)));
    }
}

#[derive(Component)]
struct FpsText;

fn spawn(mut commands: Commands) {
    commands.spawn((
        FpsText,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            bottom: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(Color::BLACK),
        Text::new("-- fps"),
        TextColor(Color::WHITE),
        GlobalZIndex(100),
        Pickable::IGNORE,
    ));
}

fn update(diagnostics: Res<DiagnosticsStore>, mut text: Single<&mut Text, With<FpsText>>) {
    if let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|fps| fps.smoothed())
    {
        text.0 = format!("{fps:.0} fps");
    }
}

fn despawn(texts: Query<Entity, With<FpsText>>, mut commands: Commands) {
    for entity in &texts {
        commands.entity(entity).despawn();
    }
}
