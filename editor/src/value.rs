use engine::core::content;
use serde::Serialize;
use serde_json::Value;

pub fn json<T: Serialize>(value: &T) -> serde_json::Result<Value> {
    content::serial::json_of(value)
}
