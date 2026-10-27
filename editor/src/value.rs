use engine::core::content;
use serde::Serialize;
use serde_json::Value;

pub fn json<T: Serialize>(value: &T) -> serde_json::Result<Value> {
    content::serial::json_of(value)
}

pub fn rgba(value: &Value) -> [u8; 4] {
    serde_json::from_value(value.clone()).unwrap_or([255; 4])
}

pub fn rgb(value: &Value) -> [u8; 3] {
    serde_json::from_value(value.clone()).unwrap_or([255; 3])
}
