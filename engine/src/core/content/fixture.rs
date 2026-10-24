use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::VariantArray;

use super::check::RowCheck;
use super::files::{content_path, fixture_dir};
use super::fresh::Nearby;
use super::id::ContentId;
use super::locate::unescape;
use super::serial::TypeTag;
use super::table::{Content, ContentFile};

/// Content code reads by name, which every game provides at the path the fixture's declaration
/// builds: `fixtures/<FOLDER>/<variant>.<type>`, as one file or a bundle by its type's shape.
pub trait Fixture: Copy + Into<&'static str> + VariantArray + 'static {
    type Row: ContentFile;
    /// The folder under `fixtures/` of the system that declares the fixture, or `None` for
    /// `fixtures/` itself.
    const FOLDER: Option<&'static str>;
    /// Whether the fixtures are every file of their type, which content adds none of its own to.
    const WHOLE_TABLE: bool = false;

    fn name(self) -> &'static str {
        self.into()
    }

    fn path(self) -> String {
        content_path(
            &fixture_dir(Self::FOLDER),
            self.name(),
            Self::Row::TABLE,
            Self::Row::SHAPE,
        )
    }

    fn id(self, content: &Content) -> ContentId<Self::Row> {
        *content
            .built::<FixtureIds<Self>>()
            .0
            .get(self.name())
            .unwrap_or_else(|| panic!("no fixture at {} in this game", self.path()))
    }

    fn get(self, content: &Content) -> &Self::Row {
        self.id(content).get(content)
    }

    /// The parts of the fixture's file the engine reads beyond the file being there.
    fn locks(self) -> Vec<FixtureLock> {
        Vec::new()
    }

    /// What the fixture's file holds when a designer makes it where it is missing: its type's
    /// fresh file, holding whatever its locks keep.
    fn fresh(self, near: &Nearby) -> Result<Self::Row, String> {
        Self::Row::fresh(near)
    }
}

/// Where each fixture of `F` resolved at load, so reading one costs no path to build.
pub struct FixtureIds<F: Fixture>(pub HashMap<&'static str, ContentId<F::Row>>);

/// A part of a fixture's row the engine reads: content tunes its values, but never removes or
/// reshapes it, so the engine finds what it reads in every game.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FixtureLock {
    /// The list at `list` keeps an entry that carries every field of `entry` as it reads there.
    Entry { list: String, entry: Value },
    /// Every entry of the list at `list` is one of `variants`, by its `type`.
    Variants { list: String, variants: Vec<String> },
    /// The field at `field` reads `value`.
    Field { field: String, value: Value },
}

impl FixtureLock {
    /// A lock on the list at `list` keeping an entry that carries `entry`'s fields at `fields` as
    /// `entry` has them.
    pub fn entry<T: Serialize>(list: &str, entry: &T, fields: &[&str]) -> FixtureLock {
        let whole = serde_json::to_value(entry).unwrap_or_default();
        let carried = fields
            .iter()
            .filter_map(|field| {
                let value = whole.pointer(field)?.clone();
                Some((unescape(field.strip_prefix('/')?), value))
            })
            .collect();
        FixtureLock::Entry {
            list: list.to_owned(),
            entry: Value::Object(carried),
        }
    }

    /// Whether `held`, an entry of the list at `list`, is one this lock keeps.
    pub fn keeps(&self, list: &str, held: &Value) -> bool {
        matches!(self, FixtureLock::Entry { list: locked, entry } if locked == list && carries(held, entry))
    }

    /// The fields of `held`, an entry of the list at `list`, that this lock pins as they read.
    pub fn pinned(&self, list: &str, held: &Value) -> Vec<&str> {
        match self {
            FixtureLock::Entry { entry, .. } if self.keeps(list, held) => entry
                .as_object()
                .into_iter()
                .flat_map(|fields| fields.keys().map(String::as_str))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A lock holding the field at `field` to `value`.
    pub fn field<T: Serialize>(field: &str, value: &T) -> FixtureLock {
        FixtureLock::Field {
            field: field.to_owned(),
            value: serde_json::to_value(value).unwrap_or_default(),
        }
    }

    /// The value the field at `field` is held to, when this lock holds it.
    pub fn fixed(&self, field: &str) -> Option<&Value> {
        match self {
            FixtureLock::Field {
                field: locked,
                value,
            } if locked == field => Some(value),
            _ => None,
        }
    }

    /// The variants every entry of the list at `list` is held to, when this lock holds them.
    pub fn variants(&self, list: &str) -> Option<&[String]> {
        match self {
            FixtureLock::Variants {
                list: locked,
                variants,
            } if locked == list => Some(variants),
            _ => None,
        }
    }

    pub fn check(&self, row: &Value, check: &mut RowCheck) {
        match self {
            FixtureLock::Entry { list, entry } => {
                let kept = entries(row, list).any(|(_, held)| carries(held, entry));
                if !kept {
                    check.error(
                        list.as_str(),
                        "fixture entry missing",
                        "no such entry",
                        format!("an entry with {entry}"),
                    );
                }
            }
            FixtureLock::Field { field, value } => {
                let held = row.pointer(field).cloned().unwrap_or(Value::Null);
                if held != *value {
                    check.error(
                        field.as_str(),
                        "fixture field changed",
                        held,
                        format!("{value}, as the engine reads it"),
                    );
                }
            }
            FixtureLock::Variants { list, variants } => {
                for (index, held) in entries(row, list) {
                    let variant = TypeTag::of_json(held).unwrap_or_default();
                    if !variants.contains(&variant) {
                        check.error(
                            format!("{list}/{index}"),
                            "variant the engine never reads",
                            variant,
                            variants.join(" or "),
                        );
                    }
                }
            }
        }
    }
}

fn entries<'v>(row: &'v Value, list: &str) -> impl Iterator<Item = (usize, &'v Value)> {
    row.pointer(list)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
}

fn carries(held: &Value, entry: &Value) -> bool {
    entry.as_object().is_some_and(|fields| {
        fields
            .iter()
            .all(|(name, value)| held.get(name) == Some(value))
    })
}
