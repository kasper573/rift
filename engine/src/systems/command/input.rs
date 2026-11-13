//! What a command line types into a command's inputs. An input is a variable of the command's
//! script, and its type says what may be typed for it and what the terminal suggests.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use super::TerminalCommandDef;
use crate::core::content::files::name_of;
use crate::core::content::{Content, ContentFile, NamedContent};
use crate::core::math::Pos;
use crate::core::script::VariableType;
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::Name as ActorName;
use crate::systems::area::{self, AreaDef, AreaId};
use crate::systems::class::ClassDef;
use crate::systems::item::ItemDef;
use crate::systems::memory::MemoryDef;
use crate::systems::movement::position;
use crate::systems::notification::NotificationDef;
use crate::systems::npc::NpcDef;
use crate::systems::player::Players;
use crate::systems::quest::QuestDef;
use crate::systems::script::{Place, VarValue, Variable};
use crate::systems::shop::ShopDef;
use crate::systems::skill::SkillDef;
use crate::systems::status::StatusEffectDef;

/// A command line as far as it is typed: the command's name, and once a space ends the name,
/// what follows it cut at each comma.
pub struct TypedCommand<'t> {
    pub name: &'t str,
    pub parts: Vec<&'t str>,
    /// Whether the name is still being typed.
    pub naming: bool,
}

impl<'t> TypedCommand<'t> {
    pub fn of(line: &'t str) -> TypedCommand<'t> {
        match line.split_once(char::is_whitespace) {
            Some((name, rest)) => TypedCommand {
                name,
                parts: rest.split(SEPARATOR).collect(),
                naming: false,
            },
            None => TypedCommand {
                name: line,
                parts: Vec::new(),
                naming: true,
            },
        }
    }
}

/// What parts one input from the next on a command line.
pub const SEPARATOR: char = ',';

/// One input of a command: a variable of its script.
pub struct CommandInput<'c> {
    pub name: &'c str,
    pub kind: InputKind<'c>,
    pub optional: bool,
}

/// What a command line types for an input, by the type of its variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind<'c> {
    Integer,
    Number,
    Boolean,
    Text,
    /// A file of the table, by the name it shows.
    Row(&'c str),
    /// An area by its name, then where in it: an x and a y, or neither.
    Place,
    /// A character playing here, by name.
    Character,
}

impl<'c> InputKind<'c> {
    /// What a command line types for `variable`, when it can type it at all.
    pub fn of(variable: &'c Variable) -> Option<InputKind<'c>> {
        Some(match variable.ty {
            VariableType::Integer => InputKind::Integer,
            VariableType::Number => InputKind::Number,
            VariableType::Boolean => InputKind::Boolean,
            VariableType::Text | VariableType::Name => InputKind::Text,
            VariableType::Row => InputKind::Row(variable.table.as_deref()?),
            VariableType::Place => InputKind::Place,
            VariableType::Character => InputKind::Character,
            VariableType::Audio | VariableType::Condition => return None,
        })
    }

    fn slots(self) -> &'static [SlotPart] {
        match self {
            InputKind::Place => &[SlotPart::Whole, SlotPart::X, SlotPart::Y],
            _ => &[SlotPart::Whole],
        }
    }
}

/// One part of a command line, between two separators: the input it types, and which part of it.
#[derive(Clone, Copy)]
pub struct InputSlot<'i, 'c> {
    pub input: &'i CommandInput<'c>,
    pub part: SlotPart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotPart {
    Whole,
    X,
    Y,
}

impl InputSlot<'_, '_> {
    /// What the part is called where a command's usage is spelled out.
    pub fn name(&self) -> &str {
        match self.part {
            SlotPart::Whole => self.input.name,
            SlotPart::X => "x",
            SlotPart::Y => "y",
        }
    }

    fn optional(&self) -> bool {
        self.input.optional || self.part != SlotPart::Whole
    }
}

impl CommandInput<'_> {
    /// The input as a command's usage spells it: `<needed>`, `[optional]`.
    pub fn usage(&self) -> String {
        let parts: Vec<String> = slots(std::slice::from_ref(self))
            .iter()
            .map(|slot| {
                if slot.optional() {
                    format!("[{}]", slot.name())
                } else {
                    format!("<{}>", slot.name())
                }
            })
            .collect();
        parts.join(", ")
    }
}

