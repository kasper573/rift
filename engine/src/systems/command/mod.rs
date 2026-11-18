//! Commands a player sends through the terminal. A command is a content file, named for what is
//! typed after the prefix, that says who may use it and the script it runs. The command line
//! fills the script's variables, so a command's inputs are whatever its script declares, and
//! what a player may type for each is what the variable's type takes.

pub mod complete;
pub mod cooldown;
pub mod input;
pub mod search;

use crate::core::content::fields_file;
use crate::core::script::EventDef;
use crate::core::script::RunnerSpec;
use crate::core::script::ScriptSite;
use crate::core::script::Vocabulary;
use std::collections::BTreeMap;

use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::{COMMAND_PREFIX, TerminalCommanded};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, Nearby, SchemaMark,
};
use crate::core::script::compile::compile_row;
use crate::core::script::{Arrow, Compiler, EffectDef, NEXT, ScriptFault};
use crate::core::script::{
    EffectCall, EventKind, Expr, Instances, Owner, OwnerRow, RunnerKind, ScriptDef, ScriptId,
    ScriptModule,
};
use crate::core::text::LineText;
use crate::core::time::WallClock;
use crate::core::units::Seconds;
use crate::systems::account::{AccountIdentity, RoleId};
use crate::systems::aim::{self as effects, Reaching};
use crate::systems::messages::{self, messages};
use crate::systems::player::{self, Connected, PlayedBy};
use crate::systems::requirement;
pub use cooldown::CommandCooldowns;
use input::{CommandInput, InputRefusal};

messages! {
    /// What the game says of commands.
    pub struct CommandMessages {
        /// What a player is told sending `{command}`, which is no command of theirs.
        unknown(command: Text),
        /// What a player is told sending a command without its input `{input}`: `{usage}` spells
        /// the command out.
        needs(input: Text, usage: Text),
        /// What a player is told typing `{value}` for the input `{input}` of a command, which it
        /// can't be.
        refuses(input: Text, value: Text),
        /// What a player is told typing more than a command takes: `{usage}` spells it out.
        takes(usage: Text),
        /// What a player is told of `{command}`, cooling down after it ran, for `{left}` more.
        cooling(command: Text, left: Text),
        /// What a command that is cooling down is listed with: `{usage}` spells the command out, `{time}` is how long until it may be sent again.
        ready_in(time: Text, usage: Text),
        /// What a command with a cooldown is listed with: `{usage}` spells the command out, `{time}` is how long it cools down for.
        every(usage: Text, time: Text),
        /// What a command with a cooldown is listed with while it cools: `{usage}` spells the
        /// command out, `{every}` is how long it cools down for and `{left}` how long until it
        /// may be sent again.
        every_ready_in(usage: Text, every: Text, left: Text),
        /// How far away a player is, `{meters}`, beside their name where a command takes one.
        distance(meters: Number),
        /// What a player is told sending `{command}` without what it needs, `{condition}`.
        needs_condition[command: Line, condition: Line],
        /// What the line typed is told as it reaches an input: `{input}` is the input, `{usage}` spells the command out.
        input_usage(input: Text, usage: Text),
    }
}

/// A command of the terminal: the file's name is what a player types to send it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalCommandDef {
    /// The roles that may send the command, any one of them; without any, everyone may.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleId>,
    /// What must hold of whoever sends it. A command refused for it starts no cooldown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr>,
    /// How long after it runs before the same character may send it again.
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub cooldown: Seconds,
    /// What the command does: run with whoever sent it as its subject.
    pub script: ScriptId,
    /// The script's variables the command line fills, in the order they are typed. One with a
    /// default may be left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

pub type TerminalCommandId = ContentId<TerminalCommandDef>;

impl ContentFile for TerminalCommandDef {
    const TABLE: &'static str = "command";

    fn fresh(near: &Nearby) -> Result<TerminalCommandDef, String> {
        let script = near.file_where("a script run by commands", |script: &ScriptDef| {
            script.runs_on == COMMAND_RUNNER
        })?;
        let inputs = near
            .read(script)?
            .variables
            .into_iter()
            .filter(|(_, variable)| variable.default.is_none())
            .map(|(name, _)| name)
            .collect();
        Ok(TerminalCommandDef {
            roles: Vec::new(),
            requires: None,
            cooldown: Seconds(0.0),
            script,
            inputs,
        })
    }
}

impl TerminalCommandId {
    /// The command as a player types it, its inputs in their order: `<needed>`, `[optional]`.
    pub fn usage(self, content: &Content) -> String {
        let inputs: Vec<String> = input::inputs(content, self.get(content))
            .iter()
            .map(CommandInput::usage)
            .collect();
        format!(
            "{COMMAND_PREFIX}{} {}",
            self.name(content),
            inputs.join(", ")
        )
        .trim_end()
        .to_owned()
    }

