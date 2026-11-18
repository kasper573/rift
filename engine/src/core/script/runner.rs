use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use super::vocabulary::Vocabulary;
use crate::core::content::{Content, NamedContent, fields_file};

use super::{OwnerRow, ScriptSite, ScriptedRow};

/// What kind of thing owns a script, which decides the events raised for it and the effects it
/// may use. The mechanic that owns scripts declares its kind and registers it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunnerKind(&'static str);

/// What a registered kind of owner is.
#[derive(Clone, Copy)]
pub struct RunnerSpec {
    /// Whether its scripts keep state between the flows they start.
    pub keeps_state: bool,
    /// Whether a world of the game starts running its scripts as it begins, rather than when
    /// something places the owner.
    pub starts_with_world: bool,
    /// Where in the game's files the owner's scripts are written.
    pub site: fn(&Content, OwnerRow) -> ScriptSite,
    /// What the file the owner's scripts run in is called, where it has a name of its own.
    pub name: fn(&Content, OwnerRow) -> Option<String>,
}

impl RunnerSpec {
    /// An owner that keeps state, is placed in the world, and is written wherever `site` says.
    pub fn at(
        site: fn(&Content, OwnerRow) -> ScriptSite,
        name: fn(&Content, OwnerRow) -> Option<String>,
    ) -> RunnerSpec {
        RunnerSpec {
            keeps_state: true,
            starts_with_world: false,
            site,
            name,
        }
    }

    /// The owner that is a file of `Row`, which names itself.
    pub fn named_row<Row: ScriptedRow + NamedContent>() -> RunnerSpec {
        RunnerSpec::at(row_site::<Row>, |content, row| {
            Some(row.id::<Row>().get(content).shown_name().to_owned())
        })
    }

    /// The owner that is a file of `Row`, which has no name of its own.
    pub fn unnamed_row<Row: ScriptedRow>() -> RunnerSpec {
        RunnerSpec::at(row_site::<Row>, |_, _| None)
    }

    /// An owner whose scripts keep no state.
    pub fn stateless(mut self) -> RunnerSpec {
        self.keeps_state = false;
        self
    }

    /// An owner whose scripts start as a world of the game begins.
    pub fn starting_with_world(mut self) -> RunnerSpec {
        self.starts_with_world = true;
        self
    }
}

fn row_site<Row: ScriptedRow>(content: &Content, row: OwnerRow) -> ScriptSite {
    ScriptSite {
        file: fields_file(row.id::<Row>().path(content)),
        pointer: "/modules".to_owned(),
    }
}

impl RunnerKind {
    /// The owner of a script any other may run.
    pub const ANY: RunnerKind = RunnerKind("any");

    pub const fn new(name: &'static str) -> RunnerKind {
        RunnerKind(name)
    }

    pub fn name(self) -> &'static str {
        self.0
    }

    /// The kind of owner registered under `name`.
    pub fn named(name: &str) -> Option<RunnerKind> {
        Vocabulary::get().runner_named(name)
    }

    pub fn keeps_state(self) -> bool {
        Vocabulary::get().spec(self).keeps_state
    }
}

impl fmt::Debug for RunnerKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for RunnerKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for RunnerKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<RunnerKind, D::Error> {
        let name = String::deserialize(deserializer)?;
        let vocabulary = Vocabulary::get();
        vocabulary.runner_named(&name).ok_or_else(|| {
            let known: Vec<String> = vocabulary
                .runners()
                .map(|kind| format!("`{}`", kind.name()))
                .collect();
            de::Error::custom(format!(
                "unknown variant `{name}`, expected one of {}",
                known.join(", ")
            ))
        })
    }
}

impl JsonSchema for RunnerKind {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("RunnerKind")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<&str> = Vocabulary::get().runners().map(RunnerKind::name).collect();
        json_schema!({ "type": "string", "enum": names })
    }
}