/// The inputs of `command` that a command line can type, in the order it types them.
pub fn inputs<'c>(content: &'c Content, command: &'c TerminalCommandDef) -> Vec<CommandInput<'c>> {
    let variables = &command.script.get(content).variables;
    command
        .inputs
        .iter()
        .filter_map(|name| {
            let variable = variables.get(name)?;
            Some(CommandInput {
                name,
                kind: InputKind::of(variable)?,
                optional: variable.default.is_some(),
            })
        })
        .collect()
}

/// The parts a command line is made of for `inputs`, in the order they are typed.
pub fn slots<'i, 'c>(inputs: &'i [CommandInput<'c>]) -> Vec<InputSlot<'i, 'c>> {
    inputs
        .iter()
        .flat_map(|input| {
            input
                .kind
                .slots()
                .iter()
                .map(move |&part| InputSlot { input, part })
        })
        .collect()
}

/// Why a command line does not fill a command's inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputRefusal {
    Needs(String),
    Refuses { input: String, value: String },
    TooMany,
}

/// The values `parts` type into `inputs` for `caller`, by the name of each input's variable. An
/// optional input left out is left to its default.
pub fn bind(
    world: &World,
    caller: Entity,
    inputs: &[CommandInput],
    parts: &[&str],
) -> Result<BTreeMap<String, VarValue>, InputRefusal> {
    let slots = slots(inputs);
    if parts.iter().skip(slots.len()).any(|part| !part.is_empty()) {
        return Err(InputRefusal::TooMany);
    }
    let typed = |at: usize| parts.get(at).copied().filter(|part| !part.is_empty());
    let mut values = BTreeMap::new();
    let mut at = 0;
    for input in inputs {
        let first = at;
        at += input.kind.slots().len();
        let Some(whole) = typed(first) else {
            if input.optional {
                continue;
            }
            return Err(InputRefusal::Needs(input.name.to_owned()));
        };
        let refuses = |value: &str| InputRefusal::Refuses {
            input: input.name.to_owned(),
            value: value.to_owned(),
        };
        let value = match input.kind {
            InputKind::Integer => VarValue::Integer(whole.parse().map_err(|_| refuses(whole))?),
            InputKind::Number => VarValue::Number(whole.parse().map_err(|_| refuses(whole))?),
            InputKind::Boolean => VarValue::Boolean(match whole.to_lowercase().as_str() {
                YES => true,
                NO => false,
                _ => return Err(refuses(whole)),
            }),
            InputKind::Text => VarValue::Str(whole.to_owned()),
            InputKind::Row(table) => VarValue::Str(
                options(world, caller, input.kind, SlotPart::Whole)
                    .into_iter()
                    .find(|option| option.text.eq_ignore_ascii_case(whole) || option.key == whole)
                    .map(|option| option.key)
                    .ok_or_else(|| refuses(whole))
                    .inspect(|_| debug_assert!(!table.is_empty()))?,
            ),
            InputKind::Character => VarValue::Str(
                options(world, caller, input.kind, SlotPart::Whole)
                    .into_iter()
                    .find(|option| option.text.eq_ignore_ascii_case(whole))
                    .map(|option| option.text)
                    .ok_or_else(|| refuses(whole))?,
            ),
            InputKind::Place => {
                let content = world.resource::<Content>();
                let area: AreaId = options(world, caller, input.kind, SlotPart::Whole)
                    .into_iter()
                    .find(|option| option.text.eq_ignore_ascii_case(whole) || option.key == whole)
                    .and_then(|option| content.by_path(&option.key))
                    .ok_or_else(|| refuses(whole))?;
                let at = match (typed(first + 1), typed(first + 2)) {
                    (None, None) => landing(content, area),
                    (Some(x), Some(y)) => Pos::new(
                        x.parse().map_err(|_| refuses(x))?,
                        y.parse().map_err(|_| refuses(y))?,
                    ),
                    (None, _) => return Err(InputRefusal::Needs("x".to_owned())),
                    (_, None) => return Err(InputRefusal::Needs("y".to_owned())),
                };
                VarValue::Place(Place { area, at })
            }
        };
        values.insert(input.name.to_owned(), value);
    }
    Ok(values)
}