    /// The command as `/commands` lists it to `character`: its usage, and its cooldown with the
    /// time left while it cools.
    pub fn listed(self, world: &World, character: Entity) -> String {
        let content = world.resource::<Content>();
        let messages = messages::of(content);
        let cooldown = self.get(content).cooldown;
        if cooldown.is_zero() {
            return self.usage(content);
        }
        let usage = self.usage(content);
        let every = messages.time.exact(cooldown);
        match cooling(world, character, self) {
            Some(left) => messages
                .command
                .every_ready_in(usage, every, messages.time.rough(left)),
            None => messages.command.every(usage, every),
        }
    }
}

/// How long until `character` may send `command` again: nothing once they may.
pub fn cooling(world: &World, character: Entity, command: TerminalCommandId) -> Option<Seconds> {
    let now = world.resource::<WallClock>().now;
    world
        .get::<CommandCooldowns>(character)
        .and_then(|cooldowns| cooldowns.left(command, now))
}

/// Every command the connection `conn` may send, by name.
pub fn allowed(world: &World, conn: Entity) -> Vec<TerminalCommandId> {
    let content = world.resource::<Content>();
    let account = world.get::<AccountIdentity>(conn);
    let mut commands: Vec<TerminalCommandId> = content
        .ids::<TerminalCommandDef>()
        .filter(|command| {
            let roles = &command.get(content).roles;
            account.map_or(roles.is_empty(), |account| account.holds_any(roles))
        })
        .collect();
    commands.sort_by(|a, b| a.name(content).cmp(b.name(content)));
    commands
}

/// The commands under way, each owning the flows of the script it started until they end.
#[derive(Resource, Default)]
pub struct Invocations {
    sent: u32,
    running: Vec<Owner>,
}

/// Runs every command sent since the last tick, telling whoever sent one that could not run why.
pub fn run(world: &mut World) {
    let sent: Vec<TerminalCommanded> = world
        .resource_mut::<Messages<TerminalCommanded>>()
        .drain()
        .collect();
    for TerminalCommanded { conn, line } in sent {
        if let Err(refusal) = invoke(world, conn, &line) {
            let said = refusal.plain_words(world.resource::<Content>());
            bevy_terminal::reply_failure(world, conn, said);
        }
    }
}

/// Lets go of the commands whose scripts have run their course.
pub fn retire(world: &mut World) {
    let Some(mut invocations) = world.get_resource_mut::<Invocations>() else {
        return;
    };
    let running = std::mem::take(&mut invocations.running);
    let mut instances = world.resource_mut::<Instances>();
    let still: Vec<Owner> = running
        .into_iter()
        .filter(|&owner| {
            let ended = instances.may_do(owner).is_some() && instances.live_flows(owner) == 0;
            if ended {
                instances.remove_owner(owner);
            }
            !ended
        })
        .collect();
    world.resource_mut::<Invocations>().running = still;
}

fn invoke(world: &mut World, conn: Entity, line: &str) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).command;
    let time = &messages::of(&content).time;
    let typed = input::TypedCommand::of(line);
    let unknown = || LineText::from(words.unknown(typed.name));
    let caller = player::conn_player(world, conn).ok_or_else(unknown)?;
    let command = allowed(world, conn)
        .into_iter()
        .find(|command| command.name(&content) == typed.name)
        .ok_or_else(unknown)?;
    let def = command.get(&content);
    let named = format!("{COMMAND_PREFIX}{}", command.name(&content));
    let parts: Vec<&str> = typed.parts.iter().map(|part| part.trim()).collect();
    let values =
        input::bind(world, caller, &input::inputs(&content, def), &parts).map_err(|refusal| {
            let usage = command.usage(&content);
            LineText::from(match refusal {
                InputRefusal::Needs(input) => words.needs(input, usage),
                InputRefusal::Refuses { input, value } => words.refuses(input, value),
                InputRefusal::TooMany => words.takes(usage),
            })
        })?;
    if let Some(unmet) = requirement::first_unmet(world, caller, def.requires.iter()) {
        return Err(words.needs_condition(named.as_str(), unmet));
    }
    let cooling_said = |left: Seconds| words.cooling(&named, time.rough(left));
    if let Some(left) = cooling(world, caller, command) {
        return Err(cooling_said(left).into());
    }
    start(world, &content, command, caller, values)?;
    if !def.cooldown.is_zero() {
        let now = world.resource::<WallClock>().now;
        if let Some(mut cooldowns) = world.get_mut::<CommandCooldowns>(caller) {
            cooldowns.start(command, now, def.cooldown);
        }
        bevy_terminal::reply(world, conn, cooling_said(def.cooldown));
    }
    Ok(())
}

