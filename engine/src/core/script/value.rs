use bevy_ecs::entity::Entity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::geometry::Footprint;
use crate::core::math::Pos;
use crate::core::tiling::Tiles;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Num(f64),
    Text(String),
    Actor(Option<Entity>),
    Actors(Vec<Entity>),
    Point(Option<Pos<Tiles>>),
    Area(Option<Footprint>),
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Boolean,
    Integer,
    Number,
    Text,
    Actor,
    Actors,
    Point,
    Area,
}

impl ValueType {
    pub fn accepts(self, given: ValueType) -> bool {
        match (self, given) {
            (ValueType::Number, ValueType::Integer) | (ValueType::Actors, ValueType::Actor) => true,
            _ => self == given,
        }
    }

    pub fn zero(self) -> Value {
        match self {
            ValueType::Boolean => Value::Bool(false),
            ValueType::Integer => Value::Int(0),
            ValueType::Number => Value::Num(0.0),
            ValueType::Text => Value::Text(String::new()),
            ValueType::Actor => Value::Actor(None),
            ValueType::Actors => Value::Actors(Vec::new()),
            ValueType::Point => Value::Point(None),
            ValueType::Area => Value::Area(None),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ValueType::Boolean => "boolean",
            ValueType::Integer => "integer",
            ValueType::Number => "number",
            ValueType::Text => "text",
            ValueType::Actor => "actor",
            ValueType::Actors => "actors",
            ValueType::Point => "point",
            ValueType::Area => "area",
        }
    }
}

impl Value {
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Bool(_) => ValueType::Boolean,
            Value::Int(_) => ValueType::Integer,
            Value::Num(_) => ValueType::Number,
            Value::Text(_) => ValueType::Text,
            Value::Actor(_) => ValueType::Actor,
            Value::Actors(_) => ValueType::Actors,
            Value::Point(_) => ValueType::Point,
            Value::Area(_) => ValueType::Area,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Num(n) => *n != 0.0,
            Value::Text(t) => !t.is_empty(),
            Value::Actor(a) => a.is_some(),
            Value::Actors(all) => !all.is_empty(),
            Value::Point(p) => p.is_some(),
            Value::Area(a) => a.is_some(),
        }
    }

    pub fn as_int(&self) -> i64 {
        match self {
            Value::Bool(b) => i64::from(*b),
            Value::Int(i) => *i,
            Value::Num(n) => *n as i64,
            Value::Actors(all) => all.len() as i64,
            Value::Text(_) | Value::Actor(_) | Value::Point(_) | Value::Area(_) => 0,
        }
    }

    pub fn as_num(&self) -> f64 {
        match self {
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Int(i) => *i as f64,
            Value::Num(n) => *n,
            Value::Actors(all) => all.len() as f64,
            Value::Text(_) | Value::Actor(_) | Value::Point(_) | Value::Area(_) => 0.0,
        }
    }

    pub fn as_actor(&self) -> Option<Entity> {
        match self {
            Value::Actor(actor) => *actor,
            Value::Actors(all) => all.first().copied(),
            _ => None,
        }
    }

    pub fn as_actors(&self) -> Vec<Entity> {
        match self {
            Value::Actor(actor) => actor.iter().copied().collect(),
            Value::Actors(all) => all.clone(),
            _ => Vec::new(),
        }
    }

    pub fn as_point(&self) -> Option<Pos<Tiles>> {
        match self {
            Value::Point(point) => *point,
            Value::Area(area) => area.map(|area| area.at),
            _ => None,
        }
    }

    pub fn as_area(&self) -> Option<Footprint> {
        match self {
            Value::Area(area) => *area,
            _ => None,
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Num(n) => {
                let rounded = (n * 100.0).round() / 100.0;
                if rounded.fract() == 0.0 {
                    format!("{rounded:.0}")
                } else {
                    let text = format!("{rounded:.2}");
                    text.trim_end_matches('0').to_owned()
                }
            }
            Value::Text(t) => t.clone(),
            Value::Actor(a) => a.map(|a| format!("{a:?}")).unwrap_or_default(),
            Value::Actors(all) => all.len().to_string(),
            Value::Point(p) => p.map(|p| format!("{}, {}", p.x, p.y)).unwrap_or_default(),
            Value::Area(a) => a.map(|a| a.shape.describe()).unwrap_or_default(),
        }
    }

    pub fn widened(self, to: ValueType) -> Value {
        match (to, self) {
            (ValueType::Number, Value::Int(i)) => Value::Num(i as f64),
            (ValueType::Actors, Value::Actor(a)) => Value::Actors(a.into_iter().collect()),
            (_, value) => value,
        }
    }
}
