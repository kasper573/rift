//! A client with no window. It signs in, joins as its account's character and holds the session
//! like any player's client, so the character is in the world as any other is, but it draws
//! nothing and decides nothing: whoever runs it hands it the requests a game client sends.

use std::borrow::Cow;

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use bevy_replication::{
    ClientMessageRefusal, client_message_schema, read_client_message, write_client_message,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use super::session::{self, ClientSessionPlugin};
use super::{AreaDrawn, ArrivalHold, JoinRequest};
use crate::core::content::id::Leaf;
use crate::core::content::sync::{MANIFEST, Manifest, is_content_of};
use crate::core::content::{Content, ContentFolder, ContentScope, with_content};
use crate::core::net::SessionRequest;
use crate::core::net::auth::Credentials;
use crate::core::net::transport::{ClientTransportPlugin, Connection, read_token};
use crate::systems::probe::{self, ProbeSight};
use crate::systems::{insert_content, registry};

pub struct HeadlessClient {
    app: App,
}

impl HeadlessClient {
    /// Opens a session on the game server at `http_url` as the account `access_token` signs in,
    /// on whatever build the server runs: a client that draws nothing has none to keep in step.
    pub fn connect(http_url: &str, access_token: &str) -> Result<HeadlessClient, String> {
        let token = session_token(http_url, access_token)?;
        let token = read_token(&token).ok_or("the server handed out no connect token")?;
        let connection = Connection::open(token)
            .map_err(|error| format!("could not open a connection: {error}"))?;
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin);
        app.add_plugins((ClientSessionPlugin, ClientTransportPlugin));
        app.init_resource::<Joined>();
        app.add_systems(Update, (join_once_welcomed, report_arrivals));
        app.insert_resource(connection);
        app.finish();
        app.cleanup();
        Ok(HeadlessClient { app })
    }

    /// Gives the client the game its server runs (see [`runs`]). Only a client that knows the
    /// game can name its content, in a request or in what it sees.
    pub fn knowing(mut self, content: &Content) -> HeadlessClient {
        insert_content(self.app.world_mut(), content);
        self
    }

    /// Takes in what the server sent, and sends it what was requested since.
    pub fn update(&mut self) {
        self.app.update();
    }

    pub fn is_connected(&self) -> bool {
        self.connection().is_some_and(Connection::is_connected)
    }

    /// Whether the connection is still being made.
    pub fn is_connecting(&self) -> bool {
        self.connection().is_some_and(Connection::is_connecting)
    }

    /// Whether its character stands in the world.
    pub fn in_world(&self) -> bool {
        session::me(self.app.world()).is_some()
    }

    /// Sends the server `request`, one of [`ClientRequest`], with the next update.
    pub fn request(&mut self, request: serde_json::Value) -> Result<(), ClientMessageRefusal> {
        write_request(self.app.world_mut(), request)
    }

    /// What the client sees around its character, if it knows the game.
    pub fn sight(&mut self) -> Option<ProbeSight> {
        let world = self.app.world_mut();
        world
            .contains_resource::<Content>()
            .then(|| probe::sight(world))
    }

    fn connection(&self) -> Option<&Connection> {
        self.app.world().get_resource::<Connection>()
    }
}

/// Writes `request`, one of [`ClientRequest`], in the world of a client, as the client's own
/// systems write theirs. A client that knows its game reads the content a request names by its
/// path; one that doesn't can only make requests that name none.
pub fn write_request(
    world: &mut World,
    request: serde_json::Value,
) -> Result<(), ClientMessageRefusal> {
    let Some(scope) = world.get_resource::<Content>().map(ContentScope::of) else {
        return write_client_message(world, request);
    };
    let (read, scope) = with_content(scope, || read_client_message(world, request));
    let read = read?;
    if let Some(unknown) = scope.leaves().iter().find_map(Leaf::reported) {
        return Err(ClientMessageRefusal::Unreadable(serde::de::Error::custom(
            unknown,
        )));
    }
    read.write(world);
    Ok(())
}

