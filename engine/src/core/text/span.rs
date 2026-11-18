use std::any::{Any, TypeId};
use std::fmt;
use std::sync::OnceLock;

use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{KeyNames, TextPiece};
use crate::core::content::Content;

/// A kind of span a feature plugs into lines of text: what it draws, and how it reads as words.
/// Each kind is registered once under the name it is written with.
pub trait SpanDef:
    erased_serde::Serialize + dyn_clone::DynClone + Equatable + fmt::Debug + Send + Sync
{
    fn pieces(&self, content: &Content) -> Vec<TextPiece>;

    /// What the span reads as in words. A span that shows a key names it only where `keys` is
    /// given to name keys by.
    fn words(&self, content: &Content, keys: Option<&KeyNames>) -> String;

    fn word_count(&self, content: &Content) -> usize {
        self.words(content, None).split_whitespace().count()
    }
}

dyn_clone::clone_trait_object!(SpanDef);
erased_serde::serialize_trait_object!(SpanDef);

/// What lets a span of one kind be compared with a span of any kind.
pub trait Equatable: Any {
    fn equals(&self, other: &dyn Any) -> bool;
}

impl<T: PartialEq + Any> Equatable for T {
    fn equals(&self, other: &dyn Any) -> bool {
        other.downcast_ref::<T>().is_some_and(|other| self == other)
    }
}

/// A span of any registered kind, as a line holds it.
#[derive(Clone)]
pub struct PluggedSpan(Box<dyn SpanDef>);

impl PluggedSpan {
    pub fn new<C: SpanDef>(span: C) -> PluggedSpan {
        PluggedSpan(Box::new(span))
    }

    /// What the span is, when it is a `C`.
    pub fn downcast_ref<C: SpanDef>(&self) -> Option<&C> {
        let held: &dyn Any = &*self.0;
        held.downcast_ref()
    }
}

impl std::ops::Deref for PluggedSpan {
    type Target = dyn SpanDef;

    fn deref(&self) -> &(dyn SpanDef + 'static) {
        &*self.0
    }
}

impl fmt::Debug for PluggedSpan {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for PluggedSpan {
    fn eq(&self, other: &PluggedSpan) -> bool {
        let other: &dyn Any = &*other.0;
        self.0.equals(other)
    }
}

impl Serialize for PluggedSpan {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let held: &dyn Any = &*self.0;
        let kind = SpanKinds::get()
            .kinds
            .iter()
            .find(|kind| kind.id == held.type_id())
            .ok_or_else(|| serde::ser::Error::custom("a span of a kind nobody registered"))?;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(kind.name, &*self.0)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for PluggedSpan {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PluggedSpan, D::Error> {
        deserializer.deserialize_map(KindVisitor)
    }
}

struct KindVisitor;

impl<'de> Visitor<'de> for KindVisitor {
    type Value = PluggedSpan;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a span, written as its kind and what it holds")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<PluggedSpan, A::Error> {
        let name: String = map
            .next_key()?
            .ok_or_else(|| de::Error::custom("a span names its kind"))?;
        let kind = SpanKinds::get()
            .kinds
            .iter()
            .find(|kind| kind.name == name)
            .ok_or_else(|| {
                de::Error::custom(format!("a span of kind `{name}`, which nobody registered"))
            })?;
        map.next_value_seed(Body(kind)).map(PluggedSpan)
    }
}

struct Body<'k>(&'k Kind);

impl<'de> DeserializeSeed<'de> for Body<'_> {
    type Value = Box<dyn SpanDef>;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Box<dyn SpanDef>, D::Error> {
        let mut erased = <dyn erased_serde::Deserializer>::erase(deserializer);
        (self.0.read)(&mut erased).map_err(de::Error::custom)
    }
}

/// Every kind of span the features of the game registered, installed once as serde, which reads
/// and writes lines, takes no context to find it in.
pub struct SpanKinds {
    kinds: Vec<Kind>,
}

struct Kind {
    name: &'static str,
    id: TypeId,
    read: fn(&mut dyn erased_serde::Deserializer) -> erased_serde::Result<Box<dyn SpanDef>>,
}

static INSTALLED: OnceLock<SpanKinds> = OnceLock::new();

impl SpanKinds {
    fn get() -> &'static SpanKinds {
        INSTALLED
            .get()
            .expect("the kinds of span are installed before a line is read or written")
    }

    /// Installs the kinds `register` makes, unless some are installed already.
    pub fn install(register: impl FnOnce(&mut SpanKinds)) -> &'static SpanKinds {
        INSTALLED.get_or_init(|| {
            let mut kinds = SpanKinds { kinds: Vec::new() };
            register(&mut kinds);
            kinds
        })
    }

    /// Registers `C`, written under `name`.
    pub fn add<C: SpanDef + DeserializeOwned>(&mut self, name: &'static str) -> &mut SpanKinds {
        assert!(
            self.kinds.iter().all(|kind| kind.name != name),
            "two kinds of span are called {name}"
        );
        self.kinds.push(Kind {
            name,
            id: TypeId::of::<C>(),
            read: |deserializer| {
                erased_serde::deserialize::<C>(deserializer)
                    .map(|span| Box::new(span) as Box<dyn SpanDef>)
            },
        });
        self
    }
}