fn start(
    world: &mut World,
    content: &Content,
    command: TerminalCommandId,
    caller: Entity,
    values: BTreeMap<String, crate::core::script::VarValue>,
) -> Result<(), String> {
    let module = ScriptModule {
        script: command.get(content).script,
        values,
    };
    let row = OwnerRow {
        kind: COMMAND_RUNNER,
        row: command.raw(),
        object: None,
    };
    let procedures =
        compile_row(content, row, command.path(content), &[&module]).map_err(|errors| {
            errors
                .first()
                .map(|failed| failed.error.expected.clone())
                .unwrap_or_default()
        })?;
    let mut invocations = world.get_resource_or_init::<Invocations>();
    invocations.sent = invocations.sent.wrapping_add(1);
    let owner = Owner::Row(COMMAND_RUNNER, invocations.sent);
    invocations.running.push(owner);
    let mut instances = world.resource_mut::<Instances>();
    instances.spawn_owner(owner, procedures);
    instances.raise(
        owner,
        EventKind::of::<Invoked>(),
        Some(caller),
        Vec::new(),
        |_| true,
    );
    Ok(())
}

/// The connection of the client playing `character`.
pub fn connection(world: &World, character: Entity) -> Option<Entity> {
    let client = world.get::<PlayedBy>(character)?.client;
    world.resource::<Connected>().connection(client)
}

/// Tells an actor, in their terminal, the commands they may send.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("terminal"))]
pub struct ListCommands {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for ListCommands {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                |world, character| {
                    if let Some(conn) = connection(world, character) {
                        let lines: Vec<String> = allowed(world, conn)
                            .into_iter()
                            .map(|command| command.listed(world, character))
                            .collect();
                        bevy_terminal::reply(world, conn, lines.join("\n"));
                    }
                    Ok(())
                },
            )
        }))
    }
}

/// What the loader refuses of the game's commands: two typed alike, an input no variable of the
/// script takes or of a type no command line can fill, a variable left without a value, and a
/// script that doesn't compile once its inputs are filled.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let mut named: BTreeMap<&str, TerminalCommandId> = BTreeMap::new();
    for (command, def) in loading.rows::<TerminalCommandDef>(&content) {
        if let Some(first) = named.insert(command.name(&content), command) {
            loading.row_error(
                command,
                "",
                ContentProblem::new(
                    "command named twice",
                    format!(
                        "{} is typed as {} is",
                        command.path(&content),
                        first.path(&content)
                    ),
                    "one command for each name",
                ),
            );
        }
        let variables = &def.script.get(&content).variables;
        for (at, name) in def.inputs.iter().enumerate() {
            let pointer = format!("/inputs/{at}");
            let Some(variable) = variables.get(name) else {
                loading.row_error(
                    command,
                    &pointer,
                    ContentProblem::new(
                        "unknown input",
                        name,
                        format!("a variable of {}", def.script.path(&content)),
                    ),
                );
                continue;
            };
            if input::InputKind::of(variable).is_none() {
                loading.row_error(
                    command,
                    &pointer,
                    ContentProblem::new(
                        "untypeable input",
                        format!("{name} is a {}", variable.ty.name()),
                        "a variable of a type a command line can fill",
                    ),
                );
            }
            if def.inputs[..at].contains(name) {
                loading.row_error(
                    command,
                    &pointer,
                    ContentProblem::new("input listed twice", name, "each input once"),
                );
            }
        }
        for (name, variable) in variables {
            if variable.default.is_none() && !def.inputs.contains(name) {
                loading.row_error(
                    command,
                    "/inputs",
                    ContentProblem::new(
                        "unfilled variable",
                        format!("{name} has no default and is no input"),
                        "every variable of the script filled: by an input or by its default",
                    ),
                );
            }
        }
        let Some(values) = input::stand_ins(&content, &input::inputs(&content, def)) else {
            continue;
        };
        let module = ScriptModule {
            script: def.script,
            values,
        };
        let row = OwnerRow {
            kind: COMMAND_RUNNER,
            row: command.raw(),
            object: None,
        };
        let procedures = match compile_row(&content, row, command.path(&content), &[&module]) {
            Ok(procedures) => procedures,
            Err(errors) => {
                for failed in errors {
                    let found = failed
                        .error
                        .node
                        .unwrap_or_else(|| failed.error.owner.clone());
                    loading.row_error(
                        command,
                        "/script",
                        ContentProblem::new(failed.error.rule, found, failed.error.expected),
                    );
                }
                continue;
            }
        };
        let answers = procedures
            .iter()
            .any(|procedure| !procedure.entry(EventKind::of::<Invoked>()).is_empty());
        if !answers {
            loading.row_error(
                command,
                "/script",
                ContentProblem::new(
                    "command does nothing",
                    def.script.path(&content),
                    "a script that answers the invoked event",
                ),
            );
        }
    }
}

pub const COMMAND_RUNNER: RunnerKind = RunnerKind::new("command");

/// A player sent the command, its inputs filling the script's variables.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Invoked {}

impl EventDef for Invoked {
    fn owners(&self) -> &'static [RunnerKind] {
        &[COMMAND_RUNNER]
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .runner(
            COMMAND_RUNNER,
            RunnerSpec::at(
                |content, row| ScriptSite {
                    file: fields_file(row.id::<TerminalCommandDef>().path(content)),
                    pointer: "/modules".to_owned(),
                },
                |_, _| None,
            )
            .stateless(),
        )
        .event::<Invoked>("invoked")
        .effect::<ListCommands>("list_commands");
}
