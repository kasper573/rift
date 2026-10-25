use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::math::{Pos, Rect, Size};

/// The variant a value of a tagged enum is, by the tag every tagged content type writes it with.
#[derive(Deserialize)]
pub struct TypeTag {
    #[serde(rename = "type")]
    pub tag: String,
}

impl TypeTag {
    pub fn of_json(json: &serde_json::Value) -> Option<String> {
        TypeTag::deserialize(json).ok().map(|tagged| tagged.tag)
    }

    pub fn of<T: Serialize>(value: &T) -> Option<String> {
        TypeTag::of_json(&serde_json::to_value(value).ok()?)
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("x-rect" = true))]
pub struct RectRow {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// `value` as JSON holding each number as the files print it.
// `serde_json::to_value` widens an f32 bit-exactly, so 538.33 would come out as
// 538.3300170898438 while a written file holds the shortest form; going through text keeps both
// alike.
pub fn json_of<T: Serialize>(value: &T) -> serde_json::Result<serde_json::Value> {
    serde_json::from_str(&serde_json::to_string(value)?)
}

pub fn rect<U, S: Serializer>(rect: &Rect<U>, serializer: S) -> Result<S::Ok, S::Error> {
    RectRow {
        x: rect.origin.x,
        y: rect.origin.y,
        w: rect.size.width,
        h: rect.size.height,
    }
    .serialize(serializer)
}

pub fn rect_from<'de, U, D: Deserializer<'de>>(deserializer: D) -> Result<Rect<U>, D::Error> {
    let RectRow { x, y, w, h } = RectRow::deserialize(deserializer)?;
    Ok(Rect::new(Pos::new(x, y), Size::new(w, h)))
}

pub fn opt_rect<U, S: Serializer>(
    rect: &Option<Rect<U>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match rect {
        Some(rect) => self::rect(rect, serializer),
        None => serializer.serialize_none(),
    }
}

pub fn opt_rect_from<'de, U, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Rect<U>>, D::Error> {
    rect_from(deserializer).map(Some)
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("x-pos" = true))]
pub struct PosRow {
    x: f32,
    y: f32,
}

pub fn pos<U, S: Serializer>(pos: &Pos<U>, serializer: S) -> Result<S::Ok, S::Error> {
    PosRow { x: pos.x, y: pos.y }.serialize(serializer)
}

pub fn pos_from<'de, U, D: Deserializer<'de>>(deserializer: D) -> Result<Pos<U>, D::Error> {
    let PosRow { x, y } = PosRow::deserialize(deserializer)?;
    Ok(Pos::new(x, y))
}

pub fn points<U, S: Serializer>(points: &[Pos<U>], serializer: S) -> Result<S::Ok, S::Error> {
    let rows: Vec<PosRow> = points
        .iter()
        .map(|point| PosRow {
            x: point.x,
            y: point.y,
        })
        .collect();
    rows.serialize(serializer)
}

pub fn points_from<'de, U, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Pos<U>>, D::Error> {
    let rows = Vec::<PosRow>::deserialize(deserializer)?;
    Ok(rows.into_iter().map(|row| Pos::new(row.x, row.y)).collect())
}

pub fn one() -> u32 {
    1
}

pub fn is_one(count: &u32) -> bool {
    *count == 1
}

pub fn is_zero(count: &u32) -> bool {
    *count == 0
}

pub fn yes() -> bool {
    true
}

pub fn is_true(flag: &bool) -> bool {
    *flag
}

pub fn is_false(flag: &bool) -> bool {
    !*flag
}

pub fn is_empty<T>(list: &[T]) -> bool {
    list.is_empty()
}

pub fn one_f32() -> f32 {
    1.0
}

pub fn is_one_f32(value: &f32) -> bool {
    *value == 1.0
}

pub fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

pub fn is_zero_f32(value: &f32) -> bool {
    *value == 0.0
}
