use std::any::TypeId;
use std::borrow::Cow;

use schemars::{Schema, SchemaGenerator, json_schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What writing a node as a kind gives, when the node is of that kind.
type Written = Option<Result<Value, serde_json::Error>>;

/// The kinds of one sort of script node (effects, queries, events) that features registered, each
/// by the tag it is written with.
pub(super) struct Kinds<T: ?Sized + 'static> {
    kinds: Vec<Kind<T>>,
}

struct Kind<T: ?Sized + 'static> {
    name: &'static str,
    id: TypeId,
    read: Box<dyn Fn(Value) -> Result<Box<T>, serde_json::Error> + Send + Sync>,
    write: Box<dyn Fn(&T) -> Written + Send + Sync>,
    schema: fn(&mut SchemaGenerator) -> Schema,
}

impl<T: ?Sized + 'static> Default for Kinds<T> {
    fn default() -> Kinds<T> {
        Kinds { kinds: Vec::new() }
    }
}

impl<T: ?Sized + 'static> Kinds<T> {
    pub(super) fn add<C>(
        &mut self,
        name: &'static str,
        wrap: fn(C) -> Box<T>,
        unwrap: for<'a> fn(&'a T) -> Option<&'a C>,
    ) where
        C: Serialize + DeserializeOwned + schemars::JsonSchema + 'static,
    {
        assert!(
            self.kinds.iter().all(|kind| kind.name != name),
            "two script nodes are called {name}"
        );
        self.kinds.push(Kind {
            name,
            id: TypeId::of::<C>(),
            read: Box::new(move |body| serde_json::from_value::<C>(body).map(wrap)),
            write: Box::new(move |node| unwrap(node).map(serde_json::to_value)),
            schema: C::json_schema,
        });
    }

    pub(super) fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.kinds.iter().map(|kind| kind.name)
    }

    pub(super) fn id_of(&self, name: &str) -> Option<TypeId> {
        self.kinds
            .iter()
            .find(|kind| kind.name == name)
            .map(|kind| kind.id)
    }

    pub(super) fn name_of(&self, id: TypeId) -> Option<&'static str> {
        self.kinds
            .iter()
            .find(|kind| kind.id == id)
            .map(|kind| kind.name)
    }

    pub(super) fn read<E: serde::de::Error>(&self, tag: &str, body: Value) -> Result<Box<T>, E> {
        let kind = self
            .kinds
            .iter()
            .find(|kind| kind.name == tag)
            .ok_or_else(|| E::custom(self.unknown(tag)))?;
        (kind.read)(body).map_err(E::custom)
    }

    pub(super) fn write<E: serde::ser::Error>(
        &self,
        node: &T,
    ) -> Result<(&'static str, Map<String, Value>), E> {
        for kind in &self.kinds {
            let Some(written) = (kind.write)(node) else {
                continue;
            };
            return match written.map_err(E::custom)? {
                Value::Object(body) => Ok((kind.name, body)),
                other => Err(E::custom(format!(
                    "{} is written as {other}, not as an object",
                    kind.name
                ))),
            };
        }
        Err(E::custom("a script node of a kind nobody registered"))
    }

    /// The schema of every kind, each an object that also holds its tag under the field `tag`.
    pub(super) fn schema(&self, generator: &mut SchemaGenerator, tag: &'static str) -> Schema {
        let variants: Vec<Schema> = self
            .kinds
            .iter()
            .map(|kind| tagged((kind.schema)(generator), tag, kind.name))
            .collect();
        json_schema!({ "oneOf": variants })
    }

    fn unknown(&self, tag: &str) -> String {
        let names: Vec<String> = self.names().map(|name| format!("`{name}`")).collect();
        match names.as_slice() {
            [] => format!("unknown variant `{tag}`, there are no variants"),
            [one] => format!("unknown variant `{tag}`, expected {one}"),
            many => format!(
                "unknown variant `{tag}`, expected one of {}",
                many.join(", ")
            ),
        }
    }
}

