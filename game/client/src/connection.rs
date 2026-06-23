//! A full-screen overlay shown once a mode is chosen but the game server link is not up: it reports
//! whether we're still connecting or the link was lost, and offers a reconnect in the latter case.

use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy_replicon::prelude::ClientState;
use ui::{Activate, button, text_colored};

use crate::GameScene;
use crate::net::{self, PendingSession};
use crate::replicon_renet::Client;
use crate::scenes::Mode;

const OVERLAY_BG: Color = Color::srgb(0.07, 0.07, 0.07);

pub struct ConnectionPlugin;

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Link>()
            .add_systems(Update, track)
            .add_systems(OnEnter(Link::Connecting), connecting)
            .add_systems(OnEnter(Link::Lost), lost)
            .add_systems(OnExit(Link::Connecting), despawn)
            .add_systems(OnExit(Link::Lost), despawn);
    }
}

/// State of the link to the game server, as far as the overlay cares. `Idle` covers both "no mode
/// chosen yet" and "connected and playing" — neither shows an overlay.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
enum Link {
    #[default]
    Idle,
    Connecting,
    Lost,
}

fn track(
    scene: Res<State<GameScene>>,
    client_state: Res<State<ClientState>>,
    pending: Option<Res<PendingSession>>,
    client: Option<Res<Client>>,
    link: Res<State<Link>>,
    mut next: ResMut<NextState<Link>>,
) {
    // `ClientState` mirrors the live client a frame late (it's driven in `PreUpdate`, but the client
    // is created mid-frame in `Update`), so read the client itself: a token still being fetched or any
    // non-disconnected client means an attempt is in flight — closing the one-frame gaps where the
    // mirror would otherwise read "lost" mid-connect.
    let attempting = pending.is_some() || client.is_some_and(|client| !client.0.is_disconnected());
    let target =
        if *scene.get() != GameScene::Playing || *client_state.get() == ClientState::Connected {
            Link::Idle
        } else if attempting {
            Link::Connecting
        } else {
            Link::Lost
        };
    if &target != link.get() {
        next.set(target);
    }
}

#[derive(Component, Default, Clone)]
struct ConnectionUi;

fn connecting(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ConnectionUi
        template_value(overlay_node())
        BackgroundColor({OVERLAY_BG})
        GlobalZIndex({100})
        Children [ {EntityScene(text_colored("Connecting...", Color::WHITE))} ]
    });
}

fn lost(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ConnectionUi
        template_value(overlay_node())
        BackgroundColor({OVERLAY_BG})
        GlobalZIndex({100})
        Children [
            {EntityScene(text_colored("Connection lost", Color::WHITE))},
            ( {button("Reconnect")} on(reconnect) ),
        ]
    });
}

fn reconnect(_: On<Activate>, mode: Res<Mode>, mut commands: Commands) {
    let spectate = *mode == Mode::Spectate;
    commands.queue(move |world: &mut World| net::open_session(world, spectate));
}

fn despawn(ui: Query<Entity, With<ConnectionUi>>, mut commands: Commands) {
    for entity in &ui {
        commands.entity(entity).despawn();
    }
}

fn overlay_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(16.0),
        ..default()
    }
}
