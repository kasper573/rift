use bevy_ecs::prelude::*;
use bevy_terminal_macros::command;

use crate::reply;
use crate::terminal::{TerminalAccess, TerminalInbox, TerminalKey};

pub struct TerminalCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub access: Option<TerminalAccess>,
    pub args: &'static [CommandArgSpec],
    pub run: fn(&mut World, &CommandCtx) -> Result<String, String>,
}

inventory::collect!(TerminalCommand);

pub struct CommandArgSpec {
    pub name: &'static str,
    pub required: bool,
}

impl TerminalCommand {
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

pub struct CommandCtx {
    pub conn: Entity,
    pub args: String,
}

impl CommandCtx {
    /// Positional arguments are comma-separated; an empty slot (`3,4,,bob`) skips an optional.
    pub fn split_args(&self) -> impl Iterator<Item = &str> {
        self.args.split(',').map(str::trim)
    }
}

/// How one positional command argument parses; `Option<T>` arguments may be omitted.
pub trait CommandArg: Sized {
    const REQUIRED: bool = true;
    fn parse(name: &str, raw: Option<&str>) -> Result<Self, String>;
}

impl CommandArg for f32 {
    fn parse(name: &str, raw: Option<&str>) -> Result<f32, String> {
        from_str("number", name, raw)
    }
}

impl CommandArg for u32 {
    fn parse(name: &str, raw: Option<&str>) -> Result<u32, String> {
        from_str("integer", name, raw)
    }
}

impl CommandArg for String {
    fn parse(name: &str, raw: Option<&str>) -> Result<String, String> {
        from_str("text", name, raw)
    }
}

impl<T: CommandArg> CommandArg for Option<T> {
    const REQUIRED: bool = false;

    fn parse(name: &str, raw: Option<&str>) -> Result<Option<T>, String> {
        match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
            None => Ok(None),
            Some(raw) => T::parse(name, Some(raw)).map(Some),
        }
    }
}

pub fn dispatch<K: TerminalKey>(world: &mut World) {
    let entries = world.resource::<TerminalInbox<K>>().0.clone();
    for entry in entries.iter() {
        let Some(rest) = entry.text.strip_prefix('/') else {
            continue;
        };
        entry.consume();
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let outcome = match run(world, entry.conn, name, args) {
            Ok(outcome) | Err(outcome) => outcome,
        };
        if !outcome.is_empty() {
            reply(world, entry.conn, entry.terminal, outcome);
        }
    }
}

pub fn require_arg<'r>(name: &str, raw: Option<&'r str>) -> Result<&'r str, String> {
    raw.map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| format!("missing argument `{name}`"))
}

fn run(world: &mut World, conn: Entity, name: &str, args: &str) -> Result<String, String> {
    let known = inventory::iter::<TerminalCommand>().find(|command| command.name == name);
    // Unknown and unauthorized commands reply identically so command existence is not leaked.
    let command = known
        .filter(|command| allowed(world, conn, command))
        .ok_or_else(|| format!("Unknown command /{name} — try /commands"))?;
    let ctx = CommandCtx {
        conn,
        args: args.to_owned(),
    };
    (command.run)(world, &ctx).map_err(|error| format!("{error}\nusage: {}", command.usage()))
}

fn allowed(world: &World, conn: Entity, command: &TerminalCommand) -> bool {
    command.access.is_none_or(|access| access(world, conn))
}

/// List the commands available to you.
#[command]
fn commands(world: &mut World, ctx: &CommandCtx) -> Result<String, String> {
    let mut lines: Vec<String> = inventory::iter::<TerminalCommand>()
        .filter(|command| allowed(world, ctx.conn, command))
        .map(|command| format!("{} — {}", command.usage(), command.description))
        .collect();
    lines.sort();
    Ok(lines.join("\n"))
}

fn from_str<T: std::str::FromStr>(kind: &str, name: &str, raw: Option<&str>) -> Result<T, String> {
    let raw = require_arg(name, raw)?;
    raw.parse()
        .map_err(|_| format!("`{raw}` is not a valid {kind} for `{name}`"))
}
