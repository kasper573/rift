use bevy_ecs::prelude::*;
use strum::VariantArray;

use crate::data::terminal::Id;
use crate::systems::account::identity::Identity;
use crate::systems::account::role::Role;
use crate::systems::player::ClientId;
use crate::systems::terminal::{self, TerminalEntry, TerminalInbox};

/// The `#[command]` attribute: declares a free fn as a terminal command. See [`Command`].
pub use world_macros::command;

/// A terminal command, registered where it is defined via the [`command`] attribute and
/// collected through `inventory` — there is no central command list to maintain.
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    pub role: Option<Role>,
    pub args: &'static [ArgSpec],
    pub run: fn(&mut World, &Ctx) -> Result<String, String>,
}

inventory::collect!(Command);

pub struct ArgSpec {
    pub name: &'static str,
    pub required: bool,
}

impl Command {
    pub fn usage(&self) -> String {
        let mut usage = format!("/{}", self.name);
        for (index, arg) in self.args.iter().enumerate() {
            let lead = if index == 0 { " " } else { "," };
            if arg.required {
                usage.push_str(&format!("{lead}{}", arg.name));
            } else if index == 0 {
                usage.push_str(&format!(" [{}]", arg.name));
            } else {
                usage.push_str(&format!("[{lead}{}]", arg.name));
            }
        }
        usage
    }
}

pub struct Ctx {
    pub conn: Entity,
    pub client: ClientId,
    pub player: Option<Entity>,
    pub terminal: Id,
    pub args: String,
}

impl Ctx {
    /// Positional command arguments are comma-separated; an empty slot (`3,4,,bob`) skips an
    /// optional argument.
    pub fn split_args(&self) -> impl Iterator<Item = &str> {
        self.args.split(',').map(str::trim)
    }
}

/// How one positional command argument parses. The `#[command]` attribute binds each declared
/// fn parameter via this trait; `Option<T>` arguments may be omitted.
pub trait Arg: Sized {
    const REQUIRED: bool = true;
    fn parse(name: &str, raw: Option<&str>) -> Result<Self, String>;
}

impl Arg for f32 {
    fn parse(name: &str, raw: Option<&str>) -> Result<f32, String> {
        from_str("number", name, raw)
    }
}

impl Arg for u32 {
    fn parse(name: &str, raw: Option<&str>) -> Result<u32, String> {
        from_str("integer", name, raw)
    }
}

impl Arg for String {
    fn parse(name: &str, raw: Option<&str>) -> Result<String, String> {
        from_str("text", name, raw)
    }
}

impl Arg for crate::data::area::Id {
    fn parse(name: &str, raw: Option<&str>) -> Result<Self, String> {
        let raw = present(name, raw)?;
        Self::VARIANTS
            .iter()
            .copied()
            .find(|id| format!("{id:?}").eq_ignore_ascii_case(raw))
            .ok_or_else(|| format!("`{raw}` is not a known area"))
    }
}

impl<T: Arg> Arg for Option<T> {
    const REQUIRED: bool = false;

    fn parse(name: &str, raw: Option<&str>) -> Result<Option<T>, String> {
        match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
            None => Ok(None),
            Some(raw) => T::parse(name, Some(raw)).map(Some),
        }
    }
}

/// Runs `/name args...` inputs. Every `/` input is consumed — chat must never rebroadcast a
/// command, valid or not. Unknown and unauthorized commands get the same reply so command
/// existence is not leaked.
pub fn dispatch(world: &mut World) {
    let entries = world.resource::<TerminalInbox>().0.clone();
    for entry in entries.iter() {
        let Some(rest) = entry.text.strip_prefix('/') else {
            continue;
        };
        entry.consume();
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let reply = match run(world, entry, name, args) {
            Ok(reply) | Err(reply) => reply,
        };
        if !reply.is_empty() {
            terminal::reply(world, entry.conn, entry.terminal, reply);
        }
    }
}

fn run(world: &mut World, entry: &TerminalEntry, name: &str, args: &str) -> Result<String, String> {
    let known = inventory::iter::<Command>().find(|command| command.name == name);
    let command = known
        .filter(|command| allowed(world, entry.conn, command))
        .ok_or_else(|| format!("Unknown command /{name} — try /commands"))?;
    let ctx = Ctx {
        conn: entry.conn,
        client: entry.client,
        player: entry.player,
        terminal: entry.terminal,
        args: args.to_owned(),
    };
    (command.run)(world, &ctx).map_err(|error| format!("{error}\nusage: {}", command.usage()))
}

fn allowed(world: &World, conn: Entity, command: &Command) -> bool {
    match command.role {
        None => true,
        Some(role) => world
            .get::<Identity>(conn)
            .is_some_and(|identity| identity.has_role(role)),
    }
}

/// List the commands available to you.
#[command]
fn commands(world: &mut World, ctx: &Ctx) -> Result<String, String> {
    let mut lines: Vec<String> = inventory::iter::<Command>()
        .filter(|command| allowed(world, ctx.conn, command))
        .map(|command| format!("{} — {}", command.usage(), command.description))
        .collect();
    lines.sort();
    Ok(lines.join("\n"))
}

fn from_str<T: std::str::FromStr>(kind: &str, name: &str, raw: Option<&str>) -> Result<T, String> {
    let raw = present(name, raw)?;
    raw.parse()
        .map_err(|_| format!("`{raw}` is not a valid {kind} for `{name}`"))
}

fn present<'r>(name: &str, raw: Option<&'r str>) -> Result<&'r str, String> {
    raw.map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| format!("missing argument `{name}`"))
}