/// Whether the game server at `http_url` runs `content`, the game loaded out of `folder`: the
/// two then name every file of the game by the same id.
pub fn runs(http_url: &str, content: &Content, folder: &dyn ContentFolder) -> Result<bool, String> {
    is_content_of(&manifest(http_url)?, registry(), content, folder)
        .map_err(|error| format!("could not read the game: {error}"))
}

/// A request a client sends the game server, as [`HeadlessClient::request`] takes it: an object
/// of one key, the request's name, over the request itself. Whatever stands in the world (a
/// character, a monster, a thing on the ground) is named by the id the game client's probe
/// or a headless client's sight reports it by, and content by its path in the game folder.
pub struct ClientRequest;

impl JsonSchema for ClientRequest {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ClientRequest")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut app = App::new();
        crate::systems::protocol(&mut app);
        client_message_schema(app.world(), generator)
    }
}

/// The JSON Schema of what whoever runs a headless client speaks with it: the requests it takes,
/// and the lines it answers with.
pub fn schema() -> String {
    let schema = schemars::schema_for!(PlayProtocol);
    serde_json::to_string_pretty(&schema).expect("a serializable schema")
}

/// Everything headless clients run from outside speak, for its schema.
#[derive(JsonSchema)]
pub struct PlayProtocol {
    pub ask: PlayAsk,
    pub line: PlayLine,
}

/// What whoever runs headless clients from outside tells them, one JSON line each. It names
/// each client by a name of its own choosing, `player`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PlayAsk {
    /// Signs a client in with the access token `token`, to join the game as its account's
    /// character.
    Join { player: String, token: String },
    /// Sends the server `request` from `player`'s client.
    Request {
        player: String,
        #[schemars(with = "ClientRequest")]
        request: serde_json::Value,
    },
    /// Asks what `player` sees, answered with [`PlayLine::Sight`].
    Look { player: String },
    /// Closes `player`'s client, as a player leaving the game does.
    Leave { player: String },
}

/// What headless clients run from outside say of how it goes, one JSON line each.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PlayLine {
    /// The character of `player` stands in the world: from here on the server heeds its
    /// requests.
    InWorld { player: String },
    /// The character of `player` left the world, as it does on its way from one area to the
    /// next.
    OutOfWorld { player: String },
    /// What `player` sees.
    Sight {
        player: String,
        sight: Box<ProbeSight>,
    },
    /// The server closed the connection of `player`, whose client is gone.
    Dropped { player: String },
    /// A line could not be done as it asked, and why.
    Refused { line: String, why: String },
}

fn manifest(http_url: &str) -> Result<Manifest, String> {
    let manifest = ureq::get(format!("{http_url}{MANIFEST}"))
        .call()
        .and_then(|mut answer| answer.body_mut().read_to_string())
        .map_err(|error| format!("could not ask {http_url} what it runs: {error}"))?;
    serde_json::from_str(&manifest)
        .map_err(|error| format!("{http_url} does not say what it runs: {error}"))
}

fn session_token(http_url: &str, access_token: &str) -> Result<String, String> {
    let manifest = manifest(http_url)?;
    let request = SessionRequest {
        build: manifest.build,
        rows: manifest.rows.hash,
    };
    ureq::post(format!("{http_url}/session"))
        .header(
            "Authorization",
            Credentials::from_access_token(access_token).authorization,
        )
        .header("Content-Type", "application/json")
        .send(serde_json::to_string(&request).expect("a session request serializes"))
        .and_then(|mut answer| answer.body_mut().read_to_string())
        .map_err(|error| format!("{http_url} opened no session: {error}"))
}

#[derive(Resource, Default)]
struct Joined(bool);

fn join_once_welcomed(world: &mut World) {
    if world.resource::<Joined>().0 || session::my_id(world).is_none() {
        return;
    }
    world.write_message(JoinRequest);
    world.resource_mut::<Joined>().0 = true;
}

// The server holds a character that arrives somewhere until its client has drawn the place, and
// a client that draws nothing has drawn it at once.
fn report_arrivals(held: Query<(), Changed<ArrivalHold>>, mut drawn: MessageWriter<AreaDrawn>) {
    if !held.is_empty() {
        drawn.write(AreaDrawn);
    }
}