/// `schema`, the object of one kind, with the tag field `field` naming `name` placed first.
fn tagged(mut schema: Schema, field: &'static str, name: &'static str) -> Schema {
    let object = schema.ensure_object();
    let mut properties = Map::new();
    properties.insert(
        field.to_owned(),
        Value::from(json_schema!({ "type": "string", "const": name })),
    );
    if let Some(Value::Object(own)) = object.remove("properties") {
        properties.extend(own);
    }
    object.insert("properties".to_owned(), Value::Object(properties));
    let mut required = vec![Value::from(field)];
    if let Some(Value::Array(own)) = object.remove("required") {
        required.extend(own);
    }
    object.insert("required".to_owned(), Value::Array(required));
    schema
}

/// How a tagged node reads and writes: its tag, and every other field beside it.
pub(super) trait Wire: Serialize + DeserializeOwned {
    /// The field the tag is written in.
    const TAG: &'static str;

    fn made(kind: &'static str, body: Map<String, Value>) -> Self;
    fn split(self) -> (String, Map<String, Value>);
}

#[derive(Serialize, Deserialize)]
pub(super) struct ByType {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    body: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ByEvent {
    #[serde(rename = "event")]
    kind: String,
    #[serde(flatten)]
    body: Map<String, Value>,
}

impl Wire for ByType {
    const TAG: &'static str = "type";

    fn made(kind: &'static str, body: Map<String, Value>) -> ByType {
        ByType {
            kind: kind.to_owned(),
            body,
        }
    }

    fn split(self) -> (String, Map<String, Value>) {
        (self.kind, self.body)
    }
}

impl Wire for ByEvent {
    const TAG: &'static str = "event";

    fn made(kind: &'static str, body: Map<String, Value>) -> ByEvent {
        ByEvent {
            kind: kind.to_owned(),
            body,
        }
    }

    fn split(self) -> (String, Map<String, Value>) {
        (self.kind, self.body)
    }
}

pub(super) fn schema_name(name: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(name)
}

/// Declares a box that holds any registered kind of one sort of script node: it clones, reads and
/// writes as the kind it holds, and describes every registered kind in its schema.
macro_rules! registered_node {
    (
        $(#[$doc:meta])*
        $wrapper:ident of $def:ident, kinds $kinds:ident, tag $wire:ident
    ) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $wrapper(Box<dyn $def>);

        impl $wrapper {
            pub fn new<C: $def>(kind: C) -> $wrapper {
                $wrapper(Box::new(kind))
            }

            /// What the box holds, when it is a `C`.
            pub fn downcast_ref<C: $def>(&self) -> Option<&C> {
                let held: &dyn std::any::Any = &*self.0;
                held.downcast_ref()
            }

            pub fn downcast_mut<C: $def>(&mut self) -> Option<&mut C> {
                let held: &mut dyn std::any::Any = &mut *self.0;
                held.downcast_mut()
            }

            pub fn is<C: $def>(&self) -> bool {
                self.downcast_ref::<C>().is_some()
            }

            /// The tag the kind held is written with.
            pub fn name(&self) -> &'static str {
                let held: &dyn std::any::Any = &*self.0;
                super::vocabulary::Vocabulary::get()
                    .$kinds
                    .name_of(held.type_id())
                    .expect("a node is of a registered kind")
            }
        }

        impl std::ops::Deref for $wrapper {
            type Target = dyn $def;

            fn deref(&self) -> &(dyn $def + 'static) {
                &*self.0
            }
        }

        impl std::ops::DerefMut for $wrapper {
            fn deref_mut(&mut self) -> &mut (dyn $def + 'static) {
                &mut *self.0
            }
        }

        impl std::fmt::Debug for $wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl serde::Serialize for $wrapper {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let (kind, body) = super::vocabulary::Vocabulary::get().$kinds.write(&*self.0)?;
                serde::Serialize::serialize(
                    &<super::kinds::$wire as super::kinds::Wire>::made(kind, body),
                    serializer,
                )
            }
        }

        impl<'de> serde::Deserialize<'de> for $wrapper {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<$wrapper, D::Error> {
                let (kind, body) = <super::kinds::$wire as serde::Deserialize>::deserialize(deserializer)
                    .map(super::kinds::Wire::split)?;
                super::vocabulary::Vocabulary::get()
                    .$kinds
                    .read(&kind, serde_json::Value::Object(body))
                    .map($wrapper)
            }
        }

