use std::io::Cursor;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy_app::App;
use bevy_replicon::prelude::RepliconChannels;
use bevy_replicon_renet::netcode::{ClientAuthentication, ConnectToken, NetcodeClientTransport};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RepliconRenetPlugins};
use bevy_state::app::StatesPlugin;
use bevy_time::TimePlugin;
use world::session::ClientSessionPlugin;

/// A headless client app connected to the running server over netcode with `token`.
pub fn connect(token: &[u8]) -> App {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        ClientSessionPlugin,
        RepliconRenetPlugins,
    ));

    let channels = app.world().resource::<RepliconChannels>();
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    });
    let connect_token = ConnectToken::read(&mut Cursor::new(token)).expect("read connect token");
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("bind udp");
    let transport = NetcodeClientTransport::new(
        SystemTime::now().duration_since(UNIX_EPOCH).expect("epoch"),
        ClientAuthentication::Secure { connect_token },
        socket,
    )
    .expect("client transport");

    app.insert_resource(client);
    app.insert_resource(transport);
    app.finish();
    app
}

/// Ticks `app` until `ready`, or returns `false` past the timeout.
pub fn wait(app: &mut App, seconds: f32, mut ready: impl FnMut(&mut App) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs_f32(seconds);
    loop {
        app.update();
        if ready(app) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
