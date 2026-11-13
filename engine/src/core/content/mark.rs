//! The marks content's JSON Schema carries beside the standard keywords, telling a tool driven by
//! the schema what a value means where its type alone doesn't say: a unit, a count, a reference to
//! a file, a step a flow goes on to.

use std::sync::LazyLock;

use schemars::Schema;
use serde_json::Value;
use strum::{IntoStaticStr, VariantArray};

/// Each mark's key, in the order of the variants: the `x-` JSON Schema leaves to its users, then the
/// mark's word.
static KEYS: LazyLock<Vec<String>> = LazyLock::new(|| {
    SchemaMark::VARIANTS
        .iter()
        .map(|mark| format!("x-{}", mark.word()))
        .collect()
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum SchemaMark {
    /// The step of a flow a field goes on to.
    Arrow,
    /// A media file of the kind it holds.
    Asset,
    /// A script's assignment to a variable.
    Assignment,
    /// What a field left blank stands for, in words.
    Blank,
    /// The name of one of a skeleton's builds.
    Build,
    /// A colour.
    Color,
    /// A script's condition.
    Condition,
    /// A whole number counting something.
    Count,
    /// The times a day is divided into, which begin and end at one another.
    DayTimes,
    /// The arrow a flow takes when its step fails.
    Fails,
    /// A map whose keys are fixed: none is added, removed or renamed.
    FixedRows,
    /// The name of one of a skeleton's gestures.
    Gesture,
    /// The name of one of a skeleton's groups.
    Group,
    /// The name of one of a skeleton's joints.
    Joint,
    /// The name of one of a skeleton's moves.
    Move,
    /// A name others refer to it by.
    Name,
    /// Parameters filled by whoever plays what declares them.
    ParamFill,
    /// Parameters keyed by those the file in the sibling field it holds declares.
    ParamsOf,
    /// A place in the world.
    Place,
    /// A position on the ground.
    Pos,
    /// A rectangle on the ground.
    Rect,
    /// The name of one of a skeleton's regions.
    Region,
    /// A script's condition that reads the sky alone.
    SkyCondition,
    /// The name of one of a skeleton's slots.
    Slot,
    /// A run of styled text.
    Span,
    /// The system a choice belongs to, by the name it holds.
    System,
    /// A reference to a file of the table it holds.
    Table,
    /// The name of one of the game's times of day.
    TimeOfDay,
    /// The name of one of a skeleton's tints.
    Tint,
    /// The unit of a number, as it holds it.
    Unit,
    /// What a list is for, in words.
    Usage,
    /// The type of value a script's expression in the field gives, as JSON Schema names it.
    Value,
}

impl SchemaMark {
    /// The key the mark has in a schema.
    pub fn key(self) -> &'static str {
        &KEYS[self as usize]
    }

    /// The mark's name, as a word for it.
    pub fn word(self) -> &'static str {
        self.into()
    }

    /// The mark whose key is `key`.
    pub fn of_key(key: &str) -> Option<SchemaMark> {
        SchemaMark::VARIANTS
            .iter()
            .copied()
            .find(|mark| mark.key() == key)
    }

    /// The first mark `schema` carries.
    pub fn first_on(schema: &Value) -> Option<SchemaMark> {
        schema
            .as_object()?
            .keys()
            .find_map(|key| SchemaMark::of_key(key))
    }

    /// What `schema` holds under the mark.
    pub fn on(self, schema: &Value) -> Option<&Value> {
        schema.get(self.key())
    }

    /// Whether `schema` carries the mark.
    pub fn is_on(self, schema: &Value) -> bool {
        self.on(schema).is_some()
    }

    /// What `schema` holds under the mark, where it is words.
    pub fn text_on(self, schema: &Value) -> Option<&str> {
        self.on(schema).and_then(Value::as_str)
    }

    /// Marks `schema` with `value`.
    pub fn set(self, schema: &mut Schema, value: impl Into<Value>) {
        schema.insert(self.key().to_owned(), value.into());
    }

    /// Marks `schema`, an object, with `value`.
    pub fn put(self, schema: &mut Value, value: impl Into<Value>) {
        if let Some(object) = schema.as_object_mut() {
            object.insert(self.key().to_owned(), value.into());
        }
    }

    /// Marks a type's or a field's schema with `value`, as a derive's `transform`.
    pub fn with(self, value: impl Into<Value>) -> impl FnMut(&mut Schema) {
        let value = value.into();
        move |schema: &mut Schema| self.set(schema, value.clone())
    }
}
