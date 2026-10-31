use std::num::NonZeroU32;
use std::sync::Arc;

use bevy::window::Window;
use engine::core::os::{Http, Profile, SignIn, StartParams, exit_when_stdin_closes};
use engine::systems::launch::{
    AuthMode, BUILD_INFO_FLAG, CLIENT_ENV_PREFIX, ClientConfig, LaunchSettings, build_info_line,
};
use engine::systems::{client_app, probe};

service::allocator!();

fn main() {
    if std::env::args().any(|arg| arg == BUILD_INFO_FLAG) {
        println!("{}", build_info_line());
        return;
    }
    let settings = LaunchSettings::of_this_binary();
    let config: ClientConfig = settings.get(CLIENT_ENV_PREFIX);
    let http = Http::default();
    let sign_in = match (config.access_token, config.auth) {
        (Some(token), _) => SignIn::Token(token),
        (None, AuthMode::Fake) => SignIn::AnyAccount,
        (None, AuthMode::Oidc) => SignIn::Browser(Arc::new(oidc::BrowserSignIn::new(
            settings.get(oidc::ENV_PREFIX),
            http.0.clone(),
        ))),
    };
    let params = StartParams {
        sign_in,
        game_server_url: config.game_server_url,
        game_server_ws_url: config.game_server_ws_url,
    };
    let profile = Profile(settings.path(&config.profile_dir));
    let mut app = client_app(profile, http, params, window());
    if let Some(driver) = config.probe {
        probe::install(&mut app, driver);
    }
    if config.exit_on_stdin_eof {
        exit_when_stdin_closes(&mut app);
    }
    app.run();
}

fn window() -> Window {
    Window {
        desired_maximum_frame_latency: Some(FRAMES_IN_FLIGHT),
        ..Default::default()
    }
}

// A compositor hands a frame's buffer back only when it next refreshes, so it's the frames in
// flight, not the client, that would otherwise cap the frame rate at a few per refresh.
const FRAMES_IN_FLIGHT: NonZeroU32 = NonZeroU32::new(8).unwrap();