/// Values of the right types for every input that needs one, by which the loader checks a
/// command's script compiles: none where a table an input picks from holds no file.
pub fn stand_ins(content: &Content, inputs: &[CommandInput]) -> Option<BTreeMap<String, VarValue>> {
    inputs
        .iter()
        .filter(|input| !input.optional)
        .map(|input| {
            let value = match input.kind {
                InputKind::Integer => VarValue::Integer(0),
                InputKind::Number => VarValue::Number(0.0),
                InputKind::Boolean => VarValue::Boolean(false),
                InputKind::Text | InputKind::Character => VarValue::Str(String::new()),
                InputKind::Row(table) => {
                    VarValue::Str(rows(content, table).into_iter().next()?.key)
                }
                InputKind::Place => VarValue::Place(content.built::<GameSettings>().initial_spawn),
            };
            Some((input.name.to_owned(), value))
        })
        .collect()
}

/// Something a part of a command line may be: what is typed for it, and what it is besides.
#[derive(Clone, Debug, PartialEq)]
pub struct InputOption {
    pub text: String,
    pub detail: String,
    /// The file the option stands for, where it stands for one.
    key: String,
}

const YES: &str = "yes";
const NO: &str = "no";

/// Everything `caller` may type for the `part` of an input of `kind`, likeliest first: nothing
/// where anything of the right shape will do.
pub fn options(world: &World, caller: Entity, kind: InputKind, part: SlotPart) -> Vec<InputOption> {
    let content = world.resource::<Content>();
    match (kind, part) {
        (InputKind::Row(table), SlotPart::Whole) => rows(content, table),
        (InputKind::Place, SlotPart::Whole) => rows(content, AreaDef::TABLE),
        (InputKind::Boolean, SlotPart::Whole) => [YES, NO]
            .map(|word| InputOption {
                text: word.to_owned(),
                detail: String::new(),
                key: String::new(),
            })
            .to_vec(),
        (InputKind::Character, SlotPart::Whole) => characters(world, caller),
        _ => Vec::new(),
    }
}

/// Where a place lands in `area` when a command line names no point of it.
fn landing(content: &Content, area: AreaId) -> Pos<Meters> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    if spawn.area == area {
        spawn.at
    } else {
        area::load(content, area).fallback_spawn()
    }
}

/// The characters playing in this world, the nearest to `caller` first.
fn characters(world: &World, caller: Entity) -> Vec<InputOption> {
    let from = position(world, caller);
    let mut near: Vec<(f32, InputOption)> = world
        .resource::<Players>()
        .0
        .values()
        .filter_map(|&character| {
            let name = world.get::<ActorName>(character)?.name.clone();
            let away = from
                .zip(position(world, character))
                .map_or(f32::MAX, |(from, at)| from.distance(at).0);
            let detail = if away == f32::MAX {
                String::new()
            } else {
                format!("{away:.0} m")
            };
            Some((
                away,
                InputOption {
                    text: name,
                    detail,
                    key: String::new(),
                },
            ))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.text.cmp(&b.1.text)));
    near.into_iter().map(|(_, option)| option).collect()
}

/// The files of `table` a script variable may name, each by the name it shows.
fn rows(content: &Content, table: &str) -> Vec<InputOption> {
    match table {
        AreaDef::TABLE => shown::<AreaDef>(content),
        ItemDef::TABLE => shown::<ItemDef>(content),
        NpcDef::TABLE => shown::<NpcDef>(content),
        QuestDef::TABLE => shown::<QuestDef>(content),
        ShopDef::TABLE => shown::<ShopDef>(content),
        MemoryDef::TABLE => shown::<MemoryDef>(content),
        ClassDef::TABLE => shown::<ClassDef>(content),
        SkillDef::TABLE => shown::<SkillDef>(content),
        StatusEffectDef::TABLE => shown::<StatusEffectDef>(content),
        NotificationDef::TABLE => content
            .ids::<NotificationDef>()
            .map(|id| InputOption {
                text: name_of(id.path(content)).to_owned(),
                detail: NotificationDef::TABLE.to_owned(),
                key: id.path(content).to_owned(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn shown<Row: NamedContent>(content: &Content) -> Vec<InputOption> {
    let mut rows: Vec<InputOption> = content
        .table::<Row>()
        .iter()
        .map(|(id, row)| InputOption {
            text: row.shown_name().to_owned(),
            detail: Row::TABLE.to_owned(),
            key: id.path(content).to_owned(),
        })
        .collect();
    rows.sort_by(|a, b| a.text.cmp(&b.text));
    rows
}