        impl schemars::JsonSchema for $wrapper {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                super::kinds::schema_name(stringify!($wrapper))
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                super::vocabulary::Vocabulary::get()
                    .$kinds
                    .schema(generator, <super::kinds::$wire as super::kinds::Wire>::TAG)
            }
        }
    };
}

pub(super) use registered_node;

/// The one type a feature registered for a part of script files that has a single shape: what a
/// dialogue line says, what a choice is labelled, where a place is.
pub(super) struct Slot<T: ?Sized + 'static> {
    kind: Option<SlotKind<T>>,
}

struct SlotKind<T: ?Sized + 'static> {
    read: Box<dyn Fn(Value) -> Result<Box<T>, serde_json::Error> + Send + Sync>,
    write: Box<dyn Fn(&T) -> Written + Send + Sync>,
    schema: fn(&mut SchemaGenerator) -> Schema,
}

impl<T: ?Sized + 'static> Default for Slot<T> {
    fn default() -> Slot<T> {
        Slot { kind: None }
    }
}

impl<T: ?Sized + 'static> Slot<T> {
    pub(super) fn fill<C>(
        &mut self,
        wrap: fn(C) -> Box<T>,
        unwrap: for<'a> fn(&'a T) -> Option<&'a C>,
    ) where
        C: Serialize + DeserializeOwned + schemars::JsonSchema + 'static,
    {
        assert!(self.kind.is_none(), "a script slot is filled once");
        self.kind = Some(SlotKind {
            read: Box::new(move |body| serde_json::from_value::<C>(body).map(wrap)),
            write: Box::new(move |held| unwrap(held).map(serde_json::to_value)),
            schema: C::json_schema,
        });
    }

    fn filled(&self) -> &SlotKind<T> {
        self.kind
            .as_ref()
            .expect("every slot of the script vocabulary is filled")
    }

    pub(super) fn read<E: serde::de::Error>(&self, body: Value) -> Result<Box<T>, E> {
        (self.filled().read)(body).map_err(E::custom)
    }

    pub(super) fn write<E: serde::ser::Error>(&self, held: &T) -> Result<Value, E> {
        (self.filled().write)(held)
            .ok_or_else(|| E::custom("a slot holds the type registered for it"))?
            .map_err(E::custom)
    }

    pub(super) fn schema(&self, generator: &mut SchemaGenerator) -> Schema {
        (self.filled().schema)(generator)
    }
}

/// Declares a box that holds the one type registered in a slot: it clones, reads and writes as
/// that type.
macro_rules! registered_slot {
    (
        $(#[$doc:meta])*
        $wrapper:ident of $def:ident, slot $slot:ident
    ) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $wrapper(Box<dyn $def>);

        impl $wrapper {
            pub fn new<C: $def>(held: C) -> $wrapper {
                $wrapper(Box::new(held))
            }

            pub fn downcast_ref<C: $def>(&self) -> Option<&C> {
                let held: &dyn std::any::Any = &*self.0;
                held.downcast_ref()
            }

            pub fn downcast_mut<C: $def>(&mut self) -> Option<&mut C> {
                let held: &mut dyn std::any::Any = &mut *self.0;
                held.downcast_mut()
            }
        }

        impl std::ops::Deref for $wrapper {
            type Target = dyn $def;

            fn deref(&self) -> &(dyn $def + 'static) {
                &*self.0
            }
        }

        impl serde::Serialize for $wrapper {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                super::vocabulary::Vocabulary::get()
                    .$slot
                    .write(&*self.0)?
                    .serialize(serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $wrapper {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<$wrapper, D::Error> {
                let body = serde_json::Value::deserialize(deserializer)?;
                super::vocabulary::Vocabulary::get()
                    .$slot
                    .read(body)
                    .map($wrapper)
            }
        }

        impl schemars::JsonSchema for $wrapper {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                super::kinds::schema_name(stringify!($wrapper))
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                super::vocabulary::Vocabulary::get().$slot.schema(generator)
            }
        }
    };
}

pub(super) use registered_slot;
